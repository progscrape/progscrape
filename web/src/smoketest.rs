#[cfg(test)]
mod test {
    use std::cmp::Ordering;

    use axum::{body::HttpBody, http::HeaderValue, routing::IntoMakeService, Router};
    use hyper::{header::CONTENT_TYPE, service::Service, Body, Method, Request};
    use progscrape_application::StoryIndex;
    use progscrape_scrapers::{
        hacker_news::{HackerNewsStory}, StoryUrl,
    };
    use serde::Deserialize;
    use tracing_subscriber::EnvFilter;

    use crate::{
        index::Index,
        resource::Resources,
        web::{create_feeds, FeedStory},
    };

    fn create_request(
        path: &'static str,
        query: &'static str,
    ) -> Result<Request<Body>, Box<dyn std::error::Error>> {
        let uri = format!("http://localhost{}{}", path, query).parse()?;
        let mut req = Request::<Body>::default();
        *req.method_mut() = Method::GET;
        *req.uri_mut() = uri;
        Ok(req)
    }

    /// Given a router, send a mock request to it and check the response.
    async fn assert_response(
        router: &mut IntoMakeService<Router>,
        path: &'static str,
        query: &'static str,
        mime: &'static str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut router = router.call(()).await?;
        let resp = router.call(create_request(path, query)?).await?;

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

    #[derive(Deserialize)]
    struct Feed {
        v: i32,
        tags: Vec<String>,
        stories: Vec<FeedStory>,
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

        // Load sample scrapes, and one scrape we can use for testing search
        let mut scrapes = progscrape_scrapers::load_sample_scrapes(&resources.config().scrape);
        let date = scrapes.last().expect("No scrapes").date;
        // This should match four search terms: Cobsteme, whooperchia, buwheal, saskimplaid
        scrapes.push(progscrape_scrapers::TypedScrape::HackerNews(
            HackerNewsStory::new_with_defaults(
                "1",
                date,
                "Cobsteme whooperchia",
                StoryUrl::parse("https://buwheal.example.com/saskimplaid").expect("url"),
            ),
        ));

        let tempdir = tempfile::tempdir()?;
        let index = Index::<StoryIndex>::initialize_with_persistence(tempdir)?;
        index
            .insert_scrapes(resources.story_evaluator(), scrapes)
            .await?;
        index.refresh_hot_set().await?;

        // Create a router that we can send mock requests to
        let router = create_feeds::<()>(index, resources);
        let mut router = router.into_make_service();

        macro_rules! compare {
            ($query:expr, $count:expr, $ordering:expr, $value:expr) => {
                let count = $count;
                assert_eq!(
                    count.cmp(&$value),
                    $ordering,
                    "Got {} stories, but expected {:?} {} for query '{}'",
                    count,
                    count.cmp(&$value),
                    $value,
                    $query
                );
            };
        }

        for (query, ordering, expected) in [
            ("", Ordering::Greater, 10),
            ("?search=rust", Ordering::Greater, 2),
            ("?search=cobsteme", Ordering::Equal, 1),
            ("?search=Cobsteme", Ordering::Equal, 1),
        ] {
            // Test the front page
            let s = assert_response(&mut router, "/", query, "text/html; charset=utf-8").await?;
            compare!(
                query,
                s.matches(r#"<div class="story">"#).into_iter().count(),
                ordering,
                expected
            );

            // Test the JSON feed
            let s = assert_response(&mut router, "/feed.json", query, "application/json").await?;
            let feed: Feed = serde_json::from_str(&s)?;
            assert_eq!(feed.v, 1);
            assert!(feed.tags.len() > 2);
            compare!(query, feed.stories.len(), ordering, expected);

            // Test the Atom feed
            let s = assert_response(&mut router, "/feed", query, "application/atom+xml").await?;
            compare!(
                query,
                s.matches(r#"<entry>"#).into_iter().count(),
                ordering,
                expected
            );
        }

        Ok(())
    }
}
