import lib

import os
import re
import logging
import rfc3339
import urllib

from datetime import datetime
from datetime import timedelta
import simplejson as json

import webapp2
from tags import *
from scrapers import *
from stories import *

from google.appengine.api import urlfetch
from google.appengine.api import memcache
from google.appengine.ext import db
from google.appengine.ext.webapp import template

scrapers = ScraperFactory(AppEngineHttp(urlfetch))
stories = Stories()

def computeTopTags(stories):
    popular_tags = {}
    for story in stories:
        for tag in story.tags:
            if tag in popular_tags:
                popular_tags[tag] = popular_tags[tag] + 1
            else: 
                popular_tags[tag] = 1

    top_tags = [x for x in list(popular_tags.keys()) if popular_tags[x] > 1]
    top_tags.sort(lambda x, y: popular_tags[y] - popular_tags[x])

    return top_tags

class StoryPage(webapp2.RequestHandler):
    def loadStories(self, search, ignore_cache, force_update):
        return stories.load(search=search, ignore_cache=ignore_cache, force_update=force_update)

class SitemapPage(StoryPage):
    def get(self):
        stories = self.loadStories(None, False, False)
        searches = set()
        for story in stories:
            for tag in story.guessTags():
                searches.add(tag)
            
        template_values = {'now': datetime.now().strftime("%Y-%m-%d"),
                           'searches': searches
                           }
        
        path = os.path.join(os.path.dirname(__file__), 'templates/sitemap.xml')
        self.response.headers['Content-Type'] = 'text/xml; charset=utf-8';
        self.response.out.write(template.render(path, template_values))


class FeedPage(StoryPage):
    def get(self):
        search = self.request.get("search")
        agent = self.request.environ.get('HTTP_USER_AGENT', "")
        # These guys don't respect robots.txt
        if "ahrefsbot" in agent.lower():
            self.response.set_status(403)
            self.response.headers['Cache-Control'] = 'private'
            self.response.headers['Vary'] = 'User-Agent'
            self.response.out.write('You are a bad bot for not reading robots.txt often enough.\n\nAnd you should feel bad.\n')
            print(self.request.headers)
            print(self.request.environ)
            return

        stories = self.loadStories(search, False, False)
        stories = stories[:15]
        
        template_values = {
                           'search': search,
                           'stories': stories
                           }
        
        path = os.path.join(os.path.dirname(__file__), 'templates/atom.xml')
        self.response.headers['Content-Type'] = 'application/atom+xml; charset=utf-8';
        self.response.out.write(template.render(path, template_values))


class FeedJsonPage(StoryPage):
    def get(self):
        search = self.request.get("search")

        KEY = "rendered_default_feed"

        if not search:
            rendered_json = memcache.get(KEY)
            if rendered_json:
                self.response.headers['Content-Type'] = 'application/json'
                self.response.headers['X-From-Cache'] = 'true'
                self.response.headers['Cache-Control'] = 'public, max-age=120, s-maxage=120'
                self.response.headers['Pragma'] = 'Public'
                self.response.out.write(rendered_json)
                return

        stories = self.loadStories(search, True, False)
        top_tags = computeTopTags(stories)
        
        json_stories = []
        for story in stories:
            s = { 
                'title': story.title,
                'href': story.url,
                'date': story.rfc3339_date,
                'tags': story.tags
            }
            if story.redditUrl:
                s['reddit'] = story.redditUrl
            if story.hackernewsUrl:
                s['hnews'] = story.hackernewsUrl
            if story.lobstersUrl:
                s['lobsters'] = story.lobstersUrl
            if story.slashdotUrl:
                s['slashdot'] = story.slashdotUrl
            json_stories.append(s)

        feed = { 'v': 1, 'tags': top_tags, 'stories': json_stories }
        rendered_json = json.dumps(feed)
        
        if not search:
            memcache.add(KEY, rendered_json, 60*5)

        self.response.headers['Content-Type'] = 'application/json'
        self.response.headers['X-From-Cache'] = 'false'
        self.response.headers['Cache-Control'] = 'public, max-age=120, s-maxage=120'
        self.response.headers['Pragma'] = 'Public'
        self.response.out.write(rendered_json)

