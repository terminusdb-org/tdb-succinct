//! Fixed-format ISO 8601 date/time parsing at TerminusDB storage
//! resolution: `i128` years (≈10³⁶, far past deep time), arbitrary-precision
//! decimal fractions (Planck-scale and beyond), the six timezone classes,
//! and IERS leap-second awareness.
//!
//! Single pass over the input bytes, no allocation — substantially faster
//! than going through a format-string parser, and higher resolution than
//! `chrono` (which caps at nanoseconds and ±262,143 years). Instants are
//! `i128` seconds — signed, so deep-past values carry their full negative
//! magnitude with no wrap.
//!
//! Datatype contract: <https://terminusdb.com/docs/xdd-datetimeinterval/>
//!
//! Accepted forms (extended):
//! ```text
//! date:     [-]YYYY-MM-DD[zone]
//! dateTime: [-]YYYY-MM-DD T HH:MM:SS[.frac][zone]   — the `T` is mandatory
//! zone:     Z | +HH:MM | -HH:MM | absent (zoneless)
//!
//! Strict conformance, no leniency: `xsd:dateTime` lexical space — seconds
//! required, `T` required, `Z`/colon offsets only, no `+` year sign,
//! no space separator, no basic-format anything.
//! ```
//! `23:59:60` is accepted only on IERS leap days. `24:00`, leap-second
//! `:60` elsewhere, and comma decimals are rejected.

use std::fmt;

// -- leap seconds ------------------------------------------------------------
//
// The instant domain counts leap seconds: a day that had a `23:59:60`
// contributes 86401 seconds, not 86400. IERS positive leap days as YYYYMMDD
// keys — the set is closed (last was 2016-12-31), the table never grows.
const LEAP_DAYS: &[u32] = &[
    19720630, 19721231, 19731231, 19741231, 19751231, 19761231, 19771231, 19781231, 19791231,
    19810630, 19820630, 19830630, 19850630, 19871231, 19891231, 19901231, 19920630, 19930630,
    19940630, 19951231, 19970630, 19981231, 20051231, 20081231, 20120630, 20150630, 20161231,
];

fn day_key(y: i128, m: u8, d: u8) -> i128 {
    y * 10000 + m as i128 * 100 + d as i128
}

/// `true` when `(y, m, d)` is an IERS day that carried a `23:59:60`.
pub fn is_leap_day(y: i128, m: u8, d: u8) -> bool {
    let key = day_key(y, m, d);
    key >= 0 && LEAP_DAYS.binary_search(&(key as u32)).is_ok()
}

/// Leap days strictly before `(y, m, d)` — the leap-second shift that day.
pub fn leaps_before(y: i128, m: u8, d: u8) -> i128 {
    LEAP_DAYS.partition_point(|&l| (l as i128) < day_key(y, m, d)) as i128
}

// -- civil calendar -----------------------------------------------------------

/// Proleptic Gregorian leap year (year 0 = 1 BCE is a leap year).
pub fn is_leap_year(y: i128) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Days in `month` (1–12) of `year`.
pub fn days_in_month(y: i128, m: u8) -> u8 {
    const DIM: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if m == 2 && is_leap_year(y) {
        29
    } else {
        DIM[(m - 1) as usize]
    }
}

/// Days from 1970-01-01 to `y-m-d` on the proleptic Gregorian calendar —
/// Howard Hinnant's `days_from_civil`, `None` on `i128` overflow.
pub fn days_from_civil(y: i128, m: u8, d: u8) -> Option<i128> {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i128 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i128 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era.checked_mul(146097)?.checked_add(doe - 719468)
}

/// Same, in `i64` — the hot path. Covers every year whose epoch-day fits
/// `i64` (±~2.5·10¹⁰ days ≈ ±6.9·10⁹ years); wider inputs use `days_from_civil`.
fn days_from_civil_i64(y: i64, m: u8, d: u8) -> Option<i64> {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era.checked_mul(146097)?.checked_add(doe - 719468)
}

/// Inverse of `days_from_civil`: epoch-day → (year, month, day), i64-fast.
/// i64 days cover ±~6.9·10⁹ years; beyond that the i128 chain runs.
pub fn civil_from_days(z: i128) -> (i128, u8, u8) {
    if let Ok(z) = i64::try_from(z) {
        let (y, m, d) = civil_from_days_i64(z);
        return (y as i128, m, d);
    }
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = (mp + if mp < 10 { 3 } else { -9 }) as u8;
    (y + i128::from(m <= 2), m, d)
}

fn civil_from_days_i64(z: i64) -> (i64, u8, u8) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = (mp + if mp < 10 { 3 } else { -9 }) as u8;
    (y + i64::from(m <= 2), m, d)
}

