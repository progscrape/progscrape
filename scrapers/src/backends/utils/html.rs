use tl::{HTMLTag, NodeHandle, Parser};

/// Takes an Option<QuerySelectorIterator> and makes it return a stream of nodes.
pub fn html_tag_iterator<'a, T: IntoIterator<Item = NodeHandle> + 'a>(
    p: &'a Parser<'a>,
    it: Option<T>,
) -> impl Iterator<Item = &'a HTMLTag<'a>> + 'a {
    let it = Iterator::flatten(it.into_iter().map(|x| x.into_iter()));
    it.filter_map(|node| node.get(p).and_then(|node| node.as_tag()))
}

/// Find the first child node matching the selector.
pub fn find_first<'a>(
    p: &'a Parser<'a>,
    parent: &'a HTMLTag,
    selector: &'static str,
) -> Option<&'a HTMLTag<'a>> {
    html_tag_iterator(p, parent.query_selector(p, selector)).next()
}

pub fn get_attribute<'a>(
    _p: &'a Parser<'a>,
    parent: &'a HTMLTag,
    attribute: &'static str,
) -> Option<String> {
    parent
        .attributes()
        .get(attribute)
        .unwrap_or_default()
        .map(|f| f.as_utf8_str().into())
}

/// This method will unescape standard HTML entities. It is limited to a subset of the most common entities and the decimal/hex
/// escapes for arbitrary characters. It will attempt to pass through any entity that doesn't match.
pub fn unescape_entities(input: &str) -> String {
    const ENTITIES: [(&str, &str); 6] = [
        ("amp", "&"),
        ("lt", "<"),
        ("gt", ">"),
        ("quot", "\""),
        ("squot", "'"),
        ("nbsp", "\u{00a0}"),
    ];
    let mut s = String::new();
    let mut entity = false;
    let mut entity_name = String::new();
    'char: for c in input.chars() {
        if entity {
            if c == ';' {
                entity = false;
                if entity_name.starts_with("#x") {
                    if let Ok(n) = u32::from_str_radix(&entity_name[2..entity_name.len()], 16) {
                        if let Some(c) = char::from_u32(n) {
                            s.push(c);
                            entity_name.clear();
                            continue 'char;
                        }
                    }
                } else if entity_name.starts_with('#') {
                    if let Ok(n) = str::parse(&entity_name[1..entity_name.len()]) {
                        if let Some(c) = char::from_u32(n) {
                            s.push(c);
                            entity_name.clear();
                            continue 'char;
                        }
                    }
                } else {
                    for (name, value) in ENTITIES {
                        if entity_name == name {
                            s += value;
                            entity_name.clear();
                            continue 'char;
                        }
                    }
                }
                s += &format!("&{};", entity_name);
                entity_name.clear();
                continue 'char;
            }
            entity_name.push(c);
        } else if c == '&' {
            entity = true;
        } else {
            s.push(c);
        }
    }
    if !entity_name.is_empty() {
        s += &format!("&{}", entity_name);
    }
    s
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("a b", "a b")]
    #[case("a&amp;b", "a&b")]
    #[case("a&#x27;b", "a'b")]
    #[case("a&#160;b", "a\u{00a0}b")]
    #[case("a&squot;&quot;b", "a'\"b")]
    fn test_unescape(#[case] a: &str, #[case] b: &str) {
        assert_eq!(unescape_entities(a), b.to_owned());
    }

    #[rstest]
    #[case("a&amp")]
    #[case("a&fake;")]
    #[case("a?a=b&b=c")]
    fn test_bad_escape(#[case] a: &str) {
        assert_eq!(unescape_entities(a), a.to_owned());
    }
}
