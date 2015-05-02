import lib
import logging
import unittest

from scrapers import ScrapedStory
from score import scoreStory
from datetime import datetime;
import rfc3339

if __name__ == '__main__':
    import dev_appserver
    dev_appserver.fix_sys_path()

from google.appengine.api import memcache
from google.appengine.ext import ndb
from google.appengine.ext import testbed

__all__ = [ 'Stories' ]

"""The amount of time we cache the fact that we've attempted to store 
a given scraped story (cached by scrape source + id).
"""
CACHE_SEEN_STORY = 24 * 60 * 60

"""Current entity version"""
VERSION = 11

def scrapedStoryToDict(story):
    """Writes a scraped story to a dict"""
    out = {
        'source': story.source,
        'id': story.id,
        'tags': story.tags,
        'title': story.title,
        'index': story.index
    }
    return out

def dictToScrapedStory(dict):
    """Reads a scraped story from a dict"""
    return ScrapedStory(id=dict['id'], url=None, source=dict['source'], 
        title=dict['title'], index=dict['index'], tags=dict['tags'])

class Scrape(ndb.Expando):
    # Story URL (indexed for de-duplication across scrape sites)
    url = ndb.StringProperty(indexed=True)

    # Date created (indexed so we can sort)
    date = ndb.DateTimeProperty(indexed=True, auto_now_add=True)
    
    # Current version (indexed so we can batch upgrade without full table scans)
    version = ndb.IntegerProperty(indexed=True, default=VERSION)

    # Stemmed search terms (indexed for search)
    search = ndb.StringProperty(indexed=True, repeated=True)

    # Scraped entries, stored as compressed json blobs (unindexed)
    scraped = ndb.JsonProperty(indexed=False, compressed=False, repeated=True)

"""
The story model is returned to the caller of Stories methods.
"""
class StoryModel:
    def __init__(self, scrape):
        self._scrape = scrape
        _cachedTags = None
        _cachedScore = None
        _cachedTitle = None

    @property
    def title(self):
        """Computes the best title for the story given the scaped versions"""
        if self._cachedTitle == None:
            # Priority order for titles
            s = (scrape(Scrapers.HACKERNEWS) or scrape(Scrapers.LOBSTERS) or 
                scrape(Scrapers.REDDIT_PROG) or scrape(Scrapers.REDDIT_TECH))

            if s:
                self._cachedTitle = s.title
            else:
                self._cachedTitle = "(missing title)"

        return self._cachedTitle

    @property
    def autoBreakTitle(self):
        """Returns the title with embedded zero-width spaces"""
        return re.sub("([^\s]{7})([^\s]{3})", u"\\1\u200B\\2", self.title)

    @property
    def url(self):
        return self._scrape.url

    @property
    def hackernewsUrl(self):
        s = scrape(Scrapers.HACKERNEWS)
        if s:
            return 'http://news.ycombinator.com/item?id=%d' % s.id
        return None
    
    @property
    def redditUrl(self):
        s = scrape(Scrapers.REDDIT_PROG) or scrape(Scrapers.REDDIT_TECH)
        if s:
            return 'http://www.reddit.com/comments/%d' % s.id
        return None
    
    @property
    def lobstersUrl(self):
        s = scrape(Scrapers.LOBSTERS)
        if s:
            return 'https://lobste.rs/s/=%d' % s.id
        return None
    
    @property
    def date(self):
        return rfc3339.rfc3339(self._scrape.date)

    @property
    def datetime(self):
        return self._scrape.date

    @property
    def age(self):
        """Computes a relative date based on the current time"""
        date = self._store.date
        timespan = (datetime.now() - date)
        if timespan.days > 0:
            if timespan.days == 1:
                return "1 day ago"
            else:
                # TODO: "months" if > 30
                return "%s days ago" % timespan.days

        age = timespan.seconds
        if age < 60 * 60:
            return "recently added"
        elif age < 60 * 60 * 2:
            return "2 hours ago"
        else:
            return "%s hours ago " % (age / (60 * 60)) 

    @property
    def tags(self):
        """Computes the display (not search) tags from the scraped information"""
        if self._cachedTags == None:
            tags = Set()

            # Accumulate tags from scrapes
            for scrape in self._scrape.scraped:
                for tag in scrape.tags:
                    tags.add(tag)

            # Add keyword tags
            # TODO

            self._cachedTags = tags

        return self._cachedTags
    
    @property
    def score(self):
        if self._cachedScore == None:
            self._cachedScore = -scoreStory(self)
        return self._cachedScore
    
    @property
    def isEnglish(self):
        ascii = 0
        non_ascii = 0
        for c in self.title:
            if ' ' < c < '\x7f':
                ascii += 1 
            else:
                non_ascii += 1
                
        return ascii > (ascii + non_ascii) / 2

    def scrape(self, source):
        """Returns the given scrape for a source if it exists, otherwise None"""
        if self._cachedScrapes == None:
            scrapes = []
            for scrape in self._scrape.scraped:
                scrapes.add(dictToScrapedStory(scrape))

        return self._cachedScrapes[source] if source in self._cachedScrapes else None

    def _update_search(self):
        self._scrape.search = generate_search_field(self)

    def __cmp__(self, other):
        return cmp(self.score, other.score)