/// Inverse of `leap_aware_instant` at zero offset: the wall-clock reading of
/// a leap-aware instant — (year, month, day, seconds_of_day, is_leap_second).
/// `seconds_of_day` is `0..=86399`; the `23:59:60` second reports via the
/// flag instead of a value (POSIX has no name for it).
/// `None` only for instants outside the representable day range.
pub fn leap_instant_to_wall(s: i128) -> Option<(i128, u8, u8, u32, bool)> {
    // The wall day is within a couple of days of the POSIX approximation:
    // the leap shift is bounded by the table size (27 s).
    let approx = s.div_euclid(86400);
    for dd in -3i128..=3 {
        let day = approx + dd;
        let (y, m, d) = civil_from_days(day);
        let start = day.checked_mul(86400)?.checked_add(leaps_before(y, m, d))?;
        let span = if is_leap_day(y, m, d) { 86401 } else { 86400 };
        if s >= start && s < start + span {
            let sod = s - start;
            return Some((y, m, d, sod.min(86399) as u32, sod == 86400));
        }
    }
    None
}

// -- types -------------------------------------------------------------------

/// Timezone designator — all six storage classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tz {
    /// No designator — local wall-clock time.
    Zoneless,
    /// `Z`
    Utc,
    /// `+00:00` — a distinct spelling from `Z`.
    PosZero,
    /// `-00:00` — ISO 8601-2 "offset not intended to be known".
    NegZero,
    /// `+HH:MM` with `HH:MM` nonzero, offset minutes.
    Pos(u16),
    /// `-HH:MM` with `HH:MM` nonzero, offset minutes.
    Neg(u16),
}

impl Tz {
    /// Signed offset minutes applied to reach UTC; zoneless reads naive.
    pub fn offset_minutes(self) -> i16 {
        match self {
            Tz::Pos(m) => m as i16,
            Tz::Neg(m) => -(m as i16),
            _ => 0,
        }
    }
}

/// A parsed `date` (whole-day value; the day is a duration, not an instant).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i128,
    pub month: u8,
    pub day: u8,
    pub tz: Tz,
}

impl Date {
    /// Leap-aware instant of the day's start, per the storage domain:
    /// `days*86400 + leaps_before(date) − offset*60`.
    pub fn leap_aware_start(&self) -> Option<i128> {
        let leaps = leaps_before(self.year, self.month, self.day);
        if let Ok(y) = i64::try_from(self.year) {
            if let Some(s) = days_from_civil_i64(y, self.month, self.day)?
                .checked_mul(86400)
                .and_then(|s| s.checked_add(leaps as i64))
                .and_then(|s| s.checked_sub(self.tz.offset_minutes() as i64 * 60))
            {
                return Some(s as i128);
            }
        }
        let base = days_from_civil(self.year, self.month, self.day)?.checked_mul(86400)?;
        base.checked_add(leaps)?
            .checked_sub(self.tz.offset_minutes() as i128 * 60)
    }

    /// The calendar's next day, `None` at the representable limit.
    pub fn next_day(&self) -> Option<Date> {
        let (mut y, mut m, mut d) = (self.year, self.month, self.day + 1);
        if d > days_in_month(y, m) {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y = y.checked_add(1)?;
            }
        }
        Some(Date { year: y, month: m, day: d, tz: self.tz })
    }
}

/// A parsed `dateTime` — one instant, with the fraction kept verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateTime<'a> {
    pub year: i128,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    /// `0..=60`; `60` only on `23:59:60` of an IERS leap day.
    pub second: u8,
    /// Fraction digits after `.`, verbatim — `""` when absent. At most 44
    /// digits (10⁻⁴⁴ s ≈ Planck scale — the precision boundary).
    pub frac: &'a str,
    pub tz: Tz,
}

impl<'a> DateTime<'a> {
    /// Seconds of the written day, `0..=86400` (86400 = the leap second).
    pub fn seconds_of_day(&self) -> i128 {
        self.hour as i128 * 3600 + self.minute as i128 * 60 + self.second as i128
    }

    /// POSIX epoch seconds of the instant — leap seconds fold onto the
    /// next-day boundary, as POSIX defines. `i128`: deep-time values carry
    /// their full signed magnitude (±~10³⁸ s).
    pub fn posix_instant(&self) -> Option<i128> {
        self.instant(0)
    }

    /// Leap-aware instant per the storage domain — `leap_aware_secs`:
    /// POSIX base + leap days strictly before the written date + seconds
    /// of day − offset.
    pub fn leap_aware_instant(&self) -> Option<i128> {
        self.instant(leaps_before(self.year, self.month, self.day))
    }

