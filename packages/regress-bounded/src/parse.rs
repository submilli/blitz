//! Parser from regex patterns to IR

use crate::{
    api, charclasses,
    codepointset::{CodePoint, CodePointSet, Interval, interval_contains},
    insn::MAX_CHAR_SET_LENGTH,
    ir,
    types::{
        BracketContents, CaptureGroupID, CaptureGroupName, CharacterClassType, MAX_CAPTURE_GROUPS,
        MAX_LOOPS, MAX_NESTING_DEPTH,
    },
    unicode::{
        self, PropertyEscapeKind, unicode_property_from_str, unicode_property_name_from_str,
    },
    unicodetables::{id_continue_ranges, id_start_ranges},
    util::to_char_sat,
};
use core::{fmt, iter::Peekable, mem};
#[cfg(feature = "std")]
use std::collections::HashMap;
#[cfg(all(not(feature = "std"), feature = "alloc"))]
use {
    alloc::{
        boxed::Box,
        string::{String, ToString},
        vec::Vec,
    },
    hashbrown::HashMap,
};

/// Represents an error encountered during regex compilation.
///
/// The text contains a human-readable error message.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Error {
    pub text: String,
    pub kind: ErrorKind,
}

/// Internal compilation failures retain resource identity across parser helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorKind {
    Syntax,
    Resource,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl From<crate::ResourceError> for Error {
    fn from(_: crate::ResourceError) -> Self {
        Self {
            text: "pattern resource budget exceeded".into(),
            kind: ErrorKind::Resource,
        }
    }
}

enum ClassAtom {
    CodePoint(CodePoint),
    CharacterClass {
        class_type: CharacterClassType,
        positive: bool,
    },
    Range {
        iv: CodePointSet,
        negate: bool,
    },
}

/// Represents the result of a class set.
#[derive(Debug, Clone)]
struct ClassSet {
    codepoints: CodePointSet,
    alternatives: ClassSetAlternativeStrings,
    may_contain_strings: bool,
}

impl ClassSet {
    fn new() -> Self {
        ClassSet {
            codepoints: CodePointSet::new(),
            alternatives: ClassSetAlternativeStrings::new(),
            may_contain_strings: false,
        }
    }

    fn node(
        mut self,
        icase: bool,
        negate_set: bool,
        budget: &crate::Budget,
    ) -> Result<ir::Node, Error> {
        let n = self.alternatives.strings.len();
        budget.charge(
            n.saturating_mul(n.checked_ilog2().unwrap_or(0) as usize + 1),
            n.saturating_mul(32),
        )?;
        let has_empty = self.alternatives.strings.iter().any(|s| s.is_empty());
        self.alternatives.strings.retain(|s| !s.is_empty());
        let mut choices = Vec::new();
        if !self.alternatives.strings.is_empty() {
            choices.push(self.alternatives.into_node(icase));
        }
        if negate_set || !self.codepoints.is_empty() {
            choices.push(ir::Node::Bracket(BracketContents {
                invert: negate_set,
                cps: self.codepoints,
            }));
        }
        // Class strings are longest-first, including when single codepoints
        // were normalized into the bracket. Lookarounds make this observable.
        if has_empty {
            choices.push(ir::Node::Empty);
        }
        if choices.is_empty() {
            return Ok(ir::Node::make_always_fails());
        }
        Ok(make_alt(choices))
    }

    fn admit_operand(
        &self,
        operand: &ClassSetOperand,
        budget: &crate::Budget,
        compare: bool,
    ) -> Result<(), Error> {
        let a = self.codepoints.intervals().len();
        budget.charge(self.alternatives.strings.len(), 0)?;
        let x = self
            .alternatives
            .strings
            .iter()
            .map(|s| s.len() + 1)
            .sum::<usize>();
        let (b, strings): (usize, &[Box<[u32]>]) = match operand {
            ClassSetOperand::ClassSetCharacter(_) => (1, &[]),
            ClassSetOperand::CharacterClassEscape(cps) => (cps.intervals().len(), &[]),
            ClassSetOperand::Class(c) => (c.codepoints.intervals().len(), &c.alternatives.strings),
            ClassSetOperand::ClassStringDisjunction(s) => (0, &s.strings),
        };
        budget.charge(strings.len(), 0)?;
        let y = strings.iter().map(|s| s.len() + 1).sum::<usize>();
        let size = a
            .saturating_add(b)
            .saturating_add(x)
            .saturating_add(y)
            .saturating_add(1);
        let work = if compare {
            // Pairwise interval/string tests and intermediate retained sets.
            size.saturating_mul(size)
        } else {
            b.saturating_mul(a.saturating_add(b)).saturating_add(size)
        };
        budget.charge(work.saturating_mul(4), size.saturating_mul(128))?;
        Ok(())
    }

    fn union_operand(
        &mut self,
        operand: ClassSetOperand,
        budget: &crate::Budget,
        icase: bool,
    ) -> Result<(), Error> {
        self.may_contain_strings |= operand.may_contain_strings();
        let operand = operand.canonicalize(icase, budget)?;
        self.admit_operand(&operand, budget, false)?;
        match operand {
            ClassSetOperand::ClassSetCharacter(c) => {
                self.codepoints.add_one(c);
            }
            ClassSetOperand::CharacterClassEscape(cps) => {
                self.codepoints.add_set(cps);
            }
            ClassSetOperand::Class(class) => {
                self.codepoints.add_set(class.codepoints);
                self.alternatives.extend(class.alternatives);
            }
            ClassSetOperand::ClassStringDisjunction(s) => {
                self.alternatives.extend(s);
            }
        }
        Ok(())
    }

    fn intersect_operand(
        &mut self,
        operand: ClassSetOperand,
        budget: &crate::Budget,
        icase: bool,
    ) -> Result<(), Error> {
        self.may_contain_strings &= operand.may_contain_strings();
        let operand = operand.canonicalize(icase, budget)?;
        self.admit_operand(&operand, budget, true)?;
        match operand {
            ClassSetOperand::ClassSetCharacter(c) => {
                if self.codepoints.contains(c) {
                    self.codepoints =
                        CodePointSet::from_sorted_disjoint_intervals(Vec::from([Interval {
                            first: c,
                            last: c,
                        }]));
                } else {
                    self.codepoints.clear();
                }
                let mut found_alternative = false;
                for alternative in &mut self.alternatives.strings {
                    if alternative.len() == 1 && alternative[0] == c {
                        found_alternative = true;
                        break;
                    }
                }
                self.alternatives.strings.clear();
                if found_alternative {
                    self.alternatives.strings.push(Box::from([c]));
                }
            }
            ClassSetOperand::CharacterClassEscape(cps) => {
                self.codepoints.intersect(cps.intervals());
                let mut retained = Vec::new();
                for alternative in &self.alternatives.strings {
                    if alternative.len() == 1 && cps.contains(alternative[0]) {
                        retained.push(alternative.clone());
                    }
                }
                self.alternatives.strings = retained;
            }
            ClassSetOperand::Class(class) => {
                let mut retained_codepoints = CodePointSet::new();
                for alternative in &class.alternatives.strings {
                    if alternative.len() == 1 && self.codepoints.contains(alternative[0]) {
                        retained_codepoints.add_one(alternative[0]);
                    }
                }
                let mut retained_alternatives = ClassSetAlternativeStrings::new();
                for alternative in &self.alternatives.strings {
                    if alternative.len() == 1 && class.codepoints.contains(alternative[0]) {
                        retained_alternatives.strings.push(alternative.clone());
                    }
                }
                self.codepoints.intersect(class.codepoints.intervals());
                self.codepoints.add_set(retained_codepoints);
                self.alternatives.intersect(&class.alternatives);
                self.alternatives.extend(retained_alternatives);
            }
            ClassSetOperand::ClassStringDisjunction(s) => {
                let mut retained = CodePointSet::new();
                for alternative in &s.strings {
                    if alternative.len() == 1 && self.codepoints.contains(alternative[0]) {
                        retained.add_one(alternative[0]);
                    }
                }
                self.codepoints = retained;
                self.alternatives.intersect(&s);
            }
        }
        Ok(())
    }

    fn subtract_operand(
        &mut self,
        operand: ClassSetOperand,
        budget: &crate::Budget,
        icase: bool,
    ) -> Result<(), Error> {
        let operand = operand.canonicalize(icase, budget)?;
        self.admit_operand(&operand, budget, true)?;
        match operand {
            ClassSetOperand::ClassSetCharacter(c) => {
                self.codepoints.remove(&[Interval { first: c, last: c }]);
                self.alternatives.remove(&[Box::from([c])]);
            }
            ClassSetOperand::CharacterClassEscape(cps) => {
                self.codepoints.remove(cps.intervals());
                let mut to_remove = ClassSetAlternativeStrings::new();
                for alternative in &self.alternatives.strings {
                    if alternative.len() == 1 && cps.contains(alternative[0]) {
                        to_remove.strings.push(alternative.clone());
                    }
                }
                self.alternatives.remove(&to_remove);
            }
            ClassSetOperand::Class(class) => {
                let mut codepoints_removed = CodePointSet::new();
                for alternative in &class.alternatives.strings {
                    if alternative.len() == 1 && self.codepoints.contains(alternative[0]) {
                        codepoints_removed.add_one(alternative[0]);
                    }
                }
                let mut alternatives_removed = ClassSetAlternativeStrings::new();
                for alternative in &self.alternatives.strings {
                    if alternative.len() == 1 && class.codepoints.contains(alternative[0]) {
                        alternatives_removed.strings.push(alternative.clone());
                    }
                }
                self.codepoints.remove(codepoints_removed.intervals());
                self.codepoints.remove(class.codepoints.intervals());
                self.alternatives.remove(&alternatives_removed);
                self.alternatives.remove(&class.alternatives);
            }
            ClassSetOperand::ClassStringDisjunction(s) => {
                let mut to_remove = CodePointSet::new();
                for alternative in &s.strings {
                    if alternative.len() == 1 && self.codepoints.contains(alternative[0]) {
                        to_remove.add_one(alternative[0]);
                    }
                }
                self.codepoints.remove(to_remove.intervals());
                self.alternatives.remove(&s);
            }
        }
        Ok(())
    }
}

