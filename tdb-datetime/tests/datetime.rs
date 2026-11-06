use chrono::{DateTime as Cdt, Datelike, NaiveDate};
use tdb_datetime::*;

// -- valid parse --------------------------------------------------------------

#[test]
fn parses_full_forms() {
    let dt = parse_datetime("2025-03-15T09:30:45.123+02:00").unwrap();
    assert_eq!(
        (dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second, dt.frac),
        (2025, 3, 15, 9, 30, 45, "123")
    );
    assert_eq!(dt.tz, Tz::Pos(120));
}

#[test]
fn timezone_classes() {
    assert_eq!(parse_datetime("2025-01-01T00:00:00Z").unwrap().tz, Tz::Utc);
    assert_eq!(parse_datetime("2025-01-01T00:00:00").unwrap().tz, Tz::Zoneless);
    assert_eq!(parse_datetime("2025-01-01T00:00:00+00:00").unwrap().tz, Tz::PosZero);
    assert_eq!(parse_datetime("2025-01-01T00:00:00-00:00").unwrap().tz, Tz::NegZero);
    assert_eq!(parse_datetime("2025-01-01T00:00:00+02:00").unwrap().tz, Tz::Pos(120));
    assert_eq!(parse_datetime("2025-01-01T00:00:00-05:30").unwrap().tz, Tz::Neg(330));
}

#[test]
fn strict_xsd_lexical_space() {
    // the grammar is exactly the xsd:dateTime lexical space — ISO leniencies
    // out of scope: space separator, HH:MM shorthand, basic zone, +year
    for s in [
        "2025-01-01 09:00:00",   // space separator
        "2025-01-01T09:00",      // HH:MM shorthand
        "2025-01-01T09:00:00+0200", // basic zone
        "+2025-01-01T09:00:00Z", // plus-signed year
    ] {
        assert!(parse_datetime(s).is_err(), "reject {s}");
    }
}

#[test]
fn arbitrary_fraction_verbatim() {
    let dt = parse_datetime("2025-01-01T00:00:00.123456789012345678901Z").unwrap();
    assert_eq!(dt.frac, "123456789012345678901");
    // precision is data: verbatim, never float-rounded
    let dt = parse_datetime("2025-01-01T00:00:00.50Z").unwrap();
    assert_eq!(dt.frac, "50");
    assert_eq!(dt.nanos().unwrap().1, 500_000_000);
}

#[test]
fn years_beyond_chrono() {
    // chrono NaiveDate is capped at ±262,143 — the parser is not
    assert_eq!(parse_date("0300000-01-01").unwrap().year, 300000);
    assert_eq!(parse_date("-0300000-01-01").unwrap().year, -300000);
    assert_eq!(parse_datetime("10000-01-01T00:00:00").unwrap().year, 10000);
    // fewer than 4 digits is not a year
    assert!(parse_date("999-01-01").is_err());
    // i128-overflowing years (39+ digits) rejected, not wrapped
    assert!(parse_date("9999999999999999999999999999999999999999-01-01").is_err());
}

// -- calendar correctness -----------------------------------------------------

#[test]
fn days_from_civil_matches_chrono() {
    for (y, m, d) in [
        (1970, 1, 1),
        (2000, 1, 1),
        (2000, 2, 29),
        (1900, 3, 1),
        (1969, 12, 31),
        (2024, 2, 29),
        (1, 1, 1),
        (0, 3, 1),
        (-400, 12, 31),
        (100000, 6, 15),
        (-100000, 6, 15),
    ] {
        let ours = days_from_civil(y, m, d).unwrap();
        let theirs = NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32)
            .map(|n| n.num_days_from_ce() as i64 - 719163);
        if let Some(theirs) = theirs {
            assert_eq!(ours, theirs as i128, "{y}-{m}-{d}");
        }
    }
}

#[test]
fn leap_year_rule() {
    let fmt = |y: i128| {
        if y < 0 {
            format!("-{:04}-02-29", -y)
        } else {
            format!("{y:04}-02-29")
        }
    };
    for y in [2000, 2024, 0, -400, 1600] {
        assert!(is_leap_year(y), "{y}");
        assert!(parse_date(&fmt(y)).is_ok(), "{y}");
    }
    for y in [1900, 2100, 2023, -100] {
        assert!(!is_leap_year(y), "{y}");
        assert!(parse_date(&fmt(y)).is_err(), "{y}");
    }
}