class MainPage(StoryPage):
    def get(self):
        # Should be checking to see if this req is from Cloudflare
        # if ".appspot.com" in self.request.environ["HTTP_HOST"]:
        #     self.redirect("http://www.progscrape.com%s" % self.request.path_qs, True)
        #     return

        # These guys don't respect robots.txt
        agent = self.request.environ.get('HTTP_USER_AGENT', "")
        if "ahrefsbot" in agent.lower():
            self.response.set_status(403)
            self.response.headers['Cache-Control'] = 'private'
            self.response.headers['Vary'] = 'User-Agent'
            self.response.out.write('You are a bad bot for not reading robots.txt often enough.\n\nAnd you should feel bad.\n')
            print(self.request.headers)
            print(self.request.environ)
            return

        FRONT_PAGE_KEY = "rendered_front_page"
        
        # Fast path: cache the front page with no query strings
        if not self.request.query_string:
            rendered_front_page = memcache.get(FRONT_PAGE_KEY)
            if rendered_front_page:
                self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
                self.response.headers['X-From-Cache'] = 'true';
                self.response.headers['Cache-Control'] = 'public, max-age=120, s-maxage=120'
                self.response.headers['Pragma'] = 'Public'
                self.response.out.write(rendered_front_page)
                return

        FETCH_COUNT = 150

        count = 30
        offset = 0
        debug = bool(self.request.get_all('show_debug_info'))
        ignore_cache = bool(self.request.get_all('ignore_cache'))
        force_update = bool(self.request.get_all('force_update'))
        cursor = self.request.get("with_cursor")

        try:
            count = min(60, max(1, int(self.request.get("count"))))
        except ValueError:
            1

        try:
            offset = max(0, int(self.request.get("offset")))
        except ValueError:
            1
            
        search = self.request.get("search")
        stories = self.loadStories(search, ignore_cache, force_update)
        top_tags = computeTopTags(stories)

        if offset < len(stories) - count:
            next = offset + count
        else:
            next = None

        stories = stories[offset:offset + count]

        template_values = {
            'stories': stories,
            'debug': debug,
            'search': search,
            'ignore_cache': ignore_cache,
            'force_update': force_update,
            'cursor': cursor,
            'top_tags': top_tags[:13],
            'next': next,
            'first': offset == 0
            }

        path = os.path.join(os.path.dirname(__file__), 'templates/index.html')
        self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
        self.response.headers['X-From-Cache'] = 'false';
        self.response.headers['Cache-Control'] = 'public, max-age=120, s-maxage=120'
        self.response.headers['Pragma'] = 'Public'
        rendered_front_page = template.render(path, template_values)
        if not self.request.query_string:
            memcache.add(FRONT_PAGE_KEY, rendered_front_page, 60*5)
        self.response.out.write(rendered_front_page)

class ScrapePage(webapp2.RequestHandler):
    def get(self):
        scraper = self.request.get("scraper")
        scraped = scrapers.scraper(scraper).scrape()

        stories.store(scraped)

        template_values = { 'stories': scraped, 'scraper': scraper }

        path = os.path.join(os.path.dirname(__file__), 'templates/scrape.html')
        self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
        self.response.out.write(template.render(path, template_values))

class ScrapeTestPage(webapp2.RequestHandler):
    def get(self, url):
        url = urllib.unquote(url)
        self.response.headers['Content-Type'] = 'text/plain; charset=utf-8';
        self.response.out.write(url)
        self.response.out.write('\n')

        rpc = urlfetch.create_rpc(deadline=10)
        urlfetch.make_fetch_call(rpc, url=url, headers={'User-Agent': 'ProgScrape (+http://progscrape.com)'})
        rpc.wait()
        
        self.response.out.write(rpc.get_result().status_code)
        self.response.out.write('\n')
        self.response.out.write(rpc.get_result().headers)
        self.response.out.write('\n')
        self.response.out.write(rpc.get_result().content)
        
class DumpPage(webapp2.RequestHandler):
    def get(self):
        stories_query = Scrape.query()
        stories, cursor, more = stories_query.fetch_page(500)
        template_values = {
            'stories': stories,
            'cursor': cursor.urlsafe()
        }

#        for story in stories:
#            if story.current_version != VERSION:
#                story.updateVersion()
#                story.put()

        path = os.path.join(os.path.dirname(__file__), 'templates/dump.html')
        self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
        self.response.out.write(template.render(path, template_values))

class CleanOldStoriesPage(webapp2.RequestHandler):
    def get(self):
        # We no longer clean stories, but we could consider it
        return

app = webapp2.WSGIApplication([
    ('/', MainPage),
    ('/feed', FeedPage),
    ('/feed.json', FeedJsonPage),
    ('/sitemap.xml', SitemapPage),
    ('/dump__internal', DumpPage),
    ('/clean__old__stories', CleanOldStoriesPage),
    ('/scrape__internal', ScrapePage),
    ('/scrape__test__page/(.*)', ScrapeTestPage),
], debug=True)