/// Represents all different types of class set operands.
#[derive(Debug, Clone)]
enum ClassSetOperand {
    ClassSetCharacter(CodePoint),
    CharacterClassEscape(CodePointSet),
    Class(ClassSet),
    ClassStringDisjunction(ClassSetAlternativeStrings),
}

impl ClassSetOperand {
    fn may_contain_strings(&self) -> bool {
        match self {
            Self::Class(class) => class.may_contain_strings,
            Self::ClassStringDisjunction(strings) => strings.may_contain_strings,
            Self::ClassSetCharacter(_) | Self::CharacterClassEscape(_) => false,
        }
    }

    fn canonicalize(self, icase: bool, budget: &crate::Budget) -> Result<Self, Error> {
        let mut class = ClassSet::new();
        match self {
            Self::Class(_) => return Ok(self), // Nested classes have already applied their flags.
            Self::ClassSetCharacter(cp) => class.codepoints.add_one(cp),
            Self::CharacterClassEscape(cps) => class.codepoints = cps,
            Self::ClassStringDisjunction(strings) => {
                class.may_contain_strings = strings.may_contain_strings;
                budget.charge(
                    strings.strings.len(),
                    strings.strings.len().saturating_mul(64),
                )?;
                for mut string in strings.strings {
                    if icase {
                        budget
                            .charge(string.len().saturating_mul(unicode::literal_fold_cost()), 0)?;
                        for cp in &mut string {
                            *cp = unicode::fold(*cp);
                        }
                    }
                    if string.len() == 1 {
                        budget.charge(class.codepoints.intervals().len() + 1, 32)?;
                        class.codepoints.add_one(string[0]);
                    } else {
                        class.alternatives.strings.push(string);
                    }
                }
            }
        }
        if icase {
            class.codepoints = unicode::add_icase_code_points_budget(class.codepoints, budget)?;
        }
        Ok(Self::Class(class))
    }
}

/// A list of strings matching some property, for use in 'v' regular expressions.
#[derive(Debug, Clone)]
struct ClassSetAlternativeStrings {
    strings: Vec<Box<[CodePoint]>>,
    // Syntactic eligibility for negation, retained independently of filtering.
    may_contain_strings: bool,
}

impl ClassSetAlternativeStrings {
    fn new() -> Self {
        ClassSetAlternativeStrings {
            strings: Vec::new(),
            may_contain_strings: false,
        }
    }

    fn extend(&mut self, other: impl IntoIterator<Item = Box<[CodePoint]>>) {
        self.strings.extend(other);
    }

    fn intersect(&mut self, other: &[Box<[CodePoint]>]) {
        self.strings.retain(|string| other.contains(string));
    }

    fn remove(&mut self, other: &[Box<[CodePoint]>]) {
        self.strings.retain(|string| !other.contains(string));
    }

    fn into_node(self, icase: bool) -> ir::Node {
        // ClassSetAlternativeStrings are matched longest first.
        // "iterating in descending order of length" per ES2027
        let mut alternatives = self.strings;
        #[allow(clippy::unnecessary_sort_by)]
        alternatives.sort_by(|a, b| b.len().cmp(&a.len()));
        ir::Node::StringSet {
            alternatives,
            icase,
        }
    }
}

impl core::ops::Deref for ClassSetAlternativeStrings {
    type Target = [Box<[CodePoint]>];
    fn deref(&self) -> &Self::Target {
        &self.strings
    }
}

impl IntoIterator for ClassSetAlternativeStrings {
    type Item = Box<[CodePoint]>;
    type IntoIter = <Vec<Box<[CodePoint]>> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.strings.into_iter()
    }
}

fn error<S, T>(text: S) -> Result<T, Error>
where
    S: ToString,
{
    Err(Error {
        text: text.to_string(),
        kind: ErrorKind::Syntax,
    })
}

fn make_cat(nodes: ir::NodeList) -> ir::Node {
    match nodes.len() {
        0 => ir::Node::Empty,
        1 => nodes.into_iter().next().unwrap(),
        _ => ir::Node::Cat(nodes),
    }
}

fn make_alt(mut nodes: ir::NodeList) -> ir::Node {
    match nodes.len() {
        0 => ir::Node::Empty,
        1 => nodes.pop().expect("one alternative remains"),
        n => {
            let right = nodes.split_off(n / 2);
            ir::Node::Alt(Box::new(make_alt(nodes)), Box::new(make_alt(right)))
        }
    }
}

/// \return a CodePointSet for a given character escape (positive or negative).
/// See ES9 21.2.2.12.
/// Returns the positive (non-inverted) code point set for a character class.
fn codepoints_from_class_positive(ct: CharacterClassType) -> CodePointSet {
    let mut cps;
    match ct {
        CharacterClassType::Digits => {
            cps = CodePointSet::from_sorted_disjoint_intervals(charclasses::DIGITS.to_vec())
        }
        CharacterClassType::Words => {
            cps = CodePointSet::from_sorted_disjoint_intervals(charclasses::WORD_CHARS.to_vec())
        }
        CharacterClassType::Spaces => {
            cps = CodePointSet::from_sorted_disjoint_intervals(charclasses::WHITESPACE.to_vec());
            for &iv in charclasses::LINE_TERMINATOR.iter() {
                cps.add(iv)
            }
        }
    }
    cps
}

/// Returns code points for a character class, optionally inverted.
fn codepoints_from_class(ct: CharacterClassType, positive: bool) -> CodePointSet {
    let cps = codepoints_from_class_positive(ct);
    if positive { cps } else { cps.inverted() }
}

/// \return a Bracket for a given character escape (positive or negative).
/// For icase mode, we expand the positive set first, then invert if needed.
fn make_bracket_class(
    ct: CharacterClassType,
    positive: bool,
    icase: bool,
    budget: &crate::Budget,
) -> Result<ir::Node, Error> {
    // Get the positive (non-inverted) set, perform any icase expansion, then maybe invert.
    let mut cps = codepoints_from_class_positive(ct);
    if icase {
        cps = unicode::add_icase_code_points_budget(cps, budget)?;
    }
    if !positive {
        cps = cps.inverted();
    }
    Ok(ir::Node::Bracket(BracketContents { invert: false, cps }))
}

fn add_class_atom(bc: &mut BracketContents, atom: ClassAtom) {
    match atom {
        ClassAtom::CodePoint(c) => bc.cps.add_one(c),
        ClassAtom::CharacterClass {
            class_type,
            positive,
        } => {
            bc.cps.add_set(codepoints_from_class(class_type, positive));
        }
        ClassAtom::Range { iv, negate } => {
            if negate {
                bc.cps.add_set(iv.inverted());
            } else {
                bc.cps.add_set(iv);
            }
        }
    }
}

struct LookaroundParams {
    negate: bool,
    backwards: bool,
}

/// Represents an alternative path in a regex pattern.
/// For example, in `/(?<a>x)|(?<a>y)/`, the two occurrences of 'a' are in different
/// alternative paths (separated by |), so they don't conflict.
/// Each element in the vector is (depth, alternative_index) where:
/// - depth: parenthesis nesting level (0 = top level)
/// - alternative_index: which alternative at that depth (0 = first, 1 = second after |, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
struct AlternativePath {
    /// Vector of (depth, alternative_index) pairs representing the path through alternatives
    segments: Vec<(usize, usize)>,
}

impl AlternativePath {
    /// Check if two alternative paths conflict (i.e., are in the same alternative branch).
    /// Two paths conflict if they share the same alternative indices at all common depth levels.
    /// Example:
    ///   - [(0, 0)] and [(0, 0), (1, 0)] conflict (second is nested within first)
    ///   - [(0, 0)] and [(0, 1)] don't conflict (different alternatives at depth 0)
    fn conflicts_with(&self, other: &AlternativePath) -> bool {
        let min_len = self.segments.len().min(other.segments.len());
        self.segments[..min_len] == other.segments[..min_len]
    }
}

/// Represents the state used to parse a regex.
struct Parser<I>
where
    I: Iterator<Item = u32>,
{
    budget: crate::Budget,
    bounded: bool,

    /// The remaining input.
    input: Peekable<I>,

    /// Flags used.
    flags: api::Flags,

    /// Number of loops.
    loop_count: u32,

    /// Number of capturing groups.
    group_count: CaptureGroupID,

    /// Maximum number of capturing groups.
    group_count_max: u32,

    /// A map each from capture group name to corresponding group indices in order.
    /// Note duplicate names may appear in distinct alternations, per the TC39 proposal.
    /// See <https://github.com/tc39/proposal-duplicate-named-capturing-groups>
    named_group_indices: HashMap<CaptureGroupName, Vec<u32>>,

    /// Whether a lookbehind was encountered.
    has_lookbehind: bool,

    /// Current recursion nesting depth.
    depth: u32,
}

