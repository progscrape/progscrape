use tl::{HTMLTag, NodeHandle, Parser, ParserOptions};

use super::unescape_entities;

/// Takes an Option<QuerySelectorIterator> and makes it return a stream of nodes.
pub fn html_tag_iterator<'a, T: IntoIterator<Item = NodeHandle> + 'a>(
    p: &'a Parser<'a>,
    it: Option<T>,
) -> impl Iterator<Item = &'a HTMLTag> + 'a {
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
