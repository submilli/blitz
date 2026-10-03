//! HTML rendered text, using the resolved CSS tree and lossless DOM code units.
//! https://html.spec.whatwg.org/multipage/dom.html#the-innertext-idl-attribute

use style::computed_values::{visibility::T as Visibility, white_space_collapse::T as WhiteSpace};
use style::values::computed::TextTransform;
use style::values::specified::box_::{DisplayInside, DisplayOutside};

#[path = "inner_text_selection.rs"]
mod selection;
#[path = "inner_text_transform.rs"]
mod transform;

use crate::{BaseDocument, DocumentMutator, DomString, NodeBudgetExceeded, NodeId, QualName};

impl BaseDocument {
    /// Rendered descendant text. Resolve styles/layout before calling, as for
    /// geometry queries. Detached and display:none roots return descendant text.
    pub fn inner_text(&self, id: NodeId) -> DomString {
        if !self.inner_text_is_rendered(id) {
            return self.text_content_dom(id).unwrap_or_default();
        }
        if self.inner_text_replaced(id) {
            return DomString::new();
        }
        let mut output = RenderedText::default();
        let mut pending = Vec::new();
        self.enqueue_inner_text_children(id, &mut pending);
        while let Some(visit) = pending.pop() {
            match visit {
                Visit::EndAtomic => {
                    output.space = false;
                    output.line_start = false;
                }
                Visit::Break(count) => output.require_break(count),
                Visit::Tab => {
                    output.space = false;
                    output.flush_breaks();
                    output.units.push(9);
                }
                Visit::Node(id) => {
                    self.collect_inner_text_node(id, &mut pending, &mut output, None)
                }
            }
        }
        DomString::from_utf16(output.units)
    }

    fn inner_text_is_rendered(&self, id: NodeId) -> bool {
        let mut ancestor = Some(id);
        while let Some(id) = ancestor {
            let node = &self.nodes[id];
            if node.is_element()
                && node
                    .primary_styles()
                    .is_none_or(|s| s.clone_display().inside() == DisplayInside::None)
            {
                return false;
            }
            if matches!(node.data, crate::node::NodeData::Document(_)) {
                return true;
            }
            ancestor = node.parent.or_else(|| self.shadow_host_of(id));
        }
        false
    }

    fn collect_inner_text_node(
        &self,
        id: NodeId,
        pending: &mut Vec<Visit>,
        output: &mut RenderedText,
        selection: Option<[crate::ranges::Boundary; 2]>,
    ) {
        let node = &self.nodes[id];
        if let crate::node::NodeData::Text(text) = &node.data {
            if let Some(style) = node.parent.and_then(|id| self.nodes[id].primary_styles()) {
                if style.clone_visibility() == Visibility::Visible {
                    let content = if let Some(points) = selection {
                        self.selected_text_slice(id, &text.content, points)
                    } else {
                        text.content.clone()
                    };
                    output.text(
                        &content,
                        style.clone_white_space_collapse(),
                        style.clone_text_transform(),
                        style.clone__x_lang().0.as_ref(),
                    );
                }
            }
            return;
        }
        let Some(element) = node.element_data() else {
            return;
        };
        let Some(style) = node.primary_styles() else {
            return;
        };
        let display = style.clone_display();
        if display.inside() == DisplayInside::None {
            return;
        }
        // Replaced boxes separate whitespace runs even though their DOM text
        // children do not contribute rendered text.
        if self.inner_text_replaced(id) {
            output.atomic_boundary();
            output.line_start = false;
            return;
        }
        if display.outside() == DisplayOutside::Inline
            && matches!(
                display.inside(),
                DisplayInside::FlowRoot | DisplayInside::Flex | DisplayInside::Grid
            )
        {
            output.atomic_boundary();
            output.line_start = true;
            pending.push(Visit::EndAtomic);
        }
        let visible = style.clone_visibility() == Visibility::Visible;
        let has_box = display.inside() != DisplayInside::Contents;
        let breaks = if !visible || !has_box {
            0
        } else if element.name.local.as_ref() == "p" {
            2
        } else if display.outside() == DisplayOutside::Block
            || matches!(element.name.local.as_ref(), "option" | "optgroup")
        {
            1
        } else {
            0
        };
        if visible && has_box && element.name.local.as_ref() == "br" {
            output.newline();
        }
        if breaks > 0 {
            if selection.is_none_or(|p| !self.is_inclusive_ancestor(id, p[0].node)) {
                output.require_break(breaks);
            }
            if selection.is_none_or(|p| !self.is_inclusive_ancestor(id, p[1].node)) {
                pending.push(Visit::Break(breaks));
            }
        }
        if visible && has_box && display.inside() == DisplayInside::TableRow {
            pending.push(Visit::Break(1));
        }
        self.enqueue_inner_text_children(id, pending);
    }

    fn inner_text_replaced(&self, id: NodeId) -> bool {
        self.nodes[id].element_data().is_some_and(|element| {
            matches!(
                element.name.local.as_ref(),
                "textarea" | "input" | "img" | "video" | "audio" | "iframe" | "canvas" | "embed"
            )
        })
    }

