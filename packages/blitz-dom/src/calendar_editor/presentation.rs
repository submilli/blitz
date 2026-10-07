//! Scalar display spans are a projection; pointer selection chooses whole fields.
use super::{CalendarEditor, Field, Kind};
use std::ops::Range;
impl CalendarEditor {
    pub(crate) fn presentation(&self) -> (String, Range<usize>) {
        let (text, spans) = self.display();
        (text, spans[self.active].clone())
    }
    pub(super) fn select_at(&mut self, byte: usize) {
        let (_, spans) = self.display();
        let index = spans
            .iter()
            .position(|span| byte < span.end)
            .unwrap_or(spans.len() - 1);
        if index != self.active {
            self.commit_field();
            self.active = index;
        }
    }
    fn display(&self) -> (String, Vec<Range<usize>>) {
        let mut text = if self.kind == Kind::Week {
            "Week ".to_owned()
        } else {
            String::new()
        };
        let mut spans = Vec::with_capacity(self.fields().len());
        for (index, &field) in self.fields().iter().enumerate() {
            if index > 0 {
                text.push_str(match (self.kind, field) {
                    (Kind::Week, Field::Year) => ", ",
                    (Kind::Month, Field::Year) => " ",
                    (_, Field::Month | Field::Year) => "/",
                    (_, Field::Hour) => ", ",
                    (_, Field::Millis) => ".",
                    _ => ":",
                });
            }
            let start = text.len();
            let typed = (index == self.active && self.digits > 0).then_some(self.typed);
            if let Some(value) = typed.or(self.values[field as usize]) {
                if self.kind == Kind::Month && field == Field::Month {
                    let names = [
                        "January",
                        "February",
                        "March",
                        "April",
                        "May",
                        "June",
                        "July",
                        "August",
                        "September",
                        "October",
                        "November",
                        "December",
                    ];
                    text.push_str(
                        names
                            .get(value.saturating_sub(1) as usize)
                            .unwrap_or(&"---------"),
                    );
                } else {
                    text.push_str(&format!(
                        "{:0width$}",
                        value,
                        width = if field == Field::Year {
                            4
                        } else {
                            usize::from(field.width())
                        }
                    ));
                }
            } else {
                text.push_str(field.placeholder());
            }
            spans.push(start..text.len());
        }
        (text, spans)
    }
}
