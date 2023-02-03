use serde::{ser::SerializeMap, Deserialize, Serialize};
use std::{borrow::Cow, fmt::Debug};

pub use self::def::ScrapeCore;
pub(crate) use self::def::*;
use crate::types::*;

mod def;
pub mod hacker_news;
pub mod legacy;
pub mod lobsters;
pub mod reddit;
pub mod slashdot;
mod utils;

macro_rules! scrapers {
    ($($package:ident :: $name:ident ,)*) => {
        pub mod export {
            $( pub use super::$package; )*
        }

        pub fn scrape(
            config: &ScrapeConfig,
            source: ScrapeSource,
            input: &str,
        ) -> Result<(Vec<TypedScrape>, Vec<String>), ScrapeError> {
            match source {
                $(
                    ScrapeSource::$name => {
                        let scraper = <$package::$name as ScrapeSourceDef>::Scraper::default();
                        let (res, warnings) = scraper.scrape(&config.$package, input)?;
                        Ok((res.into_iter().map(|x| x.into()).collect(), warnings))
                    },
                )*
                ScrapeSource::Other => unreachable!(),
            }
        }

        /// Configuration for all scrapers.
        #[derive(Clone, Default, Serialize, Deserialize)]
        pub struct ScrapeConfig {
            $(
                #[doc="Configuration for the "]
                #[doc=stringify!($name)]
                #[doc=" backend."]
                pub $package : <$package :: $name as ScrapeSourceDef>::Config
            ),*
        }

        impl ScrapeConfig {
            pub fn get(&self, source: ScrapeSource) -> Option<&dyn ScrapeConfigSource> {
                match source {
                    $( ScrapeSource::$name => Some(&self.$package), )*
                    ScrapeSource::Other => None,
                }
            }
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
        pub enum ScrapeSource {
            $($name,)*
            Other,
        }

        impl ScrapeSource {
            pub fn into_str(&self) -> &'static str {
                match self {
                    $(Self::$name => stringify!($package),)*
                    Self::Other => "other",
                }
            }

            pub fn try_from_str(s: &str) -> Option<Self> {
                match s {
                    $(stringify!($package) => Some(Self::$name),)*
                    "other" => Some(Self::Other),
                    _ => None,
                }
            }

            pub const fn all() -> &'static [ScrapeSource] {
                &[$(Self::$name),*]
            }

            pub fn comments_url(&self, id: &str, subsource: Option<&str>) -> String {
                match self {
                    $(Self::$name => $package :: $name :: comments_url(id, subsource),)*
                    _ => unimplemented!()
                }
            }

            pub fn is_comments_host(&self, host: &str) -> bool {
                match self {
                    $(Self::$name => $package :: $name :: is_comments_host(host),)*
                    _ => unimplemented!()
                }
            }

            pub fn id<'a, ID: Clone + Into<Cow<'a, str>>>(&self, id: ID) -> ScrapeId {
                ScrapeId::new(*self, None, id.into().into())
            }

            pub fn subsource_id<'a, ID: Clone + Into<Cow<'a, str>>>(&self, subsource: ID, id: ID) -> ScrapeId {
                ScrapeId::new(*self, Some(subsource.into().into()),  id.into().into())
            }
        }

        #[derive(Clone, Debug, Deserialize, Serialize)]
        pub enum TypedScrape {
            $( $name (GenericScrape<<$package::$name as ScrapeSourceDef>::Scrape>), )*
        }

        impl TypedScrape {
            pub fn merge(&mut self, b: Self) {
                match (self, b) {
                    $( (Self::$name(a), Self::$name(b)) => a.merge_generic(b), )*
                    (_a, _b) => {
                        // tracing::warn!(
                        //     "Unable to merge incompatible scrapes, ignoring",
                        // );
                    }
                }
            }

            pub(crate) fn extract(&self, config: &ScrapeConfig) -> ScrapeCore {
                match self {
                    $(
                        Self::$name(a) => {
                            let scraper = <$package::$name as ScrapeSourceDef>::Scraper::default();
                            scraper.extract_core(&config.$package, a)
                        }
                    )*
                }
            }

            $(
            /// Attempt to coerce this `TypedScrape` into a `GenericScrape` of the given type.
            pub fn $package(&self) -> Option<&GenericScrape<<$package::$name as ScrapeSourceDef>::Scrape>> {
                match self {
                    Self::$name(a) => Some(&a),
                    _ => None,
                }
            }
            )*
        }

        impl std::ops::Deref for TypedScrape {
            type Target = ScrapeShared;
            fn deref(&self) -> &Self::Target {
                match self {
                    $( Self::$name(a) => &a.shared, )*
                }
            }
        }

        impl std::ops::DerefMut for TypedScrape {
            fn deref_mut(&mut self) -> &mut Self::Target {
                match self {
                    $( Self::$name(a) => &mut a.shared, )*
                }
            }
        }

        $(
            impl From<GenericScrape<<$package::$name as ScrapeSourceDef>::Scrape>> for TypedScrape {
                fn from(x: GenericScrape<<$package::$name as ScrapeSourceDef>::Scrape>) -> Self {
                    TypedScrape::$name(x)
                }
            }
        )*

        /// A strongly-typed scrape map that can be used to collect values by scrape source without allocations.
        pub struct TypedScrapeMap<V> {
            $( pub $package: V, )*
            pub other: V
        }

        impl <V: Default> TypedScrapeMap<V> {
            pub fn new() -> Self {
                Self {
                    $( $package: Default::default(), )*
                    other: Default::default(),
                }
            }
        }

        impl <V: Copy> TypedScrapeMap<V> {
            pub fn new_with_all(v: V) -> Self {
                Self {
                    $( $package: v, )*
                    other: v,
                }
            }
        }

        impl <V: Default> Default for TypedScrapeMap<V> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl <V: Clone> Clone for TypedScrapeMap<V> {
            fn clone(&self) -> Self {
                Self {
                    $( $package: self.$package.clone(), )*
                    other: self.other.clone(),
                }
            }
        }

        impl <V> TypedScrapeMap<V> {
            pub fn get(&self, source: ScrapeSource) -> &V {
                match (source) {
                    $( ScrapeSource::$name => &self.$package, )*
                    ScrapeSource::Other => &self.other,
                }
            }

            pub fn set(&mut self, source: ScrapeSource, mut value: V) -> V {
                match (source) {
                    $( ScrapeSource::$name => std::mem::swap(&mut value, &mut self.$package), )*
                    ScrapeSource::Other => std::mem::swap(&mut value, &mut self.other),
                }
                value
            }

            pub fn iter<'a>(&'a self) -> impl Iterator<Item = &'a V> {
                [$( &self.$package, )* &self.other ].into_iter()
            }
        }

        const fn one(_: &'static str) -> usize {
            1
        }

        impl <V> IntoIterator for TypedScrapeMap<V> {
            type Item = V;
            type IntoIter = <[V; 1 $( + one(stringify!($package)) )* ] as IntoIterator>::IntoIter;

            fn into_iter(self) -> Self::IntoIter {
                [$(self.$package,)* self.other].into_iter()
            }
        }

        impl <V: Serialize> Serialize for TypedScrapeMap<V> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: serde::Serializer {
                let mut map = serializer.serialize_map(None)?;
                $(
                    map.serialize_entry(stringify!($package), &self.$package)?;
                )*
                map.serialize_entry("other", &self.other)?;
                map.end()
            }
        }

        /// Implement `Deserialize` if and only if `V` is `Default` as well.
        impl <'de, V: Default + Deserialize<'de>> Deserialize<'de> for TypedScrapeMap<V> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: serde::Deserializer<'de> {

                #[derive(Deserialize)]
                struct Temp<V> {
                    $( #[serde(default)] $package: V, )*
                    #[serde(default)] other: V,
                }

                let temp = Temp::deserialize(deserializer)?;
                Ok(TypedScrapeMap::<V> {
                    $( $package: temp.$package, )*
                    other: temp.other,
                })
            }
        }

    };
}