class Stories:
    # Loads a set of stories from the datastore
    def load(search=None, ignore_cache=False, force_update=True):
        scraped = Scrape.query().fetch()
        results = []
        for scrape in scraped:
            results.add(StoryModel(scrape))

        return results

    # Stores scraped stories to the datastore
    def store(self, stories):
        logging.info("Processing %d stor(ies):", len(stories))
        count = 0
        for story in stories:
            if self._store(story):
                count += 1
        logging.info("%d of %d were not seen recently according to the cache.", count, len(stories))
        pass

    def _store(self, story):
        # Have we seen this scraped story recently?
        existingKey = '%s-%s' % (story.source, story.id)
        if memcache.get(existingKey):
            story.new = False
            return False
        self._storeToDb(story)
        memcache.add(existingKey, True, CACHE_SEEN_STORY)
        return True

    def _storeToDb(self, story):
        # TODO: Should we restrict this to the last X months to allow a story to re-bubble?
        # TODO: Create a canonical version of the URL so we can fuzzy-match stories?

        existing = Scrape.query(Scrape.url == story.url).get()
        if existing:
            story.new = False
        else:
            existing = Scrape(url = story.url, scraped=[])
            story.new = True

        # Replace any existing scrapes that match this one's source
        sources = []
        replaced = False
        for scrape in existing.scraped:
            if scrape['source'] == story.source:
                existing.scraped.remove(scrape)
                replaced = True
            else:
                sources += [ scrape['source'] ]

        logging.info(" - %s:", story.url)
        logging.info("   - id=%s scrapes=%s+%s%s",
            existing.key,
            ('[' + ', '.join(sources) + ']') if sources else '[]', 
            story.source, 
            ' (replaced)' if replaced else '')

        existing.scraped += [ scrapedStoryToDict(story) ]
        existing.put()

class DemoTestCase(unittest.TestCase):
    def setUp(self):
        # First, create an instance of the Testbed class.
        self.testbed = testbed.Testbed()
        # Then activate the testbed, which prepares the service stubs for use.
        self.testbed.activate()
        # Next, declare which service stubs you want to use.
        self.testbed.init_datastore_v3_stub()
        self.testbed.init_memcache_stub()

    def test_save_one(self):
        stories = Stories()
        scrape = [
            ScrapedStory(id='1', url='http://example.com/1', title='title', source='source', index=1, tags=['a']),
            ScrapedStory(id='2', url='http://example.com/2', title='title', source='source', index=2, tags=['b']),
        ]
        stories.store(scrape)
        self.assertEquals(2, len(Scrape.query().fetch()))

    def test_save_dupe(self):
        stories = Stories()
        scrape = [
            ScrapedStory(id='1', url='http://example.com/1', title='title', source='a', index=1, tags=['a']),
        ]
        stories.store(scrape)
        scrape = [
            ScrapedStory(id='1', url='http://example.com/1', title='title', source='b', index=1, tags=['b']),
        ]
        stories.store(scrape)
        self.assertEquals(1, len(Scrape.query().fetch()))

    def tearDown(self):
        self.testbed.deactivate()

if __name__ == '__main__':
    unittest.main()

