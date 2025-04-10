use serde::{Deserialize, Serialize, ser::SerializeMap};
use std::{borrow::Cow, fmt::Debug};

pub use self::def::ScrapeCore;
pub(crate) use self::def::*;
use crate::types::*;

mod def;
pub mod feed;
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

            pub fn id_from_comments_url(&self, url: &str) -> Option<ScrapeId> {
                match self {
                    $(Self::$name => {
                        let (source, subsource) = $package :: $name :: id_from_comments_url(url)?;
                        Some(ScrapeId :: new( *self, subsource.map(|s| s.to_owned()), source.to_owned() ))
                    },)*
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
        #[derive(Debug, Eq, PartialEq)]
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
            /// Get the given value based on a dynamic source.
            pub fn get(&self, source: ScrapeSource) -> &V {
                match (source) {
                    $( ScrapeSource::$name => &self.$package, )*
                    ScrapeSource::Other => &self.other,
                }
            }

            /// Set the given value based on a dynamic source.
            pub fn set(&mut self, source: ScrapeSource, mut value: V) -> V {
                match (source) {
                    $( ScrapeSource::$name => std::mem::swap(&mut value, &mut self.$package), )*
                    ScrapeSource::Other => std::mem::swap(&mut value, &mut self.other),
                }
                value
            }

            /// Remove the given value based on a dynamic source, if values have
            /// a default.
            pub fn remove(&mut self, source: ScrapeSource) -> V where V: Default {
                self.set(source, V::default())
            }

            /// Iterate over the underlying values.
            pub fn values(&self) -> impl Iterator<Item = &'_ V> {
                [$( &self.$package, )* &self.other ].into_iter()
            }

            /// Iterate over the underlying keys/values.
            pub fn iter(&self) -> impl Iterator<Item = (ScrapeSource, &'_ V)> {
                [$( (ScrapeSource::$name, &self.$package), )* (ScrapeSource::Other, &self.other) ].into_iter()
            }

            pub fn into_with_map<T>(self, f: impl Fn(ScrapeSource, V) -> T) -> TypedScrapeMap<T> {
                TypedScrapeMap {
                    $( $package: f(ScrapeSource::$name, self.$package), )*
                    other: f(ScrapeSource::Other, self.other),
                }
            }

            pub fn into_with_map_fallible<T, E>(self, f: impl Fn(ScrapeSource, V) -> Result<T, E>) -> Result<TypedScrapeMap<T>, E> {
                Ok(TypedScrapeMap {
                    $( $package: f(ScrapeSource::$name, self.$package)?, )*
                    other: f(ScrapeSource::Other, self.other)?,
                })
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
    feed::Feed,
}

#[cfg(any(test, feature = "scrape_test"))]
pub mod test {
    use super::*;

    macro_rules! stringify_all {
        ( $($s:literal),* ) => {
            vec![ $( include_str!( concat!("../../testdata/", $s ) ) ),* ]
        };
    }

    fn slashdot_files() -> Vec<&'static str> {
        stringify_all!["slashdot1.html", "slashdot2.html", "slashdot3.html"]
    }

    fn hacker_news_files() -> Vec<&'static str> {
        stringify_all![
            "hn1.html", "hn2.html", "hn3.html", "hn4.html", "hn5.html", "hn6.html"
        ]
    }

    fn lobsters_files() -> Vec<&'static str> {
        stringify_all!["lobsters1.rss", "lobsters2.rss"]
    }

    fn reddit_files() -> Vec<&'static str> {
        stringify_all![
            "reddit-prog-tag1.json",
            "reddit-prog-tag2.json",
            "reddit-prog1.json",
            "reddit-science1.json",
            "reddit-science2.json"
        ]
    }

    pub fn files_by_source(source: ScrapeSource) -> Vec<&'static str> {
        match source {
            ScrapeSource::HackerNews => hacker_news_files(),
            ScrapeSource::Slashdot => slashdot_files(),
            ScrapeSource::Reddit => reddit_files(),
            ScrapeSource::Lobsters => lobsters_files(),
            ScrapeSource::Feed => vec![],
            ScrapeSource::Other => vec![],
        }
    }

    /// Loads the various sample stories we've collected.
    pub fn load_sample_scrapes(config: &ScrapeConfig) -> Vec<TypedScrape> {
        let mut v = vec![];
        for source in [
            ScrapeSource::HackerNews,
            ScrapeSource::Lobsters,
            ScrapeSource::Reddit,
            ScrapeSource::Slashdot,
        ] {
            for file in files_by_source(source) {
                let mut res = scrape(config, source, file)
                    .unwrap_or_else(|_| panic!("Scrape of {:?} failed", source));
                if res.0.is_empty() {
                    panic!("Failed to scrape anything! {file} {:?}", res.1);
                }
                v.append(&mut res.0);
            }
            v.sort_by_key(|scrape| scrape.date);
        }
        v
    }

    #[test]
    fn test_scrape_all() {
        use crate::ScrapeExtractor;

        let config = ScrapeConfig::default();
        let extractor = ScrapeExtractor::new(&config);
        for scrape in load_sample_scrapes(&config) {
            let scrape = extractor.extract(&scrape);
            // Sanity check the scrapes
            assert!(
                !scrape.title.contains("&amp")
                    && !scrape.title.contains("&quot")
                    && !scrape.title.contains("&squot")
            );
            assert!(!scrape.url.raw().contains("&amp"));
            assert!(scrape.date.year() >= 2022 && scrape.date.year() <= 2024);
        }
    }
}