impl From<TypedScrape> for (ScrapeId, TypedScrape) {
    fn from(val: TypedScrape) -> Self {
        (val.id.clone(), val)
    }
}

impl Serialize for ScrapeSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.into_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ScrapeSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if let Some(source) = ScrapeSource::try_from_str(&s) {
            Ok(source)
        } else {
            Err(serde::de::Error::custom("Invalid source"))
        }
    }
}

scrapers! {
    hacker_news::HackerNews,
    slashdot::Slashdot,
    lobsters::Lobsters,
    reddit::Reddit,
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::extractor::ScrapeExtractor;
    use std::fs::read_to_string;
    use std::path::PathBuf;
    use std::str::FromStr;

    pub fn slashdot_files() -> Vec<&'static str> {
        vec!["slashdot1.html", "slashdot2.html", "slashdot3.html"]
    }

    pub fn hacker_news_files() -> Vec<&'static str> {
        vec!["hn1.html", "hn2.html", "hn3.html", "hn4.html"]
    }

    pub fn lobsters_files() -> Vec<&'static str> {
        vec!["lobsters1.rss", "lobsters2.rss"]
    }

    pub fn reddit_files() -> Vec<&'static str> {
        vec![
            "reddit-prog-tag1.json",
            "reddit-prog-tag2.json",
            "reddit-prog1.json",
            "reddit-science1.json",
            "reddit-science2.json",
        ]
    }

    pub fn files_by_source(source: ScrapeSource) -> Vec<&'static str> {
        match source {
            ScrapeSource::HackerNews => hacker_news_files(),
            ScrapeSource::Slashdot => slashdot_files(),
            ScrapeSource::Reddit => reddit_files(),
            ScrapeSource::Lobsters => lobsters_files(),
            ScrapeSource::Other => vec![],
        }
    }

    pub fn scrape_all() -> Vec<TypedScrape> {
        let mut v = vec![];
        let config = ScrapeConfig::default();
        for source in [
            ScrapeSource::HackerNews,
            ScrapeSource::Lobsters,
            ScrapeSource::Reddit,
            ScrapeSource::Slashdot,
        ] {
            for file in files_by_source(source) {
                let mut res = scrape(&config, source, &load_file(file))
                    .unwrap_or_else(|_| panic!("Scrape of {:?} failed", source));
                v.append(&mut res.0);
            }
        }
        v
    }

    pub fn load_file(f: &str) -> String {
        let mut path = PathBuf::from_str("testdata").unwrap();
        path.push(f);
        read_to_string(path).unwrap()
    }

    #[test]
    fn test_scrape_all() {
        let extractor = ScrapeExtractor::new(&ScrapeConfig::default());
        for scrape in scrape_all() {
            let scrape = extractor.extract(&scrape);
            // Sanity check the scrapes
            assert!(
                !scrape.title.contains("&amp")
                    && !scrape.title.contains("&quot")
                    && !scrape.title.contains("&squot")
            );
            assert!(!scrape.url.raw().contains("&amp"));
            assert!(scrape.date.year() == 2023 || scrape.date.year() == 2022);
        }
    }
}
