//! Bounded byte-level encoding discovery before markup is decoded.
//! https://html.spec.whatwg.org/multipage/parsing.html#prescan-a-byte-stream-to-determine-its-encoding
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// Prescan only the first 1024 bytes; comments and non-meta attributes cannot
/// introduce a declaration. Attribute values deliberately do not decode entities.
pub fn prescan_html_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let bytes = &bytes[..bytes.len().min(1024)];
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"<!--") {
            at += 4;
            while at < bytes.len() && !bytes[at..].starts_with(b"-->") {
                at += 1;
            }
            at = (at + 3).min(bytes.len());
            continue;
        }
        if bytes[at] != b'<' {
            at += 1;
            continue;
        }
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_alphanumeric) {
            at += 1;
        }
        let meta = bytes[start..at].eq_ignore_ascii_case(b"meta")
            && bytes
                .get(at)
                .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'));
        if start == at {
            while bytes.get(at).is_some_and(|b| *b != b'>') {
                at += 1;
            }
            continue;
        }
        let attributes = attributes(bytes, &mut at, 512);
        if !meta {
            continue;
        }
        let mut pragma = false;
        let mut needs_pragma = false;
        let mut encoding = None;
        for (name, value) in attributes {
            match name.as_slice() {
                b"http-equiv" => pragma |= value.eq_ignore_ascii_case(b"content-type"),
                b"content" if encoding.is_none() => {
                    encoding = content_encoding(&value);
                    if encoding.is_some() {
                        needs_pragma = true;
                    }
                }
                b"charset" => {
                    encoding = Encoding::for_label(&value);
                    needs_pragma = false;
                }
                _ => {}
            }
        }
        if needs_pragma && !pragma {
            continue;
        }
        if let Some(encoding) = encoding {
            return Some(if encoding == UTF_16BE || encoding == UTF_16LE {
                UTF_8
            } else if encoding == encoding_rs::X_USER_DEFINED {
                WINDOWS_1252
            } else {
                encoding
            });
        }
    }
    None
}

/// XML byte-order signatures take precedence over an ASCII declaration.
/// The caller supplies the transport encoding when one exists.
pub fn detect_xml_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return Some(encoding);
    }
    if bytes.starts_with(b"\0<\0?") {
        return Some(UTF_16BE);
    }
    if bytes.starts_with(b"<\0?\0") {
        return Some(UTF_16LE);
    }
    if !bytes.starts_with(b"<?xml") || !bytes.get(5).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let end = bytes.windows(2).position(|w| w == b"?>")?;
    let mut at = 5;
    attributes(&bytes[..end], &mut at, 3)
        .into_iter()
        .find(|(name, _)| name == b"encoding")
        .and_then(|(_, label)| Encoding::for_label(&label))
}

fn content_encoding(value: &[u8]) -> Option<&'static Encoding> {
    for start in 0..value.len() {
        let tail = &value[start..];
        if !tail
            .get(..7)
            .is_some_and(|v| v.eq_ignore_ascii_case(b"charset"))
        {
            continue;
        }
        let mut at = 7;
        skip_space(tail, &mut at);
        if tail.get(at) != Some(&b'=') {
            continue;
        }
        at += 1;
        skip_space(tail, &mut at);
        let rest = &tail[at..];
        let label = if let Some(quote) = rest.first().filter(|b| matches!(b, b'\'' | b'"')) {
            let end = rest[1..].iter().position(|b| b == quote)?;
            &rest[1..end + 1]
        } else {
            let end = rest
                .iter()
                .position(|b| b.is_ascii_whitespace() || *b == b';')
                .unwrap_or(rest.len());
            &rest[..end]
        };
        return Encoding::for_label(label);
    }
    None
}

