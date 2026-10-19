//! Fallible output growth shared by bounded and existing infallible serializers.
use crate::DomString;
use std::convert::Infallible;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlOutputLimit;

impl std::fmt::Display for HtmlOutputLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HTML output limit exceeded")
    }
}
impl std::error::Error for HtmlOutputLimit {}

pub(super) trait Sink {
    type Error;
    fn push_str(&mut self, text: &str) -> Result<(), Self::Error>;
    fn push_dom(&mut self, text: &DomString) -> Result<(), Self::Error>;
    fn push_units(&mut self, units: &[u16]) -> Result<(), Self::Error>;
    fn push(&mut self, value: char) -> Result<(), Self::Error> {
        let mut buffer = [0; 4];
        self.push_str(value.encode_utf8(&mut buffer))
    }
}

impl Sink for DomString {
    type Error = Infallible;
    fn push_str(&mut self, text: &str) -> Result<(), Infallible> {
        DomString::push_str(self, text);
        Ok(())
    }
    fn push_dom(&mut self, text: &DomString) -> Result<(), Infallible> {
        DomString::push_dom(self, text);
        Ok(())
    }
    fn push_units(&mut self, units: &[u16]) -> Result<(), Infallible> {
        DomString::push_units(self, units);
        Ok(())
    }
}

pub(super) struct Bounded {
    text: DomString,
    remaining: usize,
}
impl Bounded {
    pub(super) fn new(max_units: usize) -> Self {
        Self {
            text: DomString::new(),
            remaining: max_units,
        }
    }
    pub(super) fn finish(self) -> DomString {
        self.text
    }
    fn admit(&mut self, units: usize) -> Result<(), HtmlOutputLimit> {
        self.remaining = self.remaining.checked_sub(units).ok_or(HtmlOutputLimit)?;
        Ok(())
    }
    fn count(&self, units: impl Iterator<Item = u16>) -> usize {
        units.take(self.remaining.saturating_add(1)).count()
    }
}
impl Sink for Bounded {
    type Error = HtmlOutputLimit;
    fn push_str(&mut self, text: &str) -> Result<(), HtmlOutputLimit> {
        self.admit(self.count(text.encode_utf16()))?;
        self.text.push_str(text);
        Ok(())
    }
    fn push_dom(&mut self, text: &DomString) -> Result<(), HtmlOutputLimit> {
        self.admit(self.count(text.utf16_units()))?;
        self.text.push_dom(text);
        Ok(())
    }
    fn push_units(&mut self, units: &[u16]) -> Result<(), HtmlOutputLimit> {
        self.admit(units.len())?;
        self.text.push_units(units);
        Ok(())
    }
}

pub(super) fn finish<T>(value: Result<T, Infallible>) -> T {
    match value {
        Ok(value) => value,
        Err(never) => match never {},
    }
}
