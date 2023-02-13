#[cfg(test)]
mod test {
    use axum::{body::HttpBody, http::HeaderValue, routing::IntoMakeService, Router};
    use hyper::{header::CONTENT_TYPE, service::Service, Body, Method, Request};
    use progscrape_application::StoryIndex;
    use tracing_subscriber::EnvFilter;

    use crate::{index::Index, resource::Resources, web::create_feeds};

    fn create_request(path: &'static str) -> Result<Request<Body>, Box<dyn std::error::Error>> {
        let uri = format!("http://localhost{}", path).parse()?;
        let mut req = Request::<Body>::default();
        *req.method_mut() = Method::GET;
        *req.uri_mut() = uri;
        Ok(req)
    }

    /// Given a router, send a mock request to it and check the response.
    async fn assert_response(
        router: &mut IntoMakeService<Router>,
        path: &'static str,
        mime: &'static str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut router = router.call(()).await?;
        let resp = router.call(create_request(path)?).await?;

        assert_eq!(
            resp.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_str(mime)?),
            "Incorrect mime type for path {}",
            path
        );

        let body = resp.into_body().data().await.expect("No body")?;
        let body = String::from_utf8_lossy(&body).to_string();

        Ok(body)
    }

    /// A test that tests the whole stack: populating an index from scraped data, fetching the homepage,
    /// and rendering various feeds (HTML, JSON, XML).
    #[tokio::test]
    async fn smoke_test() -> Result<(), Box<dyn std::error::Error>> {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .init();

        // Load web resources and configuration
        let resources = Resources::get_resources("../resource/")?;

        // Load sample scrapes
        let scrapes = progscrape_scrapers::load_sample_scrapes(&resources.config().scrape);

        let tempdir = tempfile::tempdir()?;

        let index = Index::<StoryIndex>::initialize_with_persistence(tempdir)?;
        index
            .insert_scrapes(resources.story_evaluator(), scrapes.into_iter())
            .await?;
        index.refresh_hot_set().await?;

        // Create a router that we can send mock requests to
        let router = create_feeds::<()>(index, resources);
        let mut router = router.into_make_service();

        // Test that we have at least 10 stories on the front page
        let s = assert_response(&mut router, "/", "text/html; charset=utf-8").await?;
        assert!(s.matches(r#"<div class="story">"#).into_iter().count() > 10);

        // Test that we have at least 10 stories + top tags in the JSON feed
        let s = assert_response(&mut router, "/feed.json", "application/json").await?;
        let value: serde_json::Value = serde_json::from_str(&s)?;
        assert_eq!(
            value.get("v").unwrap_or(&serde_json::Value::Null).as_i64(),
            Some(1)
        );
        assert!(
            value
                .get("stories")
                .unwrap_or(&serde_json::Value::Null)
                .as_array()
                .expect("Expected an array of stories")
                .len()
                > 10
        );
        assert!(
            value
                .get("tags")
                .unwrap_or(&serde_json::Value::Null)
                .as_array()
                .expect("Expected an array of top tags")
                .len()
                > 2
        );

        // Test that we have at least 10 stories in the Atom feed
        let s = assert_response(&mut router, "/feed", "application/atom+xml").await?;
        assert!(s.matches(r#"<entry>"#).into_iter().count() > 10);

        Ok(())
    }
}
