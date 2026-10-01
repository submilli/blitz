//! The HTML pattern path bypasses speculative optimization and search-prefix analysis.
use crate::{Budget, ResourceError, api::Flags, emit, ir, parse};

/// Syntax errors disable an HTML pattern; exhaustion must instead abort validation.
#[derive(Debug)]
pub enum PatternError {
    Syntax,
    Resource(ResourceError),
}
impl From<ResourceError> for PatternError {
    fn from(e: ResourceError) -> Self {
        Self::Resource(e)
    }
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax => f.write_str("invalid pattern syntax"),
            Self::Resource(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for PatternError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Syntax => None,
            Self::Resource(e) => Some(e),
        }
    }
}

// Each source unit can introduce an AST node, prescan/map entries and names.
// 4096 units give at most 12 levels of balanced alternation copying. This
// allowance covers 16 Node-sized slots plus prescan overhead per source unit;
// named-reference expansion and Unicode property data are charged separately.
const SOURCE_BYTES: usize = 16 * std::mem::size_of::<ir::Node>() + 512;
// A regular IR node emits at most three opcodes and two traversal records.
// Allow vector capacity doubling; variable captures, brackets and strings are
// admitted separately below before calling the otherwise infallible emitter.
const EMIT_NODE_BYTES: usize = 6 * std::mem::size_of::<crate::insn::Insn>() + 256;

/// A compiled HTML pattern. Construction and matching share caller-owned accounting.
#[derive(Debug)]
pub struct Pattern(crate::insn::CompiledRegex);

impl Pattern {
    /// Match the complete value, preserving alternate paths that consume more input.
    pub fn is_full_match(&self, value: &[u16], budget: &Budget) -> Result<bool, ResourceError> {
        if value.len() > 1_048_576 {
            return budget.reject();
        }
        budget.charge(value.len(), 0)?;
        crate::classicalbacktrack::MatchAttempter::full_match(
            &self.0,
            crate::indexing::Utf16Input::new(value, true),
            budget,
        )
    }

    pub fn compile(source: &[u16], budget: &Budget) -> Result<Self, PatternError> {
        if source.len() > 4096 {
            return budget.reject().map_err(Into::into);
        }
        // Covers source-derived AST nodes, names, prescan maps, and balanced
        // alternation splitting. Expanded Unicode data is charged separately.
        budget.charge(
            source.len().saturating_mul(64),
            source.len().saturating_mul(SOURCE_BYTES),
        )?;
        let input = Metered {
            input: char::decode_utf16(source.iter().copied()).map(|c| {
                c.map(u32::from)
                    .unwrap_or_else(|e| u32::from(e.unpaired_surrogate()))
            }),
            budget: budget.clone(),
        };
        let flags = Flags {
            unicode: true,
            unicode_sets: true,
            no_opt: true,
            ..Flags::default()
        };
        let parsed = parse::try_parse_budget(input, flags, budget.clone(), true);
        budget.check()?;
        let parsed = parsed.map_err(|e| {
            if e.kind == parse::ErrorKind::Resource {
                PatternError::Resource(ResourceError)
            } else {
                PatternError::Syntax
            }
        })?;
        admit_emission(&parsed.node, budget)?;
        Ok(Self(emit::emit_bounded(&parsed)))
    }
}

#[derive(Clone)]
struct Metered<I> {
    input: I,
    budget: Budget,
}
impl<I: Iterator<Item = u32>> Iterator for Metered<I> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        // Do not synthesize EOF: lookahead may already have established the next
        // token. Result-returning parser boundaries stop on the sticky error;
        // the source admission covers the remaining bounded lexical helper.
        let _ = self.budget.charge(1, 0);
        self.input.next()
    }
}

/// Precharge the bytecode emitter's complete expansion before it allocates.
/// There is no optimizer in this path, so loop iteration counts never expand IR.
fn admit_emission(root: &ir::Node, budget: &Budget) -> Result<(), ResourceError> {
    use ir::Node;
    budget.charge(1, 64)?;
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        budget.charge(32, EMIT_NODE_BYTES)?;
        match node {
            Node::Cat(children) => {
                budget.charge(children.len(), children.len().saturating_mul(32))?;
                pending.extend(children);
            }
            Node::Alt(left, right) => pending.extend([&**left, &**right]),
            Node::CaptureGroup { contents, name, .. } => {
                if let Some(name) = name {
                    budget.charge(name.len(), name.len().saturating_mul(4))?;
                }
                pending.push(contents);
            }
            Node::LookaroundAssertion { contents, .. } => pending.push(contents),
            Node::Loop {
                loopee,
                enclosed_groups,
                ..
            } => {
                let groups = usize::from(enclosed_groups.end - enclosed_groups.start);
                budget.charge(groups.saturating_mul(32), groups.saturating_mul(256))?;
                pending.push(loopee);
            }
            Node::Bracket(contents) => {
                let count = contents.cps.intervals().len();
                budget.charge(count.saturating_mul(128), count.saturating_mul(32))?;
            }
            Node::StringSet {
                alternatives,
                icase,
            } => {
                budget.charge(alternatives.len(), alternatives.len().saturating_mul(256))?;
                for text in alternatives {
                    let cost = if *icase {
                        crate::unicode::literal_fold_cost()
                    } else {
                        64
                    };
                    budget.charge(
                        text.len().saturating_mul(cost),
                        text.len().saturating_mul(EMIT_NODE_BYTES + 128),
                    )?;
                }
            }
            // Optimizer-only nodes must never reach the bounded interpreter.
            Node::Loop1CharBody { .. } | Node::ByteSequence(_) | Node::ByteSet(_) => {
                return budget.reject();
            }
            Node::Empty
            | Node::Goal
            | Node::Char { .. }
            | Node::CharSet(_)
            | Node::MatchAny
            | Node::MatchAnyExceptLineTerminator
            | Node::Anchor { .. }
            | Node::WordBoundary { .. }
            | Node::BackRef { .. } => {}
        }
    }
    Ok(())
}
