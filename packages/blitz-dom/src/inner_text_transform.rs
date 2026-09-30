//! CSS case transformation of scalar runs, retaining original lone surrogates.
use crate::DomString;
use icu_casemap::{
    CaseMapper, TitlecaseMapper,
    options::{TitlecaseOptions, TrailingCase},
};
use icu_locale_core::LanguageIdentifier;
use icu_properties::{CodePointMapData, props::GeneralCategory};
use style::values::computed::TextTransform;

pub(super) fn apply(
    text: &DomString,
    transform: TextTransform,
    language: &str,
    word_start: &mut bool,
) -> DomString {
    let language = language
        .parse::<LanguageIdentifier>()
        .unwrap_or(icu_locale_core::langid!("und"));
    let mut result = DomString::new();
    let mut scalar_run = String::new();
    for decoded in char::decode_utf16(text.to_utf16().iter().copied()) {
        match decoded {
            Ok(ch) => scalar_run.push(ch),
            Err(error) => {
                append_run(&mut result, &scalar_run, transform, &language, word_start);
                scalar_run.clear();
                result.push_units(&[error.unpaired_surrogate()]);
                *word_start = false;
            }
        }
    }
    append_run(&mut result, &scalar_run, transform, &language, word_start);
    result
}

fn append_run(
    result: &mut DomString,
    run: &str,
    transform: TextTransform,
    language: &LanguageIdentifier,
    word_start: &mut bool,
) {
    if transform.contains(TextTransform::CAPITALIZE) {
        capitalize(result, run, language, word_start);
        return;
    }
    let mapper = CaseMapper::new();
    if transform.contains(TextTransform::UPPERCASE) {
        result.push_str(&mapper.uppercase_to_string(run, language));
    } else if transform.contains(TextTransform::LOWERCASE) {
        // Whole runs retain the context needed for mappings such as final sigma.
        result.push_str(&mapper.lowercase_to_string(run, language));
    } else {
        result.push_str(run);
    }
    for ch in run.chars() {
        update_word_start(ch, word_start);
    }
}

fn capitalize(
    result: &mut DomString,
    run: &str,
    language: &LanguageIdentifier,
    word_start: &mut bool,
) {
    let mapper = TitlecaseMapper::new();
    let mut options = TitlecaseOptions::default();
    options.trailing_case = Some(TrailingCase::Unchanged);
    for ch in run.chars() {
        let mut buffer = [0; 4];
        let text = ch.encode_utf8(&mut buffer);
        if *word_start {
            result.push_str(&mapper.titlecase_segment_to_string(text, language, options));
        } else {
            result.push_str(text);
        }
        update_word_start(ch, word_start);
    }
}

fn update_word_start(ch: char, word_start: &mut bool) {
    let category = CodePointMapData::<GeneralCategory>::new().get(ch);
    match category {
        // Combining marks and apostrophes continue the current word, including
        // when the combining sequence crosses DOM Text node boundaries.
        GeneralCategory::NonspacingMark
        | GeneralCategory::SpacingMark
        | GeneralCategory::EnclosingMark => {}
        _ if matches!(ch, '\'' | '’') => {}
        GeneralCategory::ConnectorPunctuation => *word_start = false,
        _ => *word_start = !ch.is_alphanumeric(),
    }
}
