//! An upper bound of what simplecss 0.2.2 allocates to parse a stylesheet,
//! found without parsing it.
//!
//! simplecss makes a rule of each selector in a list and gives each its own
//! copy of the block's declarations (`consume_rule_set`), so a block shared
//! by many selectors costs selectors × declarations. Where a rule set's
//! prelude and block start and end depends on strings, comments and
//! function arguments that simplecss reads differently in different places,
//! so the bound does not guess them: every rule (one per selector, at most
//! one per `,` plus one per `{` and one more) is charged the largest block
//! any `{` may open.
//!
//! A block opened at a `{` holds the declarations simplecss reads until a
//! `}` outside strings, comments and function arguments, or until an error
//! stops it. The scan skips more than simplecss ever does (every quote opens
//! a string with simplecss's escape rule, every `/*` a comment, every `(` an
//! argument list read raw to `)`), so it never finds the end earlier, and
//! each declaration needs a `:` before it.

/// What simplecss allocates per rule (its selector and declaration list),
/// per declaration, and per byte of the source for selector components.
const BYTES_PER_RULE: usize = 256;
const BYTES_PER_DECLARATION: usize = 96;
const BYTES_PER_SOURCE_BYTE: usize = 16;

/// What [`parsed_bytes`] holds per source byte while it runs: six arrays
/// of positions.
pub(super) const SCAN_BYTES_PER_BYTE: usize = 24;

/// An upper bound of what simplecss allocates to parse `css`.
pub(super) fn parsed_bytes(css: &str) -> usize {
    let bytes = css.as_bytes();
    // Positions are kept as `u32`, with `u32::MAX` for none.
    if u32::try_from(bytes.len()).is_err() {
        return usize::MAX;
    }
    let count = |byte: u8| bytes.iter().filter(|&&b| b == byte).count();
    let rules = count(b',').saturating_add(count(b'{')).saturating_add(1);
    let declarations = largest_block(bytes).saturating_add(1);
    let rule = BYTES_PER_RULE.saturating_add(declarations.saturating_mul(BYTES_PER_DECLARATION));
    rules
        .saturating_mul(rule)
        .saturating_add(bytes.len().saturating_mul(BYTES_PER_SOURCE_BYTE))
}

/// The most `:` any block opened at a `{` of `bytes` may hold.
fn largest_block(bytes: &[u8]) -> usize {
    // Colons before each position.
    let mut colons = Vec::with_capacity(bytes.len() + 1);
    let mut seen = 0u32;
    colons.push(0);
    for &byte in bytes {
        seen += u32::from(byte == b':');
        colons.push(seen);
    }
    let mut ends = Ends::new(bytes);
    let mut largest = 0;
    // From the last `{` back, so a scan stops where a later one passed.
    for open in (0..bytes.len()).rev().filter(|&at| bytes[at] == b'{') {
        let end = ends.from(open + 1);
        largest = largest.max((colons[end] - colons[open]) as usize);
    }
    largest
}

/// Where a block's declarations end, from each position read outside
/// strings, comments and arguments.
struct Ends<'a> {
    bytes: &'a [u8],
    jumps: Jumps,
    /// The end found from a position already read, or `u32::MAX`.
    known: Vec<u32>,
}

impl<'a> Ends<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            jumps: Jumps::of(bytes),
            known: vec![u32::MAX; bytes.len() + 1],
        }
    }

    /// The position of the first `}` from `start` outside strings, comments
    /// and arguments, or the end of the source. Walked twice, to find it and
    /// then to note it at each position read, so it holds nothing more.
    fn from(&mut self, start: usize) -> usize {
        let end = match self.walk(start, None) {
            Step::Known(end) | Step::Close(end) => end,
        };
        self.walk(start, Some(end));
        end
    }

    /// Walk from `start` to the end of its block, noting `end` at each
    /// position read when given.
    fn walk(&mut self, start: usize, end: Option<usize>) -> Step {
        let bytes = self.bytes;
        let mut at = start;
        loop {
            if at >= bytes.len() {
                return Step::Close(bytes.len());
            }
            if self.known[at] != u32::MAX {
                return Step::Known(self.known[at] as usize);
            }
            if let Some(end) = end {
                // Fits: the source length fits in `u32`.
                self.known[at] = end as u32;
            }
            at = match bytes[at] {
                b'}' => return Step::Close(at),
                b'"' => self.jumps.past(&self.jumps.double_quote, at + 1),
                b'\'' => self.jumps.past(&self.jumps.single_quote, at + 1),
                b'/' if bytes.get(at + 1) == Some(&b'*') => {
                    // Past the `/` of the first `*/` after the opening.
                    self.jumps.past(&self.jumps.comment_end, at + 3)
                }
                b'(' => self.jumps.past(&self.jumps.parenthesis, at + 1),
                _ => at + 1,
            };
        }
    }
}

