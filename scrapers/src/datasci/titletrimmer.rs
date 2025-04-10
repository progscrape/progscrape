use std::borrow::Cow;

pub const IDEAL_LENGTH: usize = 150;
pub const AWKWARD_LENGTH: usize = 200;
const MAX_COMMA_CHOP: usize = 50;

pub const ENGLISH_SIMPLIFICATIONS: [(&str, &str); 4] = [
    ("A study of ", "Study of "),
    ("A review", "Review"),
    (" found that ", " found "),
    (" showed that ", " showed "),
];

/// Attempt to trim a title down to the ideal length by splitting, cutting out extraneous words, and other
/// optimizations.
pub fn trim_title(mut title: &str, ideal_length: usize, awkward_length: usize) -> Cow<str> {
    if title.len() <= ideal_length {
        return Cow::Borrowed(title);
    }

    while let Some((left, _right)) = title.rsplit_once(" | ") {
        title = left;
        if title.len() <= ideal_length {
            return Cow::Borrowed(title);
        }
    }

    while let Some((left, _right)) = title.rsplit_once(". ") {
        title = left;
        if title.len() <= ideal_length {
            return Cow::Borrowed(title);
        }
    }

    if title.len() <= awkward_length {
        return Cow::Borrowed(title);
    }

    // At this point we're getting aggressive...
    let mut title = title.to_owned();

    for (left, right) in ENGLISH_SIMPLIFICATIONS {
        title = title.replace(left, right);
    }

    if title.len() <= awkward_length {
        return Cow::Owned(title);
    }

    while let Some((left, right)) = title.rsplit_once(", ") {
        if right.len() > MAX_COMMA_CHOP {
            break;
        }
        title = left.to_owned();
        title += "…";
        if title.len() <= awkward_length {
            return Cow::Owned(title);
        }
    }

    Cow::Owned(title)
}

/// Removes a `[tag]` from the beginning and/or end of a title.
pub fn remove_tags(title: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut start_tag = None;
    let mut end_tag = None;

    // Helper function to check if a string is a valid tag
    fn is_valid_tag(tag: &str) -> bool {
        !tag.is_empty() && tag.len() < 10 && tag.chars().all(|c| c.is_ascii_alphabetic())
    }

    let mut title = title.trim();
    let orig_title = title;

    // Check and remove tag at the start
    if title.starts_with('[') {
        if let Some(end_index) = title.find(']') {
            let possible_tag = &title[1..end_index];
            if is_valid_tag(possible_tag) {
                start_tag = Some(possible_tag);
                title = &title[end_index + 1..];
            }
        }
    }

    if title.trim().is_empty() {
        return (orig_title, None, None);
    }

    // Check and remove tag at the end
    if title.ends_with(']') {
        if let Some(start_index) = title.rfind('[') {
            let possible_tag = &title[start_index + 1..title.len() - 1];
            if is_valid_tag(possible_tag) {
                end_tag = Some(possible_tag);
                title = &title[..start_index - 1];
            }
        }
    }

    if title.trim().is_empty() {
        return (orig_title, None, None);
    }

    (title.trim(), start_tag, end_tag)
}

#[cfg(test)]
mod test {
    use rstest::rstest;

    use crate::datasci::titletrimmer::{trim_title, AWKWARD_LENGTH, IDEAL_LENGTH};

    use super::remove_tags;

    fn split_test_data(s: &str) -> Vec<String> {
        let mut out = vec![];
        for s in s.split('\n') {
            let s = s.trim();
            if s.starts_with('#') {
                continue;
            }
            out.push(s.to_owned());
        }
        out
    }

    #[test]
    fn test_reddit_samples() {
        let input = split_test_data(include_str!("../../testdata/titles/reddit-input.txt"));
        let output = split_test_data(include_str!("../../testdata/titles/reddit-output.txt"));

        assert_eq!(input.len(), output.len());
        for (input, output) in std::iter::zip(input, output) {
            let actual = trim_title(&input, IDEAL_LENGTH, AWKWARD_LENGTH);
            assert_eq!(output, actual);
        }
    }

    #[rstest]
    #[case("This is a title [tag]", "This is a title", None, Some("tag"))]
    #[case("[tag] This is a title", "This is a title", Some("tag"), None)]
    #[case(
        "[longtag] This is a title [anotherlongtag]",
        "This is a title [anotherlongtag]",
        Some("longtag"),
        None
    )]
    #[case(
        "[short] This is a title [valid]",
        "This is a title",
        Some("short"),
        Some("valid")
    )]
    #[case(
        " [short]  This is a title  [valid] ",
        "This is a title",
        Some("short"),
        Some("valid")
    )]
    #[case("[bad]", "[bad]", None, None)]
    #[case(" [bad] ", "[bad]", None, None)]
    fn test_remove_tags(
        #[case] input: &str,
        #[case] expected_title: &str,
        #[case] start_tag: Option<&str>,
        #[case] end_tag: Option<&str>,
    ) {
        let (title, start, end) = remove_tags(input);
        assert_eq!(title, expected_title);
        assert_eq!(start, start_tag);
        assert_eq!(end, end_tag);
    }
}
