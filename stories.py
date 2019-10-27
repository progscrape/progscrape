import lib
import logging
import unittest
import re

from scrapers import ScrapedStory, Scrapers
from search import generate_search_field, generate_search_tokens_for_query
from score import scoreStory
from datetime import datetime, timedelta
import rfc3339

if __name__ == '__main__':
    import dev_appserver
    dev_appserver.fix_sys_path()

from google.appengine.api import memcache
from google.appengine.ext import ndb
from google.appengine.ext import db
from google.appengine.ext import testbed
from google.appengine.runtime import apiproxy_errors

__all__ = [ 'Stories', 'Scrape' ]


FETCH_COUNT = 150

SEARCH_FETCH_COUNT = 25

"""The amount of time we cache the fact that we've attempted to store 
a given scraped story (cached by scrape source + id).
"""
CACHE_SEEN_STORY = 24 * 60 * 60

"""Current entity version
"""
VERSION = 1

def scraped_story_to_dict(story):
    """Writes a scraped story to a dict"""
    out = {
        'source': story.source,
        'id': story.id,
        'tags': story.tags,
        'title': story.title,
        'index': story.index,
        'subcategory': story.subcategory
    }
    return out

def dict_to_scraped_story(dict):
    """Reads a scraped story from a dict"""

    # Backwards compat with old storage
    if not 'subcategory' in dict:
        dict['subcategory'] = None

    return ScrapedStory(id=dict['id'], url=None, source=dict['source'], 
        title=dict['title'], index=dict['index'], tags=dict['tags'], subcategory=dict['subcategory'])

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

    # Story source sub-category (eg: subreddit)
    subcategory = ndb.StringProperty(indexed=True)

    # Some cached computed properties
    _cachedTags = None
    _cachedScore = None
    _cachedTitle = None
    _cachedTitles = None
    _cachedScrapes = None
    _cachedSearchResults = None

    @property
    def title(self):
        """Computes the best title for the story given the scaped versions"""
        if self._cachedTitle == None:
            # Priority order for titles
            s = (self.scrape(Scrapers.HACKERNEWS) or self.scrape(Scrapers.LOBSTERS) or 
                self.scrape(Scrapers.REDDIT_PROG) or self.scrape(Scrapers.REDDIT_TECH) or
                self.scrape(Scrapers.SLASHDOT))

            if s:
                self._cachedTitle = s.title
            else:
                self._cachedTitle = "(missing title)"

        return self._cachedTitle

    @property
    def titles(self):
        if self._cachedTitles == None:
            self._cachedTitles = [s['title'] for s in self.scraped]
        return self._cachedTitles

    @property
    def autoBreakTitle(self):
        """Returns the title with embedded zero-width spaces"""
        return re.sub("([^\s]{7})([^\s]{3})", u"\\1\u200B\\2", self.title)

    @property
    def hackernewsUrl(self):
        s = self.scrape(Scrapers.HACKERNEWS)
        if s:
            return 'https://news.ycombinator.com/item?id=%s' % s.id
        return None

    @property
    def slashdotUrl(self):
        s = self.scrape(Scrapers.SLASHDOT)
        if s:
            return 'https://slashdot.org/story/%s' % s.id
        return None

    @property
    def redditUrl(self):
        s = self.scrape(Scrapers.REDDIT_PROG) or self.scrape(Scrapers.REDDIT_TECH)
        if s:
            if s.subcategory:
                return 'https://www.reddit.com/r/%s/comments/%s' % (s.subcategory, s.id)
            else:
                return 'https://www.reddit.com/comments/%s' % s.id
        return None
    
    @property
    def lobstersUrl(self):
        s = self.scrape(Scrapers.LOBSTERS)
        if s:
            return 'https://lobste.rs/s/%s' % s.id
        return None
    
    @property
    def rfc3339_date(self):
        return rfc3339.rfc3339(self.date)

    @property
    def age(self):
        """Computes a relative date based on the current time"""
        date = self.date
        timespan = (datetime.now() - date)
        if timespan.days > 0:
            if timespan.days == 1:
                return "1 day ago"
            else:
                if timespan.days >= 60:
                    return "%s months ago" % (timespan.days / 30)
                else:
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
        return self._search_results.tags        
    
    @property
    def score(self):
        if self._cachedScore == None:
            self._cachedScore = scoreStory(self).sum
        return self._cachedScore
    
    @property
    def scoreTerms(self):
        return scoreStory(self).scores
    
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

    @property
    def _search_results(self):
        if self._cachedSearchResults == None:
            # Accumulate tags from scrapes
            tags = []
            for scrape in self.scraped:
                tags += scrape['tags']
            self._cachedSearchResults = generate_search_field(titles=self.titles, tags=tags, url=self.url)

            if set(self.search) != set(self._cachedSearchResults.search_tokens):
                self.search = self._cachedSearchResults.search_tokens
                logging.info("Writing story w/updated search: %s", self.search)
                self.put()

        return self._cachedSearchResults

    def _update_caches(self):
        # TODO: de-dupe this code with _search_results above

        # Accumulate tags from scrapes
        tags = []
        for scrape in self.scraped:
            tags += scrape['tags']
        self.search = generate_search_field(titles=self.titles, tags=tags, url=self.url).search_tokens

        # Clear caches here
        self._cachedTags = None
        self._cachedScore = None
        self._cachedTitle = None
        self._cachedTitles = None
        self._cachedScrapes = None
        self._cachedSearchResults = None

    def add_scrape(self, story):
        self.scraped += [ scraped_story_to_dict(story) ]
        self._update_caches()

    def scrape(self, source):
        """Returns the given scrape for a source if it exists, otherwise None"""
        if self._cachedScrapes == None:
            scrapes = {}
            for scrape in self.scraped:
                scrape = dict_to_scraped_story(scrape)
                scrapes[scrape.source] = scrape
            self._cachedScrapes = scrapes
        return self._cachedScrapes[source] if source in self._cachedScrapes else None

    def __cmp__(self, other):
        # Invert comparison
        return cmp(other.score, self.score)

