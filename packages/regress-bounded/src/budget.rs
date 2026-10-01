//! Operation-owned accounting. Clones share the same allowance, including nested work.
use std::{cell::Cell, rc::Rc};

/// Exhaustion is separate from a pattern syntax error or a failed match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceError;

impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pattern resource budget exceeded")
    }
}
impl std::error::Error for ResourceError {}

/// Cumulative work and allocation admission for a validation operation.
/// Allocation charges include transient copies and vector capacity headroom;
/// freeing storage does not refund the allowance.
#[derive(Clone, Debug)]
pub struct Budget(Rc<State>);

#[derive(Debug)]
struct State {
    work: Cell<usize>,
    bytes: Cell<usize>,
    exhausted: Cell<bool>,
}

impl Budget {
    pub fn new(work: usize, bytes: usize) -> Self {
        Self(Rc::new(State {
            work: Cell::new(work),
            bytes: Cell::new(bytes),
            exhausted: Cell::new(false),
        }))
    }

    pub(crate) fn unlimited() -> Self {
        Self::new(usize::MAX, usize::MAX)
    }

    /// Admit caller-owned encoding/storage before allocating it.
    pub fn charge(&self, work: usize, bytes: usize) -> Result<(), ResourceError> {
        let remaining = self
            .0
            .work
            .get()
            .checked_sub(work)
            .zip(self.0.bytes.get().checked_sub(bytes));
        if !self.0.exhausted.get() {
            if let Some((work, bytes)) = remaining {
                self.0.work.set(work);
                self.0.bytes.set(bytes);
                return Ok(());
            }
        }
        self.0.exhausted.set(true);
        Err(ResourceError)
    }

    pub(crate) fn reject<T>(&self) -> Result<T, ResourceError> {
        self.0.exhausted.set(true);
        Err(ResourceError)
    }

    pub fn check(&self) -> Result<(), ResourceError> {
        if self.0.exhausted.get() {
            Err(ResourceError)
        } else {
            Ok(())
        }
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(4_000_000, 32 * 1024 * 1024)
    }
}
