import sys
sys.path.append('..')

import lib
from google.appengine.ext import db
import simplejson as json
import gzip
import urlnorm
import datetime
import traceback
from scrapers import ScrapedStory

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

to_import = []

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
			print "Ignoring troublesome site"
			continue

		if url.startswith('http://http://'):
			print "Warning: fixing up %s to %s" % (url, url[7:])
			url = urlnorm.norm(url[7:])
		else:
			url = urlnorm.norm(url)
		title = export['title']
		if "." in export['date']:
			date = datetime.datetime.strptime(export['date'], "%Y-%m-%d %H:%M:%S.%f")
		else:
			date = datetime.datetime.strptime(export['date'], "%Y-%m-%d %H:%M:%S")

		scraped = []
		for k, v in SERVICE_MAP.items():
			if export.has_key(k + 'Position') and export[k + 'Position'] != "None":
				index = int(export[k + 'Position'])
				id = export[k + 'Id']
				scraped.append(ScrapedStory(source=v, id=id, index=index, url=url, title=title, tags=[]))

		to_import.append((url, date, scraped))

		count += 1
	except:
		print "Failed to parse:\n%s\n%s" % (sys.exc_info()[0], line)
		print traceback.format_exc()



print "%d records processed" % count
