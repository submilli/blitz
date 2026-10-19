//! DOM processing-instruction validation, distinct from HTML local-name rules.
use crate::{DocumentMutator, DomString, NodeId, dom_api::DomError};

/// XML 1.0 fifth-edition Name production, evaluated before scalar projection.
/// <https://www.w3.org/TR/xml/#NT-Name>
pub fn is_valid_xml_name(name: &DomString) -> bool {
    let mut characters = char::decode_utf16(name.utf16_units());
    let Some(Ok(first)) = characters.next() else {
        return false;
    };
    name_start(first) && characters.all(|character| character.is_ok_and(name_character))
}

fn name_start(character: char) -> bool {
    matches!(character,
        ':' | '_' | 'A'..='Z' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}'
        | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}'
        | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}'
        | '\u{10000}'..='\u{effff}'
    )
}

fn name_character(character: char) -> bool {
    name_start(character)
        || matches!(character, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

impl DocumentMutator<'_> {
    /// DOM createProcessingInstruction validation before allocation. Parser
    /// internal construction uses its own validated-input path.
    /// <https://dom.spec.whatwg.org/#dom-document-createprocessinginstruction>
    pub fn try_create_processing_instruction_checked(
        &mut self,
        target: &DomString,
        data: DomString,
    ) -> Result<NodeId, DomError> {
        if !is_valid_xml_name(target) || has_terminator(&data) {
            return Err(DomError::InvalidCharacter);
        }
        self.try_create_processing_instruction(target.as_str_lossy(), data)
            .map_err(|_| DomError::QuotaExceeded)
    }
}

fn has_terminator(data: &DomString) -> bool {
    let mut question = false;
    for unit in data.utf16_units() {
        if question && unit == u16::from(b'>') {
            return true;
        }
        question = unit == u16::from(b'?');
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BaseDocument, DocumentConfig};

    #[test]
    fn xml_names_cover_start_and_continuation_boundaries() {
        for (start, end) in [
            (0xc0, 0xd6),
            (0xd8, 0xf6),
            (0xf8, 0x2ff),
            (0x370, 0x37d),
            (0x37f, 0x1fff),
            (0x200c, 0x200d),
            (0x2070, 0x218f),
            (0x2c00, 0x2fef),
            (0x3001, 0xd7ff),
            (0xf900, 0xfdcf),
            (0xfdf0, 0xfffd),
            (0x10000, 0xeffff),
        ] {
            for value in [start, end] {
                let c = char::from_u32(value).unwrap();
                assert!(is_valid_xml_name(&c.to_string().into()));
                assert!(is_valid_xml_name(&format!("a{c}").into()));
            }
        }
        for name in [
            "xml",
            "XML",
            "a:b:c",
            "_",
            ":",
            "a0.-\u{b7}\u{300}\u{36f}\u{203f}\u{2040}",
        ] {
            assert!(is_valid_xml_name(&name.into()), "{name:?}");
        }
        for name in [
            "",
            "0x",
            "-x",
            ".x",
            "a b",
            "a\0b",
            "a/b",
            "a>b",
            "a=b",
            "\u{b7}",
            "\u{300}",
            "\u{37e}",
            "\u{200b}",
            "\u{206f}",
            "\u{2ff0}",
            "\u{3000}",
            "\u{f8ff}",
            "\u{fdd0}",
            "\u{fffe}",
            "\u{f0000}",
        ] {
            assert!(!is_valid_xml_name(&name.into()), "{name:?}");
        }
        for units in [vec![0xd800], vec![0xdc00], vec![b'a' as u16, 0xd800]] {
            assert!(!is_valid_xml_name(&DomString::from_utf16(units)));
        }
    }

    #[test]
    fn invalid_processing_instructions_allocate_nothing() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let before = doc.node_count();
        for (target, data) in [("0bad", "data"), ("a", "x?>y"), ("a", "??>"), ("a b", "")] {
            assert_eq!(
                doc.mutate()
                    .try_create_processing_instruction_checked(&target.into(), data.into()),
                Err(DomError::InvalidCharacter)
            );
            assert_eq!(doc.node_count(), before);
        }
        assert_eq!(
            doc.mutate().try_create_processing_instruction_checked(
                &DomString::from_utf16(vec![0xd800]),
                "".into()
            ),
            Err(DomError::InvalidCharacter)
        );
        assert_eq!(doc.node_count(), before);
    }

    #[test]
    fn checked_creation_preserves_data_code_units_and_reports_quota() {
        let mut doc = BaseDocument::new(DocumentConfig {
            node_limit: Some(3),
            ..Default::default()
        });
        let data = DomString::from_utf16(vec![0xd800, b'?' as u16, 0xdc00, b'>' as u16]);
        let id = doc
            .mutate()
            .try_create_processing_instruction_checked(&"xml".into(), data.clone())
            .unwrap();
        assert_eq!(doc.node_name(id), "xml");
        assert_eq!(doc.character_data_dom(id), Some(&data));
        assert_eq!(
            doc.mutate()
                .try_create_processing_instruction_checked(&"valid".into(), "".into()),
            Err(DomError::QuotaExceeded)
        );
        assert_eq!(doc.node_count(), 3);
    }
}
