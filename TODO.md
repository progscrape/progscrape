# TODO

## Launch blockers

 - [ ] Configure RPi4 for reliable hosting
 - [ ] Single-step release-to-deploy pipeline
 - [ ] Test "add to homepage" functionality on iOS/Android

## Non-blockers

 - [ ] Announcements source
 - [ ] Restore warnings to scrape test page
 - [ ] Swap Chrono for time
 - [ ] Host implies tag (ie: YouTube)
 - [ ] Rework indexing so writes don't hold a long lock 
 - [ ] Long Reddit titles should split on '|' or '.'O

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
 - [X] Dump scrapes from SQLite into GitHub as backup solution
 - [X] Top tags
   - [X] Include domains
 - [X] Implement feed JSON
 - [X] Determine why the score tuner isn't showing exactly the same results as the front page
 - [X] Choose the best option when multiple scrapes from Reddit are in the system
 - [X] Reddit flair tags with spaces should be skipped
 - [X] Atom feed
 - [X] Web interface validation (feed.json, frontpage + search at minimum)
 - [X] Tags w/internal representation from sites (ie: lobsters) should be reverse-lookup'd (ie: go -> golang)
 - [X] Slashdot tags should use URL not title
 - [X] Confirm that Reddit/HN/Slashdot times are actually correct
 - [X] Ensure we check one shard back for clustering if not found in current shard (should be based on scrape date, however, mostly for inactive subreddits)
 - [X] Aggressive cache headers on frontpage, feed, feed.json
 - [X] Search page is in backwards order
 - [X] Search box should update on search pages
 - [X] Feed link should update on search pages
 - [X] Hook up offset parameter
 - [X] Add social media banner image for better sharing support (test w/facebook + twitter + masto)
 - [X] Implement restore from backup-style JSON
 - [X] Dump existing index plus old docs to backups
