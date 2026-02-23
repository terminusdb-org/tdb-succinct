use chrono::{NaiveDate, NaiveDateTime};
use thiserror::Error;

use super::datatypes::{
    DateTimeInterval, Duration, INTERVAL_COMPONENT_DATE, INTERVAL_COMPONENT_DATETIME,
    INTERVAL_FLAG_DURATION_END, INTERVAL_FLAG_EXPLICIT, INTERVAL_FLAG_START_DURATION,
};

#[derive(Debug, Error)]
pub enum IntervalParseError {
    #[error("empty interval string")]
    Empty,
    #[error("missing '/' separator in interval: `{0}`")]
    MissingSeparator(String),
    #[error("invalid date component: `{0}`")]
    InvalidDate(String),
    #[error("invalid datetime component: `{0}`")]
    InvalidDateTime(String),
    #[error("invalid duration: `{0}`")]
    InvalidDuration(String),
    #[error("interval has two durations: `{0}`")]
    TwoDurations(String),
    #[error("interval has no date/datetime component: `{0}`")]
    NoDates(String),
}

/// Parse an ISO 8601 interval string into a `DateTimeInterval`.
///
/// Supported forms:
/// - `"start/end"` — explicit dates or datetimes
/// - `"start/duration"` — start date/datetime + ISO 8601 duration
/// - `"duration/end"` — ISO 8601 duration + end date/datetime
///
/// Date components: `YYYY-MM-DD`
/// DateTime components: `YYYY-MM-DDTHH:MM:SS[.fff]Z` (trailing Z required for now)
/// Duration: `PnYnMnDTnHnMnS` (standard ISO 8601 duration)
pub fn parse_iso_interval(s: &str) -> Result<DateTimeInterval, IntervalParseError> {
    if s.is_empty() {
        return Err(IntervalParseError::Empty);
    }

    let slash_pos = s
        .find('/')
        .ok_or_else(|| IntervalParseError::MissingSeparator(s.to_string()))?;

    let left = &s[..slash_pos];
    let right = &s[slash_pos + 1..];

    let left_is_duration = left.starts_with('P') || left.starts_with("-P");
    let right_is_duration = right.starts_with('P') || right.starts_with("-P");

    if left_is_duration && right_is_duration {
        return Err(IntervalParseError::TwoDurations(s.to_string()));
    }

    if left_is_duration {
        // duration/end
        let duration = parse_iso_duration(left)?;
        let (end_seconds, end_nanos, end_type) = parse_date_or_datetime(right)?;
        let (start_seconds, start_nanos) =
            compute_start_from_duration_and_end(&duration, end_seconds, end_nanos);
        let start_type = end_type;
        Ok(DateTimeInterval {
            start_seconds,
            start_nanos,
            end_seconds,
            end_nanos,
            start_type,
            end_type,
            flag: INTERVAL_FLAG_DURATION_END,
            duration,
        })
    } else if right_is_duration {
        // start/duration
        let (start_seconds, start_nanos, start_type) = parse_date_or_datetime(left)?;
        let duration = parse_iso_duration(right)?;
        let (end_seconds, end_nanos) =
            compute_end_from_start_and_duration(&duration, start_seconds, start_nanos);
        let end_type = start_type;
        Ok(DateTimeInterval {
            start_seconds,
            start_nanos,
            end_seconds,
            end_nanos,
            start_type,
            end_type,
            flag: INTERVAL_FLAG_START_DURATION,
            duration,
        })
    } else {
        // start/end (explicit)
        let (start_seconds, start_nanos, start_type) = parse_date_or_datetime(left)?;
        let (end_seconds, end_nanos, end_type) = parse_date_or_datetime(right)?;
        let duration = compute_duration_from_endpoints(
            start_seconds,
            start_nanos,
            end_seconds,
            end_nanos,
        );
        Ok(DateTimeInterval {
            start_seconds,
            start_nanos,
            end_seconds,
            end_nanos,
            start_type,
            end_type,
            flag: INTERVAL_FLAG_EXPLICIT,
            duration,
        })
    }
}

