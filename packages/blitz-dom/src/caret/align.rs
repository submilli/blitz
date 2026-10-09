//! Alignment of DOM characters with the text Parley laid out.
//!
//! Layout construction pushes Text node data (after `text-transform`) to
//! Parley, which collapses white space as it builds. Matching characters in
//! order recovers which DOM characters were rendered and where.

use icu_segmenter::GraphemeClusterSegmenter;
use std::ops::Range;
use style::computed_values::{visibility::T as Visibility, white_space_collapse::T as WhiteSpace};
use style::properties::ComputedValues;
use style::values::computed::TextTransform;

/// The inherited properties that decide how a Text node renders.
#[derive(Clone, Copy, Debug)]
pub(super) struct TextStyle {
    pub(super) visible: bool,
    white_space: WhiteSpace,
    transform: TextTransform,
}

impl TextStyle {
    /// The style of a Text node's parent; text without one renders as
    /// layout's default, collapsing white space.
    pub(super) fn of(style: Option<&ComputedValues>) -> Self {
        Self {
            visible: style.is_some_and(|s| s.clone_visibility() == Visibility::Visible),
            white_space: style.map_or(WhiteSpace::Collapse, |s| s.clone_white_space_collapse()),
            transform: style.map_or(TextTransform::NONE, |s| {
                s.clone_text_transform() & TextTransform::CASE_TRANSFORMS
            }),
        }
    }

    /// Preserved text, such as the line feed of a `<br>`.
    pub(super) fn preserved() -> Self {
        Self {
            visible: true,
            white_space: WhiteSpace::Preserve,
            transform: TextTransform::NONE,
        }
    }

    /// Whether `ch` is white space this style may collapse, as Parley
    /// decides it: all ASCII white space, or only spaces and tabs where
    /// segment breaks are preserved.
    pub(super) fn collapsible(&self, ch: char) -> bool {
        match self.white_space {
            WhiteSpace::Collapse => ch.is_ascii_whitespace(),
            WhiteSpace::PreserveBreaks => matches!(ch, ' ' | '\t'),
            _ => false,
        }
    }
}

/// A cursor over one layout's text.
pub(super) struct Aligner<'a> {
    text: &'a str,
    position: usize,
    graphemes: Vec<usize>,
}

impl<'a> Aligner<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self {
            text,
            position: 0,
            graphemes: GraphemeClusterSegmenter::new().segment_str(text).collect(),
        }
    }

    /// The laid-out range of one DOM character, or `None` when layout
    /// collapsed it away. Collapsible white space matches one laid-out space;
    /// other characters must match their transformed form exactly.
    pub(super) fn char(&mut self, ch: char, style: TextStyle) -> Option<Range<usize>> {
        let rest = &self.text[self.position..];
        let start = self.position;
        if style.collapsible(ch) {
            if !rest.starts_with(' ') {
                return None;
            }
            self.position += 1;
            return Some(start..self.position);
        }
        let mut matched = 0;
        for want in transformed(ch, style.transform) {
            let found = rest[matched..].chars().next()?;
            // Contextual lowercasing gives a word-final sigma.
            if found != want && !(want == 'σ' && found == 'ς') {
                return None;
            }
            matched += found.len_utf8();
        }
        if matched == 0 {
            return None;
        }
        self.position += matched;
        Some(start..self.position)
    }

    pub(super) fn position(&self) -> usize {
        self.position
    }

    pub(super) fn starts_grapheme(&self, byte: usize) -> bool {
        self.graphemes.binary_search(&byte).is_ok()
    }

    pub(super) fn slice(&self, range: Range<usize>) -> &'a str {
        &self.text[range]
    }
}

/// A character after a case `text-transform`, as layout construction pushed it.
fn transformed(ch: char, transform: TextTransform) -> Transformed {
    if transform.contains(TextTransform::UPPERCASE) {
        Transformed::Upper(ch.to_uppercase())
    } else if transform.contains(TextTransform::LOWERCASE) {
        Transformed::Lower(ch.to_lowercase())
    } else {
        Transformed::Plain(Some(ch))
    }
}

enum Transformed {
    Upper(std::char::ToUppercase),
    Lower(std::char::ToLowercase),
    Plain(Option<char>),
}

impl Iterator for Transformed {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self {
            Self::Upper(chars) => chars.next(),
            Self::Lower(chars) => chars.next(),
            Self::Plain(ch) => ch.take(),
        }
    }
}
