//! Word and sentence boundaries over a flow's text (UAX #29, as ICU segments
//! it for Chrome). Positions are UTF-8 offsets into that text.

use icu_segmenter::{SentenceSegmenter, WordSegmenter};

/// The end of the next word after `from`. Punctuation runs count as words;
/// white space and atomic objects do not.
pub(super) fn next_word_end(text: &str, from: usize) -> Option<usize> {
    words(text)
        .into_iter()
        .find(|&(start, end)| end > from && is_word(&text[start..end]))
        .map(|(_, end)| end)
}

/// The start of the last word before `from`.
pub(super) fn previous_word_start(text: &str, from: usize) -> Option<usize> {
    words(text)
        .into_iter()
        .rev()
        .find(|&(start, end)| start < from && is_word(&text[start..end]))
        .map(|(start, _)| start)
}

/// The first sentence boundary after `from`, excluding the text's start.
pub(super) fn next_sentence(text: &str, from: usize) -> Option<usize> {
    SentenceSegmenter::new(Default::default())
        .segment_str(text)
        .find(|&boundary| boundary > from)
}

/// The last sentence boundary before `from`.
pub(super) fn previous_sentence(text: &str, from: usize) -> Option<usize> {
    SentenceSegmenter::new(Default::default())
        .segment_str(text)
        .take_while(|&boundary| boundary < from)
        .last()
}

fn words(text: &str) -> Vec<(usize, usize)> {
    let boundaries: Vec<usize> = WordSegmenter::new_for_non_complex_scripts(Default::default())
        .segment_str(text)
        .collect();
    boundaries.windows(2).map(|w| (w[0], w[1])).collect()
}

fn is_word(segment: &str) -> bool {
    segment
        .chars()
        .any(|ch| !ch.is_whitespace() && ch != '\u{FFFC}')
}