    fn enqueue_inner_text_children(&self, id: NodeId, pending: &mut Vec<Visit>) {
        // One reverse scan per parent avoids quadratic sibling searches in wide tables.
        let mut later_cell = false;
        for child in self.nodes[id].children.iter().rev().copied() {
            if let Some(style) = self.nodes[child].primary_styles() {
                if style.clone_display().inside() == DisplayInside::TableCell {
                    if later_cell && style.clone_visibility() == Visibility::Visible {
                        pending.push(Visit::Tab);
                    }
                    later_cell = true;
                }
            }
            pending.push(Visit::Node(child));
        }
    }
}

enum Visit {
    EndAtomic,
    Node(NodeId),
    Break(usize),
    Tab,
}

#[derive(Clone)]
struct RenderedText {
    units: Vec<u16>,
    required_breaks: usize,
    space: bool,
    word_start: bool,
    line_start: bool,
}

impl Default for RenderedText {
    fn default() -> Self {
        Self {
            units: Vec::new(),
            required_breaks: 0,
            space: false,
            word_start: true,
            line_start: true,
        }
    }
}

impl RenderedText {
    fn atomic_boundary(&mut self) {
        self.flush_breaks();
        if self.space && !self.line_start {
            self.units.push(32);
        }
        self.space = false;
    }

    fn require_break(&mut self, count: usize) {
        self.space = false;
        self.required_breaks = self.required_breaks.max(count);
        self.word_start = true;
        self.line_start = true;
    }

    fn flush_breaks(&mut self) {
        if !self.units.is_empty() {
            self.units
                .extend(std::iter::repeat_n(10, self.required_breaks));
        }
        self.required_breaks = 0;
    }

    fn newline(&mut self) {
        self.space = false;
        self.flush_breaks();
        self.units.push(10);
        self.word_start = true;
        self.line_start = true;
    }

    fn text(
        &mut self,
        text: &DomString,
        whitespace: WhiteSpace,
        transform: TextTransform,
        language: &str,
    ) {
        let mut word_start = self.word_start;
        let text = transform::apply(text, transform, language, &mut word_start);
        let units = text.to_utf16();
        let mut chars = char::decode_utf16(units.iter().copied()).peekable();
        while let Some(decoded) = chars.next() {
            let ch = match decoded {
                Ok('\r') => {
                    if chars.peek() == Some(&Ok('\n')) {
                        chars.next();
                    }
                    Ok('\n')
                }
                other => other,
            };
            let collapse = matches!(
                whitespace,
                WhiteSpace::Collapse | WhiteSpace::PreserveBreaks
            );
            if ch == Ok('\n') && whitespace != WhiteSpace::Collapse {
                self.newline();
                continue;
            }
            if collapse && matches!(ch, Ok(' ' | '\t' | '\n')) {
                self.space = true;
                self.word_start = true;
                continue;
            }
            self.flush_breaks();
            if self.space && !self.line_start {
                self.units.push(32);
            }
            self.space = false;
            self.line_start = false;
            match ch {
                Ok(ch) => self
                    .units
                    .extend(ch.encode_utf16(&mut [0; 2]).iter().copied()),
                Err(error) => {
                    self.units.push(error.unpaired_surrogate());
                    self.word_start = false;
                }
            }
        }
        self.word_start = word_start;
    }
}

impl DocumentMutator<'_> {
    /// Replace children with text and HTML br elements, preadmitting every node
    /// before touching the tree. CRLF is one break; lone CR and LF are breaks.
    pub fn try_set_inner_text(
        &mut self,
        id: NodeId,
        value: impl Into<DomString>,
    ) -> Result<(), NodeBudgetExceeded> {
        let value = value.into();
        let units = value.to_utf16();
        let count = TextParts::new(&units).count();
        self.doc.check_node_allocation(count)?;
        self.with_one_child_list_record(id, |m| {
            m.detach_children(id);
            for part in TextParts::new(&units) {
                let child = match part {
                    Some(text) => m.create_text_node(DomString::from_utf16(text.to_vec())),
                    None => {
                        m.create_element(QualName::new(None, crate::ns!(html), "br".into()), vec![])
                    }
                };
                m.append_children(id, &[child]);
            }
        });
        Ok(())
    }
}

struct TextParts<'a> {
    remaining: &'a [u16],
}
impl<'a> TextParts<'a> {
    fn new(remaining: &'a [u16]) -> Self {
        Self { remaining }
    }
}
impl<'a> Iterator for TextParts<'a> {
    type Item = Option<&'a [u16]>;
    fn next(&mut self) -> Option<Self::Item> {
        let first = *self.remaining.first()?;
        if matches!(first, 10 | 13) {
            let count = if first == 13 && self.remaining.get(1) == Some(&10) {
                2
            } else {
                1
            };
            self.remaining = &self.remaining[count..];
            return Some(None);
        }
        let end = self
            .remaining
            .iter()
            .position(|unit| matches!(unit, 10 | 13))
            .unwrap_or(self.remaining.len());
        let text = &self.remaining[..end];
        self.remaining = &self.remaining[end..];
        Some(Some(text))
    }
}