/// Parse a date (`YYYY-MM-DD`) or datetime (`YYYY-MM-DDTHH:MM:SS[.fff][Z]`) string.
/// Returns (unix_seconds, nanos, component_type).
fn parse_date_or_datetime(s: &str) -> Result<(i64, u32, u8), IntervalParseError> {
    if s.contains('T') {
        // DateTime
        let s_trimmed = s.trim_end_matches('Z');
        let ndt = NaiveDateTime::parse_from_str(s_trimmed, "%Y-%m-%dT%H:%M:%S%.f")
            .or_else(|_| NaiveDateTime::parse_from_str(s_trimmed, "%Y-%m-%dT%H:%M:%S"))
            .map_err(|_| IntervalParseError::InvalidDateTime(s.to_string()))?;
        Ok((ndt.and_utc().timestamp(), ndt.and_utc().timestamp_subsec_nanos(), INTERVAL_COMPONENT_DATETIME))
    } else {
        // Date only
        let nd = NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| IntervalParseError::InvalidDate(s.to_string()))?;
        let ndt = nd.and_hms_opt(0, 0, 0).unwrap();
        Ok((ndt.and_utc().timestamp(), 0, INTERVAL_COMPONENT_DATE))
    }
}

/// Parse an ISO 8601 duration string like `P3M`, `P1Y2M3DT4H5M6S`, `-P1D`.
fn parse_iso_duration(s: &str) -> Result<Duration, IntervalParseError> {
    let err = || IntervalParseError::InvalidDuration(s.to_string());

    let (sign, rest) = if let Some(stripped) = s.strip_prefix('-') {
        (-1i8, stripped)
    } else {
        (1i8, s)
    };

    let rest = rest.strip_prefix('P').ok_or_else(err)?;

    let (date_part, time_part) = if let Some(t_pos) = rest.find('T') {
        (&rest[..t_pos], Some(&rest[t_pos + 1..]))
    } else {
        (rest, None)
    };

    let mut year: i64 = 0;
    let mut month: u8 = 0;
    let mut day: u8 = 0;

    if !date_part.is_empty() {
        let mut num_buf = String::new();
        for ch in date_part.chars() {
            match ch {
                'Y' => {
                    year = num_buf.parse::<i64>().map_err(|_| err())?;
                    num_buf.clear();
                }
                'M' => {
                    month = num_buf.parse::<u8>().map_err(|_| err())?;
                    num_buf.clear();
                }
                'D' => {
                    day = num_buf.parse::<u8>().map_err(|_| err())?;
                    num_buf.clear();
                }
                '0'..='9' => num_buf.push(ch),
                _ => return Err(err()),
            }
        }
        if !num_buf.is_empty() {
            return Err(err());
        }
    }

    let mut hour: u8 = 0;
    let mut minute: u8 = 0;
    let mut second: f64 = 0.0;

    if let Some(tp) = time_part {
        if tp.is_empty() {
            return Err(err());
        }
        let mut num_buf = String::new();
        for ch in tp.chars() {
            match ch {
                'H' => {
                    hour = num_buf.parse::<u8>().map_err(|_| err())?;
                    num_buf.clear();
                }
                'M' => {
                    minute = num_buf.parse::<u8>().map_err(|_| err())?;
                    num_buf.clear();
                }
                'S' => {
                    second = num_buf.parse::<f64>().map_err(|_| err())?;
                    num_buf.clear();
                }
                '0'..='9' | '.' => num_buf.push(ch),
                _ => return Err(err()),
            }
        }
        if !num_buf.is_empty() {
            return Err(err());
        }
    }

    Ok(Duration {
        sign,
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

/// Compute an approximate duration from two endpoints.
/// This produces a day-based duration (no month/year component).
fn compute_duration_from_endpoints(
    start_seconds: i64,
    _start_nanos: u32,
    end_seconds: i64,
    _end_nanos: u32,
) -> Duration {
    let diff = end_seconds - start_seconds;
    let sign = if diff >= 0 { 1 } else { -1 };
    let abs_diff = diff.unsigned_abs();
    let days = (abs_diff / 86400) as u8;
    let remainder = abs_diff % 86400;
    let hours = (remainder / 3600) as u8;
    let remainder = remainder % 3600;
    let minutes = (remainder / 60) as u8;
    let secs = (remainder % 60) as f64;
    Duration {
        sign,
        year: 0,
        month: 0,
        day: days,
        hour: hours,
        minute: minutes,
        second: secs,
    }
}

/// Compute end timestamp from start + duration.
/// For year/month durations, this uses chrono calendar arithmetic.
fn compute_end_from_start_and_duration(
    duration: &Duration,
    start_seconds: i64,
    start_nanos: u32,
) -> (i64, u32) {
    let start_dt = NaiveDateTime::from_timestamp_opt(start_seconds, start_nanos)
        .unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        });

    let sign = if duration.sign >= 0 { 1i64 } else { -1i64 };

    // Add year/month via chrono
    let mut dt = start_dt;
    if duration.year != 0 || duration.month != 0 {
        let total_months =
            sign * (duration.year * 12 + duration.month as i64);
        dt = add_months(dt, total_months as i32);
    }

    // Add day/hour/minute/second as seconds
    let day_seconds = sign
        * (duration.day as i64 * 86400
            + duration.hour as i64 * 3600
            + duration.minute as i64 * 60
            + duration.second as i64);

    let end_seconds = dt.and_utc().timestamp() + day_seconds;
    let frac_nanos = ((duration.second.fract() * 1_000_000_000.0) as u32)
        .wrapping_mul(sign as u32);
    let end_nanos = start_nanos.wrapping_add(frac_nanos);

    (end_seconds, end_nanos)
}

/// Compute start timestamp from duration + end.
fn compute_start_from_duration_and_end(
    duration: &Duration,
    end_seconds: i64,
    end_nanos: u32,
) -> (i64, u32) {
    let end_dt = NaiveDateTime::from_timestamp_opt(end_seconds, end_nanos)
        .unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        });

    let sign = if duration.sign >= 0 { -1i64 } else { 1i64 };

    let mut dt = end_dt;
    if duration.year != 0 || duration.month != 0 {
        let total_months =
            sign * (duration.year * 12 + duration.month as i64);
        dt = add_months(dt, total_months as i32);
    }

    let day_seconds = sign
        * (duration.day as i64 * 86400
            + duration.hour as i64 * 3600
            + duration.minute as i64 * 60
            + duration.second as i64);

    let start_seconds = dt.and_utc().timestamp() + day_seconds;
    let frac_nanos = ((duration.second.fract() * 1_000_000_000.0) as u32)
        .wrapping_mul(sign as u32);
    let start_nanos = end_nanos.wrapping_add(frac_nanos);

    (start_seconds, start_nanos)
}

