//! Portable durable wake/deadline types.

use serde::de::{Error as DeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

/// Portable wake deadline encoded as a canonical RFC 3339 UTC timestamp.
///
/// Canonical form is `YYYY-MM-DDTHH:MM:SS[.fraction]Z`:
/// - UTC only (`Z` suffix required)
/// - date and time are always present
/// - fractional seconds are optional
/// - when present, trailing fractional zeroes are removed
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortableWakeDeadline(String);

impl PortableWakeDeadline {
    /// Parse and canonicalize a portable wake deadline.
    pub fn parse(value: &str) -> Result<Self, WakeDeadlineError> {
        let parsed = ParsedDeadline::parse(value)?;
        Ok(Self(parsed.into_canonical_string()))
    }

    /// Borrow the canonical encoded deadline string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PortableWakeDeadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for PortableWakeDeadline {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PortableWakeDeadline {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DeadlineVisitor;

        impl<'de> Visitor<'de> for DeadlineVisitor {
            type Value = PortableWakeDeadline;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an RFC 3339 UTC timestamp string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                PortableWakeDeadline::parse(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(DeadlineVisitor)
    }
}

impl TryFrom<&str> for PortableWakeDeadline {
    type Error = WakeDeadlineError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<String> for PortableWakeDeadline {
    type Error = WakeDeadlineError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// Validation error for a portable wake deadline.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WakeDeadlineError {
    /// The timestamp does not use the required UTC `Z` suffix.
    #[error("wake deadline must use a UTC 'Z' suffix")]
    MissingUtcSuffix,
    /// The timestamp is missing the date/time separator.
    #[error("wake deadline must contain a 'T' date/time separator")]
    MissingDateTimeSeparator,
    /// The timestamp date portion is malformed.
    #[error("wake deadline date must use YYYY-MM-DD")]
    InvalidDate,
    /// The timestamp time portion is malformed.
    #[error("wake deadline time must use HH:MM:SS[.fraction]")]
    InvalidTime,
    /// A numeric component is outside its valid range.
    #[error("wake deadline component out of range: {0}")]
    ComponentOutOfRange(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedDeadline {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    fractional: Option<String>,
}

impl ParsedDeadline {
    fn parse(value: &str) -> Result<Self, WakeDeadlineError> {
        if !value.ends_with('Z') {
            return Err(WakeDeadlineError::MissingUtcSuffix);
        }

        let core = &value[..value.len() - 1];
        let (date, time) = core
            .split_once('T')
            .ok_or(WakeDeadlineError::MissingDateTimeSeparator)?;

        let (year, month, day) = parse_date(date)?;
        let (hour, minute, second, fractional) = parse_time(time)?;

        let max_day = days_in_month(year, month);
        if day == 0 || day > max_day {
            return Err(WakeDeadlineError::ComponentOutOfRange("day"));
        }

        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            fractional,
        })
    }

    fn into_canonical_string(self) -> String {
        let mut encoded = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );

        if let Some(fractional) = self.fractional
            && !fractional.is_empty()
        {
            encoded.push('.');
            encoded.push_str(&fractional);
        }

        encoded.push('Z');
        encoded
    }
}

fn parse_date(value: &str) -> Result<(u16, u8, u8), WakeDeadlineError> {
    let mut parts = value.split('-');
    let year = parts.next().ok_or(WakeDeadlineError::InvalidDate)?;
    let month = parts.next().ok_or(WakeDeadlineError::InvalidDate)?;
    let day = parts.next().ok_or(WakeDeadlineError::InvalidDate)?;
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return Err(WakeDeadlineError::InvalidDate);
    }

    let year = parse_component(year, "year", 0, u16::MAX as u32)? as u16;
    let month = parse_component(month, "month", 1, 12)? as u8;
    let day = parse_component(day, "day", 1, 31)? as u8;
    Ok((year, month, day))
}

fn parse_time(value: &str) -> Result<(u8, u8, u8, Option<String>), WakeDeadlineError> {
    let (clock, fractional) = match value.split_once('.') {
        Some((clock, fractional)) => {
            if fractional.is_empty() || !fractional.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(WakeDeadlineError::InvalidTime);
            }

            let trimmed = fractional.trim_end_matches('0').to_owned();
            (clock, Some(trimmed))
        }
        None => (value, None),
    };

    let mut parts = clock.split(':');
    let hour = parts.next().ok_or(WakeDeadlineError::InvalidTime)?;
    let minute = parts.next().ok_or(WakeDeadlineError::InvalidTime)?;
    let second = parts.next().ok_or(WakeDeadlineError::InvalidTime)?;
    if parts.next().is_some() || hour.len() != 2 || minute.len() != 2 || second.len() != 2 {
        return Err(WakeDeadlineError::InvalidTime);
    }

    let hour = parse_component(hour, "hour", 0, 23)? as u8;
    let minute = parse_component(minute, "minute", 0, 59)? as u8;
    let second = parse_component(second, "second", 0, 59)? as u8;

    Ok((hour, minute, second, fractional))
}

fn parse_component(
    value: &str,
    label: &'static str,
    min: u32,
    max: u32,
) -> Result<u32, WakeDeadlineError> {
    if !value.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(match label {
            "year" | "month" | "day" => WakeDeadlineError::InvalidDate,
            _ => WakeDeadlineError::InvalidTime,
        });
    }

    let parsed = value
        .parse::<u32>()
        .map_err(|_| WakeDeadlineError::ComponentOutOfRange(label))?;
    if parsed < min || parsed > max {
        return Err(WakeDeadlineError::ComponentOutOfRange(label));
    }
    Ok(parsed)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 31,
    }
}

fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::{PortableWakeDeadline, WakeDeadlineError};

    #[test]
    fn parse_accepts_canonical_utc_timestamp() {
        let deadline = PortableWakeDeadline::parse("2026-03-12T08:15:30Z").unwrap();
        assert_eq!(deadline.as_str(), "2026-03-12T08:15:30Z");
    }

    #[test]
    fn parse_canonicalizes_fractional_trailing_zeroes() {
        let deadline = PortableWakeDeadline::parse("2026-03-12T08:15:30.1200Z").unwrap();
        assert_eq!(deadline.as_str(), "2026-03-12T08:15:30.12Z");
    }

    #[test]
    fn parse_rejects_non_utc_offsets() {
        assert_eq!(
            PortableWakeDeadline::parse("2026-03-12T08:15:30+01:00").unwrap_err(),
            WakeDeadlineError::MissingUtcSuffix
        );
    }

    #[test]
    fn parse_rejects_invalid_calendar_dates() {
        assert_eq!(
            PortableWakeDeadline::parse("2026-02-30T08:15:30Z").unwrap_err(),
            WakeDeadlineError::ComponentOutOfRange("day")
        );
    }
}
