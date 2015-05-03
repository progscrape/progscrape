from datetime import datetime
from scrapers import Scrapers, ScrapedStory
import unittest

__all__ = [ 'scoreStory' ]

class Score:
	def __init__(self, s):
		self.scores = s
		self.sum = sum([s[x] for x in s])

def scoreStory(story, now=datetime.now()):
    s = {}
    
    # Age decay for stories
    timespan = (now - story.date)
    if timespan.days > 0:
    	# > 1day
        s['age'] = -100 * timespan.days
    else:
        age = timespan.seconds
        if age < 60 * 60:
        	# < 1hr
            s['age'] = -10
        elif age < 60 * 60 * 2:
        	# < 2hrs
            s['age'] = -20
        else:
        	# > 2hrs
            s['age'] = -20 + (-5 * (age / (60 * 60)))

	# A small random bump for stories based on the hash of its URL
    s['random'] = (story.url.__hash__() % 600) / 100.0

    # The number of places we've scraped a story from
    count = 0

    redditProgPosition = story.scrape(Scrapers.REDDIT_PROG).index if story.scrape(Scrapers.REDDIT_PROG) else None
    redditTechPosition = story.scrape(Scrapers.REDDIT_TECH).index if story.scrape(Scrapers.REDDIT_TECH) else None
    hackerNewsPosition = story.scrape(Scrapers.HACKERNEWS).index if story.scrape(Scrapers.HACKERNEWS) else None
    lobstersPosition = story.scrape(Scrapers.LOBSTERS).index if story.scrape(Scrapers.LOBSTERS) else None

    if redditProgPosition:
        s['reddit1'] = max(0, 30 - redditProgPosition)
        count += 1
    if redditTechPosition:
        s['reddit2'] = max(0, (30 - redditTechPosition) * 0.5)
        count += 1
    if hackerNewsPosition:
        s['hnews'] = max(0, (30 - hackerNewsPosition) * 1.2)
        count += 1
    if lobstersPosition:
        count += 1
        s['lobsters'] = max(0, (30 - lobstersPosition) * 1.2)

    # Penalize long reddit titles
    if (redditProgPosition or redditTechPosition) and len(story.title) > 130:
        s['long_title'] = -5

    # Penalize long titles from anywhere
    if len(story.title) > 250:
        s['really_long_title'] = -10

    # Penalize image links that only appear on reddit
    if 'imgur.com' in story.url or 'gfycat.com' in story.url:
    	if not hackerNewsPosition and not lobstersPosition:
    		s['reddit_only_imgur'] = -20

    # Found in more than one place: bonus
    if count > 1:
        s['multiple_service'] = 10

    return Score(s)

class MockStory():
	def __init__(self, scraped, title, url, date):
		self.scraped = scraped
		self.title = title
		self.url = url
		self.date = date

	def scrape(self, source):
		for scrape in self.scraped:
			if scrape.source == source:
				return scrape

		return None

class TestScores(unittest.TestCase):
    def test_legacy(self):
    	"""Tests that our legacy scoring hasn't changed"""
    	hn = ScrapedStory(source=Scrapers.HACKERNEWS, index=7)
    	reddit = ScrapedStory(source=Scrapers.REDDIT_PROG, index=1)

    	story = MockStory([hn, reddit], 
    		'EarthBound\xe2s Copy Protection (Super Nintendo game)', 
    		'http://earthboundcentral.com/2011/05/earthbounds-copy-protection/',
    		datetime(2015, 05, 02, 5))

    	# Six hours later
    	s = scoreStory(story, now=datetime(2015, 05, 02, 11))
    	self.assertAlmostEqual(s.scores['hnews'], 27.6)
    	self.assertAlmostEqual(s.scores['reddit1'], 29)
    	self.assertAlmostEqual(s.scores['multiple_service'], 10)
    	self.assertAlmostEqual(s.scores['age'], -50)

    	keys = s.scores.keys()
    	keys.sort()
    	self.assertEquals(['age', 'hnews', 'multiple_service', 'random', 'reddit1'], keys)

if __name__ == '__main__':
    unittest.main()