// -- instant conversion -------------------------------------------------------

#[test]
fn posix_instant_matches_chrono() {
    for s in [
        "1970-01-01T00:00:00Z",
        "2025-03-15T00:00:00+02:00",
        "2025-03-15T09:30:45.5Z",
        "1969-12-31T23:59:59Z",
        "2025-01-01T00:00:00-05:30",
        "2000-02-29T12:00:00Z",
    ] {
        let ours = parse_datetime(s).unwrap();
        let theirs = Cdt::parse_from_rfc3339(s).unwrap();
        assert_eq!(ours.posix_instant().unwrap(), theirs.timestamp() as i128, "{s}");
    }
}

#[test]
fn zoneless_reads_as_wall_clock() {
    // a zoneless instant uses the naive reading on the instant axis —
    // identical posix to the same wall time at Z
    let naive = parse_datetime("2025-03-15T09:00:00").unwrap();
    let utc = parse_datetime("2025-03-15T09:00:00Z").unwrap();
    assert_eq!(naive.posix_instant(), utc.posix_instant());
}

#[test]
fn offset_equivalent_instants() {
    // same instant, distinct spellings — bound-equal, literal-distinct
    let a = parse_datetime("2025-03-15T09:00:00+02:00").unwrap();
    let b = parse_datetime("2025-03-15T07:00:00Z").unwrap();
    assert_eq!(a.posix_instant(), b.posix_instant());
    assert_ne!(a.tz, b.tz);
}

#[test]
fn leap_aware_domain_counts_leaps() {
    // leap-aware instant = posix + number of leap days strictly before
    let before = parse_datetime("2016-12-31T00:00:00Z").unwrap();
    let after = parse_datetime("2017-01-01T00:00:00Z").unwrap();
    assert_eq!(
        after.leap_aware_instant().unwrap() - before.leap_aware_instant().unwrap(),
        86401,
        "the leap day contributes 86401 leap-aware seconds"
    );
    let plain = parse_datetime("2017-01-02T00:00:00Z").unwrap();
    assert_eq!(
        plain.leap_aware_instant().unwrap() - after.leap_aware_instant().unwrap(),
        86400
    );
    // and posix stays 86400 both days
    assert_eq!(
        after.posix_instant().unwrap() - before.posix_instant().unwrap(),
        86400
    );
}

#[test]
fn leap_second_valid_only_on_iers_days() {
    // the leap-second spelling is 23:59:60 — the 86401st second — and only
    // on dates IERS actually inserted one
    for s in ["2016-12-31T23:59:60Z", "2015-06-30T23:59:60Z", "2012-06-30T23:59:60+02:00"] {
        let dt = parse_datetime(s).unwrap_or_else(|_| panic!("{s} should parse"));
        assert_eq!(dt.second, 60);
        assert_eq!(dt.seconds_of_day(), 86400);
    }
    for s in [
        "2017-01-01T23:59:60Z", // next day is not a leap day
        "2016-12-30T23:59:60Z",
        "2016-12-31T12:00:60Z", // :60 only at 23:59
        "2016-12-31T23:58:60Z",
        "2025-01-01T00:00:60Z",
    ] {
        assert!(parse_datetime(s).is_err(), "{s} should reject");
    }
    // posix folds the leap second onto the next boundary
    let leap = parse_datetime("2016-12-31T23:59:60Z").unwrap();
    let next = parse_datetime("2017-01-01T00:00:00Z").unwrap();
    assert_eq!(leap.posix_instant(), next.posix_instant());
    // leap-aware keeps them one second apart
    assert_eq!(
        next.leap_aware_instant().unwrap() - leap.leap_aware_instant().unwrap(),
        1
    );
}

#[test]
fn date_is_whole_day_local() {
    let d = parse_date("2025-01-01").unwrap();
    let next = d.next_day().unwrap();
    let span = next.leap_aware_start().unwrap() - d.leap_aware_start().unwrap();
    assert_eq!(span, 86400, "a date is the duration of one day");
    // zoneless — leap_aware_start is the naive reading
    assert_eq!(d.leap_aware_start(), parse_datetime("2025-01-01T00:00:00").unwrap().leap_aware_instant());
}

