TODO:

 - [ ] Announcements source
 - [ ] Restore warnings to scrape test page
 - [ ] Slashdot tags should use URL not title
 - [ ] Swap Chrono for time
 - [ ] Host implies tag (ie: YouTube)
 - [ ] Rework indexing so writes don't hold a long lock 
 - [ ] Configure RPi4 for reliable hosting
 - [ ] Dump scrapes from SQLite into GitHub as backup solution
 - [ ] Dump existing index plus old docs to backups
 - [ ] Ensure we check one shard back for clustering if not found in current shard
 - [ ] Long Reddit titles should split on '|' or '.'

Maybe:
 - [ ] Metrics?

Done:
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
