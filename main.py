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

import webapp2

from google.appengine.api import users
from google.appengine.api import urlfetch
from google.appengine.api import memcache
from google.appengine.ext import db
from google.appengine.ext import search
from google.appengine.ext.webapp import template

VERSION = 5
TAG_WHITELIST_LIST = [
                      # General types of story
                      'video', 'music', 'audio', 'tutorials', 'tutorial', 'media',
                      
                      # General concepts
                      'algorithm', 'algorithms', 'compiler', '3d', 'hash', 'vc', 
                      
                      # Concrete concepts
                      'drm', 'nosql', 'sql', 'copyright', 'trademark', 'patent', 'encryption', 'economy', 'investing',
                      'privacy', 'autism', 'lawsuit', 'universe', 'assemblers', 'proxy', 'censorship', 'firewall', 'trial',
                      'piracy', 'ipo', 
                      
                      # Orgs
                      'intel', 'apple', 'facebook', 'google', 'yahoo', 'microsoft', 'twitter', 'zynga',
                      'techcrunch', 'htc', 'amazon', 'mozilla', 'dell', 'nokia', 'novell', 'lenovo', 'nasa',
                      'ubuntu', 'adobe', 'github', 'cisco', 'motorola', 'samsung', 'verizon', 'sprint', 'tmobile',
                      'instagram', 'square', 'stripe', 'anonymous', 'webkit', 'opera', 'tesla', 'redhat', 'centos',
                      'gnu', 'mpaa', 'riaa', 'w3c', 'isohunt', 'obama', 'ifpi', 'nsa', 'cia', 'fbi', 'csis', 'wikileaks',
                      'snowden', 'kde', 'gnome', 'comcast', 'fcc', 'china', 'canada', 'usa',
                      
                      # Languages
                      'php', 'javascript', 'java', 'perl', 'python', 'ruby', 'html', 'html5',
                      'css', 'css2', 'css3', 'flash', 'lisp', 'clojure', 'arc', 'scala', 'lua', 
                      'haxe', 'ocaml', 'erlang', 'go', 'c',
                      
                      # Technologies
                      'linux', 'mongodb', 'cassandra', 'hadoop', 'android', 'node',
                      'iphone', 'ipad', 'ipod', 'ec2', 'firefox', 'safari', 'chrome', 'windows', 'mac', 'osx',
                      'git', 'subversion', 'mercurial', 'vi', 'emacs',
                      'bitcoin', 'drupal', 'wordpress', 'unicode', 'pdf', 'wifi', 
                      'phonegap', 'minecraft', 'mojang', 'svg', 'jpeg', 'jpg', 'gif', 'png', 'dns', 'torrent',
                    
                      # Frameworks
                      'django', 'rails', 'jquery', 'prototype', 'mootools', 'angular', 'ember'
                      ]
TAG_WHITELIST = {}

for tag in TAG_WHITELIST_LIST:
    TAG_WHITELIST[tag] = True

