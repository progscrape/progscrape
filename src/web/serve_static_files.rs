use lazy_static::lazy_static;

use std::sync::Arc;

use axum::{
    body::Bytes,
    http::HeaderValue,
    response::IntoResponse,
    {self},
};
use hyper::{header::*, HeaderMap, StatusCode};

use super::{static_files::StaticFileRegistry, WebError};

lazy_static! {
    /// Immutable caching header, for files that never, ever, ever change.
    pub static ref IMMUTABLE_CACHE_HEADER: HeaderValue = "public, max-age=31536000, immutable"
        .parse()
        .expect("Failed to parse header");

        /// Immutable caching header for files that are aggressively cache, but may change (rarely).
    pub static ref IMMUTABLE_CACHE_WELL_KNOWN_HEADER: HeaderValue =
        "public, max-age=86400, immutable"
            .parse()
            .expect("Failed to parse header");

            /// Our server brag header.
    pub static ref SERVER_HEADER: HeaderValue =
        "progscrape".parse().expect("Failed to parse header");
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
pub async fn immutable(
    headers_in: HeaderMap,
    key: String,
    static_files: Arc<StaticFileRegistry>,
) -> Result<impl IntoResponse, WebError> {
    let mut headers = HeaderMap::new();
    headers.append(ETAG, key.parse()?);
    headers.append(SERVER, SERVER_HEADER.clone());

    if let Some((bytes, mime)) = static_files.get_bytes_from_key(&key) {
        headers.append(CACHE_CONTROL, IMMUTABLE_CACHE_HEADER.clone());
        headers.append(CONTENT_LENGTH, bytes.len().into());
        headers.append(CONTENT_TYPE, mime.parse()?);
        if let Some(etag) = headers_in.get(IF_NONE_MATCH) {
            if *etag == key {
                return Ok(not_modified(headers));
            }
        }
        Ok(ok(bytes, headers))
    } else {
        // In the case we don't have the file, but the client has specified an ETAG that matches, we assume this is some ancient file and let them keep it.
        if let Some(etag) = headers_in.get(IF_NONE_MATCH) {
            if *etag == key {
                return Ok(not_modified(headers));
            }
        }
        Ok(not_found(&key, headers))
    }
}

/// Serve a well-known static file that may change occasionally.
pub async fn well_known(
    headers_in: HeaderMap,
    file: String,
    static_files: Arc<StaticFileRegistry>,
) -> Result<impl IntoResponse, WebError> {
    let mut headers = HeaderMap::new();
    headers.append(SERVER, SERVER_HEADER.clone());

    if let Some(key) = static_files.lookup_key(&file) {
        headers.append(ETAG, key.parse()?);

        if let Some((bytes, mime)) = static_files.get_bytes_from_key(key) {
            headers.append(CACHE_CONTROL, IMMUTABLE_CACHE_WELL_KNOWN_HEADER.clone());
            headers.append(CONTENT_LENGTH, bytes.len().into());
            headers.append(CONTENT_TYPE, mime.parse()?);
            if let Some(etag) = headers_in.get(IF_NONE_MATCH) {
                if *etag == key {
                    return Ok(not_modified(headers));
                }
            }
            Ok(ok(bytes, headers))
        } else {
            Ok(not_found(key, headers))
        }
    } else {
        Ok(not_found(&file, headers))
    }
}