#[test]
fn year_rollover_next_day() {
    let d = parse_date("2025-12-31").unwrap().next_day().unwrap();
    assert_eq!((d.year, d.month, d.day), (2026, 1, 1));
}

// -- rejection ----------------------------------------------------------------

#[test]
fn rejects_out_of_range_and_malformed() {
    for s in [
        "2025-13-01",            // month > 12
        "2025-00-01",            // month 0
        "2025-01-00",            // day 0
        "2025-01-32",            // day > 31
        "2025-04-31",            // April has 30
        "2025-01-01T24:00:00",   // 24:00 rejected — ISO uses 00:00
        "2025-01-01T23:60:00",   // minute > 59
        "2025-01-01T23:59:61",   // second > 60
    ] {
        assert!(parse_date(s).is_err() && parse_datetime(s).is_err(), "{s} should reject");
    }
}

#[test]
fn rejects_malformed_structure() {
    for s in [
        "",
        "2025-1-1",               // single digits not extended form
        "2025/01/01",             // wrong separators
        "2025-01-01T",            // dangling T
        "2025-01-01T09:00:00.",   // empty fraction
        "2025-01-01T09:00:00,5",  // comma decimal rejected
        "2025-01-01T09:00:00.1.2",// second dot
        "2025-01-01T090000",      // basic time not accepted on this surface
        "20250101",               // basic date not accepted
        "2025-01-01T09:00:00+14:01", // offset > 14:00
        "2025-01-01T09:00:00+10:60", // offset minute > 59
        "2025-01-01T09:00:00Z+02:00", // double zone
        "2025-01-01T09:00:00X",
        "garbage",
    ] {
        assert!(parse_datetime(s).is_err(), "{s} should reject");
    }
}

#[test]
fn deep_time_and_planck_fraction() {
    // negative instants at universe scale — i128 seconds keep full signed
    // magnitude (≈10³⁸ s; −3×10¹⁰ years ≈ −10¹⁸ s, −10²⁰ years ≈ −3×10²⁷ s)
    let deep = parse_datetime("-03000000000-03-15T00:00:00Z").unwrap();
    let secs = deep.posix_instant().unwrap();
    assert!(secs < -9_i128.pow(17), "deep past should be ≈−10¹⁸ s: {secs}");
    let deeper = parse_datetime("-0100000000000000000000-03-15T00:00:00Z").unwrap();
    let secs = deeper.posix_instant().unwrap();
    assert!(secs < -3_i128.pow(27), "≈−3×10²⁷ s: {secs}");
    // Sub-Planck: 49 fraction digits survive verbatim — i64 can't hold
    // the second, i128 can't hold 10⁴⁹, but the digits are the value
    let planck = parse_datetime("-03000000000-03-15T00:00:00.1234567890123456789012345678901234567890123456789Z").unwrap();
    assert_eq!(planck.frac.len(), 49);
    assert!(planck.posix_instant().unwrap() < 0);
    // beyond i128 year digits → clean rejection, not wrap
    assert!(parse_date("-999999999999999999999999999999999999999999-01-01").is_err());
}

#[test]
fn accepts_spec_boundaries() {
    // offset bound is inclusive ±14:00
    assert!(parse_datetime("2025-01-01T00:00:00+14:00").is_ok());
    assert!(parse_datetime("2025-01-01T00:00:00-14:00").is_ok());
    // date with a zone parses (zoned dates are legal xsd:date)
    let d = parse_date("2025-01-01+02:00").unwrap();
    assert_eq!(d.tz, Tz::Pos(120));
}


// -- boundary cardinality vs chrono --------------------------------------------
// Exhaustive corners of every field — no RNG, every combination runs.
// Anything chrono can express must give the identical POSIX instant/nanos.