/// How a walk ended: at a position whose end is known, or at its own end.
enum Step {
    Known(usize),
    Close(usize),
}

/// For each position, the next byte that closes a string, comment or
/// argument list at or after it, or `u32::MAX`, found in one pass each so
/// that scans never search.
struct Jumps {
    /// A `"` or `'` not right after a backslash (`Stream::consume_string`).
    double_quote: Vec<u32>,
    single_quote: Vec<u32>,
    /// The `/` of a `*/`.
    comment_end: Vec<u32>,
    parenthesis: Vec<u32>,
}

impl Jumps {
    fn of(bytes: &[u8]) -> Self {
        let next = |closes: &dyn Fn(usize) -> bool| {
            let mut next = vec![u32::MAX; bytes.len() + 1];
            for at in (0..bytes.len()).rev() {
                // Fits: the source length fits in `u32`.
                next[at] = if closes(at) { at as u32 } else { next[at + 1] };
            }
            next
        };
        let unescaped =
            |quote: u8| move |at: usize| bytes[at] == quote && (at == 0 || bytes[at - 1] != b'\\');
        Self {
            double_quote: next(&unescaped(b'"')),
            single_quote: next(&unescaped(b'\'')),
            comment_end: next(&|at| bytes[at] == b'/' && at > 0 && bytes[at - 1] == b'*'),
            parenthesis: next(&|at| bytes[at] == b')'),
        }
    }

    /// Past the closing byte `closes` finds from `from`, or the end.
    fn past(&self, closes: &[u32], from: usize) -> usize {
        match closes.get(from) {
            Some(&at) if at != u32::MAX => at as usize + 1,
            _ => closes.len() - 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BYTES_PER_DECLARATION, BYTES_PER_RULE, parsed_bytes};

    /// What simplecss's rules and declarations take, at the bound's rates.
    fn parsed(css: &str) -> usize {
        let sheet = simplecss::StyleSheet::parse(css);
        sheet
            .rules
            .iter()
            .map(|rule| BYTES_PER_RULE + rule.declarations.len() * BYTES_PER_DECLARATION)
            .sum()
    }

    /// The bound holds however strings, comments, arguments, stray
    /// separators and at-rules make simplecss read a stylesheet.
    #[test]
    fn the_bound_covers_what_simplecss_parses() {
        let pieces = [
            "a",
            "b",
            ",",
            "{",
            "}",
            ";",
            ":",
            "x:1",
            "y: \"}\"",
            "'",
            "\"",
            "\\",
            "/*",
            "*/",
            "url(",
            ")",
            "@m ",
            "[c=\"{\"]",
            "!important",
            " ",
            "#f0f",
            ".5em",
            "*",
        ];
        // A deterministic walk over sequences of the pieces.
        let mut seed = 0x9e37_79b9_u32;
        for _ in 0..20_000 {
            let mut css = String::new();
            for _ in 0..(seed % 24) {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                css.push_str(pieces[seed as usize % pieces.len()]);
            }
            assert!(parsed(&css) <= parsed_bytes(&css), "{css:?}");
        }
        let shared = format!(
            "{} {{ {} }}",
            vec!["a"; 300].join(","),
            "x: 1; ".repeat(300)
        );
        for css in [
            shared.clone(),
            format!("@m '{shared}"),
            format!("a,a,a,a ; }} {shared}"),
            format!("a{{y:url(}}x)}} {shared}"),
            format!("a{{z:\"\\\"}}\"}} {shared}"),
        ] {
            assert!(parsed(&css) <= parsed_bytes(&css), "{css:?}");
        }
    }

    /// Every scan is linear in the source, whatever it skips.
    #[test]
    fn the_bound_takes_linear_time() {
        for piece in ["{(", "{\"", "{'", "{/*", "{"] {
            let css = piece.repeat(500_000);
            let started = std::time::Instant::now();
            let _ = parsed_bytes(&css);
            let elapsed = started.elapsed();
            assert!(elapsed.as_secs() < 5, "{piece}: {elapsed:?}");
        }
    }

    /// Ordinary stylesheets are charged little.
    #[test]
    fn ordinary_stylesheets_cost_little() {
        let classes: String = (0..300)
            .map(|i| {
                format!(".st{i}{{fill:#{i:06X};stroke:none;font-family:'Myriad Pro', sans-serif}}")
            })
            .collect();
        assert!(
            parsed_bytes(&classes) < 4 * 1024 * 1024,
            "{}",
            parsed_bytes(&classes)
        );
    }
}
