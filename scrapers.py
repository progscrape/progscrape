import lib
import urllib
import sys

import simplejson as json
import feedparser
from BeautifulSoup import *
import xml.sax.saxutils
import urlnorm

__all__ = [ 'ScraperFactory', 'AppEngineHttp', 'ScrapedStory', 'Scrapers' ]

# Programming topics we can't tag
REDDIT_PROG_NO_TAG = [ 'programming', 'compsci', 'csbooks', 'types', 'systems' ]
# Programming topics we can tag
REDDIT_PROG_TAG = [ 'compilers', 'llvm', 'rust', 'golang', 'appengine', 'javascript', 'python', 'java' ]

# Tech topics
REDDIT_TECH = [ 'technology', 'science' ]

# Reddits that have flair good enough to be tags
REDDIT_FLAIR_TAGS = [ 'science' ]

class Scrapers:
    HACKERNEWS, LOBSTERS, REDDIT_TECH, REDDIT_PROG = ['hackernews', 'lobsters', 'reddit.tech', 'reddit.prog']

class ScraperFactory:
    def __init__(self, http):
        self.http = http

    def scrapers(self):
        return [Scrapers.__getattr__(i) for i in dir(Scrapers) if not i.startswith('_')]

    def scraper(self, name):
        if name == Scrapers.HACKERNEWS:
            return self._hackernews()
        if name == Scrapers.LOBSTERS:
            return self._lobsters()
        if name == Scrapers.REDDIT_PROG:
            return self._reddit_prog()
        if name == Scrapers.REDDIT_TECH:
            return self._reddit_tech()

    def _hackernews(self):
        return HackerNewsScraper(self.http)

    def _lobsters(self):
        return LobstersScraper(self.http)

    def _reddit_prog(self):
        return RedditScraper(self.http, 'prog', REDDIT_PROG_TAG + REDDIT_PROG_NO_TAG, 100)

    def _reddit_tech(self):
        return RedditScraper(self.http, 'tech', REDDIT_TECH, 25)

class ScrapedStory:
    def __init__(self, source=None, id=None, url=None, title=None, index=None, tags=None):
        self.source = source
        self.id = id
        self.url = url
        self.title = title
        self.index = index
        self.tags = tags
        self.new = None

    @staticmethod
    def from_string(str):
        dict = eval(str)
        return ScrapedStory(source=dict['source'], id=dict['id'], url=dict['url'], 
            title=dict['title'], index=dict['index'], tags=dict['tags'])

    def to_string(self):
        return str({'source': self.source, 'id': self.id, 'url': self.url, 
            'title': self.title, 'index': self.index, 'tags': self.tags})

class Scraper:
    def __init__(self, http):
        self.http = http
        pass

    def scrape(self):
        stories = self._scrape()
        # If we've scraped the same canonical URL twice, we will just choose the first one
        urls = set()
        for story in stories:
            try:
                url = urlnorm.norm(story.url)
            except:
                # If we've scraped a bad UTF-8 character here, this might fail
                url = story.url

            if url in urls:
                stories.remove(story)
            else:
                urls.add(url)
                story.url = url
                story.title = story.title.strip()

        return stories

class HackerNewsScraper(Scraper):
    def __init__(self, http):
        Scraper.__init__(self, http)
        pass

    def _scrape(self):
        rawHtml = BeautifulSoup(self.http.fetch("https://news.ycombinator.com/"))
        stories = []
        index = 0
        for story in rawHtml.findAll('td', {'class':'title'})[1::2]:
            index += 1
            a = story.findAll('a')
            if len(a) == 0:
                continue
            a = a[0]
            href = a['href']
            title = a.text.strip()

            infoNode = story.parent.nextSibling
            if isinstance(infoNode, NavigableString):
                infoNode = infoNode.nextSibling
            infoSpans = infoNode.findAll('span')
            if len(infoSpans) == 0:
                continue
            scoreNode = infoSpans[0]
            id = scoreNode['id'][6:]
            
            # We don't use score right now
            # score = int(scoreNode.text.split(' ')[0])

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
                 stories.append(ScrapedStory(source=Scrapers.HACKERNEWS, id=id, url=href, title=title, index=index, tags=tags))
        
        return stories

class RedditScraper(Scraper):
    def __init__(self, http, category, reddits, limit):
        Scraper.__init__(self, http)
        self.category = category
        self.url = "http://reddit.com/r/%s/.json?limit=%d" % ('+'.join(reddits), limit)
        pass

    def _scrape(self):
        rawJson = json.loads(self.http.fetch(self.url))
        stories = []
        index = 0
        for story in rawJson['data']['children']:
            index += 1
            if story['data']['domain'].find('self.') != 0 and story['data']['score'] > 10:
                tags = []
                subreddit = story['data']['subreddit'].lower()
                if subreddit in REDDIT_PROG_TAG:
                    tags += [subreddit]
                if subreddit in REDDIT_FLAIR_TAGS and story['data'].has_key('link_flair_text'):
                    # Include flair if it doesn't have a space
                    flair = story['data']['link_flair_text']
                    if flair:
                        if flair.find(' ') == -1:
                            tags += [flair.lower()]

                stories.append(ScrapedStory(source='reddit.%s' % self.category, 
                    id=story['data']['id'], 
                    url=story['data']['url'], 
                    # XML in JSON
                    title=xml.sax.saxutils.unescape(story['data']['title'].strip().replace("\n", ""), 
                                {"&apos;": "'", "&quot;": '"'}), 
                    index=index, 
                    tags=tags))
        
        return stories

class LobstersScraper(Scraper):
    def __init__(self, http):
        Scraper.__init__(self, http)
        pass

    def _scrape(self):
        d = feedparser.parse(self.http.fetch("https://lobste.rs/rss"))
        stories = []
        index = 0
        for story in d['entries']:
            index += 1
            tags = []
            for tag in story['tags']:
                tags += [tag.term]

            # Skip lobste.rs meta posts
            if 'meta' in tags:
                continue

            # Remove these special tags
            if 'person' in tags:
                tags.remove('person')
            if 'programming' in tags:
                tags.remove('programming')
            if 'practices' in tags:
                tags.remove('practices')

            stories.append(ScrapedStory(source=Scrapers.LOBSTERS, id=story['id'].split('/s/')[-1], 
                url=story['link'], title=story['title'], index=index, tags=tags))
            
        return stories

class AppEngineHttp:
    def __init__(self, urlfetch):
        self.urlfetch = urlfetch

    def fetch(self, url):
        rpc = self.urlfetch.create_rpc(deadline=20)
        self.urlfetch.make_fetch_call(rpc, url=url, 
            headers={'User-Agent': 'progscrape feed fetcher (+http://progscrape.com)'})
        return rpc.get_result().content

class PythonHttp:
    def fetch(self, url):
        response = urllib.urlopen(url)
        return response.read()

if __name__ == '__main__':
    http = PythonHttp()
    factory = ScraperFactory(http)

    if len(sys.argv) == 1:
        print "Specify one of %s" % (', '.join(factory.scrapers()))
        sys.exit(1)

    stories = factory.scraper(sys.argv[1]).scrape()
    for story in stories:
        tags = ' (tags: ' + ', '.join(story.tags) + ')' if story.tags else ''
        print "%d %s%s\n%s\n" % (story.index, story.title, tags, story.url)
