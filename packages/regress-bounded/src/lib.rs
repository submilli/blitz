//! Budgeted UTF-16 full matching for HTML's Unicode-v pattern constraint.
//!
//! ```
//! use regress_bounded::{Budget, Pattern};
//! let budget = Budget::default();
//! let source: Vec<u16> = "a|ab".encode_utf16().collect();
//! let value: Vec<u16> = "ab".encode_utf16().collect();
//! let pattern = Pattern::compile(&source, &budget).unwrap();
//! assert!(pattern.is_full_match(&value, &budget).unwrap());
//! ```
//!
//! Compilation errors distinguish invalid syntax from exhausted resources.
//! Reuse the same budget across every control/token in one validation operation.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(feature = "pattern", feature(pattern))]
#![warn(clippy::all)]
#![allow(
    clippy::upper_case_acronyms,
    clippy::match_like_matches_macro,
    clippy::uninlined_format_args,
    clippy::collapsible_if
)]
// Clippy's manual_range_contains suggestion produces worse codegen.
#![allow(clippy::manual_range_contains)]

#[cfg(all(not(feature = "std"), not(feature = "alloc")))]
compile_error!(
    "`regress` requires the `alloc` feature when building without `std`; use `--no-default-features --features alloc`."
);

#[cfg(not(feature = "std"))]
#[macro_use]
extern crate alloc;

/// Upstream compatibility API for differential tests. These entry points do not
/// provide bounded execution; browser validation uses [`crate::Pattern`].
pub mod legacy {
    pub use crate::api::*;
}

#[macro_use]
mod util;

mod api;
mod bytesearch;
mod charclasses;
mod classicalbacktrack;
mod codepointset;
mod cursor;
mod emit;
mod exec;
mod indexing;
mod insn;
mod ir;
// UTF-16 never matches against bytes, so the byte-oriented literal lowering is
// UTF-8 only. See `emit_code_point_sequence` for the UTF-16 path.
#[cfg(not(feature = "utf16"))]
mod literal;
mod matchers;
mod optimizer;
mod parse;
mod position;
mod scm;
mod startpredicate;
mod types;
mod unicode;
mod unicodetables;

#[cfg(feature = "backend-pikevm")]
mod pikevm;

mod budget;
pub use budget::{Budget, ResourceError};
mod bounded;
pub use bounded::{Pattern, PatternError};