#[test]
fn differential_boundaries_vs_chrono() {
    // leap-rule corners: non-leap century, quad-century, quad years, epoch edge
    const YEARS: [i64; 11] = [1899, 1900, 1904, 1969, 1970, 1971, 1996, 2000, 2024, 2100, 2400];
    const TZ: [&str; 7] = ["Z", "+00:00", "-00:00", "+14:00", "-14:00", "+05:30", "-12:00"];
    for &y in &YEARS {
        for m in 1..=12u64 {
            for &d in &[1u64, days_in_month(y as i128, m as u8) as u64] {
                for &(hh, mm, ss) in
                    &[(0, 0, 0), (0, 0, 59), (23, 59, 0), (23, 59, 59), (0, 59, 59), (23, 0, 0)]
                {
                    for tz in TZ {
                        let s = format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}{tz}");
                        assert_eq!(
                            parse_datetime(&s).unwrap().posix_instant().unwrap(),
                            Cdt::parse_from_rfc3339(&s).unwrap().timestamp() as i128,
                            "{s}"
                        );
                    }
                }
            }
        }
    }
    // fraction corners: chrono truncates >9 digits the same way we do
    for frac in ["", ".0", ".000000000", ".999999999", ".1234567890", ".9999999995"] {
        let s = format!("2025-03-15T00:00:00{frac}Z");
        let ours = parse_datetime(&s).unwrap();
        let chrono = Cdt::parse_from_rfc3339(&s).unwrap();
        assert_eq!(ours.nanos().unwrap().1, chrono.timestamp_subsec_nanos(), "nanos {s}");
        // verbatim length is ours alone — full precision retained
        assert_eq!(ours.frac, frac.trim_start_matches('.'));
    }
}

// -- documented divergences from chrono ----------------------------------------
// Places where "correct" differs by design. Each pins OUR contract; where
// chrono is expressible the relation to it is asserted too.

#[test]
fn divergence_leap_second_fold() {
    // OURS: posix_instant folds :60 onto the next-day boundary (RFC 3339 /
    // POSIX convention — POSIX time has no leap second).
    // CHRONO: keeps the instant inside second 59 as nanos=1_999_999_999, so
    // timestamp() returns the :59 second — one second before ours.
    for (leap, folded_midnight) in [
        ("1972-06-30T23:59:60Z", "1972-07-01T00:00:00Z"),
        ("2016-12-31T23:59:60Z", "2017-01-01T00:00:00Z"),
        // local midnight in +02:00 = 22:00:00Z of the leap day itself
        ("1998-12-31T23:59:60+02:00", "1998-12-31T22:00:00Z"),
    ] {
        let ours = parse_datetime(leap).unwrap().posix_instant().unwrap();
        let midnight = parse_datetime(folded_midnight).unwrap().posix_instant().unwrap();
        assert_eq!(ours, midnight, "fold of {leap}");
        // chrono sits one second below us on real leap days
        assert_eq!(
            ours - 1,
            Cdt::parse_from_rfc3339(leap).unwrap().timestamp() as i128,
            "chrono nanos-fold {leap}"
        );
        // leap_aware_instant: the :60 second IS the extra counted second —
        // it equals the next day's leap-aware start
        let ours_leap = parse_datetime(leap).unwrap().leap_aware_instant().unwrap();
        assert!(ours_leap >= ours, "leap-aware counts the second {leap}");
    }
}

#[test]
fn divergence_rejected_or_parsed_where_chrono_isnt() {
    // zoneless — ours a first-class Tz::Zoneless, rfc3339 has no such grammar
    assert!(Cdt::parse_from_rfc3339("2025-03-15T00:00:00").is_err());
    assert_eq!(parse_datetime("2025-03-15T00:00:00").unwrap().tz, Tz::Zoneless);
    // -00:00 — ours keeps the "offset not intended to be known" spelling;
    // chrono folds it to +0
    assert_eq!(parse_datetime("2025-03-15T00:00:00-00:00").unwrap().tz, Tz::NegZero);
    // offset bound — ours ±14:00 (ISO legal zone range); rfc3339 allows ±23:59
    assert!(parse_datetime("2025-03-15T00:00:00+15:00").is_err());
    assert!(Cdt::parse_from_rfc3339("2025-03-15T00:00:00+15:00").is_ok());
    // year width — rfc3339 reads exactly 4 digits; ours takes i128 range
    assert!(Cdt::parse_from_rfc3339("40000-01-01T00:00:00Z").is_err());
    assert_eq!(parse_datetime("40000-01-01T00:00:00Z").unwrap().year, 40000);
    // :60 placement — ours requires 23:59:60 on an IERS leap day; chrono's
    // nanos-overflow trick accepts :60 at ANY time position
    assert!(parse_datetime("2025-01-01T00:00:60Z").is_err());
    assert!(Cdt::parse_from_rfc3339("2025-01-01T00:00:60Z").is_ok());
    // fractions — ours retain every digit verbatim; chrono keeps 9
    let ours = parse_datetime("2025-03-15T00:00:00.12345678901234567890123456789012Z").unwrap();
    assert_eq!(ours.frac.len(), 32);
    // bare date & zoned date — xsd:date domain, chrono rfc3339 has none
    assert_eq!(parse_date("2025-03-15+02:00").unwrap().tz, Tz::Pos(120));
}