class Stories:
    # Loads a set of stories from the datastore
    def load(self, search=None, ignore_cache=False, force_update=True):
        try:
            if search and search.strip():
                return self._maybe_load_search(search, ignore_cache, force_update)
            else:
                return self._maybe_load_default(ignore_cache, force_update)
        except db.NeedIndexError, err:
            logging.error("Index appears to be missing %s", err)
            return []

    def _maybe_load_default(self, ignore_cache, force_update):
        try:
            if not ignore_cache:
                stories = memcache.get("stories-default")
                if stories:
                    logging.info("Found %d stor(ies) in memcache for default feed", len(stories))
                    return stories
            stories = self._load_default(FETCH_COUNT)
            memcache.add("stories-default", stories, 10 * 60)
            memcache.add("stories-default-last-ditch", stories, 24 * 60 * 60)
            return stories
        except apiproxy_errors.OverQuotaError, err:
            logging.error("Uh-oh: we are over quota for the front page. Let's use the last-ditch results: %s", err)
            stories = memcache.get("stories-default-last-ditch")
            if stories == None:
                stories = []
            return stories

    def _load_default(self, count):
        scraped = Scrape.query().order(-Scrape.date).fetch(count)
        scraped.sort()

        logging.info("Loaded %d stor(ies) for default feed", len(scraped))
        return scraped

    def _maybe_load_search(self, search, ignore_cache, force_update):
        try:
            if not ignore_cache:
                stories = memcache.get("stories-search-" + search)
                if stories:
                    logging.info("Found %d stor(ies) in memcache for search '%s'", len(stories), search)
                    return stories

            stories = self._load_search(search, SEARCH_FETCH_COUNT)
            memcache.add("stories-search-" + search, stories, 60 * 60)
            return stories
        except apiproxy_errors.OverQuotaError, err:
            logging.error("Uh-oh: we are over quota for the search page: %s", err)
            # TODO: Return a placeholder story here or just run through the last-ditch results
            return []

    def _load_search(self, search, count):
        tokens = generate_search_tokens_for_query(search)
        # No results if a search term results in nothing but stop words
        if len(tokens) == 0:
            return []

        query = None
        for token in tokens:
            if query:
                query = ndb.AND(Scrape.search == token, query)
            else:
                query = (Scrape.search == token)

        logging.info("For search '%s', running query: %s", search, query)
        scraped = Scrape.query(query).order(-Scrape.date).fetch(count)
        scraped.sort()
        logging.info("Query retured %d stor(ies)", len(scraped))

        return scraped

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
        self._store_to_db(story)
        memcache.add(existingKey, True, CACHE_SEEN_STORY)
        return True

    def _store_to_db(self, story):
        # TODO: Should we restrict this to the last X months to allow a story to re-bubble?
        # TODO: Create a canonical version of the URL so we can fuzzy-match stories?
        cutoff = datetime.today() - timedelta(days=30)
        existing = Scrape.query(Scrape.url == story.url, Scrape.date > cutoff).get()
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

        existing.add_scrape(story)
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
        return
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

    def test_computed_title(self):
        stories = Stories()

        # Scrape reddit first
        scrape = [
            ScrapedStory(id='1', url='http://example.com/1', title='title reddit', source='reddit.prog', index=1, tags=['a']),
        ]
        stories.store(scrape)
        loaded = stories.load(ignore_cache=True)
        self.assertEquals(1, len(loaded))
        self.assertEquals('title reddit', loaded[0].title)

        # Once we scrape the same URL from HN, it should replace the reddit titles (which are usually worse)
        scrape = [
            ScrapedStory(id='1', url='http://example.com/1', title='title hn', source='hackernews', index=1, tags=['b']),
        ]
        stories.store(scrape)
        loaded = stories.load(ignore_cache=True)
        self.assertEquals(1, len(loaded))
        self.assertEquals('title hn', loaded[0].title)

    def tearDown(self):
        self.testbed.deactivate()

if __name__ == '__main__':
    unittest.main()

