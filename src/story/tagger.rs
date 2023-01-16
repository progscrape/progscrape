use std::{collections::{HashMap, HashSet}, iter::Peekable};

use itertools::Itertools;
use serde::{Serialize, Deserialize};

use super::TagSet;

#[derive(Default, Serialize, Deserialize)]
pub struct TagConfig {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    alt: Option<String>,
    #[serde(default)]
    alts: Vec<String>,
    #[serde(default)]
    implies: Option<String>,
    #[serde(default)]
    internal: Option<String>,
    #[serde(default)]
    excludes: Vec<String>,
    #[serde(default)]
    symbol: bool,
}

#[derive(Default, Serialize, Deserialize)]
pub struct TaggerConfig {
    tags: HashMap<String, HashMap<String, TagConfig>>
}

#[derive(Debug)]
struct TagRecord {
    output: String,
    implies: Vec<String>,
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct MultiTokenTag {
    tag: Vec<String>,
}

impl MultiTokenTag {
    pub fn matches<T: AsRef<str> + std::cmp::PartialEq<String>>(&self, slice: &[T]) -> bool {
        if slice.len() < self.tag.len() {
            return false;
        }
        for i in 0..self.tag.len() {
            if slice[i] != self.tag[i] {
                return false;
            }
        }
        true
    }