fn attributes(bytes: &[u8], at: &mut usize, limit: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut values = Vec::new();
    for _ in 0..limit {
        if *at >= bytes.len() {
            break;
        }
        skip_space(bytes, at);
        if bytes.get(*at) == Some(&b'>') {
            *at += 1;
            break;
        }
        if bytes.get(*at) == Some(&b'/') {
            *at += 1;
            continue;
        }
        let start = *at;
        while bytes
            .get(*at)
            .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'/' | b'>'))
        {
            *at += 1;
        }
        let name = bytes[start..*at].to_ascii_lowercase();
        skip_space(bytes, at);
        let value = if bytes.get(*at) == Some(&b'=') {
            *at += 1;
            skip_space(bytes, at);
            attribute_value(bytes, at)
        } else {
            Vec::new()
        };
        if !name.is_empty() && !values.iter().any(|(old, _)| *old == name) {
            values.push((name, value));
        }
        if *at == start {
            *at += 1;
        }
    }
    values
}
fn attribute_value(bytes: &[u8], at: &mut usize) -> Vec<u8> {
    let quote = bytes
        .get(*at)
        .copied()
        .filter(|b| matches!(b, b'\'' | b'"'));
    if quote.is_some() {
        *at += 1;
    }
    let start = *at;
    while bytes.get(*at).is_some_and(|b| match quote {
        Some(q) => *b != q,
        None => !b.is_ascii_whitespace() && *b != b'>',
    }) {
        *at += 1;
    }
    let value = bytes[start..*at].to_vec();
    if quote.is_some() && *at < bytes.len() {
        *at += 1;
    }
    value
}
fn skip_space(bytes: &[u8], at: &mut usize) {
    while bytes.get(*at).is_some_and(u8::is_ascii_whitespace) {
        *at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_declarations_respect_comments_attributes_pragmas_and_bounds() {
        for source in [b"<meta charset=windows-1252>".as_slice(), b"<!--<meta charset=utf-8>--><META content='text/html; charset=windows-1252' HTTP-EQUIV=content-type>", b"<div data='<meta charset=utf-8>'><meta charset=windows-1252>"] {
            assert_eq!(prescan_html_encoding(source), Some(WINDOWS_1252));
        }
        assert_eq!(
            prescan_html_encoding(b"<meta content='text/html; charset=windows-1252'>"),
            None
        );
        assert_eq!(
            prescan_html_encoding(b"<meta charset=utf-16le>"),
            Some(UTF_8)
        );
        assert_eq!(
            prescan_html_encoding(b"<meta charset=x-user-defined>"),
            Some(WINDOWS_1252)
        );
        assert_eq!(
            prescan_html_encoding(b"<meta charset=utf-8 charset=windows-1252>"),
            Some(UTF_8)
        );
        let mut late = vec![b' '; 1024];
        late.extend_from_slice(b"<meta charset=windows-1252>");
        assert_eq!(prescan_html_encoding(&late), None);
    }
    #[test]
    fn meta_content_encoding_obeys_parameter_delimiters() {
        assert_eq!(prescan_html_encoding(b"<meta http-equiv='content-type' content='text/html; charset=windows-1252; foo=bar'>"),Some(WINDOWS_1252));
        assert_eq!(
            prescan_html_encoding(
                b"<meta http-equiv='content-type' content=\"text/html; charset='windows-1252\">"
            ),
            None
        );
    }
    #[test]
    fn xml_signatures_and_declarations_select_encoding() {
        assert_eq!(
            detect_xml_encoding(b"<?xml version='1.0' encoding='windows-1252'?><root/>"),
            Some(WINDOWS_1252)
        );
        assert_eq!(detect_xml_encoding(b"\xff\xfe<\0r\0"), Some(UTF_16LE));
        assert_eq!(detect_xml_encoding(b"\0<\0?"), Some(UTF_16BE));
        assert_eq!(
            detect_xml_encoding(b"<root encoding='windows-1252'/>"),
            None
        );
        assert_eq!(
            detect_xml_encoding(b"<?xml version='1.0' encoding='unknown'?><root/>"),
            None
        );
    }
}
