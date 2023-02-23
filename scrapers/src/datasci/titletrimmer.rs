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

    while let Some((left, right)) = title.rsplit_once(" | ") {
        title = left;
        if title.len() <= ideal_length {
            return Cow::Borrowed(title);
        }
    }

    while let Some((left, right)) = title.rsplit_once(". ") {
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

#[cfg(test)]
mod test {
    use crate::datasci::titletrimmer::{trim_title, AWKWARD_LENGTH, IDEAL_LENGTH};

    fn split_test_data(s: &str) -> Vec<String> {
        let mut out = vec![];
        for s in s.split("\n") {
            let s = s.trim();
            if s.starts_with("#") {
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
}
