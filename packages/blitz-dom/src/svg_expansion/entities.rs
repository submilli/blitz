//! The source an XML parser expands a document to through its internal
//! entities ([XML 1.0 §4.4]).
//!
//! roxmltree expands each reference again, and an entity holding markup
//! builds its elements again: thousands of references to a nested entity
//! build millions of nodes from a few kilobytes. Its own guard only bounds
//! the references one top-level reference expands.
//!
//! Declarations are found by a scan of the whole source, so one in a
//! comment counts too; a name declared more than once expands to its
//! largest value. Both keep the measure an upper bound.
//!
//! [XML 1.0 §4.4]: https://www.w3.org/TR/xml/#entproc

use super::MAX_XML_DEPTH;
use super::markup::scan;
use std::collections::HashMap;
use std::ops::Range;

/// What entities may add to a source: libxml2, Chrome's XML parser, likewise
/// refuses a document whose entities amplify it past a fixed allowance and a
/// multiple of its size.
const ALLOWED_EXPANSION: usize = 1_000_000;
const MAX_AMPLIFICATION: usize = 5;
/// roxmltree refuses references nested deeper.
const MAX_ENTITY_DEPTH: usize = 10;
/// Character comparisons spent scanning declarations and resolving references.
/// roxmltree linearly searches declarations, including duplicate names.
const MAX_ENTITY_WORK: usize = 2_000_000;
/// Entities XML defines without a declaration.
const PREDEFINED: [&str; 5] = ["lt", "gt", "amp", "apos", "quot"];

/// What a source expands to: its bytes, the `<` (tags) and `=`
/// (attributes) the parser allocates for, and how deep its elements nest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpandedSource {
    pub bytes: usize,
    pub tags: usize,
    pub attributes: usize,
    pub depth: usize,
    /// References encountered, including references expanding to empty strings.
    references: usize,
}

impl ExpandedSource {
    fn literal(text: &str) -> Self {
        let count = |byte: u8| text.bytes().filter(|&b| b == byte).count();
        Self {
            bytes: text.len(),
            tags: count(b'<'),
            attributes: count(b'='),
            depth: 0,
            references: count(b'&'),
        }
    }

    /// Both, as one after the other; `other`'s depth is already where it
    /// stands.
    fn add(self, other: Self) -> Self {
        Self {
            bytes: self.bytes.saturating_add(other.bytes),
            tags: self.tags.saturating_add(other.tags),
            attributes: self.attributes.saturating_add(other.attributes),
            depth: self.depth.max(other.depth),
            references: self.references.saturating_add(other.references),
        }
    }

    fn max(self, other: Self) -> Self {
        Self {
            bytes: self.bytes.max(other.bytes),
            tags: self.tags.max(other.tags),
            attributes: self.attributes.max(other.attributes),
            depth: self.depth.max(other.depth),
            references: self.references.max(other.references),
        }
    }

    /// The expansion of a reference inside elements nested `depth` deep.
    fn at_depth(self, depth: usize) -> Self {
        Self {
            depth: self.depth.saturating_add(depth),
            ..self
        }
    }
}

/// What `text` expands to once its entity references are replaced, or `None`
/// when the expansion passes its bound, its elements nest deeper than
/// [`MAX_XML_DEPTH`], markup runs unclosed to the end of the source or of an
/// entity's value, a reference names no entity, or entities refer to each
/// other in a cycle (which roxmltree refuses too).
pub(crate) fn expand_entities(text: &str) -> Option<ExpandedSource> {
    let markup = scan(text);
    if markup.unterminated {
        return None;
    }
    let literal = ExpandedSource {
        depth: markup.depth,
        ..ExpandedSource::literal(text)
    };
    if !text.contains("<!ENTITY") {
        // Without declarations a reference is predefined or refused.
        return within_depth(literal);
    }
    // Scan this before parsing declarations: even looking for their closing
    // quotes must not repeatedly search an arbitrarily long suffix.
    let count = text.matches("<!ENTITY").count();
    if count.saturating_mul(text.len()) > MAX_ENTITY_WORK {
        return None;
    }
    let declarations = Declarations::scan(text);
    let longest_name = declarations
        .by_name
        .keys()
        .map(|name| name.len())
        .max()
        .unwrap_or(0);
    let lookup = count.saturating_mul(longest_name.saturating_add(1));
    let limit = ALLOWED_EXPANSION.max(text.len().saturating_mul(MAX_AMPLIFICATION));
    let mut expansions = Expansions {
        declarations: &declarations,
        known: HashMap::new(),
    };
    let mut added = ExpandedSource::default();
    for (name, depth) in markup.references {
        added = added.add(expansions.of(name, 0)?.at_depth(depth));
        let references = literal.references.saturating_add(added.references);
        if added.bytes > limit || references.saturating_mul(lookup) > MAX_ENTITY_WORK {
            return None;
        }
    }
    within_depth(literal.add(added))
}

