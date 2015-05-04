# progscrape

[progscrape.com](http://www.progscrape.com) is a scraper for Hacker News, Reddit and Lobste.rs. It contains a naive ranking/tagging engine that tries to keep a good mix of interesting stories on the front page.

The app is designed to run on Google's AppEngine, at a low-enough load to stay in the free tier. There is a fair bit of caching and we avoid interactivity where possible to make this possible.

## Android

There's also an open-source [Android app](https://github.com/mmastrac/progscrape-android). 
