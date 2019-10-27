# progscrape

[progscrape.com](http://www.progscrape.com) is a scraper for Hacker News, Reddit, Lobste.rs, and Slashdot. It contains a naive ranking/tagging engine that tries to keep a good mix of interesting stories on the front page.

The app is designed to run on Google's AppEngine, at a low-enough load to stay in the free tier. There is a fair bit of caching and we avoid interactivity where possible to make this possible.

## Python

This web application currently runs on Python 2.7, only because Python 3.x is a big undertaking on AppEngine. Patches welcome.

## Android

There's also an open-source [Android app](https://github.com/mmastrac/progscrape-android). 
