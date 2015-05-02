import lib

import re
import string

from stemming import porter2
from sets import *

# From AppEngine's SearchableModel
WORD_DELIMITER_REGEX = re.compile('[' + re.escape(string.punctuation) + ']')
FULL_TEXT_MIN_LENGTH = 2
FULL_TEXT_STOP_WORDS = frozenset([
   'a', 'about', 'according', 'accordingly', 'affected', 'affecting', 'after',
   'again', 'against', 'all', 'almost', 'already', 'also', 'although',
   'always', 'am', 'among', 'an', 'and', 'any', 'anyone', 'apparently', 'are',
   'arise', 'as', 'aside', 'at', 'away', 'be', 'became', 'because', 'become',
   'becomes', 'been', 'before', 'being', 'between', 'both', 'briefly', 'but',
   'by', 'came', 'can', 'cannot', 'certain', 'certainly', 'could', 'did', 'do',
   'does', 'done', 'during', 'each', 'either', 'else', 'etc', 'ever', 'every',
   'following', 'for', 'found', 'from', 'further', 'gave', 'gets', 'give',
   'given', 'giving', 'gone', 'got', 'had', 'hardly', 'has', 'have', 'having',
   'here', 'how', 'however', 'i', 'if', 'in', 'into', 'is', 'it', 'itself',
   'just', 'keep', 'kept', 'knowledge', 'largely', 'like', 'made', 'mainly',
   'make', 'many', 'might', 'more', 'most', 'mostly', 'much', 'must', 'nearly',
   'necessarily', 'neither', 'next', 'no', 'none', 'nor', 'normally', 'not',
   'noted', 'now', 'obtain', 'obtained', 'of', 'often', 'on', 'only', 'or',
   'other', 'our', 'out', 'owing', 'particularly', 'past', 'perhaps', 'please',
   'poorly', 'possible', 'possibly', 'potentially', 'predominantly', 'present',
   'previously', 'primarily', 'probably', 'prompt', 'promptly', 'put',
   'quickly', 'quite', 'rather', 'readily', 'really', 'recently', 'regarding',
   'regardless', 'relatively', 'respectively', 'resulted', 'resulting',
   'results', 'said', 'same', 'seem', 'seen', 'several', 'shall', 'should',
   'show', 'showed', 'shown', 'shows', 'significantly', 'similar', 'similarly',
   'since', 'slightly', 'so', 'some', 'sometime', 'somewhat', 'soon',
   'specifically', 'state', 'states', 'strongly', 'substantially',
   'successfully', 'such', 'sufficiently', 'than', 'that', 'the', 'their',
   'theirs', 'them', 'then', 'there', 'therefore', 'these', 'they', 'this',
   'those', 'though', 'through', 'throughout', 'to', 'too', 'toward', 'under',
   'unless', 'until', 'up', 'upon', 'use', 'used', 'usefully', 'usefulness',
   'using', 'usually', 'various', 'very', 'was', 'we', 'were', 'what', 'when',
   'where', 'whether', 'which', 'while', 'who', 'whose', 'why', 'widely',
   'will', 'with', 'within', 'without', 'would', 'yet', 'you'])

# Common but useless terms in URLs
URL_STOP_WORDS = frozenset([
	'www', 'html', 'post', 'story', 'archive'])

# Inspired by AppEngine's SearchableModel
def tokenize(text):
	"""Tokenize an english phrase"""
	text = WORD_DELIMITER_REGEX.sub(' ', text)
	words = text.lower().split()
	words = set(unicode(w) for w in words if len(w) > FULL_TEXT_MIN_LENGTH)
	words -= FULL_TEXT_STOP_WORDS
	return words

def tokenize_url(url):
	url_tokens = tokenize(url)
	url_tokens -= URL_STOP_WORDS
	return url_tokens

def tokenizeStory(story):
	# Tokenize all the story titles, even the ones we are hiding from different scrapes
	tokens = tokenize(story.titles)

	# Add the tags we've generated
	tokens = tokens.union(story.tags)

	# And the URL (but ignore common terms we don't care about)
	# TODO: Clean the url using the old code
	tokens = tokens.union(tokenize_url(story.url))

	return tokens

def generateSearchField(story):
	tokens = [porter2.stem(x) for x in tokenizeStory(story)]
	return tokens

if __name__ == '__main__':
	print tokenize("http://google.com/foo")
