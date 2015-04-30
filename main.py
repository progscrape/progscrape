import cgi
import os
import hmac
import simplejson as json
import re
import feedparser
import logging
import xml.sax.saxutils
import rfc3339
import urllib

from BeautifulSoup import *

from datetime import datetime;
from datetime import timedelta;
from urlparse import urlparse;
from sets import *;

import webapp2
import urlnorm
from tags import *

from google.appengine.api import users
from google.appengine.api import urlfetch
from google.appengine.api import memcache
from google.appengine.ext import db
from google.appengine.ext import search
from google.appengine.ext.webapp import template
from google.appengine.runtime import apiproxy_errors

# Used for the SearchableModel upgrade hack
import string
from google.appengine.ext.search import SearchableEntity

VERSION = 11

class Story(search.SearchableModel):
    __cachedGuessedTags = None
    __cachedScore = None
    
    title = db.StringProperty(indexed=False)
    url = db.StringProperty()
    # This will eventually be used for host-search optimization (maybe?)
    searchable_host = db.StringProperty()
    # This is purely to support the search field, so it is not indexed
    searchable_url = db.StringProperty(indexed=False)
    date = db.DateTimeProperty(auto_now_add=True)
    current_version = db.IntegerProperty(default=VERSION)
    
    # Scraped IDs
    redditProgId = db.StringProperty()
    redditProgPosition = db.IntegerProperty(indexed=False)
    redditTechId = db.StringProperty()
    redditTechPosition = db.IntegerProperty(indexed=False)
    hackerNewsId = db.StringProperty()
    hackerNewsPosition = db.IntegerProperty(indexed=False)
    lobstersId = db.StringProperty()
    lobstersPosition = db.IntegerProperty(indexed=False)
    
    tags = db.StringListProperty()
    
    @classmethod
    def SearchableProperties(cls):
        # Remove this old set of searchable properties (ie: w/o tags) once all the old version stories fall out
        return [['title', 'searchable_url', 'searchable_host', 'tags']]

    def rfc3339Date(self):
        return rfc3339.rfc3339(self.date)

    def isEnglish(self):
        ascii = 0
        non_ascii = 0
        for c in self.title:
            if ' ' < c < '\x7f':
                ascii += 1 
            else:
                non_ascii += 1
                
        return ascii > (ascii + non_ascii) / 2

    def scoreElements(self):
        s = {}
        
        timespan = (datetime.now() - self.date)
        if timespan.days > 0:
            s["age"] = -100 * timespan.days
        else:  
            age = timespan.seconds
            if age < 60 * 60:
                s['age'] = -10
            elif age < 60 * 60 * 2:
                s['age'] = -20
            else:
                s['age'] = -20 + (-5 * (age / (60 * 60)))
        
        s['random'] = (self.url.__hash__() % 600) / 100.0
        count = 0
        if self.redditProgPosition:
            s['reddit1'] = max(0, 30 - self.redditProgPosition)
            count += 1
        if self.redditTechPosition:
            s['reddit2'] = max(0, (30 - self.redditTechPosition) * 0.5)
            count += 1
        if self.hackerNewsPosition:
            s['hnews'] = max(0, (30 - self.hackerNewsPosition) * 1.2)
            count += 1
        if self.lobstersPosition:
            count += 1
            s['lobsters'] = max(0, (30 - self.lobstersPosition) * 1.2)

        if (self.redditProgPosition or self.redditTechPosition) and len(self.title) > 130:
            s['long_title'] = -5
        if len(self.title) > 250:
            s['really_long_title'] = -10


        # Found in more than one place: bonus
        if count > 1:
            s["multiple_service"] = 10

        return s

    def score(self):
        if self.__cachedScore:
            return self.__cachedScore
        
        elements = self.scoreElements()
