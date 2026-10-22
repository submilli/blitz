//! Infra tokens and exact UTF-16 class matching, including quirks ASCII folding.
use crate::DomString;
use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hasher},
};

pub(super) struct Classes(Vec<Token>);
struct Token {
    units: Vec<u16>,
    hash: u64,
    lowercase_hash: u64,
}

impl Classes {
    pub(super) fn new(value: &DomString) -> Self {
        let units = value.to_utf16();
        let mut seen = HashSet::new();
        let tokens = tokens(&units)
            .filter_map(|token| {
                if !seen.insert(token.to_vec()) {
                    return None;
                }
                Some(Token {
                    units: token.to_vec(),
                    hash: hash(token, false),
                    lowercase_hash: hash(token, true),
                })
            })
            .collect();
        Self(tokens)
    }

    pub(super) fn matches(&self, value: &DomString, quirks: bool) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let units = value.to_utf16();
        // Common one/two-class queries avoid a table. Slice length checks also
        // avoid scanning a long query against short native attribute tokens.
        if self.0.len() <= 8 {
            return self
                .0
                .iter()
                .all(|class| tokens(&units).any(|token| equal(token, &class.units, quirks)));
        }
        let mut index: HashMap<u64, Vec<&[u16]>> = HashMap::new();
        for token in tokens(&units) {
            index.entry(hash(token, quirks)).or_default().push(token);
        }
        self.0.iter().all(|class| {
            let hash = if quirks {
                class.lowercase_hash
            } else {
                class.hash
            };
            index.get(&hash).is_some_and(|bucket| {
                bucket
                    .iter()
                    .any(|token| equal(token, &class.units, quirks))
            })
        })
    }
}

fn tokens(units: &[u16]) -> impl Iterator<Item = &[u16]> {
    units
        .split(|unit| matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
        .filter(|token| !token.is_empty())
}

fn equal(left: &[u16], right: &[u16], lowercase_ascii: bool) -> bool {
    left.len() == right.len()
        && if lowercase_ascii {
            left.iter()
                .copied()
                .map(lowercase)
                .eq(right.iter().copied().map(lowercase))
        } else {
            left == right
        }
}

// Hashes narrow the search only. Compare authoritative units after a match;
// collisions and lone-surrogate/scalar-projection collisions stay distinct.
fn hash(units: &[u16], lowercase_ascii: bool) -> u64 {
    let mut state = DefaultHasher::new();
    state.write_usize(units.len());
    for &unit in units {
        state.write_u16(if lowercase_ascii {
            lowercase(unit)
        } else {
            unit
        });
    }
    state.finish()
}

pub(super) fn lowercase(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit + 32
    } else {
        unit
    }
}
