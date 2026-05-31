use std::sync::OnceLock;

use axum::{body::Bytes, http::HeaderValue, response::IntoResponse};
use hyper::{HeaderMap, StatusCode, header::*};

use crate::{static_files::StaticFileRegistry, web::WebError};

fn immutable_cache_header() -> &'static HeaderValue {
    static CELL: OnceLock<HeaderValue> = OnceLock::new();
    CELL.get_or_init(|| {
        "public, max-age=31536000, immutable"
            .parse()
            .expect("Failed to parse header")
    })
}

fn immutable_cache_well_known_header() -> &'static HeaderValue {
    static CELL: OnceLock<HeaderValue> = OnceLock::new();
    CELL.get_or_init(|| {
        "public, max-age=86400, immutable"
            .parse()
            .expect("Failed to parse header")
    })
}

fn server_header() -> &'static HeaderValue {
    static CELL: OnceLock<HeaderValue> = OnceLock::new();
    CELL.get_or_init(|| "progscrape".parse().expect("Failed to parse header"))
}

#[allow(clippy::declare_interior_mutable_const)]
const NO_RESPONSE: Bytes = Bytes::new();

type StaticResponse = (StatusCode, HeaderMap, axum::body::Bytes);

fn not_found(key: &str, headers: HeaderMap) -> StaticResponse {
    tracing::warn!("File not found: {}", key);
    (StatusCode::NOT_FOUND, headers, NO_RESPONSE)
}

fn not_modified(headers: HeaderMap) -> StaticResponse {
    (StatusCode::NOT_MODIFIED, headers, NO_RESPONSE)
}

fn ok(bytes: Bytes, headers: HeaderMap) -> StaticResponse {
    (StatusCode::OK, headers, bytes)
}

/// Serve an immutable static file with a hash name.
pub fn immutable(
    headers_in: HeaderMap,
    key: String,
    static_files: &StaticFileRegistry,
) -> Result<impl IntoResponse + use<>, WebError> {
    let mut headers = HeaderMap::new();
    headers.append(ETAG, key.parse()?);
    headers.append(SERVER, server_header().clone());

    if let Some((bytes, mime)) = static_files.get_bytes_from_key(&key) {
        headers.append(CACHE_CONTROL, immutable_cache_header().clone());
        headers.append(CONTENT_LENGTH, bytes.len().into());
        headers.append(CONTENT_TYPE, mime.parse()?);
        if let Some(etag) = headers_in.get(IF_NONE_MATCH)
            && *etag == key
        {
            return Ok(not_modified(headers));
        }
        Ok(ok(bytes, headers))
    } else {
        // In the case we don't have the file, but the client has specified an ETAG that matches, we assume this is some ancient file and let them keep it.
        if let Some(etag) = headers_in.get(IF_NONE_MATCH)
            && *etag == key
        {
            return Ok(not_modified(headers));
        }
        Ok(not_found(&key, headers))
    }
}

/// Serve a well-known static file that may change occasionally.
pub fn well_known(
    headers_in: HeaderMap,
    file: String,
    static_files: &StaticFileRegistry,
) -> Result<impl IntoResponse + use<>, WebError> {
    let mut headers = HeaderMap::new();
    headers.append(SERVER, server_header().clone());

    if let Some(key) = static_files.lookup_key(&file) {
        headers.append(ETAG, key.parse()?);

        if let Some((bytes, mime)) = static_files.get_bytes_from_key(key) {
            headers.append(CACHE_CONTROL, immutable_cache_well_known_header().clone());
            headers.append(CONTENT_LENGTH, bytes.len().into());
            headers.append(CONTENT_TYPE, mime.parse()?);
            if let Some(etag) = headers_in.get(IF_NONE_MATCH)
                && *etag == key
            {
                return Ok(not_modified(headers));
            }
            Ok(ok(bytes, headers))
        } else {
            Ok(not_found(key, headers))
        }
    } else {
        Ok(not_found(&file, headers))
    }
}