    pub fn chomp<T: AsRef<str> + std::cmp::PartialEq<String>>(&self, slice: &mut &[T]) -> bool {
        if self.matches(slice) {
            *slice = &mut &slice[self.tag.len()..];
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
/// The `Tagger` creates a list of tag symbols from a story.
pub struct Tagger {
    records: Vec<TagRecord>,
    /// Maps tags to internal symbols
    forward: HashMap<String, usize>,
    /// Forward-maps multi-token tags.
    forward_multi: HashMap<MultiTokenTag, usize>,
    /// Exclusion tokens that mute other tags.
    exclusions: HashMap<MultiTokenTag, String>,
    /// Maps internal symbols to tags (only required in a handful of cases)
    backward: HashMap<String, String>,
    /// 
    symbols: HashMap<String, usize>,
}

impl Tagger {
    // TODO: These methods allocate a lot of temporaries that probably don't need to be allocated
    fn compute_tag(tag: &str) -> Vec<String> {
        // Optional hyphen/space
        if tag.contains("(-)") {
            let mut v = Self::compute_tag(&tag.replace("(-)", "-"));
            v.extend(Self::compute_tag(&tag.replace("(-)", " ")));
            return v;
        }
        if let Some(tag) = tag.strip_suffix("(s)") {
            vec![tag.to_owned(), tag.to_owned() + "s"]
        } else {
            vec![tag.to_owned()]
        }
    }

    /// From a tag and list of alts, compute all possible permutations
    fn compute_all_tags(tag: &str, alt: &Option<String>, alts: &Vec<String>) -> (String, HashSet<String>) {
        let mut tags = HashSet::new();
        let v = Self::compute_tag(tag);
        let primary = v[0].clone();
        tags.extend(v);
        if let Some(alt) = alt {
            tags.extend(Self::compute_tag(&alt));
        }
        for alt in alts {
            tags.extend(Self::compute_tag(alt));
        }
        (primary, tags)
    }

    pub fn new(config: &TaggerConfig) -> Self {
        let mut new = Self {
            forward: HashMap::new(),
            forward_multi: HashMap::new(),
            backward: HashMap::new(),
            records: vec![],
            symbols: HashMap::new(),
            exclusions: HashMap::new(),
        };
        for (category, tags) in &config.tags {
            for (tag, tags) in tags {
                let (primary, all_tags) = Self::compute_all_tags(tag, &tags.alt, &tags.alts);
                let excludes = tags.excludes.iter().flat_map(|s| Self::compute_tag(&s)).map(|s| MultiTokenTag { tag: s.split_ascii_whitespace().map(str::to_owned).collect() });
                for exclude in excludes {
                    new.exclusions.insert(exclude, primary.clone());
                }
                let record = TagRecord {
                    output: match tags.internal {
                        Some(ref s) => s.clone(),
                        None => primary,
                    },
                    implies: tags.implies.clone().into_iter().collect(),
                };

                for tag in all_tags {
                    if tags.symbol {
                        new.symbols.insert(tag, new.records.len());
                    } else {
                        if tag.contains(' ') {
                            let tag = MultiTokenTag { tag: tag.split_ascii_whitespace().map(str::to_owned).collect() };
                            new.forward_multi.insert(tag, new.records.len());
                        } else {
                            new.forward.insert(tag, new.records.len());
                        }
                    }
                }

                new.records.push(record);
            }
        }

        new
    }

    pub fn tag(&self, s: &str, tag_set: &mut TagSet) {
        let mut s = s.to_lowercase();

        // First, we replace all symbols and generate tags
        for (symbol, rec) in &self.symbols {
            if s.contains(symbol) {
                s = s.replace(symbol, " ");
                tag_set.add(self.records[*rec].output.clone());
                for implies in &self.records[*rec].implies {
                    tag_set.add(implies.clone());
                }
            }
        }

        // Next, we check all the word-like tokens for potential matches
        let tokens_vec = s.split_ascii_whitespace().map(|s| s.replace(|c: char| !c.is_alphanumeric() && c != '-', "")).filter(|s| !s.is_empty()).collect_vec();
        let mut tokens = tokens_vec.as_slice();

        let mut mutes = HashMap::new();

        while !tokens.is_empty() {
            mutes.retain(|k, v| {
                if *v == 0 {
                    false
                } else {
                    *v -= 1;
                    true
                }
            });
            for (exclusion, tag) in &self.exclusions {
                println!("check {:?} {:?}", exclusion, tokens);
                if exclusion.matches(tokens) {
                    println!("mute {:?}", exclusion);
                    mutes.insert(tag.clone(), exclusion.tag.len() - 1);
                }
            }
            for (multi, rec) in &self.forward_multi {
                if multi.chomp(&mut tokens) {
                    let rec = &self.records[*rec];
                    tag_set.add(rec.output.clone());
                    for implies in &rec.implies {
                        tag_set.add(implies.clone());
                    }
                    continue;
                }
            }
            if let Some(rec) = self.forward.get(&tokens[0]) {
                if !mutes.contains_key(&tokens[0]) {
                    let rec = &self.records[*rec];
                    tag_set.add(rec.output.clone());
                    for implies in &rec.implies {
                        tag_set.add(implies.clone());
                    }
                }
            }
            tokens = &tokens[1..];
        }
    }
}

#[cfg(test)]
mod test {
    use rstest::*;
    use serde_json::json;

    use crate::story::TagSet;

    use super::{TaggerConfig, Tagger};

    #[fixture]
    fn tagger_config() -> TaggerConfig {
        serde_json::from_value(json!({
            "tags": {
                "testing": {
                    "video(s)": {},
                    "rust": {},
                    "chrome": {"alt": "chromium"},
                    "neovim": {"implies": "vim"},
                    "vim": {},
                    "3d": {"alts": ["3(-)d", "3(-)dimension(s)", "three(-)d", "three(-)dimension(s)", "three(-)dimensional", "3(-)dimensional"]},
                    "usbc": {"alt": "usb(-)c"},
                    "at&t": {"internal": "atandt", "symbol": true},
                    "angular": {"alt": "angularjs"},
                    "vi": {"internal": "vieditor"},
                    "c": {"internal": "clanguage"},
                    "d": {"internal": "dlanguage", "excludes": ["vitamin d", "d wave", "d waves"]},
                    "c++": {"internal": "cplusplus", "symbol": true},
                    "c#": {"internal": "csharp", "symbol": true},
                    "f#": {"internal": "fsharp", "symbol": true}
                }
            }
        })).expect("Failed to parse test config")
    }

    #[fixture]
    fn tagger(tagger_config: TaggerConfig) -> Tagger {
        let tagger = Tagger::new(&tagger_config);
        println!("{:?}", tagger);
        tagger
    } 

    #[rstest]
    #[case("I love rust!", &["rust"])]
    #[case("Good old video", &["video"])]
    #[case("Good old videos", &["video"])]
    #[case("Chromium is a project", &["chrome"])]
    #[case("AngularJS is fun", &["angular"])]
    #[case("Chromium is the open Chrome", &["chrome"])]
    #[case("Neovim is kind of cool", &["neovim", "vim"])]
    #[case("Neovim is a kind of vim", &["neovim", "vim"])]
    #[case("C is hard", &["clanguage"])]
    #[case("D is hard", &["dlanguage"])]
    #[case("C# is hard", &["csharp"])]
    #[case("C++ is hard", &["cplusplus"])]
    #[case("AT&T has an ampersand", &["atandt"])]
    fn test_tag_extraction(tagger: Tagger, #[case] s: &str, #[case] tags: &[&str]) {
        let mut tag_set = TagSet::new();
        tagger.tag(s, &mut tag_set);
        assert_eq!(tag_set.collect(), tags.to_vec(), "while checking tags for {}", s);
    }

    #[rstest]
    #[case("Usbc.wtf - an article and quiz to find the right USB-C cable", &["usbc"])]
    #[case("D&D Publisher Addresses Backlash Over Controversial License", &[])]
    #[case("Microfeatures I'd like to see in more languages", &[])]
    #[case("What are companies doing with D-Wave's quantum hardware?", &[])]
    #[case("What are companies doing with D Wave's quantum hardware?", &[])]
    #[case("Conserving Dürer's Triumphal Arch: coming apart at the seams (2016)", &[])]
    #[case("J.D. Vance Is Coming for You", &[])]
    #[case("Rewriting TypeScript in Rust? You'd have to be crazy", &["rust"])]
    #[case("Vitamin D Supplementation Does Not Influence Growth in Children", &[])]
    #[case("Vitamin-D Supplementation Does Not Influence Growth in Children", &[])]
    #[case("They'd rather not", &[])]
    #[case("Apple Music deletes your original songs and replaces them with DRM'd versions", &[])]
    fn test_c_and_d_cases(tagger: Tagger, #[case] s: &str, #[case] tags: &[&str]) {
        let mut tag_set = TagSet::new();
        tagger.tag(s, &mut tag_set);
        assert_eq!(tag_set.collect(), tags.to_vec(), "while checking tags for {}", s);
    }

    #[rstest]
    #[case("New Process Allows 3-D Printing of Microscale Metallic Parts", &["3d"])]
    #[case("3D printing is wild", &["3d"])]
    #[case("3 D printing is hard", &["3d"])]
    #[case("3-D printing is hard", &["3d"])]
    #[case("three-d printing is hard", &["3d"])]
    #[case("three d printing is hard", &["3d"])]
    #[case("three dimensional printing is hard", &["3d"])]
    #[case("3 dimensional printing is hard", &["3d"])]
    #[case("3-dimensional printing is hard", &["3d"])]
    fn test_3d_cases(tagger: Tagger, #[case] s: &str, #[case] tags: &[&str]) {
        let mut tag_set = TagSet::new();
        tagger.tag(s, &mut tag_set);
        assert_eq!(tag_set.collect(), tags.to_vec(), "while checking tags for {}", s);
    }
}
