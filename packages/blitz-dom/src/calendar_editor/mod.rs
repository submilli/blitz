//! Bounded native calendar fields. The deterministic presentation uses DMY and
//! a 24-hour clock; DOM values always use HTML's locale-independent syntax.
mod fields;
mod integration;
mod limits;
mod names;
mod presentation;
#[cfg(test)]
mod tests;
use fields::Field;
use keyboard_types::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Date,
    Month,
    Week,
    Time,
    DateTime,
}
impl Kind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("date") {
            Some(Self::Date)
        } else if value.eq_ignore_ascii_case("month") {
            Some(Self::Month)
        } else if value.eq_ignore_ascii_case("week") {
            Some(Self::Week)
        } else if value.eq_ignore_ascii_case("time") {
            Some(Self::Time)
        } else if value.eq_ignore_ascii_case("datetime-local") {
            Some(Self::DateTime)
        } else {
            None
        }
    }
    fn canonical_fields(self) -> &'static [Field] {
        use Field::*;
        match self {
            Self::Date => &[Year, Month, Day],
            Self::Month => &[Year, Month],
            Self::Week => &[Year, Week],
            Self::Time => &[Hour, Minute, Second, Millis],
            Self::DateTime => &[Year, Month, Day, Hour, Minute, Second, Millis],
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Month => "month",
            Self::Week => "week",
            Self::Time => "time",
            Self::DateTime => "datetime-local",
        }
    }
}

