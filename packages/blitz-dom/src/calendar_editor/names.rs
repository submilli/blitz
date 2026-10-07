//! English named-month type ahead with a fixed maximum prefix length.
use super::{CalendarEditor, Field};
const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
impl CalendarEditor {
    pub(super) fn month_letter(&mut self, letter: u8) {
        self.digits = 0;
        if self.name_length == self.name_prefix.len() {
            self.name_length = 0;
        }
        self.name_prefix[self.name_length] = letter;
        self.name_length += 1;
        let mut found = MONTHS.iter().position(|s| {
            s.as_bytes()
                .starts_with(&self.name_prefix[..self.name_length])
        });
        if found.is_none() {
            self.name_prefix[0] = letter;
            self.name_length = 1;
            let start = self.values[Field::Month as usize].unwrap_or(0) as usize;
            found = (0..12)
                .map(|n| (start + n) % 12)
                .find(|&n| MONTHS[n].as_bytes()[0] == letter);
        }
        if let Some(index) = found {
            self.values[Field::Month as usize] = Some(index as u32 + 1);
        }
    }
}
