//! Fixed calendar slots, with HTML's representable year range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Day,
    Month,
    Year,
    Week,
    Hour,
    Minute,
    Second,
    Millis,
}
impl Field {
    pub(super) const ALL: [Self; 8] = [
        Self::Day,
        Self::Month,
        Self::Year,
        Self::Week,
        Self::Hour,
        Self::Minute,
        Self::Second,
        Self::Millis,
    ];
    pub(super) fn bounds(self) -> (u32, u32) {
        match self {
            Self::Day => (1, 31),
            Self::Month => (1, 12),
            Self::Year => (1, 275760),
            Self::Week => (1, 53),
            Self::Hour => (0, 23),
            Self::Minute | Self::Second => (0, 59),
            Self::Millis => (0, 999),
        }
    }
    pub(super) fn width(self) -> u8 {
        match self {
            Self::Year => 6,
            Self::Millis => 3,
            _ => 2,
        }
    }
    pub(super) fn placeholder(self) -> &'static str {
        match self {
            Self::Day => "dd",
            Self::Month => "mm",
            Self::Year => "yyyy",
            Self::Millis => "---",
            _ => "--",
        }
    }
}