class Story(search.SearchableModel):
    __cachedGuessedTags = None
    __cachedScore = None
    
    title = db.StringProperty()
    url = db.StringProperty()
    searchable_host = db.StringProperty()
    searchable_url = db.StringProperty()
    date = db.DateTimeProperty(auto_now_add=True)
    current_version = db.IntegerProperty(default=VERSION)
    
    # Scraped IDs
    redditProgId = db.StringProperty()
    redditProgPosition = db.IntegerProperty()
    redditTechId = db.StringProperty()
    redditTechPosition = db.IntegerProperty()
    hackerNewsId = db.StringProperty()
    hackerNewsPosition = db.IntegerProperty()
    deliciousId = db.StringProperty()
    deliciousProgrammingPosition = db.IntegerProperty()
    deliciousDevelopmentPosition = db.IntegerProperty()
    deliciousJavascriptPosition = db.IntegerProperty()
    
    tags = db.StringListProperty()
    
    @classmethod
    def SearchableProperties(cls):
        return [['title', 'searchable_url', 'searchable_host']]

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

        if (self.redditProgPosition or self.redditTechPosition) and len(self.title) > 130:
            s['long_title'] = -5
        if len(self.title) > 250:
            s['really_log_title'] = -10

        deliciousScores = [1 for x in [self.deliciousProgrammingPosition, self.deliciousDevelopmentPosition, self.deliciousJavascriptPosition] if x]
        if len(deliciousScores) > 0:
            s['delicious'] = 1
            count += 1
            lowerTitle = self.title.lower() + ' ' + self.url.lower()
            if 'payday' in lowerTitle:
                s['spam:paydal'] = -20
            if 'cash' in lowerTitle or 'money' in lowerTitle:
                s['spam:cash'] = -20
            if 'loan' in lowerTitle or 'loans' in lowerTitle or 'lender' in lowerTitle:
                s['spam:loan'] = -20 
            if 'credit' in lowerTitle:
                s['spam:credit'] = -20

            # Short/long titles
            if len(self.title) < 20:
                s['short_title'] = -5
            if len(self.title) > 150:
                s['long_title'] = -5

            # Spammy-looking stuff
            if not re.match(".*[a-z].*", self.title):
                s["no_lowercase"] = -20
            if not re.match(".*[A-Z].*", self.title):
                s["no_uppercase"] = -20

        # Found in more than one place: bonus
        if count > 1:
            s["multiple_service"] = 10

        return s

    def score(self):
        if self.__cachedScore:
            return self.__cachedScore
        
        elements = self.scoreElements()
        print elements
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
        
        tags = []
        host = urlparse(self.url).netloc
        host = re.sub("^www[0-9]*\.", "", host)
        tags.append(host)
        if host.find("youtube.com") != -1:
            tags.append("video")
        
        tags_so_far = {}
        for bit in re.split("[^A-Za-z0-9]+", self.title.lower()):
            if TAG_WHITELIST.has_key(bit) and not tags_so_far.has_key(bit):
                tags_so_far[bit] = True
                tags.append(bit)    
        
        self.__cachedGuessedTags = tags
        return self.__cachedGuessedTags
        
    def updateVersion(self):
        self.searchable_url = cleanUrl(self.url)
        host = urlparse(self.url).netloc
        host = re.sub("^www[0-9]*\.", "", host)
        host = re.sub("\.", "", host)
        self.searchable_host = host
        self.current_version = VERSION
        # Fixup old story titles
        if self.title.find("&amp;") > -1:
            self.title = unescapeHtml(self.title)

    def __cmp__(self, other):
        return cmp(self.score(), other.score())

def redditScrape(rpc):
    rawJson = json.loads(rpc.get_result().content)
    stories = []
    index = 0
    for story in rawJson['data']['children']:
        index += 1
        if story['data']['domain'].find('self.') != 0 and story['data']['score'] > 10:
             processed = {
                              'id': story['data']['id'],
                              'url': story['data']['url'],
                              # HTML-escaping in JSON? WTF.
                              'title': xml.sax.saxutils.unescape(story['data']['title'].strip().replace("\n", ""), {"&apos;": "'", "&quot;": '"'}),
                              'index': index,
                              'new': False
                              }
             stories.append(processed)
    
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
        # Workaround for HTML parser bug
        title = title.replace("AT&T;", "AT&T")
        infoNode = story.parent.nextSibling
        infoSpans = infoNode.findAll('span')
        if len(infoSpans) == 0:
            continue;
        scoreNode = infoSpans[0]
        id = scoreNode['id'][6:]
        score = int(scoreNode.text.split(' ')[0])
        if href.find('http') == 0:
             processed = {
                              'id': id,
                              'url': href,
                              'title': title,
                              'index': index,
                              'new': False
                              }
             stories.append(processed)
    
    return stories

def deliciousScrape(rpc):
    d = feedparser.parse(rpc.get_result().content)
    stories = []
    index = 0
    for story in d['entries']:
        index += 1
        processed = {
                     'id': story['id'].split('/')[-1].split('#')[0],
                      'url': story['link'],
                      'title': story['title'],
                      'index': index,
                      'new': False
                     } 
        stories.append(processed)
        
    return stories

def cleanUrl(url):
    # Chop off protocol
    url = url[url.find("://") + 3:]
    if url.find("www.") == 0:
        url = url[4:]
    
    if url.find("?"):
        url = url[:url.find("?")]
    if url.find("#"):
        url = url[:url.find("#")]

    url = re.sub("\.[a-z]{1,4}$", '', url)
        
    return url
    
def unescapeHtml(html):
    return xml.sax.saxutils.unescape(html, {"&apos;": "'", "&quot;": '"'})

def findOrCreateStory(story):
    existingKey = '%s-%s' % (story['source'], story['url'])
    if memcache.get(existingKey):
        story['new'] = False
        return None
    
    existingStory = db.GqlQuery("SELECT * FROM Story WHERE url = :1", story['url']).get()
    if existingStory is None:
        story['new'] = True
        existingStory = Story(url=story['url'],
                               title=story['title'])
    else:
        story['new'] = False
         
    existingStory.updateVersion()
    
    # Cache that we've seen this story for a while
    memcache.add(existingKey, True, 24 * 60 * 60)
    
    return existingStory


