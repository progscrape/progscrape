use std::str::Chars;

use regex::Regex;
use url::Url;

const IGNORED_QUERY_PARAMS: [&'static str; 13] = [
"utm_source",
"utm_medium",
"utm_campaign",
"utm_term",
"utm_content",
"utm_expid",
"gclid",
"_ga",
"_gl",
"msclkid",
"fbclid",
"mc_cid",
"mc_eid",
];

#[derive(Debug, PartialEq)]
pub struct CompareToken<'a>(&'a str);

/// We will need to use this if we end up with a non-unescaping URL parser. Not currently used, but tested at a basic level.
#[derive(Debug)]
pub struct EscapedCompareToken<'a>(&'a str);

impl <'a> PartialEq for EscapedCompareToken<'a> {
    fn eq(&self, other: &Self) -> bool {
        fn consume_with_escape(c: char, ci: &mut Chars) -> char {
            const HEX_DIGIT: &'static str = "0123456789abcdef0123456789ABCDEF";
            if c == '+' {
                return ' ';
            }
            if c != '%' {
                return c;
            }
            let a = ci.next().unwrap_or_default();
            let a = HEX_DIGIT.find(a).unwrap_or_default() as u8;
            let b = ci.next().unwrap_or_default();
            let b = HEX_DIGIT.find(b).unwrap_or_default() as u8;
            return ((a << 4) | b) as char;
        }

        if self.0 == other.0 {
            return true;
        }
        let mut it1 = self.0.chars();
        let mut it2 = other.0.chars();
        while let Some(c) = it1.next() {
            let c = consume_with_escape(c, &mut it1);
            let c2 = it2.next().unwrap_or_default();
            let c2 = consume_with_escape(c2, &mut it2);
            if c != c2 {
                return false;
            }
        }
        return it2.next().is_none();
    }
}

/// Generates a stream of token bits that can be used to compare whether URLs are "normalized-equal", that is: whether two URLs normalize to the same stream of tokens.
pub fn token_stream(url: &Url) -> impl Iterator<Item = CompareToken> {
    let re = Regex::new("www?[0-9]*\\.").expect("Failed to parse regular expression");
    let re_query_param = Regex::new(&IGNORED_QUERY_PARAMS.join("|")).expect("Failed to parse regular expression");

    let mut out = vec![];
    let host = url.host_str().unwrap_or_default();
    if let Some(stripped) = re.find_at(host, 0) {
        out.push(CompareToken(&host[stripped.end()..host.len()]));
    } else {
        out.push(CompareToken(host));
    }
    let path = url.path_segments();
    if let Some(path) = path {
        let mut iter = path.filter(|path| !path.is_empty());
        if let Some(mut curr) = iter.next() {
            loop {
                if let Some(next) = iter.next() {
                    out.push(CompareToken(curr));
                    curr = next;
                } else {
                    // Remove anything that looks like a trailing file type (.html, etc)
                    if let Some((a, b)) = curr.rsplit_once('.') {
                        if b.len() <= 6 {
                            out.push(CompareToken(a));
                        } else {
                            out.push(CompareToken(curr));
                        }
                    } else {
                        out.push(CompareToken(curr));
                    }
                    break;
                }
            }
        }
    }

    if let Some(query) = url.query() {
        let mut query_pairs = vec![];
        for bit in query.split('&') {
            if let Some((a, b)) = bit.split_once('=') {
                query_pairs.push((a, b));
            } {
                query_pairs.push((bit, ""));
            }
        }
        query_pairs.sort();
        for (key, value) in query_pairs {
            if !re_query_param.is_match(key) {
                out.push(CompareToken(&key));
                out.push(CompareToken(&value));
            }
        }
    }
    out.into_iter()
}

pub fn urls_are_same(a: &Url, b: &Url) -> bool {
    itertools::equal(token_stream(a), token_stream(b))
}

pub fn url_normalization_string(url: &Url) -> String {
    let mut s = String::with_capacity(url.as_str().len());
    for bit in token_stream(url) {
        s += bit.0;
        s.push(':');
    }
    s
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("abc", "abc")]
    #[case("abc.", "abc.")]
    #[case("ab+c", "ab c")]
    #[case("ab%2ec", "ab.c")]
    fn test_compare_token(#[case] a: &str, #[case] b: &str) {
        let a = EscapedCompareToken(a);
        let b = EscapedCompareToken(b);
        assert_eq!(a, b);
    }

    #[rstest]
    #[case("abc", "abc.")]
    #[case("abc.", "abc")]
    #[case("abc", "abc%")]
    #[case("abc", "abc%xx")]
    #[case("ab+c", "ab  c")]
    #[case("ab%2ec", "ab/c")]
    fn test_compare_token_ne(#[case] a: &str, #[case] b: &str) {
        let a = EscapedCompareToken(a);
        let b = EscapedCompareToken(b);
        assert_ne!(a, b);
    }

    /// Test identical URLs on both sides.
    #[rstest]
    #[case("http://x.com")]
    #[case("http://1.2.3.4")]
    #[case("http://google.com/path/?query")]
    #[case("http://google.com/path/?query=bar")]
    #[case("http://facebook.com/path/?fbclid=bar&somequery=ok")]
    fn test_url_normalization_identical(#[case] a: &str) {
        assert!(urls_are_same(&Url::parse(a).unwrap(), &Url::parse(a).unwrap()), "{} != {}", a, a);
    }

    #[rstest]
    // http/https
    #[case("http://google.com", "https://google.com")]
    // Escaped period
    #[case("http://google%2ecom", "https://google.com")]
    // www.
    #[case("https://www.google.com", "https://google.com")]
    // .html
    #[case("https://www.google.com/foo.html", "https://www.google.com/foo")]
    // Trailing/multiple slashes
    #[case("https://www.google.com/", "https://www.google.com")]
    #[case("https://www.google.com/foo", "https://www.google.com/foo/")]
    #[case("https://www.google.com//foo", "https://www.google.com/foo")]
    // Ignored query params
    #[case("http://x.com?utm_source=foo", "http://x.com")]
    #[case("http://x.com?fbclid=foo&gclid=bar", "http://x.com")]
    #[case("http://x.com?fbclid=foo", "http://x.com?fbclid=basdf")]
    fn test_url_normalization_same(#[case] a: &str, #[case] b: &str) {
        let a = Url::parse(a).unwrap();
        let b = Url::parse(b).unwrap();
        assert!(urls_are_same(&a, &b), "{} != {}", a, b);
        assert_eq!(url_normalization_string(&a), url_normalization_string(&b));
    }

    #[rstest]
    #[case("http://1.2.3.4", "http://1.2.3.5")]
    #[case("https://google.com", "https://facebook.com")]
    #[case("https://google.com/abc", "https://google.com/def")]
    #[case("https://google.com/?page=1", "https://google.com/?page=2")]
    #[case("https://google.com/?page=%31", "https://google.com/?page=%32")]
    fn test_url_normalization_different(#[case] a: &str, #[case] b: &str) {
        let a = Url::parse(a).unwrap();
        let b = Url::parse(b).unwrap();
        assert!(!urls_are_same(&a, &b), "{} != {}", a, b);
        assert_ne!(url_normalization_string(&a), url_normalization_string(&b));
    }
}