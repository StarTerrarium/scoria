//! Release dates.
//!
//! Tags carry `2021-01-14`, `2021-01` or `2021` depending on how much MusicBrainz
//! knows, so a date is a year with optionally a month and a day rather than a
//! calendar date. Ordering is derived: `None` sorts before `Some`, so a
//! year-only release sorts ahead of a fully dated one in the same year, which is
//! the only defensible answer when the finer detail is simply unknown.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A release date, precise to the year, month or day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Date {
    year: i16,
    month: Option<u8>,
    day: Option<u8>,
}

impl Date {
    /// A year-only date.
    #[must_use]
    pub const fn from_year(year: i16) -> Self {
        Self {
            year,
            month: None,
            day: None,
        }
    }

    /// Parses the ISO-ish forms tags actually contain: `2021-01-14`, `2021-01`,
    /// `2021`. Returns `None` for anything else rather than guessing -- a wrong
    /// date silently reorders an artist's discography.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let mut parts = s.split('-');

        let year: i16 = parts.next()?.parse().ok()?;
        let month = match parts.next() {
            None => None,
            Some(m) => Some(Self::checked_month(m.parse().ok()?)?),
        };
        let day = match parts.next() {
            None => None,
            Some(d) => Some(Self::checked_day(d.parse().ok()?)?),
        };
        // A trailing component means this is not a date we understand.
        if parts.next().is_some() {
            return None;
        }
        // `2021--14` would otherwise parse as a day without a month.
        if month.is_none() && day.is_some() {
            return None;
        }
        Some(Self { year, month, day })
    }

    const fn checked_month(m: u8) -> Option<u8> {
        if m >= 1 && m <= 12 { Some(m) } else { None }
    }

    const fn checked_day(d: u8) -> Option<u8> {
        if d >= 1 && d <= 31 { Some(d) } else { None }
    }

    #[must_use]
    pub const fn year(self) -> i16 {
        self.year
    }

    #[must_use]
    pub const fn month(self) -> Option<u8> {
        self.month
    }

    #[must_use]
    pub const fn day(self) -> Option<u8> {
        self.day
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.month, self.day) {
            (Some(m), Some(d)) => write!(f, "{:04}-{m:02}-{d:02}", self.year),
            (Some(m), None) => write!(f, "{:04}-{m:02}", self.year),
            _ => write!(f, "{:04}", self.year),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parses_the_three_forms_tags_contain() {
        assert_eq!(
            Date::parse("2021-01-14"),
            Some(Date {
                year: 2021,
                month: Some(1),
                day: Some(14)
            })
        );
        assert_eq!(
            Date::parse("2021-01"),
            Some(Date {
                year: 2021,
                month: Some(1),
                day: None
            })
        );
        assert_eq!(Date::parse("2021"), Some(Date::from_year(2021)));
    }

    #[test]
    fn rejects_rather_than_guesses() {
        for bad in [
            "",
            "not a date",
            "2021-13",
            "2021-01-32",
            "2021-01-14-1",
            "2021-",
            "20x1",
        ] {
            assert_eq!(Date::parse(bad), None, "should have rejected {bad:?}");
        }
    }

    #[test]
    fn less_precise_sorts_first_within_a_year() {
        let year_only = Date::parse("2021").unwrap();
        let month = Date::parse("2021-01").unwrap();
        let full = Date::parse("2021-01-14").unwrap();
        assert!(year_only < month);
        assert!(month < full);
    }

    #[test]
    fn orders_chronologically_across_years() {
        assert!(Date::parse("1999-12-31").unwrap() < Date::parse("2000-01-01").unwrap());
    }

    #[test]
    fn round_trips_through_display() {
        for s in ["2021-01-14", "2021-01", "2021"] {
            let parsed = Date::parse(s).unwrap();
            assert_eq!(parsed.to_string(), s);
            assert_eq!(Date::parse(&parsed.to_string()), Some(parsed));
        }
    }
}