#        print elements
        self.__cachedScore = -sum([elements[x] for x in elements])
        return self.__cachedScore

    def autoBreakTitle(self):
        return re.sub("([^\s]{7})([^\s]{3})", u"\\1\u200B\\2", self.title)

    def age(self):
        timespan = (datetime.now() - self.date)
        if timespan.days > 0:
            if timespan.days == 1:
                return "1 day ago"
            else:
                return "%s days ago" % timespan.days

        age = (datetime.now() - self.date).seconds
        if age < 60 * 60:
            return "recently added"
        elif age < 60 * 60 * 2:
            return "2 hours ago"
        else:
            return "%s hours ago " % (age / (60 * 60)) 

    def guessTags(self):
        if self.__cachedGuessedTags:
            return self.__cachedGuessedTags
        
        tags = list(self.tags)
        host = urlparse(self.url).netloc
        host = re.sub("^ww?w?[0-9]*\.", "", host)
        
        # Displayed tags come from the title
        tags += extractTags(self.title)
        
        normalizeTags(host, tags)

        # Uniquify and sort tags
        tags = list(Set(tags))
        tags.sort()

        # Put host at start of tags
        if host in tags:
            tags.remove(host)
        tags.insert(0, host)

        self.__cachedGuessedTags = tags
        return self.__cachedGuessedTags
        
    def updateImplicitFields(self):
        self.searchable_url = cleanUrl(self.url)
        self.searchable_host = cleanHost(self.url) 

    def updateVersion(self):
        # ensure tags
        if not self.tags:
            self.tags = []

        # fix for stop words
        host = urlparse(self.url).netloc
        normalizeTags(host, self.tags)

        # Accidentally omitted search fields for version 6
        self.updateImplicitFields()
        self.url = urlnorm.norm(self.url)
        self.current_version = VERSION

    def __cmp__(self, other):
        return cmp(self.score(), other.score())

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

def processScrapedStory(story):
    host = urlparse(story['url']).netloc
    normalizeTags(host, story['tags'])

    return story

def redditScrape(rpc):
    rawJson = json.loads(rpc.get_result().content)
    stories = []
    index = 0
    for story in rawJson['data']['children']:
        index += 1
        if story['data']['domain'].find('self.') != 0 and story['data']['score'] > 10:
            tags = []
            if story['data']['subreddit'].lower() in ['javascript', 'rust', 'golang', 'appengine', 'llvm', 'python']:
                tags += [story['data']['subreddit'].lower()]

            processed = {
                          'id': story['data']['id'],
                          'url': urlnorm.norm(story['data']['url']),
                          # HTML-escaping in JSON? WTF.
                          'title': xml.sax.saxutils.unescape(story['data']['title'].strip().replace("\n", ""), 
                            {"&apos;": "'", "&quot;": '"'}),
                          'index': index,
                          'tags': tags,
                          'new': False
                        }
            stories.append(processScrapedStory(processed))
    
    return stories

def hackerNewsScrape(rpc):
    rawHtml = BeautifulSoup(rpc.get_result().content)
    stories = []
    index = 0
    for story in rawHtml.findAll('td', {'class':'title'})[1::2]:
        index += 1
        a = story.findAll('a')
        if len(a) == 0:
            continue
        a = a[0]
        href = a['href']
        title = a.text

        infoNode = story.parent.nextSibling
        if isinstance(infoNode, NavigableString):
            infoNode = infoNode.nextSibling
        infoSpans = infoNode.findAll('span')
        if len(infoSpans) == 0:
            continue;
        scoreNode = infoSpans[0]
        id = scoreNode['id'][6:]
        score = int(scoreNode.text.split(' ')[0])

        tags = []

        if title.endswith('[pdf]'):
            title = title[:-5]
            tags += ['pdf']

        if title.endswith('[video]'):
            title = title[:-7]
            tags += ['video']

        if title.startswith('Ask HN'):
            tags += ['ask']

        if title.startswith('Show HN'):
            tags += ['show']

        if href.find('http') == 0:
             processed = {
                              'id': id,
                              'url': urlnorm.norm(href),
                              'title': title,
                              'index': index,
                              'tags': tags,
                              'new': False
                              }
             stories.append(processScrapedStory(processed))
    
    return stories

def lobstersScrape(rpc):
    d = feedparser.parse(rpc.get_result().content)
    stories = []
    index = 0
    for story in d['entries']:
        index += 1
        tags = []
        for tag in story['tags']:
            tags += [tag.term]

        processed = {
                     'id': story['id'].split('/s/')[-1],
                      'url': urlnorm.norm(story['link']),
                      'title': story['title'],
                      'index': index,
                      'tags': tags,
                      'new': False
                     } 
        stories.append(processScrapedStory(processed))
        
    return stories

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
    