impl<I> Parser<I>
where
    I: Iterator<Item = u32> + Clone,
{
    /// Consume a character, returning it.
    fn consume<C: Into<u32>>(&mut self, c: C) -> u32 {
        let nc = self.input.next();
        core::debug_assert!(nc == Some(c.into()), "char was not next");
        nc.unwrap()
    }

    /// If our contents begin with the char c, consume it from our contents
    /// and return true. Otherwise return false.
    fn try_consume<C: Into<u32>>(&mut self, c: C) -> bool {
        self.input.next_if_eq(&c.into()).is_some()
    }

    /// If our contents begin with the string \p s, consume it from our contents
    /// and return true. Otherwise return false.
    fn try_consume_str(&mut self, s: &str) -> bool {
        let mut cursor = self.input.clone();
        for c1 in s.chars() {
            if cursor.next() != Some(c1 as u32) {
                return false;
            }
        }
        self.input = cursor;
        true
    }

    /// Build a node matching a code point `c`, optionally with case-insensitivity.
    fn char_node(&self, c: u32) -> Result<ir::Node, Error> {
        if !self.flags.icase {
            return Ok(ir::Node::Char { c });
        }
        self.budget.charge(unicode::literal_fold_cost(), 128)?;
        let class = unicode::expand_code_point(c, self.flags.icase, self.flags.unicode);
        Ok(match class.len() {
            1 => ir::Node::Char { c: class[0] },
            2..=MAX_CHAR_SET_LENGTH => ir::Node::CharSet(class),
            _ => panic!("Unicode case fold exceeded maximum expansion"),
        })
    }

    /// Peek at the next character.
    fn peek(&mut self) -> Option<u32> {
        self.input.peek().copied()
    }

    /// \return the next character.
    fn next(&mut self) -> Option<u32> {
        self.input.next()
    }

    fn try_parse(&mut self) -> Result<ir::Regex, Error> {
        self.parse_capture_groups()?;

        // Parse a catenation. If we consume everything, it's success. If there's
        // something left, it's an error (for example, an excess closing paren).
        let body = self.consume_disjunction()?;
        match self.input.peek().copied() {
            Some(c) if c == ')' as u32 => error("Unbalanced parenthesis"),
            Some(c) => error(format!(
                "Unexpected char: {}",
                char::from_u32(c)
                    .map(String::from)
                    .unwrap_or_else(|| format!("\\u{c:04X}"))
            )),
            None => self.finalize(ir::Regex {
                node: make_cat(vec![body, ir::Node::Goal]),
                flags: self.flags,
            }),
        }
    }

    /// ES6 21.2.2.3 Disjunction.
    fn consume_disjunction(&mut self) -> Result<ir::Node, Error> {
        self.budget.charge(1, 0)?;
        self.depth += 1;
        if self.bounded && self.depth > 32 {
            return Err(self.budget.reject::<()>().unwrap_err().into());
        }
        if self.depth > MAX_NESTING_DEPTH {
            return error("Regular expression is too deeply nested");
        }
        let mut terms = vec![self.consume_term()?];
        while self.try_consume('|') {
            terms.push(self.consume_term()?)
        }
        self.depth -= 1;
        Ok(make_alt(terms))
    }

    /// ES6 21.2.2.5 Term.
    fn consume_term(&mut self) -> Result<ir::Node, Error> {
        let mut result: Vec<ir::Node> = Vec::new();
        loop {
            self.budget.check()?;
            let start_group = self.group_count;
            let mut start_offset = result.len();
            let mut quantifier_allowed = true;

            let nc = self.peek();
            if nc.is_none() {
                return Ok(make_cat(result));
            }
            let c = nc.unwrap();
            match to_char_sat(c) {
                // A concatenation is terminated by closing parens or vertical bar (alternations).
                ')' | '|' => break,
                // Term :: Assertion :: ^
                '^' => {
                    self.consume('^');
                    result.push(ir::Node::Anchor {
                        anchor_type: ir::AnchorType::StartOfLine,
                        multiline: self.flags.multiline,
                    });
                    quantifier_allowed = false;
                }
                // Term :: Assertion :: $
                '$' => {
                    self.consume('$');
                    result.push(ir::Node::Anchor {
                        anchor_type: ir::AnchorType::EndOfLine,
                        multiline: self.flags.multiline,
                    });
                    quantifier_allowed = false;
                }

                '\\' => {
                    self.consume('\\');
                    let Some(c) = self.peek() else {
                        return error("Incomplete escape");
                    };
                    match to_char_sat(c) {
                        // Term :: Assertion :: \b
                        'b' => {
                            self.consume('b');
                            result.push(ir::Node::WordBoundary {
                                invert: false,
                                unicode_icase: self.flags.unicode && self.flags.icase,
                            });
                        }
                        // Term :: Assertion :: \B
                        'B' => {
                            self.consume('B');
                            result.push(ir::Node::WordBoundary {
                                invert: true,
                                unicode_icase: self.flags.unicode && self.flags.icase,
                            });
                        }
                        // Term :: Atom :: \ AtomEscape :: CharacterEscape :: c AsciiLetter
                        // Term :: ExtendedAtom :: \ [lookahead = c]
                        'c' if !self.flags.unicode => {
                            self.consume('c');
                            if self
                                .peek()
                                .and_then(char::from_u32)
                                .map(|c| c.is_ascii_alphabetic())
                                == Some(true)
                            {
                                let cp = self.next().expect("char was not next") % 32;
                                result.push(self.char_node(cp)?);
                            } else {
                                start_offset += 1;
                                result.push(self.char_node(u32::from('\\'))?);
                                result.push(self.char_node(u32::from('c'))?);
                            }
                        }
                        // Term :: Atom :: \ AtomEscape
                        _ => {
                            result.push(self.consume_atom_escape()?);
                        }
                    }
                }

                // Term :: Atom :: .
                '.' => {
                    self.consume('.');
                    result.push(if self.flags.dot_all {
                        ir::Node::MatchAny
                    } else {
                        ir::Node::MatchAnyExceptLineTerminator
                    });
                }

                '(' => {
                    if self.try_consume_str("(?=") {
                        // Positive lookahead.
                        quantifier_allowed = !self.flags.unicode;
                        result.push(self.consume_lookaround_assertion(LookaroundParams {
                            negate: false,
                            backwards: false,
                        })?);
                    } else if self.try_consume_str("(?!") {
                        // Negative lookahead.
                        quantifier_allowed = !self.flags.unicode;
                        result.push(self.consume_lookaround_assertion(LookaroundParams {
                            negate: true,
                            backwards: false,
                        })?);
                    } else if self.try_consume_str("(?<=") {
                        // Positive lookbehind.
                        quantifier_allowed = false;
                        self.has_lookbehind = true;
                        result.push(self.consume_lookaround_assertion(LookaroundParams {
                            negate: false,
                            backwards: true,
                        })?);
                    } else if self.try_consume_str("(?<!") {
                        // Negative lookbehind.
                        quantifier_allowed = false;
                        self.has_lookbehind = true;
                        result.push(self.consume_lookaround_assertion(LookaroundParams {
                            negate: true,
                            backwards: true,
                        })?);
                    } else if self.try_consume_str("(?:") {
                        // Non-capturing group.
                        result.push(self.consume_disjunction()?);
                    } else if let Some(group) = self.try_consume_modifier_group()? {
                        result.push(group);
                    } else {
                        // Capturing group.
                        self.consume('(');
                        let group = self.group_count;
                        if self.group_count as usize >= MAX_CAPTURE_GROUPS {
                            return error("Capture group count limit exceeded");
                        }
                        self.group_count += 1;

                        // Maybe the capture group has a name!
                        let mut group_name = None;
                        if self.try_consume_str("?") {
                            group_name = self.try_consume_named_capture_group_name();
                            if group_name.is_none() {
                                return error("Invalid token at named capture group identifier");
                            };
                        }
                        let contents = Box::new(self.consume_disjunction()?);
                        result.push(ir::Node::CaptureGroup {
                            id: group,
                            contents,
                            name: group_name,
                        })
                    }
                    if !self.try_consume(')') {
                        return error("Unbalanced parenthesis");
                    }
                }

                // CharacterClass :: ClassContents :: ClassSetExpression
                '[' if self.flags.unicode_sets => {
                    self.consume('[');
                    let negate_set = self.try_consume('^');
                    result.push(self.consume_class_set_expression(negate_set)?.node(
                        self.flags.icase,
                        negate_set,
                        &self.budget,
                    )?);
                }

                '[' => {
                    result.push(self.consume_bracket()?);
                }

                // Term :: ExtendedAtom :: InvalidBracedQuantifier
                '{' if !self.flags.unicode => {
                    if self.try_consume_braced_quantifier().is_some() {
                        return error("Invalid braced quantifier");
                    }

                    // Term :: ExtendedAtom :: ExtendedPatternCharacter
                    let cp = self.consume(c);
                    result.push(self.char_node(cp)?)
                }

                // Term :: Atom :: PatternCharacter :: SourceCharacter but not ^ $ \ . * + ? ( ) [ ] { } |
                '*' | '+' | '?' | ']' | '{' | '}' if self.flags.unicode => {
                    return error("Invalid atom character");
                }

                // Term :: ExtendedAtom :: SourceCharacter but not ^ $ \ . * + ? ( ) [ |
                '*' | '+' | '?' => {
                    return error("Invalid atom character");
                }

                // Term :: Atom :: PatternCharacter
                // Term :: ExtendedAtom :: ExtendedPatternCharacter
                _ => {
                    self.consume(c);
                    result.push(self.char_node(c)?)
                }
            }

            // We just parsed a term; try parsing a quantifier.
            if let Some(quant) = self.try_consume_quantifier()? {
                if !quantifier_allowed {
                    return error("Quantifier not allowed here");
                }
                // Validate the quantifier.
                // Note we don't want to do this as part of parsing the quantiifer in some cases
                // an incomplete quantifier is not recognized as a quantifier, e.g. `/{3/` is
                // valid.
                if matches!(quant.max, Some(max) if quant.min > max) {
                    return error("Invalid quantifier");
                }
                let quantifee = result.split_off(start_offset);
                if self.loop_count as usize >= MAX_LOOPS {
                    return error("Loop count limit exceeded");
                }
                self.loop_count += 1;
                result.push(ir::Node::Loop {
                    loopee: Box::new(make_cat(quantifee)),
                    quant,
                    enclosed_groups: start_group..self.group_count,
                });
            }
        }
        Ok(make_cat(result))
    }

    fn try_consume_modifier_group(&mut self) -> Result<Option<ir::Node>, Error> {
        let mut cursor = self.input.clone();
        if cursor.next() != Some('(' as u32) {
            return Ok(None);
        }
        if cursor.next() != Some('?' as u32) {
            return Ok(None);
        }
        let Some(mut current) = cursor.next() else {
            return Ok(None);
        };

        if to_char_sat(current) == '<' {
            return Ok(None);
        }

        let mut seen_hyphen = false;
        let mut saw_flag = false;
        let mut icase_override: Option<bool> = None;
        let mut multiline_override: Option<bool> = None;
        let mut dot_all_override: Option<bool> = None;

        loop {
            let ch = to_char_sat(current);
            match ch {
                'i' | 'm' | 's' => {
                    let value = !seen_hyphen;
                    let target = match ch {
                        'i' => &mut icase_override,
                        'm' => &mut multiline_override,
                        's' => &mut dot_all_override,
                        _ => unreachable!(),
                    };
                    if target.is_some() {
                        return error("Invalid group modifier");
                    }
                    *target = Some(value);
                    saw_flag = true;
                }
                '-' => {
                    if seen_hyphen {
                        return error("Invalid group modifier");
                    }
                    seen_hyphen = true;
                }
                ':' => {
                    if !saw_flag {
                        return error("Invalid group modifier");
                    }
                    self.input = cursor;
                    let mut new_flags = self.flags;
                    if let Some(value) = icase_override {
                        new_flags.icase = value;
                    }
                    if let Some(value) = multiline_override {
                        new_flags.multiline = value;
                    }
                    if let Some(value) = dot_all_override {
                        new_flags.dot_all = value;
                    }
                    let saved_flags = self.flags;
                    self.flags = new_flags;
                    let contents = match self.consume_disjunction() {
                        Ok(node) => node,
                        Err(err) => {
                            self.flags = saved_flags;
                            return Err(err);
                        }
                    };
                    self.flags = saved_flags;
                    return Ok(Some(contents));
                }
                _ => {
                    return error("Invalid group modifier");
                }
            }

            current = match cursor.next() {
                Some(next) => next,
                None => return error("Invalid group modifier"),
            };
        }
    }

    /// ES6 21.2.2.13 CharacterClass.
    fn consume_bracket(&mut self) -> Result<ir::Node, Error> {
        self.consume('[');
        let invert = self.try_consume('^');
        let mut result = BracketContents {
            invert,
            cps: CodePointSet::default(),
        };

        loop {
            match self.peek().map(to_char_sat) {
                None => {
                    return error("Unbalanced bracket");
                }
                Some(']') => {
                    self.consume(']');
                    if self.flags.icase {
                        result.cps =
                            unicode::add_icase_code_points_budget(result.cps, &self.budget)?;
                    }
                    return Ok(ir::Node::Bracket(result));
                }
                _ => {}
            }

            // Parse a code point or character class.
            let Some(first) = self.try_consume_bracket_class_atom()? else {
                continue;
            };

            // Check for a dash; we may have a range.
            if !self.try_consume('-') {
                add_class_atom(&mut result, first);
                continue;
            }

            let Some(second) = self.try_consume_bracket_class_atom()? else {
                // No second atom. For example: [a-].
                add_class_atom(&mut result, first);
                add_class_atom(&mut result, ClassAtom::CodePoint(u32::from('-')));
                continue;
            };

            // Ranges must also be in order: z-a is invalid.
            // ES6 21.2.2.15.1 "If i > j, throw a SyntaxError exception"
            if let (ClassAtom::CodePoint(c1), ClassAtom::CodePoint(c2)) = (&first, &second) {
                if c1 > c2 {
                    return error(
                        "Range values reversed, start char code is greater than end char code.",
                    );
                }
                result.cps.add(Interval {
                    first: *c1,
                    last: *c2,
                });

                continue;
            }

            if self.flags.unicode {
                return error("Invalid character range");
            }

            // If it does not match a range treat as any match single characters.
            add_class_atom(&mut result, first);
            add_class_atom(&mut result, ClassAtom::CodePoint(u32::from('-')));
            add_class_atom(&mut result, second);
        }
    }

    fn try_consume_bracket_class_atom(&mut self) -> Result<Option<ClassAtom>, Error> {
        let c = self.peek();
        if c.is_none() {
            return Ok(None);
        }
        let c = c.unwrap();
        match to_char_sat(c) {
            // End of bracket.
            ']' => Ok(None),

            // ClassEscape
            '\\' => {
                self.consume('\\');
                let ec = if let Some(ec) = self.peek() {
                    ec
                } else {
                    return error("Unterminated escape");
                };
                match to_char_sat(ec) {
                    // ClassEscape :: b
                    'b' => {
                        self.consume('b');
                        Ok(Some(ClassAtom::CodePoint(u32::from('\x08'))))
                    }
                    // ClassEscape :: [+UnicodeMode] -
                    '-' if self.flags.unicode => {
                        self.consume('-');
                        Ok(Some(ClassAtom::CodePoint(u32::from('-'))))
                    }
                    'c' if !self.flags.unicode => {
                        let input = self.input.clone();
                        self.consume('c');
                        match self.peek().map(to_char_sat) {
                            // ClassEscape :: [~UnicodeMode] c ClassControlLetter
                            Some('0'..='9' | '_') => {
                                let next = self.next().expect("char was not next");
                                Ok(Some(ClassAtom::CodePoint(next & 0x1F)))
                            }
                            // CharacterEscape :: c AsciiLetter
                            Some('a'..='z' | 'A'..='Z') => {
                                let next = self.next().expect("char was not next");
                                Ok(Some(ClassAtom::CodePoint(next % 32)))
                            }
                            // ClassAtomNoDash :: \ [lookahead = c]
                            _ => {
                                self.input = input;
                                Ok(Some(ClassAtom::CodePoint(u32::from('\\'))))
                            }
                        }
                    }
                    // ClassEscape :: CharacterClassEscape :: d
                    'd' => {
                        self.consume('d');
                        Ok(Some(ClassAtom::CharacterClass {
                            class_type: CharacterClassType::Digits,
                            positive: true,
                        }))
                    }
                    // ClassEscape :: CharacterClassEscape :: D
                    'D' => {
                        self.consume('D');
                        Ok(Some(ClassAtom::CharacterClass {
                            class_type: CharacterClassType::Digits,
                            positive: false,
                        }))
                    }
                    // ClassEscape :: CharacterClassEscape :: s
                    's' => {
                        self.consume('s');
                        Ok(Some(ClassAtom::CharacterClass {
                            class_type: CharacterClassType::Spaces,
                            positive: true,
                        }))
                    }
                    // ClassEscape :: CharacterClassEscape :: S
                    'S' => {
                        self.consume('S');
                        Ok(Some(ClassAtom::CharacterClass {
                            class_type: CharacterClassType::Spaces,
                            positive: false,
                        }))
                    }
                    // ClassEscape :: CharacterClassEscape :: w
                    'w' => {
                        self.consume('w');
                        Ok(Some(ClassAtom::CharacterClass {
                            class_type: CharacterClassType::Words,
                            positive: true,
                        }))
                    }
                    // ClassEscape :: CharacterClassEscape :: W
                    'W' => {
                        self.consume('W');
                        Ok(Some(ClassAtom::CharacterClass {
                            class_type: CharacterClassType::Words,
                            positive: false,
                        }))
                    }
                    // ClassEscape :: CharacterClassEscape :: [+UnicodeMode] p{ UnicodePropertyValueExpression }
                    // ClassEscape :: CharacterClassEscape :: [+UnicodeMode] P{ UnicodePropertyValueExpression }
                    'p' | 'P' if self.flags.unicode => {
                        self.consume(ec);
                        let negate = ec == 'P' as u32;
                        match self.try_consume_unicode_property_escape()? {
                            PropertyEscapeKind::CharacterClass(s) => Ok(Some(ClassAtom::Range {
                                iv: CodePointSet::from_sorted_disjoint_intervals(s.to_vec()),
                                negate,
                            })),
                            PropertyEscapeKind::StringSet(_) => error("Invalid property escape"),
                        }
                    }
                    // ClassEscape :: CharacterEscape
                    _ => {
                        let cc = self.consume_character_escape()?;
                        Ok(Some(ClassAtom::CodePoint(cc)))
                    }
                }
            }

            _ => Ok(Some(ClassAtom::CodePoint(self.consume(c)))),
        }
    }

    // CharacterClass :: ClassContents :: ClassSetExpression
    // `in_negated_class` forbids string operands. It does not invert the result.
    fn consume_class_set_expression(&mut self, in_negated_class: bool) -> Result<ClassSet, Error> {
        let result = self.consume_class_set_expression_body()?;
        if in_negated_class && result.may_contain_strings {
            return error("Negated class may contain strings");
        }
        Ok(result)
    }

    fn consume_class_set_expression_body(&mut self) -> Result<ClassSet, Error> {
        let mut result = ClassSet::new();
        if self.try_consume(']') {
            return Ok(result);
        }
        let first = self.consume_class_set_operand()?;
        let operator = self.class_set_operator();
        if let Some(operator) = operator {
            result.union_operand(first, &self.budget, self.flags.icase)?;
            loop {
                self.consume(operator);
                self.consume(operator);
                if operator == '&' && self.peek() == Some('&' as u32) {
                    return error("Invalid class intersection operand");
                }
                let operand = self.consume_class_set_operand()?;
                if operator == '&' {
                    result.intersect_operand(operand, &self.budget, self.flags.icase)?;
                } else {
                    result.subtract_operand(operand, &self.budget, self.flags.icase)?;
                }
                if self.try_consume(']') {
                    return Ok(result);
                }
                if self.class_set_operator() != Some(operator) {
                    return error("Invalid class set operator");
                }
            }
        }
        let first = self.consume_class_range(first)?;
        result.union_operand(first, &self.budget, self.flags.icase)?;
        loop {
            if self.try_consume(']') {
                return Ok(result);
            }
            let operand = self.consume_class_set_operand()?;
            let operand = self.consume_class_range(operand)?;
            result.union_operand(operand, &self.budget, self.flags.icase)?;
        }
    }

    fn class_set_operator(&self) -> Option<char> {
        let mut lookahead = self.input.clone();
        let first = lookahead.next()?;
        if matches!(first, 0x26 | 0x2d) && lookahead.next() == Some(first) {
            char::from_u32(first)
        } else {
            None
        }
    }

    fn consume_class_range(&mut self, first: ClassSetOperand) -> Result<ClassSetOperand, Error> {
        if !self.try_consume('-') {
            return Ok(first);
        }
        let ClassSetOperand::ClassSetCharacter(first) = first else {
            return error("Invalid class set range");
        };
        let ClassSetOperand::ClassSetCharacter(last) = self.consume_class_set_operand()? else {
            return error("Invalid class set range");
        };
        if first > last {
            return error("Invalid class set range");
        }
        self.budget.charge(1, 32)?;
        Ok(ClassSetOperand::CharacterClassEscape(
            CodePointSet::from_sorted_disjoint_intervals(vec![Interval { first, last }]),
        ))
    }

    fn consume_class_set_operand(&mut self) -> Result<ClassSetOperand, Error> {
        use ClassSetOperand::*;
        let Some(cp) = self.peek() else {
            return error("Empty class set operand");
        };
        match cp {
            // ClassSetOperand :: NestedClass :: [ [lookahead ≠ ^] ClassContents[+UnicodeMode, +UnicodeSetsMode] ]
            // ClassSetOperand :: NestedClass :: [^ ClassContents[+UnicodeMode, +UnicodeSetsMode] ]
            0x5B /* [ */ => {
                self.depth += 1;
                if self.bounded && self.depth > 32 {
            return Err(self.budget.reject::<()>().unwrap_err().into());
        }
        if self.depth > MAX_NESTING_DEPTH {
                    return error("Regular expression is too deeply nested");
                }
                self.consume('[');
                let negate_set = self.try_consume('^');
                let mut result = self.consume_class_set_expression(negate_set)?;
                if negate_set {
                    self.budget.charge(result.codepoints.intervals().len() + 1, (result.codepoints.intervals().len() + 1) * 32)?;
                    result.codepoints = result.codepoints.inverted();
                }
                self.depth -= 1;
                Ok(Class(result))
            }
            // ClassSetOperand :: NestedClass :: \ CharacterClassEscape
            // ClassSetOperand :: ClassStringDisjunction
            // ClassSetOperand :: ClassSetCharacter :: \...
            // ClassSetRange :: ClassSetCharacter :: \...
            0x5C /* \ */ => {
                self.consume('\\');
                let Some(cp) = self.peek() else {
                    return error("Incomplete class set escape");
                };
                match cp {
                    // ClassStringDisjunction  \q{ ClassStringDisjunctionContents }
                    0x71 /* q */ => {
                        self.consume('q');
                        if !self.try_consume('{') {
                            return error("Invalid class set escape: expected {");
                        }
                        let mut alternatives = Vec::new();
                        let mut alternative = Vec::new();
                        loop {
                            match self.peek() {
                                Some(0x7D /* } */) => {
                                    self.consume('}');
                                    alternatives.push(alternative.into_boxed_slice());
                                    break;
                                }
                                Some(0x7C /* | */) => {
                                    self.consume('|');
                                    let alternative = mem::take(&mut alternative).into_boxed_slice();
                                    alternatives.push(alternative);
                                }
                                Some(_) => {
                                    alternative.push(self.consume_class_set_character()?);
                                }
                                None => {
                                    return error("Unbalanced class set string disjunction");
                                }
                            }
                        }
                        let may_contain_strings = alternatives.iter().any(|s| s.len() != 1);
                        Ok(ClassStringDisjunction(ClassSetAlternativeStrings { strings: alternatives, may_contain_strings }))
                    }
                    // CharacterClassEscape :: d
                    0x64 /* d */ => {
                        self.consume('d');
                        self.class_set_escape(CharacterClassType::Digits, true)
                    }
                    // CharacterClassEscape :: D
                    0x44 /* D */ => {
                        self.consume('D');
                        self.class_set_escape(CharacterClassType::Digits, false)
                    }
                    // CharacterClassEscape :: s
                    0x73 /* s */ => {
                        self.consume('s');
                        self.class_set_escape(CharacterClassType::Spaces, true)
                    }
                    // CharacterClassEscape :: S
                    0x53 /* S */ => {
                        self.consume('S');
                        self.class_set_escape(CharacterClassType::Spaces, false)
                    }
                    // CharacterClassEscape :: w
                    0x77 /* w */ => {
                        self.consume('w');
                        self.class_set_escape(CharacterClassType::Words, true)
                    }
                    // CharacterClassEscape :: W
                    0x57 /* W */ => {
                        self.consume('W');
                        self.class_set_escape(CharacterClassType::Words, false)
                    }
                    // CharacterClassEscape :: [+UnicodeMode] p{ UnicodePropertyValueExpression }
                    0x70 /* p */ => {
                        self.consume('p');
                        match self.try_consume_unicode_property_escape()? {
                            PropertyEscapeKind::CharacterClass(intervals) => {
                                Ok(CharacterClassEscape(CodePointSet::from_sorted_disjoint_intervals(
                                    intervals.to_vec(),
                                )))
                            }
                            PropertyEscapeKind::StringSet(strings) => {
                                Ok(ClassStringDisjunction(ClassSetAlternativeStrings { strings: strings.iter().map(|s| Box::from(*s)).collect(), may_contain_strings: true }))
                            }
                        }
                    }
                    // CharacterClassEscape :: [+UnicodeMode] P{ UnicodePropertyValueExpression }
                    0x50 /* P */ => {
                        self.consume('P');
                        match self.try_consume_unicode_property_escape()? {
                            PropertyEscapeKind::CharacterClass(s) => {
                                let mut cps = CodePointSet::from_sorted_disjoint_intervals(s.to_vec());
                                if self.flags.icase { cps = unicode::add_icase_code_points_budget(cps, &self.budget)?; }
                                self.budget.charge(cps.intervals().len() + 1, (cps.intervals().len() + 1).saturating_mul(32))?;
                                let mut class = ClassSet::new();
                                class.codepoints = cps.inverted();
                                Ok(Class(class))
                            }
                            PropertyEscapeKind::StringSet(_) => error("Invalid character escape"),
                        }
                    }
                    // ClassSetCharacter:: \b
                    0x62 /* b */ => {
                        self.consume(cp);
                        Ok(ClassSetCharacter(8))
                    }
                    // ClassSetCharacter:: \ ClassSetReservedPunctuator
                    _ if Self::is_class_set_reserved_punctuator(cp) => Ok(ClassSetCharacter(self.consume(cp))),
                    // ClassSetCharacter:: \ CharacterEscape[+UnicodeMode]
                    _ => Ok(ClassSetCharacter(self.consume_character_escape()?))
                }
            }
            // ClassSetOperand :: ClassSetCharacter
            // ClassSetRange :: ClassSetCharacter
            _ => Ok(ClassSetCharacter(self.consume_class_set_character()?)),
        }
    }

    fn class_set_escape(
        &self,
        kind: CharacterClassType,
        positive: bool,
    ) -> Result<ClassSetOperand, Error> {
        let mut class = ClassSet::new();
        let mut cps = codepoints_from_class_positive(kind);
        if self.flags.icase {
            cps = unicode::add_icase_code_points_budget(cps, &self.budget)?;
        }
        self.budget.charge(
            cps.intervals().len() + 1,
            (cps.intervals().len() + 1).saturating_mul(32),
        )?;
        class.codepoints = if positive { cps } else { cps.inverted() };
        Ok(ClassSetOperand::Class(class))
    }

    // ClassSetCharacter
    fn consume_class_set_character(&mut self) -> Result<u32, Error> {
        let Some(cp) = self.next() else {
            return error("Incomplete class set character");
        };
        match cp {
            0x5C /* \ */ => {
                let Some(cp) = self.peek() else {
                    return error("Incomplete class set escape");
                };
                match cp {
                    // \b
                    0x62 /* b */ => {
                        self.consume(cp);
                        Ok(8)
                    }
                    // \ ClassSetReservedPunctuator
                    _ if Self::is_class_set_reserved_punctuator(cp) => Ok(self.consume(cp)),
                    // \ CharacterEscape[+UnicodeMode]
                    _ => Ok(self.consume_character_escape()?)
                }
            }
            // [lookahead ∉ ClassSetReservedDoublePunctuator] SourceCharacter but not ClassSetSyntaxCharacter
            0x28 /* ( */ | 0x29 /* ) */ | 0x7B /* { */ | 0x7D /* } */ | 0x2F /* / */
            | 0x2D /* - */ | 0x7C /* | */ | 0x5B /* [ */ | 0x5D /* ] */ => error("Invalid class set character"),
            _ => {
                if Self::is_class_set_reserved_double_punctuator(cp)
                    && self.peek() == Some(cp) {
                            return error("Invalid class set character");
                        }
                Ok(cp)
            }
        }
    }

    // ClassSetReservedPunctuator
    fn is_class_set_reserved_punctuator(cp: u32) -> bool {
        match cp {
            0x26 /* & */ | 0x2D /* - */ | 0x21 /* ! */ | 0x23 /* # */ | 0x25 /* % */
            | 0x2C /* , */ | 0x3A /* : */ | 0x3B /* ; */ | 0x3C /* < */ | 0x3D /* = */
            | 0x3E /* > */ | 0x40 /* @ */ | 0x60 /* ` */ | 0x7E /* ~ */ => true,
            _ => false,
        }
    }

    fn is_class_set_reserved_double_punctuator(cp: u32) -> bool {
        match cp {
            0x26 /* & */ | 0x21 /* ! */ | 0x23 /* # */ | 0x24 /* $ */ | 0x25 /* % */
            | 0x2A /* * */ | 0x2B /* + */ | 0x2C /* , */ | 0x2E /* . */ | 0x3A /* : */
            | 0x3B /* ; */ | 0x3C /* < */ | 0x3D /* = */ | 0x3E /* > */ | 0x3F /* ? */
            | 0x40 /* @ */ | 0x5E /* ^ */ | 0x60 /* ` */ | 0x7E /* ~ */ => true,
            _ => false,
        }
    }

    fn try_consume_quantifier(&mut self) -> Result<Option<ir::Quantifier>, Error> {
        if let Some(mut quant) = self.try_consume_quantifier_prefix()? {
            quant.greedy = !self.try_consume('?');
            Ok(Some(quant))
        } else {
            Ok(None)
        }
    }

    fn try_consume_quantifier_prefix(&mut self) -> Result<Option<ir::Quantifier>, Error> {
        let nc = self.peek();
        if nc.is_none() {
            return Ok(None);
        }
        let c = nc.unwrap();
        match char::from_u32(c) {
            Some('+') => {
                self.consume('+');
                Ok(Some(ir::Quantifier {
                    min: 1,
                    max: None,
                    greedy: true,
                }))
            }
            Some('*') => {
                self.consume('*');
                Ok(Some(ir::Quantifier {
                    min: 0,
                    max: None,
                    greedy: true,
                }))
            }
            Some('?') => {
                self.consume('?');
                Ok(Some(ir::Quantifier {
                    min: 0,
                    max: Some(1),
                    greedy: true,
                }))
            }
            Some('{') => {
                if let Some(quantifier) = self.try_consume_braced_quantifier() {
                    Ok(Some(quantifier))
                } else if self.flags.unicode {
                    // if there was a brace '{' that doesn't parse into a valid quantifier,
                    // it's not valid with the unicode flag
                    error("Invalid quantifier")
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn try_consume_braced_quantifier(&mut self) -> Option<ir::Quantifier> {
        // if parsed input is actually invalid, keep the previous one for rollback
        let pre_input = self.input.clone();
        self.consume('{');
        let optmin = self.try_consume_decimal_integer_literal();
        let Some(optmin) = optmin else {
            // not a valid quantifier, rollback consumption
            self.input = pre_input;
            return None;
        };
        let mut quant = ir::Quantifier {
            min: optmin,
            max: Some(optmin),
            greedy: true,
        };
        if self.try_consume(',') {
            let max = self.try_consume_decimal_integer_literal();
            // Either like {3,4} in which case we want to set the max;
            // or like {3,} in which case the max should be None to indicate unbounded.
            quant.max = max;
        } else {
            // Like {3}.
        }
        if !self.try_consume('}') {
            // not a valid quantifier, rollback consumption
            self.input = pre_input;
            return None;
        }
        Some(quant)
    }

    /// ES6 11.8.3 DecimalIntegerLiteral.
    /// If the value would overflow, usize::MAX is returned.
    /// All decimal digits are consumed regardless.
    fn try_consume_decimal_integer_literal(&mut self) -> Option<usize> {
        let mut result: usize = 0;
        let mut char_count = 0;
        while let Some(c) = self.peek() {
            if let Some(digit) = char::from_u32(c).and_then(|c| char::to_digit(c, 10)) {
                self.consume(c);
                char_count += 1;
                if self.bounded && result > (u32::MAX as usize - digit as usize) / 10 {
                    let _ = self.budget.reject::<()>();
                }
                result = result.saturating_mul(10);
                result = result.saturating_add(digit as usize);
                if self.bounded && result > u32::MAX as usize {
                    let _ = self.budget.reject::<()>();
                }
            } else {
                break;
            }
        }
        if char_count > 0 { Some(result) } else { None }
    }

    fn consume_lookaround_assertion(
        &mut self,
        params: LookaroundParams,
    ) -> Result<ir::Node, Error> {
        let start_group = self.group_count;
        let contents = self.consume_disjunction()?;
        let end_group = self.group_count;
        Ok(ir::Node::LookaroundAssertion {
            negate: params.negate,
            backwards: params.backwards,
            start_group,
            end_group,
            contents: Box::new(contents),
        })
    }

    fn consume_character_escape(&mut self) -> Result<u32, Error> {
        let c = self.next().expect("Should have a character");
        let ch = to_char_sat(c);
        match ch {
            // CharacterEscape :: ControlEscape :: f
            'f' => Ok(0xC),
            // CharacterEscape :: ControlEscape :: n
            'n' => Ok(0xA),
            // CharacterEscape :: ControlEscape :: r
            'r' => Ok(0xD),
            // CharacterEscape :: ControlEscape :: t
            't' => Ok(0x9),
            // CharacterEscape :: ControlEscape :: v
            'v' => Ok(0xB),
            // CharacterEscape :: c AsciiLetter
            'c' => {
                if let Some(nc) = self.next().and_then(char::from_u32)
                    && (nc.is_ascii_lowercase() || nc.is_ascii_uppercase())
                {
                    return Ok((nc as u32) % 32);
                }
                error("Invalid character escape")
            }
            // CharacterEscape :: 0 [lookahead ∉ DecimalDigit]
            '0' if self
                .peek()
                .and_then(char::from_u32)
                .map(|c: char| c.is_ascii_digit())
                != Some(true) =>
            {
                Ok(0x0)
            }
            // CharacterEscape :: HexEscapeSequence :: x HexDigit HexDigit
            'x' => {
                let orig_input = self.input.clone();
                let hex_to_digit = |c: char| c.to_digit(16);
                let x1 = self.next().and_then(char::from_u32).and_then(hex_to_digit);
                let x2 = self.next().and_then(char::from_u32).and_then(hex_to_digit);
                match (x1, x2) {
                    (Some(x1), Some(x2)) => Ok(x1 * 16 + x2),
                    // CharacterEscape :: IdentityEscape :: SourceCharacterIdentityEscape
                    // Not a valid HexEscapeSequence. Restore the input.
                    _ if !self.flags.unicode => {
                        self.input = orig_input;
                        Ok(c)
                    }
                    _ => error("Invalid character escape"),
                }
            }
            // CharacterEscape :: RegExpUnicodeEscapeSequence
            'u' => {
                if let Some(c) = self.try_escape_unicode_sequence() {
                    Ok(c)
                } else if !self.flags.unicode {
                    // CharacterEscape :: IdentityEscape :: SourceCharacterIdentityEscape
                    Ok(c)
                } else {
                    error("Invalid unicode escape")
                }
            }
            // CharacterEscape :: [~UnicodeMode] LegacyOctalEscapeSequence
            '0'..='7' if !self.flags.unicode => {
                let Some(c1) = self.peek() else {
                    return Ok(c - '0' as u32);
                };
                let ch1 = to_char_sat(c1);

                match ch {
                    // 0 [lookahead ∈ { 8, 9 }]
                    '0' if ('8'..='9').contains(&ch1) => Ok(0x0),
                    // NonZeroOctalDigit [lookahead ∉ OctalDigit]
                    _ if !('0'..='7').contains(&ch1) => Ok(c - '0' as u32),
                    // FourToSeven OctalDigit
                    '4'..='7' => {
                        self.consume(c1);
                        Ok((c - '0' as u32) * 8 + c1 - '0' as u32)
                    }
                    // ZeroToThree OctalDigit [lookahead ∉ OctalDigit]
                    // ZeroToThree OctalDigit OctalDigit
                    '0'..='3' => {
                        self.consume(c1);
                        if self.peek().map(|c2| ('0'..='7').contains(&to_char_sat(c2)))
                            == Some(true)
                        {
                            let c2 = self.next().expect("char was not next");
                            Ok((c - '0' as u32) * 64 + (c1 - '0' as u32) * 8 + c2 - '0' as u32)
                        } else {
                            Ok((c - '0' as u32) * 8 + c1 - '0' as u32)
                        }
                    }
                    _ => unreachable!(),
                }
            }
            // CharacterEscape :: IdentityEscape :: [+UnicodeMode] SyntaxCharacter
            // CharacterEscape :: IdentityEscape :: [+UnicodeMode] /
            '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
            | '/' => Ok(c),
            // CharacterEscape :: IdentityEscape :: SourceCharacterIdentityEscape
            _ if !self.flags.unicode => Ok(c),
            _ => error("Invalid character escape"),
        }
    }

    // AtomEscape
    fn consume_atom_escape(&mut self) -> Result<ir::Node, Error> {
        let Some(c) = self.peek() else {
            return error("Incomplete escape");
        };
        match to_char_sat(c) {
            'd' | 'D' => {
                self.consume(c);
                Ok(make_bracket_class(
                    CharacterClassType::Digits,
                    c == 'd' as u32,
                    self.flags.icase,
                    &self.budget,
                )?)
            }

            's' | 'S' => {
                self.consume(c);
                Ok(make_bracket_class(
                    CharacterClassType::Spaces,
                    c == 's' as u32,
                    self.flags.icase,
                    &self.budget,
                )?)
            }

            'w' | 'W' => {
                self.consume(c);
                Ok(make_bracket_class(
                    CharacterClassType::Words,
                    c == 'w' as u32,
                    self.flags.icase,
                    &self.budget,
                )?)
            }

            // ClassEscape :: CharacterClassEscape :: [+UnicodeMode] p{ UnicodePropertyValueExpression }
            // ClassEscape :: CharacterClassEscape :: [+UnicodeMode] P{ UnicodePropertyValueExpression }
            'p' | 'P' if self.flags.unicode || self.flags.unicode_sets => {
                self.consume(c);
                let negate = c == 'P' as u32;
                let property_escape = self.try_consume_unicode_property_escape()?;
                match property_escape {
                    PropertyEscapeKind::CharacterClass(s) => {
                        let mut cps = CodePointSet::from_sorted_disjoint_intervals(s.to_vec());
                        if self.flags.icase {
                            // Per ES2024: apply SimpleCaseFolding to the property set first.
                            // For \P (inverted): complement before expansion so that case
                            // variants of the complement are included (existential quantifier).
                            if negate && !self.flags.unicode_sets {
                                cps = unicode::add_icase_code_points_budget(
                                    cps.inverted(),
                                    &self.budget,
                                )?;
                            } else {
                                cps = unicode::add_icase_code_points_budget(cps, &self.budget)?;
                            }
                            Ok(ir::Node::Bracket(BracketContents {
                                invert: negate && self.flags.unicode_sets,
                                cps,
                            }))
                        } else {
                            Ok(ir::Node::Bracket(BracketContents {
                                invert: negate,
                                cps,
                            }))
                        }
                    }
                    PropertyEscapeKind::StringSet(_) if negate => error("Invalid character escape"),
                    PropertyEscapeKind::StringSet(strings) => Ok(ir::Node::StringSet {
                        alternatives: strings.iter().map(|s| Box::from(*s)).collect(),
                        icase: self.flags.icase,
                    }),
                }
            }

            // [+UnicodeMode] DecimalEscape
            // Note: This is a backreference.
            '1'..='9' if self.flags.unicode => {
                let group = self.try_consume_decimal_integer_literal().unwrap();
                if group <= self.group_count_max as usize {
                    Ok(ir::Node::BackRef {
                        group: group as u32,
                        icase: self.flags.icase,
                    })
                } else {
                    error("Invalid character escape")
                }
            }

            // [~UnicodeMode] DecimalEscape but only if the CapturingGroupNumber of DecimalEscape
            //    is ≤ CountLeftCapturingParensWithin(the Pattern containing DecimalEscape)
            // Note: This could be either a backreference, a legacy octal escape or an identity escape.
            '1'..='9' => {
                let input = self.input.clone();
                let group = self.try_consume_decimal_integer_literal().unwrap();

                if group <= self.group_count_max as usize {
                    Ok(ir::Node::BackRef {
                        group: group as u32,
                        icase: self.flags.icase,
                    })
                } else {
                    self.input = input;
                    let c = self.consume_character_escape()?;
                    Ok(self.char_node(c)?)
                }
            }

            // [+NamedCaptureGroups] k GroupName
            'k' if self.flags.unicode || !self.named_group_indices.is_empty() => {
                // The sequence `\k` must be the start of a backreference to a named capture group.
                // Note multiple capture groups may have the same name; we must map all of them to their indices.
                self.consume('k');
                // Must have a valid group name.
                let Some(group_name) = self.try_consume_named_capture_group_name() else {
                    return error("Invalid named backreference syntax");
                };
                // The group name must be the name of a previously defined capture group.
                let Some(group_indices) = self.named_group_indices.get(&group_name) else {
                    return error(format!(
                        "Backreference to invalid named capture group: {}",
                        group_name
                    ));
                };
                // Note backreferences are 1-based.
                let node = match group_indices.len() {
                    0 => unreachable!("Should not have empty indices for group name"),
                    1 => {
                        // Common case of a backref matching a single group.
                        ir::Node::BackRef {
                            group: group_indices[0] + 1,
                            icase: self.flags.icase,
                        }
                    }
                    _ => {
                        self.budget
                            .charge(group_indices.len(), group_indices.len().saturating_mul(256))?;
                        make_cat(
                            group_indices
                                .iter()
                                .map(|group_index| ir::Node::BackRef {
                                    group: *group_index + 1,
                                    icase: self.flags.icase,
                                })
                                .collect(),
                        )
                    }
                };
                Ok(node)
            }

            // [~NamedCaptureGroups] k GroupName
            'k' => {
                self.consume('k');
                Ok(self.char_node(c)?)
            }

            _ => {
                let c = self.consume_character_escape()?;
                Ok(self.char_node(c)?)
            }
        }
    }

    #[allow(clippy::branches_sharing_code)]
    fn try_escape_unicode_sequence(&mut self) -> Option<u32> {
        let mut orig_input = self.input.clone();

        // Support \u{X..X} (Unicode CodePoint)
        if self.try_consume('{') {
            let mut s = String::new();
            loop {
                match self.next().and_then(char::from_u32) {
                    Some('}') => break,
                    Some(c) => s.push(c),
                    None => {
                        // Surrogates not supported in code point escapes.
                        self.input = orig_input;
                        return None;
                    }
                }
            }

            match u32::from_str_radix(&s, 16) {
                Ok(u) => {
                    if u > 0x10_FFFF {
                        self.input = orig_input;
                        None
                    } else {
                        Some(u)
                    }
                }
                _ => {
                    self.input = orig_input;
                    None
                }
            }
        } else {
            // Hex4Digits
            let mut s = String::new();
            for _ in 0..4 {
                if let Some(c) = self.next().and_then(char::from_u32) {
                    s.push(c);
                } else {
                    // Surrogates are not hex digits.
                    self.input = orig_input;
                    return None;
                }
            }
            match u16::from_str_radix(&s, 16) {
                Ok(u) => {
                    if (0xD800..=0xDBFF).contains(&u) {
                        // Found a high surrogate. Try to parse a low surrogate next
                        // to see if we can rebuild the original `char`

                        if !self.try_consume_str("\\u") {
                            return Some(u as u32);
                        }
                        orig_input = self.input.clone();

                        // A poor man's try block to handle the backtracking
                        // in a single place instead of every time we want to return.
                        // This allows us to use `?` within the inner block without returning
                        // from the entire parent function.
                        let result = (|| {
                            let mut s = String::new();
                            for _ in 0..4 {
                                let c = self.next().and_then(char::from_u32)?;
                                s.push(c);
                            }

                            let uu = u16::from_str_radix(&s, 16).ok()?;
                            let ch = char::decode_utf16([u, uu]).next()?.ok()?;
                            Some(u32::from(ch))
                        })();

                        result.or_else(|| {
                            self.input = orig_input;
                            Some(u as u32)
                        })
                    } else {
                        // If `u` is not a surrogate or is a low surrogate we can directly return it,
                        // since all paired low surrogates should have been handled above.
                        Some(u as u32)
                    }
                }
                _ => {
                    self.input = orig_input;
                    None
                }
            }
        }
    }

    fn try_consume_named_capture_group_name(&mut self) -> Option<String> {
        if !self.try_consume('<') {
            return None;
        }

        let orig_input = self.input.clone();
        let mut group_name = String::new();

        if let Some(mut c) = self.next().and_then(char::from_u32) {
            if c == '\\' && self.try_consume('u') {
                if let Some(escaped) = self.try_escape_unicode_sequence().and_then(char::from_u32) {
                    c = escaped;
                } else {
                    self.input = orig_input;
                    return None;
                }
            }

            if interval_contains(id_start_ranges(), c.into()) || c == '$' || c == '_' {
                group_name.push(c);
            } else {
                self.input = orig_input;
                return None;
            }
        } else {
            self.input = orig_input;
            return None;
        }

        loop {
            if let Some(mut c) = self.next().and_then(char::from_u32) {
                if c == '\\' && self.try_consume('u') {
                    if let Some(escaped) =
                        self.try_escape_unicode_sequence().and_then(char::from_u32)
                    {
                        c = escaped;
                    } else {
                        self.input = orig_input;
                        return None;
                    }
                }

                if c == '>' {
                    break;
                }

                if interval_contains(id_continue_ranges(), c.into()) || c == '$' || c == '_' || c == '\u{200C}' /* <ZWNJ> */ || c == '\u{200D}'
                /* <ZWJ> */
                {
                    group_name.push(c);
                } else {
                    self.input = orig_input;
                    return None;
                }
            } else {
                self.input = orig_input;
                return None;
            }
        }

        Some(group_name)
    }

    // Quickly parse all capture groups.
    // Per TC39 proposal, duplicate named groups are allowed in different alternatives.
    fn parse_capture_groups(&mut self) -> Result<(), Error> {
        let orig_input = self.input.clone();

        // Pass 1: Collect all named capture groups with their alternative paths
        let named_group_locations = self.collect_named_group_locations()?;

        // Pass 2: Check for conflicts (duplicates in the same alternative path)
        self.check_duplicate_conflicts(&named_group_locations)?;

        self.input = orig_input;

        Ok(())
    }

    /// Pass 1: Collect all named capture groups and record which alternative path each appears in.
    fn collect_named_group_locations(
        &mut self,
    ) -> Result<HashMap<String, Vec<AlternativePath>>, Error> {
        // Track parenthesis depth and alternative index at each depth
        let mut paren_depth: usize = 0;
        // Map from depth to current alternative index at that depth
        let mut alt_indices: HashMap<usize, usize> = HashMap::new();
        alt_indices.insert(0, 0);

        // Map from group name to all alternative paths where it appears
        let mut named_group_locations: HashMap<String, Vec<AlternativePath>> = HashMap::new();

        loop {
            self.budget.check()?;
            match self.next().map(to_char_sat) {
                Some('\\') => {
                    self.next();
                    continue;
                }
                Some('[') if self.flags.unicode_sets => {
                    // In unicode_sets (`v`) mode character classes can nest,
                    // e.g. `[[a][b]]`, so track bracket depth and stop only at
                    // the matching `]`.
                    let mut depth = 1usize;
                    loop {
                        match self.next().map(to_char_sat) {
                            Some('\\') => {
                                self.next();
                                continue;
                            }
                            Some('[') => depth += 1,
                            Some(']') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some(_) => continue,
                            None => break,
                        }
                    }
                }
                Some('[') => loop {
                    match self.next().map(to_char_sat) {
                        Some('\\') => {
                            self.next();
                            continue;
                        }
                        Some(']') => break,
                        Some(_) => continue,
                        None => break,
                    }
                },
                Some('(') => {
                    // Determine whether we're a capturing group, and optionally the name.
                    let is_capturing;
                    let group_name;
                    if self.try_consume_str("?") {
                        group_name = self.try_consume_named_capture_group_name();
                        is_capturing = group_name.is_some(); // (?:, (?=, (?!, etc. are non-capturing.
                    } else {
                        is_capturing = true;
                        group_name = None; // Unnamed capture group
                    }

                    if let Some(name) = group_name {
                        // Build current alternative path from depth 0 to current depth.
                        self.budget
                            .charge(paren_depth + 1, (paren_depth + 1) * 64)?;
                        let mut segments = Vec::new();
                        for d in 0..=paren_depth {
                            segments.push((d, *alt_indices.get(&d).unwrap_or(&0)));
                        }

                        // Record this location.
                        named_group_locations
                            .entry(name.clone())
                            .or_default()
                            .push(AlternativePath { segments });

                        // Store all occurrences in named_group_indices.
                        self.named_group_indices
                            .entry(name)
                            .or_default()
                            .push(self.group_count_max);
                    }

                    if is_capturing {
                        self.group_count_max =
                            if self.group_count_max + 1 > MAX_CAPTURE_GROUPS as u32 {
                                MAX_CAPTURE_GROUPS as u32
                            } else {
                                self.group_count_max + 1
                            };
                    }

                    // Entering a new group.
                    paren_depth += 1;
                    if self.bounded && paren_depth > 32 {
                        return Err(self.budget.reject::<()>().unwrap_err().into());
                    }
                    alt_indices.insert(paren_depth, 0);
                }
                Some(')') => {
                    // Exiting a group
                    if paren_depth > 0 {
                        alt_indices.remove(&paren_depth);
                        paren_depth -= 1;
                    }
                }
                Some('|') => {
                    // Moving to next alternative at current depth
                    *alt_indices.entry(paren_depth).or_insert(0) += 1;
                }
                Some(_) => continue,
                None => break,
            }
        }

        Ok(named_group_locations)
    }

    /// Pass 2: Check that named groups with the same name don't conflict.
    /// Groups conflict if they appear in the same alternative path.
    /// Per TC39 proposal, `/(?<a>x)|(?<a>y)/` is valid (different alternatives),
    /// but `/(?<a>x)(?<a>y)/` is invalid (same alternative).
    fn check_duplicate_conflicts(
        &self,
        named_group_locations: &HashMap<String, Vec<AlternativePath>>,
    ) -> Result<(), Error> {
        for paths in named_group_locations.values() {
            // Check each pair of paths for this group name
            for i in 0..paths.len() {
                for j in (i + 1)..paths.len() {
                    self.budget
                        .charge(paths[i].segments.len() + paths[j].segments.len() + 1, 0)?;
                    if paths[i].conflicts_with(&paths[j]) {
                        return error("Duplicate capture group name");
                    }
                }
            }
        }
        Ok(())
    }

    fn try_consume_unicode_property_escape(&mut self) -> Result<PropertyEscapeKind, Error> {
        if !self.try_consume('{') {
            return error("Invalid character at property escape start");
        }

        let mut buffer = String::new();
        let mut name = None;

        while let Some(c) = self.peek().and_then(char::from_u32) {
            match c {
                '}' => {
                    self.consume(c);
                    let Some(value) =
                        unicode_property_from_str(&buffer, name, self.flags.unicode_sets)
                    else {
                        break;
                    };
                    match &value {
                        PropertyEscapeKind::CharacterClass(ranges) => {
                            self.budget
                                .charge(ranges.len(), ranges.len().saturating_mul(64))?;
                        }
                        PropertyEscapeKind::StringSet(strings) => {
                            self.budget
                                .charge(strings.len(), strings.len().saturating_mul(64))?;
                            for text in *strings {
                                self.budget
                                    .charge(text.len(), text.len().saturating_mul(16))?;
                            }
                        }
                    }
                    return Ok(value);
                }
                '=' if name.is_none() => {
                    self.consume(c);
                    let Some(n) = unicode_property_name_from_str(&buffer) else {
                        break;
                    };
                    name = Some(n);
                    buffer.clear();
                }
                c if c.is_ascii_alphanumeric() || c == '_' => {
                    self.consume(c);
                    buffer.push(c);
                }
                _ => break,
            }
        }

        error("Invalid property name")
    }

    fn finalize(&self, mut re: ir::Regex) -> Result<ir::Regex, Error> {
        debug_assert!(self.loop_count <= MAX_LOOPS as u32);
        debug_assert!(self.group_count as usize <= MAX_CAPTURE_GROUPS);
        if self.has_lookbehind {
            ir::walk_mut(
                false,
                re.flags.unicode,
                &mut re.node,
                &mut ir::Node::reverse_cats,
            );
        }
        Ok(re)
    }
}

/// Try parsing a given pattern.
/// Return the resulting IR regex, or an error.
pub fn try_parse<I>(pattern: I, flags: api::Flags) -> Result<ir::Regex, Error>
where
    I: Iterator<Item = u32> + Clone,
{
    try_parse_budget(pattern, flags, crate::Budget::unlimited(), false)
}

pub(crate) fn try_parse_budget<I>(
    pattern: I,
    flags: api::Flags,
    budget: crate::Budget,
    bounded: bool,
) -> Result<ir::Regex, Error>
where
    I: Iterator<Item = u32> + Clone,
{
    let mut p = Parser {
        budget,
        bounded,
        input: pattern.peekable(),
        flags,
        loop_count: 0,
        group_count: 0,
        named_group_indices: HashMap::new(),
        group_count_max: 0,
        has_lookbehind: false,
        depth: 0,
    };
    p.try_parse()
}
