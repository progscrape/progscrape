TAGS = [
  # General types of story
  'video', 'music', 'audio', 'tutorials', 'tutorial', 'media', 'rfc',

  # General concepts
  'algorithm', 'algorithms', 'compiler', 'compilers', '3d', 'hash', 'vc', 'web', 'api',

  # Concrete concepts
  'drm', 'nosql', 'sql', 'copyright', 'trademark', 'patent', 'encryption', 'economy', 'investing',
  'privacy', 'autism', 'lawsuit', 'universe', 'assemblers', 'proxy', 'censorship', 'firewall', 'trial',
  'piracy', 'ipo', 'graphics', 'embedded', 'art', 'kernel', 'antimatter', 'compression',

  # Orgs
  'amd', 'intel', 'apple', 'facebook', 'google', 'yahoo', 'microsoft', 'twitter', 'zynga',
  'techcrunch', 'htc', 'amazon', 'mozilla', 'dell', 'nokia', 'novell', 'lenovo', 'nasa',
  'ubuntu', 'adobe', 'github', 'cisco', 'motorola', 'samsung', 'verizon', 'sprint', 'tmobile',
  'instagram', 'square', 'stripe', 'anonymous', 'webkit', 'opera', 'tesla', 'redhat', 'centos',
  'gnu', 'mpaa', 'riaa', 'w3c', 'isohunt', 'obama', 'ifpi', 'nsa', 'cia', 'fbi', 'csis', 'wikileaks',
  'snowden', 'kde', 'gnome', 'comcast', 'fcc', 'china', 'canada', 'usa', 'yale', 'navy', 'debian',
  'spacex', 'turing', 'mit', 'stanford', 'uber', 'lyft', 'hbo', 'sony', 'fdic', 'ucla', 'canada',
  'antarctica', 'arctic', 'tor', 'wolfram',

  # Languages
  'php', 'javascript', 'java', 'perl', 'python', 'ruby', 'html', 'html5',
  'css', 'css2', 'css3', 'flash', 'lisp', 'clojure', 'arc', 'scala', 'lua', 
  'haxe', 'ocaml', 'erlang', 'go', 'golang', 'c', 'rust', 'ecmascript', 'haskell', 'nim',
  'prolog',

  # Technologies
  'linux', 'mongodb', 'cassandra', 'hadoop', 'android', 'node',
  'iphone', 'ipad', 'ipod', 'ec2', 'firefox', 'safari', 'chrome', 'windows', 'mac', 'osx',
  'git', 'subversion', 'mercurial', 'vi', 'emacs', 
  'bitcoin', 'drupal', 'wordpress', 'unicode', 'pdf', 'wifi', 
  'phonegap', 'minecraft', 'mojang', 'svg', 'jpeg', 'jpg', 'gif', 'png', 'dns', 'torrent',
  'docker', 'drone', 'drones', 'meteor', 'react', 'openbsd',  'sass', 'scss', 'aes', 'rsa',
  'ssl', 'tls', 'http', 'https', 'ftp', 'webrtc', 'pgp', 'gpg', 'ios', 'ssd',

  # Frameworks
  'django', 'rails', 'jquery', 'prototype', 'mootools', 'angular', 'ember'
]
TAG_WHITELIST = {}

for tag in TAGS:
    TAG_WHITELIST[tag] = True
