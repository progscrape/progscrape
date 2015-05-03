import lib

import os
import re
import logging
import rfc3339
import urllib

from datetime import datetime;
from datetime import timedelta;
from urlparse import urlparse;
from sets import *;

import webapp2
import urlnorm
from tags import *
from scrapers import *
from stories import *

from google.appengine.api import urlfetch
from google.appengine.api import memcache
from google.appengine.ext import db
from google.appengine.ext import search
from google.appengine.ext.webapp import template

scrapers = ScraperFactory(AppEngineHttp(urlfetch))
stories = Stories()

def computeTopTags(stories):
    popular_tags = {}
    for story in stories:
        for tag in story.tags:
            if popular_tags.has_key(tag):
                popular_tags[tag] = popular_tags[tag] + 1
            else: 
                popular_tags[tag] = 1

    top_tags = [x for x in popular_tags.keys() if popular_tags[x] > 1]
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

        stories = self.loadStories(search, False, False)
        top_tags = computeTopTags(stories)
        
        template_values = {
                           'search': search,
                           'stories': stories,
                           'top_tags': top_tags
                           }
        
        path = os.path.join(os.path.dirname(__file__), 'templates/feed.json')
        self.response.headers['Content-Type'] = 'application/json';
        self.response.headers['Cache-Control'] = 'public, max-age=120, s-maxage=120'
        self.response.headers['Pragma'] = 'Public'
        self.response.out.write(template.render(path, template_values))

class MainPage(StoryPage):
    def get(self):    
        if ".appspot.com" in self.request.environ["HTTP_HOST"]:
            self.redirect("http://www.progscrape.com%s" % self.request.path_qs, True)
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
        stories = stories[offset:offset + count]

        template_values = {
            'stories': stories,
            'debug': debug,
            'search': search,
            'ignore_cache': ignore_cache,
            'force_update': force_update,
            'cursor': cursor,
            'top_tags': top_tags[:13],
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
        stories_query = Story.all()
        stories = stories_query.fetch(500)   
        template_values = {
            'stories': stories,
            'cursor': stories_query.cursor()
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
        cutoff = datetime.today() - timedelta(days=365)
        stories_query = Story.all().filter('date <', cutoff).order('date')
        stories_query._keys_only=True #http://code.google.com/p/googleappengine/issues/detail?id=2021
        count = 0
        while True:
            stories = stories_query.fetch(200)
            story_count = len(stories)
            if story_count == 0:
                break
            count += story_count
            db.delete(stories)
        
        template_values = { 'count': count }

        path = os.path.join(os.path.dirname(__file__), 'templates/clean.html')
        self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
        self.response.out.write(template.render(path, template_values))

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

