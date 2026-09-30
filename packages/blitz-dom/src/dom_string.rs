//! Lossless DOM character data and its scalar-value projection for layout.
//!
//! DOMString holds arbitrary UTF-16 code units, including lone surrogates.
//! UTF-8 callers keep their compact representation; UTF-16 data is converted
//! lossily only when a scalar consumer requests `as_str_lossy()`.
//! The cached projection never changes the authoritative code units.

use std::{borrow::Cow, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct DomString(Repr);

#[derive(Clone, Debug)]
enum Repr {
    Utf8(String),
    Utf16 {
        units: Vec<u16>,
        scalar: OnceLock<String>,
    },
}

impl DomString {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_utf16(units: Vec<u16>) -> Self {
        Self(Repr::Utf16 {
            units,
            scalar: OnceLock::new(),
        })
    }

    /// Original DOM code units, never the layout projection.
    pub fn to_utf16(&self) -> Cow<'_, [u16]> {
        match &self.0 {
            Repr::Utf8(s) => Cow::Owned(s.encode_utf16().collect()),
            Repr::Utf16 { units, .. } => Cow::Borrowed(units),
        }
    }

    /// Scalar-value UTF-8 for layout/byte consumers. Lone surrogates become U+FFFD.
    pub fn as_str_lossy(&self) -> &str {
        match &self.0 {
            Repr::Utf8(s) => s,
            Repr::Utf16 { units, scalar } => scalar.get_or_init(|| String::from_utf16_lossy(units)),
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.0 {
            Repr::Utf8(s) => s.is_empty(),
            Repr::Utf16 { units, .. } => units.is_empty(),
        }
    }

    /// Heap payload retained by a mutation record, including any cached view.
    pub fn retained_bytes(&self) -> usize {
        match &self.0 {
            Repr::Utf8(s) => s.capacity(),
            Repr::Utf16 { units, scalar } => units
                .capacity()
                .saturating_mul(2)
                .saturating_add(scalar.get().map_or(0, String::capacity)),
        }
    }

    pub fn push_str(&mut self, text: &str) {
        match &mut self.0 {
            Repr::Utf8(s) => s.push_str(text),
            Repr::Utf16 { units, scalar } => {
                units.extend(text.encode_utf16());
                scalar.take();
            }
        }
    }

    /// Concatenate code units, including surrogate pairs split across nodes.
    pub fn push_dom(&mut self, text: &Self) {
        match &text.0 {
            Repr::Utf8(s) => self.push_str(s),
            Repr::Utf16 { units, .. } => self.push_units(units),
        }
    }

    pub fn push_units(&mut self, text: &[u16]) {
        if text.is_empty() {
            return;
        }
        if let Repr::Utf8(s) = &self.0 {
            self.0 = Repr::Utf16 {
                units: s.encode_utf16().collect(),
                scalar: OnceLock::new(),
            };
        }
        if let Repr::Utf16 { units, scalar } = &mut self.0 {
            units.extend_from_slice(text);
            scalar.take();
        }
    }

    pub fn push(&mut self, c: char) {
        self.push_str(c.encode_utf8(&mut [0; 4]));
    }
}

impl Default for DomString {
    fn default() -> Self {
        Self(Repr::Utf8(String::new()))
    }
}
impl From<String> for DomString {
    fn from(s: String) -> Self {
        Self(Repr::Utf8(s))
    }
}
impl From<&str> for DomString {
    fn from(s: &str) -> Self {
        s.to_owned().into()
    }
}
impl From<&String> for DomString {
    fn from(s: &String) -> Self {
        s.as_str().into()
    }
}
impl From<&DomString> for DomString {
    fn from(s: &DomString) -> Self {
        s.clone()
    }
}
impl std::fmt::Display for DomString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str_lossy().fmt(f)
    }
}
impl PartialEq for DomString {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::Utf8(a), Repr::Utf8(b)) => a == b,
            _ => self.to_utf16() == other.to_utf16(),
        }
    }
}
impl Eq for DomString {}
impl PartialEq<str> for DomString {
    fn eq(&self, s: &str) -> bool {
        self == &Self::from(s)
    }
}
impl PartialEq<&str> for DomString {
    fn eq(&self, s: &&str) -> bool {
        self == *s
    }
}
impl<'a> FromIterator<&'a DomString> for DomString {
    fn from_iter<T: IntoIterator<Item = &'a DomString>>(iter: T) -> Self {
        let mut out = Self::new();
        for s in iter {
            out.push_dom(s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::DomString;

    #[test]
    fn scalar_cache_is_invalidated_when_halves_join() {
        let mut s = DomString::from_utf16(vec![0xD83D]);
        assert_eq!(s.as_str_lossy(), "�");
        s.push_units(&[0xDE00]);
        assert_eq!(s.as_str_lossy(), "😀");
        s.push_str("!");
        assert_eq!(s.as_str_lossy(), "😀!");
        assert_eq!(s.to_utf16().as_ref(), [0xD83D, 0xDE00, 33]);
    }

    #[test]
    fn equality_compares_code_units_not_scalar_projections() {
        let hi = DomString::from_utf16(vec![0xD800]);
        let lo = DomString::from_utf16(vec![0xDC00]);
        assert_ne!(hi, lo);
        assert_ne!(hi, DomString::from("�"));
        assert_eq!(
            DomString::from_utf16(vec![0xD83D, 0xDE00]),
            DomString::from("😀")
        );
    }

    #[test]
    fn retained_size_includes_utf16_and_cached_projection() {
        let s = DomString::from_utf16(vec![0xD800; 100]);
        let before = s.retained_bytes();
        assert!(before >= 200);
        let bytes = s.as_str_lossy().len();
        assert!(s.retained_bytes() >= before + bytes);
    }
}
