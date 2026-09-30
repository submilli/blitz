/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Admission for nested selectors that repeatedly search ancestor/sibling trees.
//!
//! A boolean nested match loses the outer matcher's backtracking information.
//! Repeating combinator-bearing levels therefore raises tree size to an
//! attacker-selected power. Keep that power fixed before matching, including
//! selectors synthesized by CSS nesting. This is not a wall-clock work budget.

use selectors::parser::{Component, Selector};
use selectors::{SelectorImpl, SelectorList};

const MAX_SEARCH_LEVELS: usize = 2;
const MAX_COMPONENTS: usize = 4096;
const MAX_DEPTH: usize = 128;

/// Whether every branch of a list stays within the matching complexity limits.
/// Reject the whole list: filtering branches could invert negation semantics.
pub fn allowed<I: SelectorImpl>(list: &SelectorList<I>) -> bool {
    let mut remaining = MAX_COMPONENTS;
    list.slice().iter().all(|selector| admitted(selector, 0, 0, &mut remaining))
}

fn admitted<I: SelectorImpl>(
    selector: &Selector<I>,
    searches: usize,
    depth: usize,
    remaining: &mut usize,
) -> bool {
    if depth > MAX_DEPTH || selector.len() > *remaining {
        return false;
    }
    *remaining -= selector.len();
    let searches = searches + usize::from(selector.iter_raw_match_order().any(|component| {
        matches!(component, Component::Combinator(_))
    }));
    if searches > MAX_SEARCH_LEVELS {
        return false;
    }
    selector.iter_raw_match_order().all(|component| match component {
        Component::Is(list) | Component::Where(list) | Component::Negation(list) => list
            .slice().iter().all(|inner| admitted(inner, searches, depth + 1, remaining)),
        Component::Has(list) => list.iter().all(|inner| {
            admitted(&inner.selector, searches, depth + 1, remaining)
        }),
        // nth-of scans siblings even when its inner selector has no combinator.
        Component::NthOf(data) => searches < MAX_SEARCH_LEVELS && data.selectors().iter()
            .all(|inner| admitted(inner, searches + 1, depth + 1, remaining)),
        Component::Host(Some(inner)) | Component::Slotted(inner) => {
            admitted(inner, searches, depth + 1, remaining)
        },
        _ => true,
    })
}
