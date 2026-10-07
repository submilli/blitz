//! HTML calendar values in the numeric units used by min/max/step.
const DAY: f64 = 86_400_000.0;
const MAX_DATE: f64 = 8_640_000_000_000_000.0;

fn digits(value: &str, width: usize) -> Option<i64> {
    (value.len() == width && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn year(value: &str) -> Option<i64> {
    if value.len() < 4 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i64 = value.parse().ok()?;
    (1..=275760).contains(&year).then_some(year)
}

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Proleptic Gregorian civil date to days since 1970-01-01, using 400-year eras.
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let y = year - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    era * 146097 + y * 365 + y / 4 - y / 100 + (153 * m + 2) / 5 + day - 1 - 719468
}

fn date(value: &str) -> Option<f64> {
    let (ym, day) = value.rsplit_once('-')?;
    let (y, m) = ym.split_once('-')?;
    let (y, m, d) = (year(y)?, digits(m, 2)?, digits(day, 2)?);
    let last = match m {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap(y) {
                29
            } else {
                28
            }
        }
        1..=12 => 31,
        _ => return None,
    };
    if !(1..=last).contains(&d) {
        return None;
    }
    let result = days(y, m, d) as f64 * DAY;
    (result <= MAX_DATE).then_some(result)
}

fn month(value: &str) -> Option<f64> {
    let (y, m) = value.split_once('-')?;
    let (y, m) = (year(y)?, digits(m, 2)?);
    ((1..=12).contains(&m) && (y < 275760 || m <= 9)).then_some(((y - 1970) * 12 + m - 1) as f64)
}

fn week(value: &str) -> Option<f64> {
    let (y, w) = value.split_once("-W")?;
    let (y, w) = (year(y)?, digits(w, 2)?);
    let jan1 = (days(y, 1, 1) + 3).rem_euclid(7); // Monday = 0.
    let last = if jan1 == 3 || jan1 == 2 && leap(y) {
        53
    } else {
        52
    };
    if !(1..=last).contains(&w) {
        return None;
    }
    let jan4 = days(y, 1, 4);
    let monday = jan4 - (jan4 + 3).rem_euclid(7);
    let result = (monday + (w - 1) * 7) as f64 * DAY;
    (result <= MAX_DATE).then_some(result)
}

fn time(value: &str) -> Option<f64> {
    let mut parts = value.split(':');
    let h = digits(parts.next()?, 2)?;
    let m = digits(parts.next()?, 2)?;
    if h > 23 || m > 59 {
        return None;
    }
    let mut millis = (h * 60 + m) * 60_000;
    if let Some(seconds) = parts.next() {
        let (seconds, fraction) = seconds
            .split_once('.')
            .map_or((seconds, None), |(s, f)| (s, Some(f)));
        let seconds = digits(seconds, 2)?;
        if seconds > 59 {
            return None;
        }
        millis += seconds * 1000;
        if let Some(fraction) = fraction {
            if !(1..=3).contains(&fraction.len()) {
                return None;
            }
            millis += digits(fraction, fraction.len())? * 10_i64.pow(3 - fraction.len() as u32);
        }
    }
    parts.next().is_none().then_some(millis as f64)
}

pub(crate) fn parse(kind: &str, value: &str) -> Option<f64> {
    match kind {
        "date" => date(value),
        "month" => month(value),
        "week" => week(value),
        "time" => time(value),
        "datetime-local" => {
            let (d, t) = value.split_once(['T', ' '])?;
            let value = date(d)? + time(t)?;
            (value <= MAX_DATE).then_some(value)
        }
        _ => None,
    }
}

/// Local datetimes use the shortest normalized time representation.
pub(crate) fn normalize_datetime(value: &str) -> String {
    let Some((date, time)) = value.split_once(['T', ' ']) else {
        return String::new();
    };
    let mut time = time.to_owned();
    if time.contains('.') {
        while time.ends_with('0') {
            time.pop();
        }
        if time.ends_with('.') {
            time.pop();
        }
    }
    if time.len() == 8 && time.ends_with(":00") {
        time.truncate(5);
    }
    let (y, suffix) = date
        .split_once('-')
        .expect("validated local datetime has a year separator");
    let y = y.trim_start_matches('0');
    format!("{y:0>4}-{suffix}T{time}")
}
