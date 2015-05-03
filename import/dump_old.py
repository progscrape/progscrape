from google.appengine.ext import db
import simplejson as json

# remote_api_shell.py -s remote.progscrape-hr.appspot.com
# import dump_old

class Story(db.Expando):
    pass

f = open('old.json', 'w')

query = Story.all()
entities = query.fetch(250)
print "Fetching stories..."
count = 0
while entities:
    count += len(entities)
    print count
    for entity in entities:
        f.write(json.dumps(dict([(key, unicode(entity.__getattr__(key))) 
            for key in entity.dynamic_properties() if not '__' in key])))
        f.write('\n')
    query.with_cursor(query.cursor())
    entities = query.fetch(250)

f.close()
