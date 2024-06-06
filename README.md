# progscrape

[progscrape.com](http://www.progscrape.com) is a scraper for Hacker News, Reddit, Lobste.rs, and Slashdot. It contains a naive ranking/tagging engine that tries to keep a good mix of interesting stories on the front page.

## Rust

The Rust code is divided into three projects:

 * [Scrapers](scrapers/)
 * [Application](application/)
 * [Web](web/)

Documentation for each sub-project will be available at some point.

## Running

To initialize the server index:

```
SERVER_LOG="debug,tantivy=info" cargo run -- initialize --persist-path target/index --root=.
```

To load from a set of backup scrapes:

```
SERVER_LOG="debug,tantivy=info" cargo run -- initialize --persist-path target/index --root=. backup/????-??.json
```

To run the server behind a CloudFlare Access tunnel:

```
SERVER_LOG="debug,tantivy=info" cargo run -- serve --auth-header 'cf-access-authenticated-user-email'
```

To run the server completely standlone on `localhost`:

```
SERVER_LOG="debug,tantivy=info" cargo run -- serve --fixed-auth-value 'username@example.com'
```

## Historical

The app was previously designed to run on Google's AppEngine, at a low-enough load to stay in the free tier. There was a fair bit of caching and we avoid interactivity where possible to make this possible. The [last Python version](https://github.com/mmastrac/progscrape/tree/python2) currently runs on Python 2.7, only because Python 3.x was a big undertaking on AppEngine.

## Android

There's also an open-source [Android app](https://github.com/mmastrac/progscrape-android). 
