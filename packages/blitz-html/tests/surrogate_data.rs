//! Original UTF-16 survives DOM mutations and DOMString serialization.
use blitz_dom::{DocumentConfig, DomString, mutations::MutationRecord};
use blitz_html::HtmlDocument;

#[test]
fn storage_clone_and_serialization_preserve_code_units() {
    let mut doc =
        HtmlDocument::from_html("<div></div><script></script>", DocumentConfig::default());
    let parent = doc.query_selector("div").unwrap().unwrap();
    let data = DomString::from_utf16(vec![0xD83D, 38, 0xDE00]);
    let text = doc.mutate().create_text_node(&data);
    doc.mutate().append_children(parent, &[text]);
    assert_eq!(
        doc.character_data_dom(text).unwrap().to_utf16().as_ref(),
        [0xD83D, 38, 0xDE00]
    );
    assert_eq!(doc.character_data(text), Some("�&�"));
    // Reading the scalar projection cannot alter the DOMString.
    assert_eq!(doc.text_content_dom(parent).unwrap(), data);
    let html = doc.get_node(parent).unwrap().inner_html_dom();
    assert_eq!(
        html.to_utf16().as_ref(),
        [0xD83D, 38, 97, 109, 112, 59, 0xDE00]
    );
    let clone = doc.mutate().clone_node(parent, true);
    assert_eq!(doc.get_node(clone).unwrap().inner_html_dom(), html);
    let script = doc.query_selector("script").unwrap().unwrap();
    doc.mutate().set_text_content(script, &data);
    assert_eq!(doc.get_node(script).unwrap().inner_html_dom(), data);
}

#[test]
fn comments_and_instructions_keep_data_and_mutation_old_values() {
    let mut doc = HtmlDocument::from_html("<div></div>", DocumentConfig::default());
    let data = DomString::from_utf16(vec![0xD800]);
    let comment = doc.mutate().create_comment_node(&data);
    let pi = doc.mutate().create_processing_instruction("x", &data);
    for id in [comment, pi] {
        let clone = doc.mutate().clone_node(id, false);
        assert_eq!(doc.character_data_dom(clone), Some(&data));
        let markup = doc.get_node(id).unwrap().outer_html_dom();
        assert!(markup.to_utf16().contains(&0xD800));
    }
    doc.set_mutation_recording(true);
    doc.mutate().set_character_data(comment, "replacement");
    doc.mutate().set_character_data(pi, "replacement");
    assert_eq!(
        doc.take_mutation_records(),
        vec![
            MutationRecord::CharacterData {
                target: comment,
                old_value: data.clone()
            },
            MutationRecord::CharacterData {
                target: pi,
                old_value: data
            },
        ]
    );
}

#[test]
fn descendant_collection_joins_halves_before_scalar_conversion() {
    let mut doc = HtmlDocument::from_html("<div></div>", DocumentConfig::default());
    let parent = doc.query_selector("div").unwrap().unwrap();
    let hi = doc
        .mutate()
        .create_text_node(DomString::from_utf16(vec![0xD83D]));
    let lo = doc
        .mutate()
        .create_text_node(DomString::from_utf16(vec![0xDE00]));
    doc.mutate().append_children(parent, &[hi, lo]);
    assert_eq!(
        doc.text_content_dom(parent).unwrap().to_utf16().as_ref(),
        [0xD83D, 0xDE00]
    );
    assert_eq!(doc.text_content_of(parent).as_deref(), Some("😀"));
    assert_eq!(doc.get_node(parent).unwrap().text_content(), "😀");
}
