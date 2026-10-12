//! How deep a source's elements nest, and where it refers to entities, read
//! before an XML parser runs: roxmltree recurses once per element level.
//!
//! The scan follows XML's markup ([XML 1.0 §2.4–2.8]): comments, CDATA
//! sections, processing instructions and the document type declaration hold
//! no elements and expand no references; quoted attribute values may hold
//! `>`. Malformed markup is read so that depths are never lower than a
//! parser's.
//!
//! [XML 1.0 §2.4–2.8]: https://www.w3.org/TR/xml/#syntax

/// The elements and references of one source.
#[derive(Debug, Default)]
pub(super) struct Markup<'a> {
    /// The deepest element nesting.
    pub depth: usize,
    /// Each entity reference, with the depth of the element it is in.
    pub references: Vec<(&'a str, usize)>,
    /// Whether markup runs to the end of the source unclosed. A parser
    /// refuses such a source, but the scan cannot vouch for what it read:
    /// a stray quote could have hidden references.
    pub unterminated: bool,
}

/// Scan `text` for its element nesting and entity references.
pub(super) fn scan(text: &str) -> Markup<'_> {
    let mut markup = Markup::default();
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'<' => at = tag(text, at, &mut depth, &mut markup),
            b'&' => {
                markup.reference(text, at, depth);
                at += 1;
            }
            _ => at += 1,
        }
    }
    markup
}

impl<'a> Markup<'a> {
    /// Note the reference whose `&` is at `at`, if it is an entity's.
    fn reference(&mut self, text: &'a str, at: usize, depth: usize) {
        if let Some(name) = reference_name(&text[at + 1..]) {
            self.references.push((name, depth));
        }
    }
}

/// Read the markup starting with the `<` at `at`, and where it ends.
fn tag<'a>(text: &'a str, at: usize, depth: &mut usize, markup: &mut Markup<'a>) -> usize {
    let rest = &text[at..];
    // Markup holding no elements: skipped whole.
    let inert = if rest.starts_with("<!--") {
        Some(past(text, at, "-->"))
    } else if rest.starts_with("<![CDATA[") {
        Some(past(text, at, "]]>"))
    } else if rest.starts_with("<?") {
        Some(past(text, at, "?>"))
    } else if rest.starts_with("<!") {
        Some(declaration_end(text, at))
    } else {
        None
    };
    let end = inert.unwrap_or_else(|| tag_end(text, at));
    let Some(end) = end else {
        markup.unterminated = true;
        return text.len();
    };
    if inert.is_some() {
        return end;
    }
    // Attribute values expand references too.
    for (offset, _) in text[at..end].match_indices('&') {
        markup.reference(text, at + offset, *depth);
    }
    if rest.starts_with("</") {
        *depth = depth.saturating_sub(1);
    } else if !text[..end].ends_with("/>") {
        *depth += 1;
        markup.depth = markup.depth.max(*depth);
    }
    end
}

/// Where the markup at `at` ends, just past the first `close`.
fn past(text: &str, at: usize, close: &str) -> Option<usize> {
    text[at..].find(close).map(|found| at + found + close.len())
}

/// Where a start or end tag at `at` ends: past its `>`, skipping quoted
/// attribute values, or at the next `<` of malformed markup.
fn tag_end(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut quote = None;
    let mut index = at + 1;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None if matches!(byte, b'"' | b'\'') => quote = Some(byte),
            None if byte == b'>' => return Some(index + 1),
            None if byte == b'<' => return Some(index),
            None => {}
        }
        index += 1;
    }
    None
}

/// Where a declaration such as `<!DOCTYPE … [ … ]>` at `at` ends, skipping
/// its internal subset (whose entity values count where they are referred
/// to), with the comments and processing instructions in it, whose quotes
/// open nothing.
fn declaration_end(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut quote = None;
    let mut subset = false;
    let mut index = at + 2;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None if bytes[index..].starts_with(b"<!--") => {
                index = past(text, index, "-->")?;
                continue;
            }
            None if bytes[index..].starts_with(b"<?") => {
                index = past(text, index, "?>")?;
                continue;
            }
            None if matches!(byte, b'"' | b'\'') => quote = Some(byte),
            None if byte == b'[' => subset = true,
            None if byte == b']' => subset = false,
            None if byte == b'>' && !subset => return Some(index + 1),
            None => {}
        }
        index += 1;
    }
    None
}

/// The entity name a reference starting just after its `&` names; character
/// references (`&#…;`) expand to one character and are left out.
pub(super) fn reference_name(rest: &str) -> Option<&str> {
    let end = rest.find(|c: char| {
        c == ';' || matches!(c, ' ' | '\t' | '\n' | '\r' | '&' | '<' | '"' | '\'')
    })?;
    let name = &rest[..end];
    (rest[end..].starts_with(';') && !name.is_empty() && !name.starts_with('#')).then_some(name)
}
