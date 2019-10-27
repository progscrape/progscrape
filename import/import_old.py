import sys
sys.path.append('..')

import lib
from google.appengine.ext import ndb
import simplejson as json
import gzip
import urlnorm
import traceback
from stories import Scrape
from scrapers import ScrapedStory
from datetime import datetime, timedelta

# remote_api_shell.py -s progscrape-server.appspot.com
# import import_old

SERVICE_MAP = {
	'hackerNews': 'hackernews',
	'redditProg': 'reddit.prog',
	'redditTech': 'reddit.tech',
	'lobsters': 'lobsters'
}

f = gzip.open('old.json.gz')
count = 0
recent = 0
ignored = 0

to_import = []

cutoff = datetime.strptime("2015-04-03 22:51:10", "%Y-%m-%d %H:%M:%S")
print(cutoff)

for line in f.read().split('\n'):
	if not line:
		continue
	try:
		export = json.loads(line)
		
		# Our two main fields
		url = export['url']

		# Clean up some bad UTF-8 bytes
		url = url.replace('%A9', '')
		url = url.replace('%E2??', '')

		# Some non-english hosts giving us trouble
		if "skydevelop.ir" in url or "blog.scimpr.com" in url:
			print("Ignoring troublesome site")
			continue

		if url.startswith('http://http://'):
			print("Warning: fixing up %s to %s" % (url, url[7:]))
			url = urlnorm.norm(url[7:])
		else:
			url = urlnorm.norm(url)
		title = export['title']
		if "." in export['date']:
			date = datetime.strptime(export['date'], "%Y-%m-%d %H:%M:%S.%f")
		else:
			date = datetime.strptime(export['date'], "%Y-%m-%d %H:%M:%S")

		scraped = []
		for k, v in list(SERVICE_MAP.items()):
			if k + 'Position' in export and export[k + 'Position'] != "None":
				index = int(export[k + 'Position'])
				id = export[k + 'Id']
				scraped.append(ScrapedStory(source=v, id=id, index=index, url=url, title=title, tags=[]))

		if not scraped:
			# delicious only link
			ignored += 1
		else:
			to_import.append((url, date, scraped))
			if date > cutoff:
				recent += 1

		count += 1
	except:
		print("Failed to parse:\n%s\n%s" % (sys.exc_info()[0], line))
		print(traceback.format_exc())

print("%d records processed, %d recent, %d ignored" % (count, recent, ignored))

rows = 0
inserts = 0
updates = 0
skipped = 0

batch = []

for url, date, scraped in to_import:
	rows += 1
	if rows % 1000 == 0:
		print("%d of %d" % (rows, count))
		print("%d inserts, %d updates, %d skipped" % (inserts, updates, skipped))

	# if date > cutoff:
	# 	# Recent, try to merge w/our scrapes
	# 	existing = Scrape.query(Scrape.url == story.url, Scrape.date > cutoff).get()
	# 	if existing:
	# 		added = False
	# 		for scrape in scraped:
	# 			if not story.scrape(scrape.source):
	# 				added = True
	# 				story.add_scrape(scrape)

	# 		if added:
	# 			story.put()
	# 			updates += 1

	# 		continue

	# Resume import -- need to search the first 1200 or so because of the interrupting
	if rows > 30000 and rows < 31000:
		existing = Scrape.query(ndb.AND(Scrape.url == url, Scrape.date == date)).get()
		if existing:
			skipped += 1
			continue

	if rows < 30000:
		skipped += 1
		continue

	# Old or nothing to merge, just insert
	story = Scrape()
	story.url = url
	story.version = 1
	story.date = date
	for scrape in scraped:
		story.add_scrape(scrape)
	story.put()
	inserts += 1

print("%d inserts, %d updates, %d skipped" % (inserts, updates, skipped))
print("Done.")