    /// `days*86400 + leaps + seconds_of_day − offset`, i64-fast: the i64
    /// chain covers every instant within ±~2.9·10¹¹ years; only genuinely
    /// deep-time literals take the i128 chain.
    fn instant(&self, leaps: i128) -> Option<i128> {
        if let Ok(y) = i64::try_from(self.year) {
            let sod = self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64;
            if let Some(s) = days_from_civil_i64(y, self.month, self.day)?
                .checked_mul(86400)
                .and_then(|s| s.checked_add(leaps as i64))
                .and_then(|s| s.checked_add(sod))
                .and_then(|s| s.checked_sub(self.tz.offset_minutes() as i64 * 60))
            {
                return Some(s as i128);
            }
        }
        days_from_civil(self.year, self.month, self.day)?
            .checked_mul(86400)?
            .checked_add(leaps)?
            .checked_add(self.seconds_of_day())?
            .checked_sub(self.tz.offset_minutes() as i128 * 60)
    }

    /// `(posix_seconds, nanos)` — the `chrono`-comparable view. The
    /// fraction is truncated/padded to 9 digits; the verbatim `frac`
    /// field keeps full precision. `None` beyond `i64` seconds.
    pub fn nanos(&self) -> Option<(i64, u32)> {
        let secs: i64 = self.posix_instant()?.try_into().ok()?;
        let mut n = 0u32;
        let mut place = 100_000_000u32;
        for &b in self.frac.as_bytes().iter().take(9) {
            n += (b - b'0') as u32 * place;
            place /= 10;
        }
        Some((secs, n))
    }
}

// -- errors -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    YearDigits,
    Separator,
    TwoDigits,
    Range,
    LeapSecondNotOnLeapDay,
    Fraction,
    OffsetRange,
    TrailingData,
    Incomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ErrorKind,
    /// Byte offset where parsing stopped.
    pub offset: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at byte {}", self.kind, self.offset)
    }
}

impl std::error::Error for ParseError {}

type PResult<T> = Result<T, ParseError>;

fn err(kind: ErrorKind, offset: usize) -> ParseError {
    ParseError { kind, offset }
}

// -- scanner ------------------------------------------------------------------

struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Scan<'a> {
    fn new(s: &'a str) -> Self {
        Scan { b: s.as_bytes(), i: 0 }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn take(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    /// Exactly `n` ASCII digits → u32 — single fused check+accumulate pass.
    fn digits(&mut self, n: usize, kind: ErrorKind) -> PResult<u32> {
        if self.i + n > self.b.len() {
            return Err(err(kind, self.b.len()));
        }
        let mut v: u32 = 0;
        for (j, &c) in self.b[self.i..self.i + n].iter().enumerate() {
            if !c.is_ascii_digit() {
                return Err(err(kind, self.i + j));
            }
            v = v * 10 + (c - b'0') as u32;
        }
        self.i += n;
        Ok(v)
    }
    /// Signed year: optional `-`/`+`, at least 4 digits, `i128` range
    /// (~38 digits — deep time both directions). Four digits are the hot
    /// path; longer years fall through to checked arithmetic.
    fn year(&mut self) -> PResult<i128> {
        let neg = self.take(b'-');
        // xsd dateTime years take an optional minus only — no plus sign.
        let start = self.i;
        // Dual-width: accumulate in i64 (the common 4-digit path costs the
        // same as chrono), spill to i128 only when the literal exceeds it.
        let mut v64 = self.digits(4, ErrorKind::YearDigits)? as i64;
        let mut v128: i128 = 0;
        while let Some(c @ b'0'..=b'9') = self.peek() {
            let d = (c - b'0') as i64;
            if v128 != 0 {
                v128 = v128
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(d as i128))
                    .ok_or(err(ErrorKind::Range, self.i))?;
            } else if let Some(n) = v64.checked_mul(10).and_then(|v| v.checked_add(d)) {
                v64 = n;
            } else {
                v128 = v64 as i128 * 10 + d as i128;
            }
            self.i += 1;
        }
        let _ = start;
        let v = if v128 != 0 { v128 } else { v64 as i128 };
        Ok(if neg { -v } else { v })
    }
    fn separator(&mut self, c: u8) -> PResult<()> {
        if self.take(c) {
            Ok(())
        } else {
            Err(err(ErrorKind::Separator, self.i))
        }
    }
}

