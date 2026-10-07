//! Segment arrow ranges restrict only a shared enclosing calendar period.
use super::{CalendarEditor, Field};
impl CalendarEditor {
    pub(super) fn update_step_base(&mut self, min: Option<&str>, value: Option<&str>) {
        self.step_bases = [0; 8];
        let valid = |s: &&str| crate::validation::calendar::parse(self.kind.name(), s).is_some();
        if let Some(base) = min.filter(valid).or_else(|| value.filter(valid)) {
            let parsed = Self::new(self.kind, base, None);
            for field in [Field::Hour, Field::Minute, Field::Second, Field::Millis] {
                self.step_bases[field as usize] =
                    parsed.values[field as usize].unwrap_or(0) % self.steps[field as usize];
            }
        }
    }
    pub(super) fn update_limits(&mut self, min: Option<&str>, max: Option<&str>) {
        self.ranges = Field::ALL.map(Field::bounds);
        let parse = |s: Option<&str>| {
            s.filter(|s| crate::validation::calendar::parse(self.kind.name(), s).is_some())
                .map(|s| Self::new(self.kind, s, None))
        };
        let low = parse(min);
        let high = parse(max);
        let fields = self.kind.canonical_fields();
        for &field in fields {
            let (hard_min, hard_max) = field.bounds();
            let min = low.as_ref().and_then(|s| s.values[field as usize]);
            let max = high.as_ref().and_then(|s| s.values[field as usize]);
            let range = (min.unwrap_or(hard_min), max.unwrap_or(hard_max));
            if range.0 <= range.1 {
                self.ranges[field as usize] = range;
            }
            if min.is_none() || min != max {
                break;
            }
        }
    }
}
