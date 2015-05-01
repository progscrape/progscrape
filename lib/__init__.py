import sys
import os

try:
	sys.path.index(os.path.dirname(__file__))
except:
	sys.path.insert(0, os.path.dirname(__file__))