class StoryPage(webapp2.RequestHandler):
    def postProcess(self, stories, force_update):
        # Since this is so slow, we don't want to post-process too many items
        MAX_POST_PROCESS = 10
        count = 0
        
        stories.sort()
        
        stories = [story for story in stories if story.isEnglish()]
        
        for story in stories:
            if not story.current_version or not story.current_version == VERSION or force_update:
                count = count + 1
                if count > MAX_POST_PROCESS:
                    return stories
                story.updateVersion()
                story.put()
                
        return stories

    def loadStories(self, search, ignore_cache, force_update):
        FETCH_COUNT = 150
        SEARCH_FETCH_COUNT = 50
        
        stories = []
        
        if search:
            if not ignore_cache:
                stories = memcache.get("search-" + search)
                
            if not stories:
                try:
                    stories_query = Story.all().search(re.sub("[^A-Za-z0-9]", "", search), properties=['title', 'searchable_url', 'searchable_host']).order('-date')
                    stories = stories_query.fetch(SEARCH_FETCH_COUNT)   
                    cursor = stories_query.cursor()
                except NeedIndexError:
                    stories = []
                    
                stories = self.postProcess(stories, force_update)
                        
                memcache.add("search-" + search, stories, 10 * 60)
        else:
            if not ignore_cache:
                stories = memcache.get("frontPage")
                
            if not stories:       
                stories_query = Story.all().order('-date')
                stories = stories_query.fetch(FETCH_COUNT)
                cursor = stories_query.cursor()
                
                stories = self.postProcess(stories, force_update)
                
                memcache.add("frontPage", stories, 10 * 60)

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


class MainPage(StoryPage):
    def get(self):    
        if ".appspot.com" in self.request.environ["HTTP_HOST"]:
            self.redirect("http://www.progscrape.com%s" % self.request.path_qs, True)
            return 
           
        FRONT_PAGE_KEY = "rendered_front_page.2"
        
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
        rendered_front_page = template.render(path, template_values)
        if not self.request.query_string:
            memcache.add(FRONT_PAGE_KEY, rendered_front_page, 60*5)
        self.response.out.write(rendered_front_page)


class ScrapePageReddit(webapp2.RequestHandler):
    def get(self):
        rpc1 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc1, url="http://reddit.com/r/programming+compsci+csbooks+llvm+compilers+types+systems/.json")
        rpc2 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc2, url="http://reddit.com/r/technology+science/.json")
        
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


class ScrapePageDelicious(webapp2.RequestHandler):
    def get(self):
        rpc1 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc1, url="http://feeds.delicious.com/v2/rss/popular/javascript?count=1")
        rpc2 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc2, url="http://feeds.delicious.com/v2/rss/popular/development?count=1")
        rpc3 = urlfetch.create_rpc()
        urlfetch.make_fetch_call(rpc3, url="http://feeds.delicious.com/v2/rss/popular/programming?count=1")

        stories1 = deliciousScrape(rpc1)
        stories2 = deliciousScrape(rpc2)
        stories3 = deliciousScrape(rpc3)
        
        for story in stories1:
            story['source'] = 'delicious.javascript'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.deliciousId = story['id']         
                existingStory.deliciousJavascriptPosition = story['index']       
                db.put(existingStory)

        for story in stories2:
            story['source'] = 'delicious.development'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.deliciousId = story['id']         
                existingStory.deliciousDevelopmentPosition = story['index']       
                db.put(existingStory)

        for story in stories3:
            story['source'] = 'delicious.programming'
            existingStory = findOrCreateStory(story)
            if existingStory:
                existingStory.deliciousId = story['id']         
                existingStory.deliciousProgrammingPosition = story['index']       
                db.put(existingStory)
        
        template_values = {
            'stories': stories1 + stories2 + stories3,
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
        

class Guestbook(webapp2.RequestHandler):
    def post(self):
        greeting = Greeting()

        if users.get_current_user():
            greeting.author = users.get_current_user()

        greeting.content = self.request.get('content')
        greeting.put()
        self.redirect('/')


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
                                      ('/sitemap.xml', SitemapPage),
                                      ('/dump__internal', DumpPage),
                                      ('/clean__old__stories', CleanOldStoriesPage),
                                      ('/scrape__internal__reddit', ScrapePageReddit),
                                      ('/scrape__internal__hackernews', ScrapePageHackerNews),
                                      ('/scrape__internal__delicious', ScrapePageDelicious),
                                      ('/scrape__test__page/(.*)', ScrapeTestPage),
                                     ],
                                     debug=True)