fn within_depth(expanded: ExpandedSource) -> Option<ExpandedSource> {
    (expanded.depth <= MAX_XML_DEPTH).then_some(expanded)
}

/// The internal entities a source declares.
struct Declarations<'a> {
    by_name: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> Declarations<'a> {
    /// Every `<!ENTITY name "value">`, general or parameter (roxmltree looks
    /// both up by name). External entities have no value: roxmltree is given
    /// no resolver, so they expand to nothing.
    fn scan(text: &'a str) -> Self {
        let mut declarations = Self {
            by_name: HashMap::new(),
        };
        for (start, keyword) in text.match_indices("<!ENTITY") {
            if let Some((name, value)) = declaration(text, start + keyword.len()) {
                declarations
                    .by_name
                    .entry(name)
                    .or_default()
                    .push(&text[value]);
            }
        }
        declarations
    }
}

/// The name and value range of the declaration whose body starts at `at`.
fn declaration(text: &str, at: usize) -> Option<(&str, Range<usize>)> {
    let rest = text.get(at..)?;
    let after_space = rest.trim_start_matches(is_xml_space);
    if after_space.len() == rest.len() {
        return None;
    }
    let after_percent = after_space
        .strip_prefix('%')
        .map_or(after_space, |rest| rest.trim_start_matches(is_xml_space));
    let name_len = after_percent
        .find(|c: char| is_xml_space(c) || matches!(c, '"' | '\'' | '>'))
        .unwrap_or(after_percent.len());
    let name = &after_percent[..name_len];
    let definition = after_percent[name_len..].trim_start_matches(is_xml_space);
    let quote = definition
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\''))?;
    let value_len = definition[1..].find(quote)?;
    let value_start = text.len() - definition.len() + 1;
    (!name.is_empty()).then_some((name, value_start..value_start + value_len))
}

fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// What each referenced entity expands to, computed once.
struct Expansions<'d, 'a> {
    declarations: &'d Declarations<'a>,
    /// `None` while an entity is being expanded, to find cycles.
    known: HashMap<&'a str, Option<ExpandedSource>>,
}

impl<'a> Expansions<'_, 'a> {
    fn of(&mut self, name: &'a str, depth: usize) -> Option<ExpandedSource> {
        if depth > MAX_ENTITY_DEPTH {
            return None;
        }
        if let Some(known) = self.known.get(name) {
            return *known;
        }
        let Some(values) = self.declarations.by_name.get(name) else {
            let predefined = PREDEFINED.contains(&name);
            return predefined.then_some(ExpandedSource {
                bytes: 1,
                ..Default::default()
            });
        };
        self.known.insert(name, None);
        let mut expansion = ExpandedSource::default();
        for value in values {
            let markup = scan(value);
            if markup.unterminated {
                return None;
            }
            let mut this = ExpandedSource {
                depth: markup.depth,
                ..ExpandedSource::literal(value)
            };
            for (name, at) in markup.references {
                this = this.add(self.of(name, depth + 1)?.at_depth(at));
            }
            expansion = expansion.max(this);
        }
        self.known.insert(name, Some(expansion));
        Some(expansion)
    }
}