/// Fixed slots retain incomplete fields independently of the sanitized value.
#[derive(Clone, Debug)]
pub(crate) struct CalendarEditor {
    kind: Kind,
    values: [Option<u32>; 8],
    active: usize,
    digits: u8,
    typed: u32,
    ranges: [(u32, u32); 8],
    name_prefix: [u8; 9],
    name_length: usize,
    seconds: bool,
    millis: bool,
    steps: [u32; 8],
    step_bases: [u32; 8],
    precision_step: Option<f64>,
}
impl CalendarEditor {
    pub(crate) fn new(kind: Kind, value: &str, step: Option<&str>) -> Self {
        let step = step
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0);
        let time = value.split_once('T').map_or(value, |(_, t)| t);
        let mut result = Self {
            kind,
            values: [None; 8],
            active: 0,
            digits: 0,
            typed: 0,
            ranges: Field::ALL.map(Field::bounds),
            name_prefix: [0; 9],
            name_length: 0,
            seconds: time.matches(':').count() > 1 || step.is_some_and(|s| s % 60.0 != 0.0),
            steps: [0; 8],
            step_bases: [0; 8],
            precision_step: step,
            millis: time.contains('.') || step.is_some_and(|s| s.fract() != 0.0),
        };
        result.load(value);
        result.configure_steps(step);
        result
    }
    fn fields(&self) -> &'static [Field] {
        use Field::*;
        match (self.kind, self.seconds, self.millis) {
            (Kind::Date, _, _) => &[Day, Month, Year],
            (Kind::Month, _, _) => &[Month, Year],
            (Kind::Week, _, _) => &[Week, Year],
            (Kind::Time, _, true) => &[Hour, Minute, Second, Millis],
            (Kind::Time, true, _) => &[Hour, Minute, Second],
            (Kind::Time, _, _) => &[Hour, Minute],
            (Kind::DateTime, _, true) => &[Day, Month, Year, Hour, Minute, Second, Millis],
            (Kind::DateTime, true, _) => &[Day, Month, Year, Hour, Minute, Second],
            (Kind::DateTime, _, _) => &[Day, Month, Year, Hour, Minute],
        }
    }
    fn load(&mut self, value: &str) {
        use Field::*;
        if value.is_empty() {
            return;
        }
        let mut parts = value
            .split(['-', 'W', 'T', ':', '.'])
            .filter(|s| !s.is_empty());
        let fields = self.kind.canonical_fields();
        for &field in fields {
            if let Some(part) = parts.next() {
                self.values[field as usize] = part.parse::<u32>().ok().map(|v| {
                    if field == Millis {
                        v * 10_u32.pow(3_u32.saturating_sub(part.len() as u32))
                    } else {
                        v
                    }
                });
            }
        }
        if self.seconds {
            self.values[Second as usize].get_or_insert(0);
        }
        if self.millis {
            self.values[Millis as usize].get_or_insert(0);
        }
    }
    pub(crate) fn value(&self) -> String {
        use Field::*;
        if self
            .fields()
            .iter()
            .any(|&f| self.values[f as usize].is_none())
        {
            return String::new();
        }
        let v = |f: Field| self.values[f as usize].unwrap_or(0);
        let date = || format!("{:04}-{:02}-{:02}", v(Year), v(Month), v(Day));
        let time = || {
            let mut text = format!("{:02}:{:02}", v(Hour), v(Minute));
            if self.seconds || self.millis {
                text.push_str(&format!(":{:02}", v(Second)));
            }
            if self.millis {
                text.push_str(&format!(".{:03}", v(Millis)));
            }
            text
        };
        let value = match self.kind {
            Kind::Date => date(),
            Kind::Month => format!("{:04}-{:02}", v(Year), v(Month)),
            Kind::Week => format!("{:04}-W{:02}", v(Year), v(Week)),
            Kind::Time => time(),
            Kind::DateTime => format!("{}T{}", date(), time()),
        };
        if crate::validation::calendar::parse(self.kind.name(), &value).is_none() {
            return String::new();
        }
        if self.kind == Kind::DateTime {
            crate::validation::calendar::normalize_datetime(&value)
        } else {
            value
        }
    }
    pub(crate) fn bad_input(&self) -> bool {
        self.values.iter().any(Option::is_some) && self.value().is_empty()
    }
    pub(crate) fn move_field(&mut self, backwards: bool) -> bool {
        let next = if backwards {
            self.active.checked_sub(1)
        } else {
            (self.active + 1 < self.fields().len()).then_some(self.active + 1)
        };
        if let Some(next) = next {
            self.commit_field();
            self.active = next;
            self.digits = 0;
            self.name_length = 0;
            true
        } else {
            false
        }
    }
    /// Returns false only for an unhandled key. No string or loop grows from input.
    pub(crate) fn key(&mut self, key: &Key, reference_year: u32) -> bool {
        let field = self.fields()[self.active];
        match key {
            Key::ArrowLeft => {
                self.move_field(true);
            }
            Key::ArrowRight => {
                self.move_field(false);
            }
            Key::Backspace | Key::Delete => {
                self.values[field as usize] = None;
                self.digits = 0;
            }
            Key::ArrowUp | Key::ArrowDown => {
                let (min, max) = self.ranges[field as usize];
                let step = i64::from(self.steps[field as usize]);
                let base = i64::from(self.step_bases[field as usize]);
                let round_up = |v: i64| base + (v - base + step - 1).div_euclid(step) * step;
                let round_down = |v: i64| base + (v - base).div_euclid(step) * step;
                let first = round_up(i64::from(min));
                let last = round_down(i64::from(max));
                let value = match self.values[field as usize] {
                    None if field == Field::Year => i64::from(reference_year.clamp(min, max)),
                    None if *key == Key::ArrowUp => first,
                    None => last,
                    Some(v) if *key == Key::ArrowUp => {
                        let next = round_up(i64::from(v) + 1);
                        if next > i64::from(max) { first } else { next }
                    }
                    Some(v) => {
                        let next = round_down(i64::from(v) - 1);
                        if next < i64::from(min) { last } else { next }
                    }
                }
                .clamp(i64::from(min), i64::from(max)) as u32;
                self.values[field as usize] = Some(value);
                self.digits = 0;
            }
            Key::Character(s)
                if self.kind == Kind::Month
                    && field == Field::Month
                    && s.len() == 1
                    && s.as_bytes()[0].is_ascii_alphabetic() =>
            {
                self.month_letter(s.as_bytes()[0].to_ascii_lowercase());
            }
            Key::Character(s) if s.len() == 1 && s.as_bytes()[0].is_ascii_digit() => {
                self.digit(field, u32::from(s.as_bytes()[0] - b'0'));
            }
            _ => return false,
        }
        true
    }
    fn digit(&mut self, field: Field, digit: u32) {
        self.name_length = 0;
        if self.kind == Kind::Month && field == Field::Month && digit == 0 && self.digits == 0 {
            return;
        }
        let (min, max) = field.bounds();
        if self.digits == 0 {
            self.typed = 0;
        }
        let modulus = 10_u32.pow(u32::from(field.width()) - 1);
        self.typed = (self.typed % modulus) * 10 + digit;
        self.digits = self.digits.saturating_add(1).min(field.width());
        let value = self.typed;
        self.values[field as usize] = (value >= min).then_some(value.min(max));
        let completed = self.digits >= field.width() || value * 10 > max;
        if completed {
            // Chromium's named month field keeps focus until explicit navigation.
            if self.kind == Kind::Month && field == Field::Month {
                self.digits = 0;
            } else {
                self.move_field(false);
            }
        }
    }
    fn commit_field(&mut self) {
        if self.digits > 0 {
            let field = self.fields()[self.active];
            let (min, max) = field.bounds();
            self.values[field as usize] = Some(self.typed.clamp(min, max));
        }
        self.digits = 0;
        self.name_length = 0;
    }
    fn update_precision(&mut self, value: &str, step: Option<&str>) {
        let parsed_step = step
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0);
        if self.precision_step == parsed_step && self.steps != [0; 8] {
            return;
        }
        self.precision_step = parsed_step;
        let wanted = Self::new(self.kind, value, step);
        let complete = !value.is_empty();
        self.seconds = wanted.seconds;
        self.millis = wanted.millis;
        if complete {
            if self.seconds {
                self.values[Field::Second as usize].get_or_insert(0);
            }
            if self.millis {
                self.values[Field::Millis as usize].get_or_insert(0);
            }
        }
        self.active = self.active.min(self.fields().len() - 1);
        self.configure_steps(parsed_step);
    }
    fn configure_steps(&mut self, step: Option<f64>) {
        self.steps = [1; 8];
        if let Some(step) = step {
            for (field, scale, range) in [
                (Field::Hour, 3600.0, 24),
                (Field::Minute, 60.0, 60),
                (Field::Second, 1.0, 60),
                (Field::Millis, 0.001, 1000),
            ] {
                let amount = step / scale;
                if amount.fract() == 0.0
                    && amount >= 1.0
                    && amount <= f64::from(range)
                    && range % (amount as u32) == 0
                {
                    self.steps[field as usize] = amount as u32;
                }
            }
        }
    }
}
