use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use super::{TagAcceptor, TagSet};

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
    tags: HashMap<String, HashMap<String, TagConfig>>,
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
        // If the next `self.tag.len()` items in slice match, we match (additional items are OK)
        itertools::equal(
            slice.iter().take(self.tag.len()).map(T::as_ref),
            self.tag.iter(),
        )
    }

    pub fn chomp<T: AsRef<str> + std::cmp::PartialEq<String>>(&self, slice: &mut &[T]) -> bool {
        if self.matches(slice) {
            *slice = &slice[self.tag.len()..];
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
/// The `StoryTagger` creates a list of tag symbols from a story.
pub struct StoryTagger {
    records: Vec<TagRecord>,
    /// Maps tags to internal symbols
    forward: HashMap<String, usize>,
    /// Forward-maps multi-token tags.
    forward_multi: HashMap<MultiTokenTag, usize>,
    /// Exclusion tokens that mute other tags.
    exclusions: HashMap<MultiTokenTag, String>,
    /// Maps internal symbols to tags (only required in a handful of cases)
    backward: HashMap<String, String>,
    /// Maps symbol-like tags to their internal version.
    symbols: HashMap<String, usize>,
}

impl StoryTagger {
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
    fn compute_all_tags(
        tag: &str,
        alt: &Option<String>,
        alts: &Vec<String>,
    ) -> (String, HashSet<String>) {
        let mut tags = HashSet::new();
        let v = Self::compute_tag(tag);
        let primary = v[0].clone();
        tags.extend(v);
        if let Some(alt) = alt {
            tags.extend(Self::compute_tag(alt));
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
        for tags in config.tags.values() {
            for (tag, tags) in tags {
                let (primary, all_tags) = Self::compute_all_tags(tag, &tags.alt, &tags.alts);
                let excludes = tags
                    .excludes
                    .iter()
                    .flat_map(|s| Self::compute_tag(s))
                    .map(|s| MultiTokenTag {
                        tag: s.split_ascii_whitespace().map(str::to_owned).collect(),
                    });
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
                if let Some(internal) = &tags.internal {
                    new.backward.insert(internal.clone(), tag.clone());
                }
                for tag in all_tags {
                    if tags.symbol {
                        new.backward.insert(record.output.clone(), tag.clone());
                        new.symbols.insert(tag, new.records.len());
                    } else if tag.contains(' ') {
                        let tag = MultiTokenTag {
                            tag: tag.split_ascii_whitespace().map(str::to_owned).collect(),
                        };
                        new.forward_multi.insert(tag, new.records.len());
                    } else {
                        new.forward.insert(tag, new.records.len());
                    }
                }

                new.records.push(record);
            }
        }

        new
    }

    pub fn tag<T: TagAcceptor>(&self, s: &str, tags: &mut T) {
        let s = s.to_lowercase();

        // Clean up single quotes to a standard type
        let s = s.replace(
            |c| {
                c == '`' || c == '\u{2018}' || c == '\u{2019}' || c == '\u{201a}' || c == '\u{201b}'
            },
            "'",
        );

        // Replace possessive with non-possessive
        let mut s = s.replace("'s", "");

        // First, we replace all symbols and generate tags
        for (symbol, rec) in &self.symbols {
            if s.contains(symbol) {
                s = s.replace(symbol, " ");
                tags.tag(&self.records[*rec].output);
                for implies in &self.records[*rec].implies {
                    tags.tag(implies);
                }
            }
        }

        // Next, we check all the word-like tokens for potential matches
        let tokens_vec = s
            .split_ascii_whitespace()
            .map(|s| s.replace(|c: char| !c.is_alphanumeric() && c != '-', ""))
            .filter(|s| !s.is_empty())
            .collect_vec();
        let mut tokens = tokens_vec.as_slice();

        let mut mutes = HashMap::new();

        'outer: while !tokens.is_empty() {
            mutes.retain(|_k, v| {
                if *v == 0 {
                    false
                } else {
                    *v -= 1;
                    true
                }
            });
            for (exclusion, tag) in &self.exclusions {
                if exclusion.matches(tokens) {
                    mutes.insert(tag.clone(), exclusion.tag.len() - 1);
                }
            }
            for (multi, rec) in &self.forward_multi {
                if multi.chomp(&mut tokens) {
                    let rec = &self.records[*rec];
                    tags.tag(&rec.output);
                    for implies in &rec.implies {
                        tags.tag(implies);
                    }
                    continue 'outer;
                }
            }
            if let Some(rec) = self.forward.get(&tokens[0]) {
                if !mutes.contains_key(&tokens[0]) {
                    let rec = &self.records[*rec];
                    tags.tag(&rec.output);
                    for implies in &rec.implies {
                        tags.tag(implies);
                    }
                }
            }
            tokens = &tokens[1..];
        }
    }

    /// Identify any tags in the search term and return the appropriate search term to use. If the search term is a symbol,
    /// we must use its internal version (ie: cplusplus -> c++, c -> clanguage).
    pub fn check_tag_search(&self, search: &str) -> Option<&str> {
        let lowercase = search.to_lowercase();
        if let Some(idx) = self.symbols.get(&lowercase) {
            return Some(&self.records[*idx].output);
        }
        if let Some(idx) = self.forward.get(&lowercase) {
            return Some(&self.records[*idx].output);
        }
        if let Some((k, _)) = self.backward.get_key_value(&lowercase) {
            return Some(k.as_str());
        }

        None
    }

    /// Given a raw, indexed tag, output a tag that is suitable for display purposes (ie: cplusplus -> c++).
    pub fn make_display_tag<'a, S: AsRef<str> + 'a>(&'a self, s: S) -> String {
        let lowercase = s.as_ref().to_lowercase();
        if let Some(backward) = self.backward.get(&lowercase) {
            backward.clone()
        } else {
            lowercase
        }
    }

    /// Given an iterator of raw, indexed tags, output an iterator that is suitable for display purposes (ie: cplusplus -> c++).
    pub fn make_display_tags<'a, S: AsRef<str>, I: IntoIterator<Item = S> + 'a>(
        &'a self,
        iter: I,
    ) -> impl Iterator<Item = String> + 'a {
        iter.into_iter().map(|s| self.make_display_tag(s))
    }

    pub fn tag_details() -> Vec<(String, TagSet)> {
        // let mut tags = HashMap::new();
        // let mut tag_set = TagSet::new();
        // resources.tagger().tag(story.title(), &mut tag_set);
        // tags.insert("title".to_owned(), tag_set.collect());
        // for (id, scrape) in &story.scrapes {
        //     let mut tag_set = TagSet::new();
        //     scrape.tag(&resources.config().scrape, &mut tag_set)?;
        //     tags.insert(format!("scrape {:?}", id), tag_set.collect());
        // }
        Default::default()
    }
}

#[cfg(test)]
pub(crate) mod test {
    use itertools::Itertools;
    use rstest::*;
    use serde_json::json;

    use crate::story::TagSet;

    use super::{StoryTagger, TaggerConfig};

    /// Create a tagger configuration with a wide variety of cases. Note that this is used in `StoryEvaulator`'s test mode.
    #[fixture]
    pub(crate) fn tagger_config() -> TaggerConfig {
        serde_json::from_value(json!({
            "tags": {
                "testing": {
                    "video(s)": {"hosts": ["youtube.com", "vimeo.com"]},
                    "rust": {},
                    "chrome": {"alt": "chromium"},
                    "neovim": {"implies": "vim"},
                    "vim": {},
                    "3d": {"alts": ["3(-)d", "3(-)dimension(s)", "three(-)d", "three(-)dimension(s)", "three(-)dimensional", "3(-)dimensional"]},
                    "usbc": {"alt": "usb(-)c"},
                    "at&t": {"internal": "atandt", "symbol": true},
                    "angular": {"alt": "angularjs"},
                    "vi": {"internal": "vieditor"},
                    "go": {"alt": "golang", "internal": "golang", "excludes": ["can go", "will go", "to go", "go to", "go in", "go into", "let go", "letting go", "go home"]},
                    "c": {"internal": "clanguage"},
                    "d": {"internal": "dlanguage", "excludes": ["vitamin d", "d wave", "d waves"]},
                    "c++": {"internal": "cplusplus", "symbol": true},
                    "c#": {"internal": "csharp", "symbol": true},
                    "f#": {"internal": "fsharp", "symbol": true},
                    ".net": {"internal": "dotnet", "symbol": true},
                }
            }
        })).expect("Failed to parse test config")
    }

    #[fixture]
    fn tagger(tagger_config: TaggerConfig) -> StoryTagger {
        // println!("{:?}", tagger);
        StoryTagger::new(&tagger_config)
    }

    /// Ensure that symbol-like tags are reverse-lookup'd properly for display purposes.
    #[rstest]
    fn test_display_tags(tagger: StoryTagger) {
        assert_eq!(
            tagger
                .make_display_tags(["atandt", "cplusplus", "clanguage", "rust"])
                .collect_vec(),
            vec!["at&t", "c++", "c", "rust"]
        );
    }

    /// Esnure that we can detect when symbol-like tags are passed to a search function.
    #[rstest]
    #[case("cplusplus", &["c++", "cplusplus"])]
    #[case("clanguage", &["c", "clanguage"])]
    #[case("atandt", &["at&t", "atandt"])]
    #[case("angular", &["angular", "angularjs"])]
    #[case("golang", &["go", "golang"])]
    #[case("dotnet", &[".net", "dotnet"])]
    fn test_search_mapping(tagger: StoryTagger, #[case] a: &str, #[case] b: &[&str]) {
        for b in b {
            assert_eq!(
                tagger.check_tag_search(b),
                Some(a),
                "Didn't match for '{b}'"
            );
        }
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
    fn test_tag_extraction(tagger: StoryTagger, #[case] s: &str, #[case] tags: &[&str]) {
        let mut tag_set = TagSet::new();
        tagger.tag(s, &mut tag_set);
        assert_eq!(
            tag_set.collect(),
            tags.to_vec(),
            "while checking tags for {s}"
        );
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
    fn test_c_and_d_cases(tagger: StoryTagger, #[case] s: &str, #[case] tags: &[&str]) {
        let mut tag_set = TagSet::new();
        tagger.tag(s, &mut tag_set);
        assert_eq!(
            tag_set.collect(),
            tags.to_vec(),
            "while checking tags for {s}"
        );
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
    // Multi-word token at the end
    #[case("I love printing in three dimensions", &["3d"])]
    fn test_3d_cases(tagger: StoryTagger, #[case] s: &str, #[case] tags: &[&str]) {
        let mut tag_set = TagSet::new();
        tagger.tag(s, &mut tag_set);
        assert_eq!(
            tag_set.collect(),
            tags.to_vec(),
            "while checking tags for {s}"
        );
    }
}