/// Parse the shared timezone tail — always the rest of the input, so it is
/// consumed wholesale. `Zoneless` when there is no designator. Rejects
/// out-of-range offsets (> ±14:00) and offset minutes > 59.
fn tz_tail(s: &mut Scan) -> PResult<Tz> {
    let t = &s.b[s.i..];
    match t {
        [] => Ok(Tz::Zoneless),
        [b'Z'] => {
            s.i += 1;
            Ok(Tz::Utc)
        }
        [sign @ (b'+' | b'-'), rest @ ..] => {
            // ±HH:MM only — `+HHMM` is ISO basic format, outside the
            // xsd:dateTime lexical space.
            let (h1, h2, m1, m2) = match rest {
                [a, b, b':', c, d] => (*a, *b, *c, *d),
                _ => return Err(err(ErrorKind::TrailingData, s.i)),
            };
            if ![h1, h2, m1, m2].iter().all(|c| c.is_ascii_digit()) {
                return Err(err(ErrorKind::TrailingData, s.i));
            }
            let hh = (h1 - b'0') as u16 * 10 + (h2 - b'0') as u16;
            let mm = (m1 - b'0') as u16 * 10 + (m2 - b'0') as u16;
            if mm > 59 || hh * 60 + mm > 14 * 60 {
                return Err(err(ErrorKind::OffsetRange, s.i));
            }
            let off = hh * 60 + mm;
            s.i = s.b.len();
            Ok(match (*sign, off) {
                (b'+', 0) => Tz::PosZero,
                (b'-', 0) => Tz::NegZero,
                (b'+', m) => Tz::Pos(m),
                (_, m) => Tz::Neg(m),
            })
        }
        _ => Err(err(ErrorKind::TrailingData, s.i)),
    }
}

/// `[-]YYYY-MM-DD[zone]` — the whole-day value. `is_end` bumps to the next
/// day (the exclusive bound for an inclusive date end).
pub fn parse_date(s: &str) -> PResult<Date> {
    let mut sc = Scan::new(s);
    let (year, month, day) = date_ymd(&mut sc)?;
    let tz = tz_tail(&mut sc)?;
    Ok(Date { year, month, day, tz })
}

fn date_ymd(sc: &mut Scan) -> PResult<(i128, u8, u8)> {
    let y = sc.year()?;
    sc.separator(b'-')?;
    let m = sc.digits(2, ErrorKind::TwoDigits)?;
    sc.separator(b'-')?;
    let d = sc.digits(2, ErrorKind::TwoDigits)?;
    if !(1..=12).contains(&m) {
        return Err(err(ErrorKind::Range, sc.i));
    }
    if d == 0 || d > days_in_month(y, m as u8) as u32 {
        return Err(err(ErrorKind::Range, sc.i));
    }
    Ok((y, m as u8, d as u8))
}

/// `[-]YYYY-MM-DD(T|space)HH:MM:SS[.frac][zone]` — a single instant.
pub fn parse_datetime(s: &str) -> PResult<DateTime<'_>> {
    let mut sc = Scan::new(s);
    let (year, month, day) = date_ymd(&mut sc)?;
    if !sc.take(b'T') {
        return Err(err(ErrorKind::Separator, sc.i));
    }
    let hour = sc.digits(2, ErrorKind::TwoDigits)?;
    sc.separator(b':')?;
    let minute = sc.digits(2, ErrorKind::TwoDigits)?;
    if hour > 23 || minute > 59 {
        return Err(err(ErrorKind::Range, sc.i));
    }
    // `HH:MM:SS` mandatory — xsd:dateTime requires seconds; `HH:MM` is an
    // ISO shorthand outside the XML Schema lexical space.
    sc.separator(b':')?;
    let second = sc.digits(2, ErrorKind::TwoDigits)? as u8;
    if second > 60 {
        return Err(err(ErrorKind::Range, sc.i));
    }
    let mut frac = "";
    if sc.take(b'.') {
        let start = sc.i;
        let run = sc.b[start..]
            .iter()
            .position(|c| !c.is_ascii_digit())
            .unwrap_or(sc.b.len() - start);
        // 1..=49 digits — 10⁻⁴⁹ s, safely below Planck scale (5.39×10⁻⁴⁴)
        if run == 0 || run > 49 {
            return Err(err(ErrorKind::Fraction, start));
        }
        sc.i += run;
        frac = &s[start..sc.i];
    }
    if second == 60 && !(hour == 23 && minute == 59 && is_leap_day(year, month, day)) {
        return Err(err(ErrorKind::LeapSecondNotOnLeapDay, sc.i));
    }
    let tz = tz_tail(&mut sc)?;
    Ok(DateTime {
        year,
        month,
        day,
        hour: hour as u8,
        minute: minute as u8,
        second,
        frac,
        tz,
    })
}