/// Add (or subtract) months to a NaiveDateTime using chrono calendar logic.
/// Clamps the day to the last day of the target month if needed.
fn add_months(dt: NaiveDateTime, months: i32) -> NaiveDateTime {
    let total_months = dt.date().year() as i32 * 12 + dt.date().month0() as i32 + months;
    let target_year = total_months.div_euclid(12);
    let target_month0 = total_months.rem_euclid(12) as u32;
    let target_month = target_month0 + 1;
    let max_day = days_in_month(target_year, target_month);
    let day = dt.date().day().min(max_day);
    NaiveDate::from_ymd_opt(target_year, target_month, day)
        .unwrap()
        .and_hms_nano_opt(
            dt.time().hour(),
            dt.time().minute(),
            dt.time().second(),
            dt.time().nanosecond(),
        )
        .unwrap()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

use chrono::Datelike;
use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tfc::datatypes::{FromLexical, TdbDataType, ToLexical};

    #[test]
    fn parse_explicit_dates() {
        let iv = parse_iso_interval("2025-01-01/2025-04-01").unwrap();
        assert_eq!(iv.flag, INTERVAL_FLAG_EXPLICIT);
        assert_eq!(iv.start_type, INTERVAL_COMPONENT_DATE);
        assert_eq!(iv.end_type, INTERVAL_COMPONENT_DATE);
        assert_eq!(iv.start_seconds, 1735689600); // 2025-01-01T00:00:00Z
        assert_eq!(iv.end_seconds, 1743465600); // 2025-04-01T00:00:00Z
        assert_eq!(iv.start_nanos, 0);
        assert_eq!(iv.end_nanos, 0);
    }

    #[test]
    fn parse_explicit_datetimes() {
        let iv = parse_iso_interval("2025-01-01T10:30:00Z/2025-04-01T15:45:00Z").unwrap();
        assert_eq!(iv.flag, INTERVAL_FLAG_EXPLICIT);
        assert_eq!(iv.start_type, INTERVAL_COMPONENT_DATETIME);
        assert_eq!(iv.end_type, INTERVAL_COMPONENT_DATETIME);
        assert_eq!(iv.start_seconds, 1735727400);
        assert_eq!(iv.end_seconds, 1743522300);
    }

    #[test]
    fn parse_explicit_datetime_with_nanos() {
        let iv = parse_iso_interval("2025-01-01T00:00:00.500Z/2025-04-01T00:00:00Z").unwrap();
        assert_eq!(iv.start_nanos, 500_000_000);
        assert_eq!(iv.end_nanos, 0);
    }

    #[test]
    fn parse_start_duration() {
        let iv = parse_iso_interval("2025-01-01/P3M").unwrap();
        assert_eq!(iv.flag, INTERVAL_FLAG_START_DURATION);
        assert_eq!(iv.start_seconds, 1735689600);
        assert_eq!(iv.duration.month, 3);
        assert_eq!(iv.duration.sign, 1);
        // End should be 2025-04-01
        assert_eq!(iv.end_seconds, 1743465600);
    }

    #[test]
    fn parse_duration_end() {
        let iv = parse_iso_interval("P3M/2025-04-01").unwrap();
        assert_eq!(iv.flag, INTERVAL_FLAG_DURATION_END);
        assert_eq!(iv.end_seconds, 1743465600);
        assert_eq!(iv.duration.month, 3);
        // Start should be 2025-01-01
        assert_eq!(iv.start_seconds, 1735689600);
    }

    #[test]
    fn parse_duration_with_time_parts() {
        let iv = parse_iso_interval("2025-01-01/PT1H").unwrap();
        assert_eq!(iv.flag, INTERVAL_FLAG_START_DURATION);
        assert_eq!(iv.duration.hour, 1);
        assert_eq!(iv.duration.day, 0);
        assert_eq!(iv.end_seconds, iv.start_seconds + 3600);
    }

    #[test]
    fn parse_complex_duration() {
        let iv = parse_iso_interval("2025-01-01/P1Y2M3DT4H5M6S").unwrap();
        assert_eq!(iv.duration.year, 1);
        assert_eq!(iv.duration.month, 2);
        assert_eq!(iv.duration.day, 3);
        assert_eq!(iv.duration.hour, 4);
        assert_eq!(iv.duration.minute, 5);
        assert_eq!(iv.duration.second, 6.0);
    }

    #[test]
    fn parse_negative_duration() {
        let iv = parse_iso_interval("-P1D/2025-01-02").unwrap();
        assert_eq!(iv.duration.sign, -1);
        assert_eq!(iv.duration.day, 1);
    }

    #[test]
    fn parse_error_empty() {
        assert!(parse_iso_interval("").is_err());
    }

    #[test]
    fn parse_error_no_separator() {
        assert!(parse_iso_interval("2025-01-01").is_err());
    }

    #[test]
    fn parse_error_two_durations() {
        assert!(parse_iso_interval("P1M/P2M").is_err());
    }

    #[test]
    fn parse_error_invalid_date() {
        assert!(parse_iso_interval("not-a-date/2025-01-01").is_err());
    }

    #[test]
    fn roundtrip_parse_to_lexical_to_string_explicit() {
        let iv = parse_iso_interval("2025-01-01/2025-04-01").unwrap();
        let bytes = iv.to_lexical();
        let s = <String as FromLexical<DateTimeInterval>>::from_lexical(bytes);
        assert_eq!("2025-01-01/2025-04-01", s);
    }

    #[test]
    fn roundtrip_parse_to_lexical_to_string_start_duration() {
        let iv = parse_iso_interval("2025-01-01/P3M").unwrap();
        let bytes = iv.to_lexical();
        let s = <String as FromLexical<DateTimeInterval>>::from_lexical(bytes);
        assert_eq!("2025-01-01/P3M", s);
    }

    #[test]
    fn roundtrip_parse_to_lexical_to_string_duration_end() {
        let iv = parse_iso_interval("P3M/2025-04-01").unwrap();
        let bytes = iv.to_lexical();
        let s = <String as FromLexical<DateTimeInterval>>::from_lexical(bytes);
        assert_eq!("P3M/2025-04-01", s);
    }

    #[test]
    fn roundtrip_parse_to_lexical_to_struct() {
        let iv = parse_iso_interval("2025-01-01/2025-04-01").unwrap();
        let bytes = iv.to_lexical();
        let iv2 = DateTimeInterval::from_lexical(bytes.clone());
        let bytes2 = iv2.to_lexical();
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn roundtrip_parse_to_lexical_to_struct_start_duration() {
        let iv = parse_iso_interval("2025-01-01/P3M").unwrap();
        let bytes = iv.to_lexical();
        let iv2 = DateTimeInterval::from_lexical(bytes.clone());
        assert_eq!(iv.start_seconds, iv2.start_seconds);
        assert_eq!(iv.end_seconds, iv2.end_seconds);
        assert_eq!(iv.duration.month, iv2.duration.month);
        assert_eq!(iv.flag, iv2.flag);
    }

    #[test]
    fn roundtrip_datetime_with_fractional_seconds() {
        let iv = parse_iso_interval("2025-01-01T10:30:00.500Z/2025-04-01T15:45:00Z").unwrap();
        let bytes = iv.to_lexical();
        let s = <String as FromLexical<DateTimeInterval>>::from_lexical(bytes);
        assert_eq!("2025-01-01T10:30:00.500Z/2025-04-01T15:45:00Z", s);
    }

    #[test]
    fn lexical_ordering_chronological() {
        let iv_early = parse_iso_interval("2024-01-01/2024-06-01").unwrap();
        let iv_late = parse_iso_interval("2025-01-01/2025-06-01").unwrap();

        let entry_early = DateTimeInterval::make_entry(&iv_early);
        let entry_late = DateTimeInterval::make_entry(&iv_late);
        assert!(entry_early < entry_late);
    }

    #[test]
    fn lexical_ordering_same_start_different_end() {
        let iv_short = parse_iso_interval("2025-01-01/2025-02-01").unwrap();
        let iv_long = parse_iso_interval("2025-01-01/2025-06-01").unwrap();

        let entry_short = DateTimeInterval::make_entry(&iv_short);
        let entry_long = DateTimeInterval::make_entry(&iv_long);
        assert!(entry_short < entry_long);
    }

    #[test]
    fn lexical_ordering_mixed_formats() {
        // start/duration and start/end for the same interval should sort the same
        let iv_explicit = parse_iso_interval("2025-01-01/2025-04-01").unwrap();
        let iv_duration = parse_iso_interval("2025-01-01/P3M").unwrap();

        // Same start and end timestamps
        assert_eq!(iv_explicit.start_seconds, iv_duration.start_seconds);
        assert_eq!(iv_explicit.end_seconds, iv_duration.end_seconds);

        // Lexical entries differ only in metadata suffix (flag + duration)
        // but sorting prefix is identical
        let bytes_e = iv_explicit.to_lexical();
        let bytes_d = iv_duration.to_lexical();
        // First 24 bytes (sorting prefix) should be identical
        assert_eq!(&bytes_e[..24], &bytes_d[..24]);
    }

    #[test]
    fn parse_duration_days_only() {
        let iv = parse_iso_interval("2025-01-01/P90D").unwrap();
        assert_eq!(iv.duration.day, 90);
        assert_eq!(iv.duration.month, 0);
    }

    #[test]
    fn parse_duration_fractional_seconds() {
        let iv = parse_iso_interval("2025-01-01/PT0.5S").unwrap();
        assert_eq!(iv.duration.second, 0.5);
    }
}
