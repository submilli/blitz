use super::{CalendarEditor, Kind};
use keyboard_types::Key;

fn type_digits(editor: &mut CalendarEditor, text: &str) {
    for digit in text.chars() {
        assert!(editor.key(&Key::Character(digit.to_string()), 2026));
    }
}
#[test]
fn incomplete_fields_do_not_escape_as_a_value() {
    let mut editor = CalendarEditor::new(Kind::Date, "", None);
    type_digits(&mut editor, "1");
    assert_eq!(editor.value(), "");
    assert!(editor.bad_input());
    editor.key(&Key::Delete, 2026);
    assert!(!editor.bad_input());
    type_digits(&mut editor, "25122024");
    assert_eq!(editor.value(), "2024-12-25");
}
#[test]
fn zero_field_is_committed_on_focus_exit() {
    let mut editor = CalendarEditor::new(Kind::Date, "2024-06-15", None);
    type_digits(&mut editor, "0");
    assert_eq!(editor.value(), "");
    editor.commit_field();
    assert_eq!(editor.value(), "2024-06-01");
}
#[test]
fn pointer_spans_select_year_and_minute() {
    let mut editor = CalendarEditor::new(Kind::Date, "2024-06-15", None);
    editor.select_at(8);
    editor.key(&Key::ArrowUp, 2026);
    assert_eq!(editor.value(), "2025-06-15");
    let mut editor = CalendarEditor::new(Kind::Time, "12:34", None);
    editor.select_at(4);
    editor.key(&Key::ArrowUp, 2026);
    assert_eq!(editor.value(), "12:35");
}
#[test]
fn dynamic_precision_preserves_completed_and_partial_fields() {
    let mut editor = CalendarEditor::new(Kind::Time, "12:34", None);
    editor.update_precision("12:34", Some("0.001"));
    for _ in 0..3 {
        editor.move_field(false);
    }
    editor.key(&Key::ArrowUp, 2026);
    assert_eq!(editor.value(), "12:34:00.001");
    let mut editor = CalendarEditor::new(Kind::Time, "", None);
    type_digits(&mut editor, "1");
    editor.update_precision("", Some("0.001"));
    assert!(editor.bad_input());
    assert_eq!(editor.value(), "");
}
#[test]
fn hostile_text_and_long_digit_sequences_stay_recoverable() {
    let mut editor = CalendarEditor::new(Kind::Date, "2024-01-01", None);
    assert!(!editor.key(&Key::Character("💥".repeat(65536)), 2026));
    assert_eq!(editor.value(), "2024-01-01");
    editor.move_field(false);
    editor.move_field(false);
    type_digits(&mut editor, &"9".repeat(10000));
    assert_eq!(editor.value(), "275760-01-01");
    editor.key(&Key::Delete, 2026);
    type_digits(&mut editor, "2024");
    assert_eq!(editor.value(), "2024-01-01");
}
#[test]
fn constrained_arrows_wrap_but_typing_preserves_range_failures() {
    let mut editor = CalendarEditor::new(Kind::Week, "2024-W10", None);
    editor.update_limits(Some("2024-W10"), Some("2024-W20"));
    editor.key(&Key::ArrowDown, 2026);
    assert_eq!(editor.value(), "2024-W20");
    type_digits(&mut editor, "30");
    assert_eq!(editor.value(), "2024-W30");
}

#[test]
fn invalid_min_falls_back_to_valid_value_step_base() {
    let mut editor = CalendarEditor::new(Kind::Time, "12:05", Some("900"));
    editor.update_step_base(Some("garbage"), Some("12:05"));
    editor.move_field(false);
    editor.key(&Key::ArrowUp, 2026);
    assert_eq!(editor.value(), "12:20");
}
