# TODO

## Launch blockers

 - [ ] Slashdot tags should use URL not title
 - [ ] Dump scrapes from SQLite into GitHub as backup solution
 - [ ] Dump existing index plus old docs to backups
 - [ ] Ensure we check one shard back for clustering if not found in current shard
 - [ ] Determine why the score tuner isn't showing exactly the same results as the front page
 - [ ] Configure RPi4 for reliable hosting

## Non-blockers

 - [ ] Announcements source
 - [ ] Restore warnings to scrape test page
 - [ ] Swap Chrono for time
 - [ ] Host implies tag (ie: YouTube)
 - [ ] Rework indexing so writes don't hold a long lock 
 - [ ] Long Reddit titles should split on '|' or '.'

## Maybe

 - [ ] Metrics?

## Done

 - [X] Lobsters scraper
 - [X] Cron
 - [X] Refactor ScrapeData common stuff
 - [X] Reddit position by subreddit 
 - [X] Tagging
 - [X] Hook up search
 - [X] Comment links
 - [X] Remove www. prefix on host
 - [X] Use Instant/Duration for cron
 - [X] Hook up scrape to cron
 - [X] Score tweaking interface
 - [X] Reverse order for domain segments so we can do parent domain lookup (or use a different phrase query)
 - [X] Inverse lookup for tags
