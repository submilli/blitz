use std::ops::{Deref, DerefMut};

use markup5ever::QualName;
use thin_vec::ThinVec;

/// A tag attribute, e.g. `class="test"` in `<div class="test" ...>`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct Attribute {
    /// The name of the attribute (e.g. the `class` in `<div class="test">`)
    pub name: QualName,
    /// The value of the attribute (e.g. the `"test"` in `<div class="test">`)
    pub value: crate::DomString,
}

#[derive(Clone, Debug)]
pub struct Attributes {
    inner: ThinVec<Attribute>,
}

impl Attributes {
    pub fn new(inner: Vec<Attribute>) -> Self {
        Self {
            inner: inner.into_iter().collect(),
        }
    }

    pub fn get(&mut self, name: &QualName) -> Option<&Attribute> {
        self.inner.iter().find(|attr| attr.name == *name)
    }

    pub fn set(&mut self, name: QualName, value: impl Into<crate::DomString>) {
        let value = value.into();
        let existing_attr = self.inner.iter_mut().find(|a| a.name == name);
        if let Some(existing_attr) = existing_attr {
            existing_attr.value = value;
        } else {
            self.push(Attribute {
                name: name.clone(),
                value,
            });
        }
    }

    pub fn remove(&mut self, name: &QualName) -> Option<Attribute> {
        let idx = self.inner.iter().position(|attr| attr.name == *name);
        idx.map(|idx| self.inner.remove(idx))
    }
}

impl Deref for Attributes {
    type Target = ThinVec<Attribute>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for Attributes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
