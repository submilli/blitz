use blitz_traits::node_id::NodeId;
use style::computed_values::border_collapse::T as BorderCollapse;
use taffy::style_helpers;

use super::{TableCell, TableRow};
use crate::BaseDocument;
use crate::layout::box_depth::omit_subtree_boxes;

/// Taffy's explicit grid limit. Apply it before allocating table metadata.
pub(super) const MAX_TABLE_TRACKS: u16 = 10_000;

pub(super) fn parse_table_span(value: Option<&str>, maximum: u16) -> u16 {
    let Some(value) = value else {
        return 1;
    };
    let mut bytes = value
        .as_bytes()
        .iter()
        .copied()
        .skip_while(|byte| matches!(byte, b'\t' | b'\n' | 0x0c | b'\r' | b' '))
        .peekable();
    let _ = bytes.next_if_eq(&b'+');
    let mut parsed = 0u16;
    let mut has_digit = false;
    while let Some(byte @ b'0'..=b'9') = bytes.peek().copied() {
        has_digit = true;
        bytes.next();
        parsed = parsed
            .saturating_mul(10)
            .saturating_add(u16::from(byte - b'0'))
            .min(maximum);
    }
    if has_digit {
        parsed.clamp(1, maximum)
    } else {
        1
    }
}

/// Tracks columns occupied by cells and row spans while bounding allocation.
#[derive(Debug, Default)]
pub(super) struct ColumnCursor {
    col: u16,
    pub(super) num_columns: u16,
    rowspans: Vec<u16>,
}

impl ColumnCursor {
    pub(super) fn start_row(&mut self) {
        self.col = 0;
        for remaining in self.rowspans.iter_mut() {
            *remaining = remaining.saturating_sub(1);
        }
    }

    pub(super) fn next_free(&mut self) -> u16 {
        while self
            .rowspans
            .get(self.col as usize)
            .is_some_and(|remaining| *remaining > 0)
        {
            self.col += 1;
        }
        self.col
    }

    pub(super) fn place(&mut self, colspan: u16, rowspan: u16) -> u16 {
        let end = self.col.saturating_add(colspan).min(MAX_TABLE_TRACKS);
        let admitted = end - self.col;
        if self.rowspans.len() < end as usize {
            self.rowspans.resize(end as usize, 0);
        }
        for remaining in &mut self.rowspans[self.col as usize..end as usize] {
            *remaining = rowspan - 1;
        }
        self.col = end;
        self.num_columns = self.num_columns.max(end);
        admitted
    }
}

/// Put generated boxes before or after authored rows into one anonymous row.
pub(super) fn collect_generated_table_row(
    doc: &mut BaseDocument,
    items: &[NodeId],
    border_collapse: BorderCollapse,
    row: &mut u16,
    cursor: &mut ColumnCursor,
    cells: &mut Vec<TableCell>,
    rows: &mut Vec<TableRow>,
) {
    let Some(&first) = items.first() else {
        return;
    };
    if *row == MAX_TABLE_TRACKS {
        for &node_id in items {
            omit_subtree_boxes(&mut doc.nodes, node_id);
        }
        return;
    }
    *row += 1;
    cursor.start_row();
    rows.push(TableRow {
        node_id: first,
        height: 0.0,
    });

    for &node_id in items {
        let col = cursor.next_free();
        let colspan = cursor.place(1, 1);
        if colspan == 0 {
            omit_subtree_boxes(&mut doc.nodes, node_id);
            continue;
        }
        let styles = doc.nodes[node_id].primary_styles().unwrap();
        let mut style = stylo_taffy::to_taffy_style(&styles);
        if border_collapse == BorderCollapse::Collapse {
            style.border = taffy::Rect::ZERO.map(style_helpers::length);
        }
        style.margin = taffy::Rect::ZERO.map(style_helpers::length);
        style.grid_column = taffy::Line {
            start: style_helpers::line(col as i16 + 1),
            end: style_helpers::span(colspan),
        };
        style.grid_row = taffy::Line {
            start: style_helpers::line(*row as i16),
            end: style_helpers::span(1),
        };
        style.size.width = style_helpers::auto();
        cells.push(TableCell { node_id, style });
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnCursor, MAX_TABLE_TRACKS, parse_table_span};

    #[test]
    fn table_spans_parse_before_clamping() {
        assert_eq!(parse_table_span(Some("65536"), 1000), 1000);
        assert_eq!(
            parse_table_span(Some("99999999999999999999x"), 65534),
            65534
        );
        assert_eq!(parse_table_span(Some(" +12 trailing"), 1000), 12);
        assert_eq!(parse_table_span(Some("-12"), 1000), 1);
        assert_eq!(parse_table_span(Some("0"), 1000), 1);
        assert_eq!(parse_table_span(None, 1000), 1);
    }

    #[test]
    fn column_spans_stop_at_the_grid_limit_without_overflowing() {
        let mut cursor = ColumnCursor::default();
        assert_eq!(cursor.place(u16::MAX, 3), MAX_TABLE_TRACKS);
        assert_eq!(cursor.place(1, 1), 0);
        assert_eq!(cursor.rowspans.len(), usize::from(MAX_TABLE_TRACKS));
        cursor.start_row();
        assert_eq!(cursor.next_free(), MAX_TABLE_TRACKS);
        assert_eq!(cursor.place(1, 1), 0);
        cursor.start_row();
        assert_eq!(cursor.next_free(), 0);
        assert_eq!(cursor.place(1, 1), 1);
        assert_eq!(cursor.rowspans.len(), usize::from(MAX_TABLE_TRACKS));
    }
}