def findOrCreateStory(story):
    existingKey = '%s-%s' % (story['source'], story['url'])
    if memcache.get(existingKey):
        story['new'] = False
        return None
    
    existingStory = db.GqlQuery("SELECT * FROM Story WHERE url = :1", story['url']).get()
    if existingStory is None:
        story['new'] = True
        existingStory = Story(url=story['url'],
                               title=story['title'],
                               tags=story['tags'])
    else:
        story['new'] = False

    # Merge tags
    [existingStory.tags.append(item) for item in story['tags'] if item not in existingStory.tags]
    existingStory.updateImplicitFields()
    
    # Cache that we've seen this story for a while
    memcache.add(existingKey, True, 24 * 60 * 60)
    
    return existingStory

def computeTopTags(stories):
    popular_tags = {}
    for story in stories:
        if story.redditProgPosition == 0 and story.redditTechPosition == 0 and story.hackerNewsPosition == 0:
            continue
        for tag in story.guessTags():
            if popular_tags.has_key(tag):
                popular_tags[tag] = popular_tags[tag] + 1
            else: 
                popular_tags[tag] = 1

    top_tags = [x for x in popular_tags.keys() if popular_tags[x] > 1]
    top_tags.sort(lambda x, y: popular_tags[y] - popular_tags[x])
    #top_tags = ["%s(%s)" % (x, popular_tags[x]) for x in top_tags]
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
                    print "Upgrading '%s' from %d to %d" % (story.url, story.current_version, VERSION)
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


class ScrapePageReddit(webapp2.RequestHandler):
    def get(self):
        rpc1 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc1, 
            url="http://reddit.com/r/programming+compsci+csbooks+llvm+compilers+types+systems+rust+golang+appengine+javascript+python/.json?limit=100")
        rpc2 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc2, 
            url="http://reddit.com/r/technology+science/.json")
        
        stories1 = redditScrape(rpc1)
        stories2 = redditScrape(rpc2)
        
        for story in stories1:
            story['source'] = 'reddit.programming'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.redditProgId = story['id']         
                existingStory.redditProgPosition = story['index']       
                db.put(existingStory)

        for story in stories2:
            story['source'] = 'reddit.technology'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.redditTechId = story['id']                
                existingStory.redditTechPosition = story['index']       
                db.put(existingStory)

        template_values = {
            'stories': stories1 + stories2
            }

        path = os.path.join(os.path.dirname(__file__), 'templates/scrape.html')
        self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
        self.response.out.write(template.render(path, template_values))


class ScrapePageLobsters(webapp2.RequestHandler):
    def get(self):
        rpc1 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc1, url="https://lobste.rs/rss")

        stories1 = lobstersScrape(rpc1)
        
        for story in stories1:
            story['source'] = 'lobsters'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.lobstersId = story['id']         
                existingStory.lobstersPosition = story['index']       
                db.put(existingStory)

        template_values = {
            'stories': stories1,
            }

        path = os.path.join(os.path.dirname(__file__), 'templates/scrape.html')
        self.response.headers['Content-Type'] = 'text/html; charset=utf-8';
        self.response.out.write(template.render(path, template_values))


class ScrapePageHackerNews(webapp2.RequestHandler):
    def get(self):
        rpc1 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc1, url="https://news.ycombinator.com/")
        
        stories1 = hackerNewsScrape(rpc1)
        
        for story in stories1:
            story['source'] = 'hackernews'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.hackerNewsId = story['id']                
                existingStory.hackerNewsPosition = story['index']       
                db.put(existingStory)
        
        template_values = {
            'stories': stories1,
            }

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


class CanonicalRedirectHandler(webapp2.RequestHandler):
  def get(self, path):
    self.redirect("http://www.progscrape.com%s" % (path), True)


app = webapp2.WSGIApplication(
                                     [
                                      ('/', MainPage),
                                      ('/feed', FeedPage),
                                      ('/feed.json', FeedJsonPage),
                                      ('/sitemap.xml', SitemapPage),
                                      ('/dump__internal', DumpPage),
                                      ('/clean__old__stories', CleanOldStoriesPage),
                                      ('/scrape__internal__reddit', ScrapePageReddit),
                                      ('/scrape__internal__hackernews', ScrapePageHackerNews),
                                      ('/scrape__internal__lobsters', ScrapePageLobsters),
                                      ('/scrape__test__page/(.*)', ScrapeTestPage),
                                     ],
                                     debug=True)