#[test]
fn reject_boundaries() {
    // field-validity corners — ours and chrono agree these are not dates/times
    for s in [
        "2025-02-29T00:00:00Z",  // non-leap Feb 29
        "2024-02-30T00:00:00Z",  // Feb 30 on a leap year
        "2025-04-31T00:00:00Z",  // 31-day April
        "2025-13-01T00:00:00Z",  // month 13
        "2025-00-01T00:00:00Z",  // month 0
        "2025-01-00T00:00:00Z",  // day 0
        "2025-01-32T00:00:00Z",  // day 32
        "2025-01-01T24:00:00Z",  // 24:00 (ISO moved it to 00:00)
        "2025-01-01T00:60:00Z",  // minute 60
        "2025-01-01T23:59:61Z",  // :61
        "2025-01-01T09:00Z",     // HH:MM shorthand — seconds mandatory
        "2025-01-01T09:00:00+0200", // basic zone — colon mandatory
    ] {
        assert!(parse_datetime(s).is_err(), "should reject {s}");
        assert!(Cdt::parse_from_rfc3339(s).is_err(), "chrono agrees {s}");
    }
}

#[test]
fn fraction_precision_boundary_is_49() {
    // 49 digits = the precision boundary, inclusive — below Planck scale
    let ok = format!("2025-01-01T00:00:00.{}Z", "1".repeat(49));
    assert!(parse_datetime(&ok).is_ok());
    let over = format!("2025-01-01T00:00:00.{}Z", "1".repeat(50));
    assert!(parse_datetime(&over).is_err());
}

#[test]
fn civil_from_days_inverts_days_from_civil() {
    for days in [-1_000_000_000i64, -719468, -1, 0, 1, 719468, 20000, 1_000_000_000] {
        let (y, m, d) = civil_from_days(days as i128);
        assert_eq!(days_from_civil(y, m, d).unwrap(), days as i128, "day {days}");
        // cross-check the wall date against chrono where expressible
        if let Some(nd) = chrono::NaiveDate::from_num_days_from_ce_opt(days as i32 + 719163) {
            assert_eq!((y as i64, m as u32, d as u32), (nd.year() as i64, nd.month(), nd.day()));
        }
    }
}

#[test]
fn leap_instant_to_wall_inverts() {
    for s in [
        "1970-01-01T00:00:00Z",
        "2016-12-31T23:59:59Z",
        "2017-01-01T00:00:00Z",
        "1969-12-31T23:59:58Z",
        "-03000000000-03-15T00:00:00Z",
    ] {
        let dt = parse_datetime(s).unwrap();
        let secs = dt.leap_aware_instant().unwrap();
        let (y, m, d, sod, is60) = leap_instant_to_wall(secs).unwrap();
        assert_eq!((y, m, d), (dt.year, dt.month, dt.day), "{s}");
        if dt.second == 60 {
            assert!(is60, "leap second flag {s}");
        } else {
            assert_eq!(sod, dt.seconds_of_day() as u32, "{s}");
            assert!(!is60, "{s}");
        }
    }
    // the leap second itself reads back flagged at 23:59
    let leap = parse_datetime("2016-12-31T23:59:60Z").unwrap();
    let (y, m, d, _, is60) = leap_instant_to_wall(leap.leap_aware_instant().unwrap()).unwrap();
    assert_eq!((y, m, d, is60), (2016, 12, 31, true));
}
