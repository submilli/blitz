//! Lossless replacements at the scalar editor boundary. Offsets are captured
//! before editing; identical scalar projections never determine what changed.
use super::TextBrush;
use crate::DomString;
use parley::editing::PlainEditorDriver;
use std::ops::{Deref, Range};

pub(crate) struct TextEdit {
    range: Range<usize>,
    text: String,
}

impl TextEdit {
    pub(crate) fn apply(self, value: &mut DomString) {
        let mut units = value.to_utf16().into_owned();
        if units.get(self.range.clone()).is_none() {
            return;
        }
        units.splice(self.range, self.text.encode_utf16());
        *value = DomString::from_utf16(units);
    }
}

pub(super) struct EditingDriver<'a> {
    driver: PlainEditorDriver<'a, TextBrush>,
    edits: &'a mut Vec<TextEdit>,
}

impl<'a> EditingDriver<'a> {
    pub(super) fn new(
        driver: PlainEditorDriver<'a, TextBrush>,
        edits: &'a mut Vec<TextEdit>,
    ) -> Self {
        Self { driver, edits }
    }

    fn record(&mut self, range: Range<usize>, text: &str) {
        if range.is_empty() && text.is_empty() {
            return;
        }
        let raw = self.driver.editor.raw_text();
        let (Some(before), Some(removed)) = (raw.get(..range.start), raw.get(range.clone())) else {
            return;
        };
        let start = before.encode_utf16().count();
        let end = start + removed.encode_utf16().count();
        self.edits.push(TextEdit {
            range: start..end,
            text: text.to_owned(),
        });
    }

    pub(super) fn insert_or_replace_selection(&mut self, text: &str) {
        self.record(self.driver.editor.raw_selection().text_range(), text);
        self.driver.insert_or_replace_selection(text);
    }

    pub(super) fn delete_selection(&mut self) {
        self.insert_or_replace_selection("");
    }

    pub(super) fn delete(&mut self) {
        let selection = self.driver.editor.raw_selection();
        let range = if selection.is_collapsed() {
            selection.focus().logical_clusters(self.driver.layout())[1]
                .as_ref()
                .map(|cluster| cluster.text_range())
        } else {
            Some(selection.text_range())
        };
        if let Some(range) = range {
            self.record(range, "");
        }
        self.driver.delete();
    }

    pub(super) fn backdelete(&mut self) {
        let selection = self.driver.editor.raw_selection();
        let range = if selection.is_collapsed() {
            selection.focus().logical_clusters(self.driver.layout())[0]
                .map(|cluster| {
                    (
                        cluster.text_range(),
                        cluster.is_hard_line_break() || cluster.is_emoji(),
                    )
                })
                .and_then(|(range, whole_cluster)| {
                    if whole_cluster {
                        return Some(range);
                    }
                    let start = self
                        .driver
                        .editor
                        .raw_text()
                        .get(..range.end)?
                        .char_indices()
                        .next_back()?
                        .0;
                    Some(start..range.end)
                })
        } else {
            Some(selection.text_range())
        };
        if let Some(range) = range {
            self.record(range, "");
        }
        self.driver.backdelete();
    }

    pub(super) fn delete_word(&mut self) {
        let selection = self.driver.editor.raw_selection();
        let range = if selection.is_collapsed() {
            let focus = selection.focus();
            focus.index()..focus.next_logical_word(self.driver.layout()).index()
        } else {
            selection.text_range()
        };
        self.record(range, "");
        self.driver.delete_word();
    }

    pub(super) fn backdelete_word(&mut self) {
        let selection = self.driver.editor.raw_selection();
        let range = if selection.is_collapsed() {
            let focus = selection.focus();
            focus.previous_logical_word(self.driver.layout()).index()..focus.index()
        } else {
            selection.text_range()
        };
        self.record(range, "");
        self.driver.backdelete_word();
    }

    pub(super) fn set_compose(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        let range = self
            .driver
            .editor
            .raw_compose()
            .clone()
            .unwrap_or_else(|| self.driver.editor.raw_selection().text_range());
        self.record(range, text);
        self.driver.set_compose(text, cursor);
    }

    pub(super) fn clear_compose(&mut self) {
        if let Some(range) = self.driver.editor.raw_compose().clone() {
            self.record(range, "");
        }
        self.driver.clear_compose();
    }

    pub(super) fn commit_compose(&mut self, text: &str) {
        self.clear_compose();
        self.insert_or_replace_selection(text);
    }
}

impl<'a> Deref for EditingDriver<'a> {
    type Target = PlainEditorDriver<'a, TextBrush>;
    fn deref(&self) -> &Self::Target {
        &self.driver
    }
}

// Navigation is explicitly forwarded; new Parley mutations cannot bypass recording.
macro_rules! forward_navigation {
    ($($name:ident),* $(,)?) => {
        impl EditingDriver<'_> {
            $(pub(super) fn $name(&mut self) { self.driver.$name(); })*
        }
    };
}
forward_navigation!(
    collapse_selection,
    move_down,
    move_left,
    move_right,
    move_to_hard_line_end,
    move_to_hard_line_start,
    move_to_line_end,
    move_to_line_start,
    move_to_text_end,
    move_to_text_start,
    move_up,
    move_word_left,
    move_word_right,
    select_all,
    select_down,
    select_left,
    select_right,
    select_to_hard_line_end,
    select_to_hard_line_start,
    select_to_line_end,
    select_to_line_start,
    select_to_text_end,
    select_to_text_start,
    select_up,
    select_word_left,
    select_word_right,
);
