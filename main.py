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
from google.appengine.runtime import apiproxy_errors

# Used for the SearchableModel upgrade hack
import string
from google.appengine.ext.search import SearchableEntity

scrapers = ScraperFactory(AppEngineHttp(urlfetch))
stories = Stories()


# Given a host and set of tags, adds/removes tags as needed
def normalizeTags(host, tags):
    # Normalize 'go' -> 'golang' in tags
    if 'go' in tags:
        tags.remove('go')
        tags += ['golang']

    # Normalize 'c' -> 'clanguage' in tags
    if 'c' in tags:
        tags.remove('c')
        tags += ['clanguage']

    # youtube/vimeo implicitly have a video tag
    if (host.find("youtube.com") != -1 or host.find("vimeo.com") != -1) and not ("video" in tags):
        tags += ['video']
    # msdn->microsoft
    if (host.find("msdn.com") != -1) and not ("microsoft" in tags):
        tags += ['microsoft']

def cleanHost(url):
    host = urlparse(url).netloc
    host = re.sub("^ww?w?[0-9]*\.", "", host)
    host = re.sub("\.", "", host)    
    return host

def cleanUrl(url):
    url = urlparse(url).path

    # Chop off any extension-ish looking things at the end
    url = re.sub("\.[a-z]{1,5}$", '', url)
    # Chop the url into alphanumeric segments
    url = re.sub("[^A-Za-z0-9]", " ", url)

    return url
    
def computeTopTags(stories):
    popular_tags = {}
    for story in stories:
        for tag in story.guessTags():
            if popular_tags.has_key(tag):
                popular_tags[tag] = popular_tags[tag] + 1
            else: 
                popular_tags[tag] = 1

    top_tags = [x for x in popular_tags.keys() if popular_tags[x] > 1]
    top_tags.sort(lambda x, y: popular_tags[y] - popular_tags[x])

    return top_tags

class StoryPage(webapp2.RequestHandler):
    def postProcess(self, stories, force_update):
        # Since this is so slow, we don't want to post-process too many items
        MAX_POST_PROCESS = 10
        count = 0
        
        stories.sort()
        
        stories = [story for story in stories if story.isEnglish()]
        
        try:
            for story in stories:
                if not story.current_version or not story.current_version == VERSION or force_update:
                    print "Upgrading '%s' from %d to %d" % (story.url.encode('utf-8'), story.current_version, VERSION)
                    count = count + 1
                    if count > MAX_POST_PROCESS:
                        return stories
                    story.updateVersion()
                    story.put()
        except apiproxy_errors.OverQuotaError:
            print "Uh-oh: over quota while upgrading stories"

        return stories

    def loadStories(self, search, ignore_cache, force_update):
        FETCH_COUNT = 150
        SEARCH_FETCH_COUNT = 25
        
        stories = []

        search = re.sub("[^A-Za-z0-9]", "", search)

        # This is one of AppEngine's stop words
        if search == "go":
            search = "golang";
        if search == "c":
            search = "clanguage";
        
        if search:
            if not ignore_cache:
                stories = memcache.get("search-" + search)
                
            if not stories:
                try:
                    # Modern query
                    stories_query = Story.all().search(search, properties=['title', 'searchable_url', 'searchable_host', 'tags']).order('-date')
                    stories = stories_query.fetch(SEARCH_FETCH_COUNT)
                    cursor = stories_query.cursor()
                    stories = self.postProcess(stories, force_update)

                    # Old query (can be removed in 2016 when all the old stories fall out)
                    # This is going to be a bit expensive
                    if len(stories) < SEARCH_FETCH_COUNT and (not memcache.get("updated-" + search) or force_update):
                        internal_query = list(SearchableEntity._FullTextIndex(search, re.compile('[' + re.escape(string.punctuation) + ']')))
                        print "Upgrading query '%s' (%s)" % (internal_query, search)
                        old_stories_query = db.GqlQuery("SELECT * FROM Story WHERE __searchable_text_index_title_searchable_url_searchable_host = :1", internal_query)
                        old_stories = old_stories_query.fetch(SEARCH_FETCH_COUNT)
                        if len(old_stories) > 0:
                            print "Upgrading older stories (count = %d)" % len(old_stories)
                            self.postProcess(old_stories, force_update)
                            # Now re-query for the items old and new
                            print "Old search count = %d" % len(stories)
                            stories = stories_query.fetch(SEARCH_FETCH_COUNT)
                            cursor = stories_query.cursor()
                            stories = self.postProcess(stories, force_update)
                            print "New search count = %d" % len(stories)
                        memcache.add("updated-" + search, True)
                    # End old query

                    memcache.add("search-" + search, stories, 60 * 60)
                except db.NeedIndexError, err:
                    print "Index appears to be missing %s" % err
                    stories = []

        else:
            if not ignore_cache:
                stories = memcache.get("frontPage")
                
            if not stories:
                try:
                    stories_query = Story.all().order('-date')
                    stories = stories_query.fetch(FETCH_COUNT)
                    cursor = stories_query.cursor()
                    stories = self.postProcess(stories, force_update)
                    
                    memcache.add("frontPage", stories, 10 * 60)
                    memcache.add("frontPageLast", stories, 24 * 60 * 60)
                except apiproxy_errors.OverQuotaError:
                    print "Uh-oh: we are over quota for the front page. Let's use the last-ditch results"

                    # Last ditch
                    stories = memcache.get("frontPageLast")
                    if stories == None:
                        stories = []

        return stories
    

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

app = webapp2.WSGIApplication(
                                     [
                                      ('/', MainPage),
                                      ('/feed', FeedPage),
                                      ('/feed.json', FeedJsonPage),
                                      ('/sitemap.xml', SitemapPage),
                                      ('/dump__internal', DumpPage),
                                      ('/clean__old__stories', CleanOldStoriesPage),
                                      ('/scrape__internal', ScrapePage),
                                      ('/scrape__test__page/(.*)', ScrapeTestPage),
                                     ],
                                     debug=True)

