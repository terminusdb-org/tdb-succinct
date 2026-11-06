//! Parse-cost comparison: `tdb_datetime` vs `chrono`.
//!
//! chrono's `DateTime::parse_from_rfc3339` is its fast path (fixed layout,
//! still goes through the Parsed builder); `NaiveDate(Time)::parse_from_str`
//! walks format items. The fixed-format parser avoids both — and carries
//! higher resolution (i64 years, arbitrary-precision fractions).
//!
//! Run: cargo run --release -p tdb-datetime --example datetime_bench

use std::hint::black_box;
use std::time::Instant;

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime};
use tdb_datetime::{parse_date, parse_datetime};

const ITERS: u32 = 100_000;

fn time<T, F: FnMut() -> T>(mut f: F) -> u128 {
    let t = Instant::now();
    for _ in 0..ITERS {
        black_box(f());
    }
    t.elapsed().as_nanos() / ITERS as u128
}

fn main() {
    println!("=== {ITERS} iterations, ns/op ===");
    println!("{:<44}{:>10}{:>10}", "input", "tdb", "chrono");
    for s in [
        "2025-03-15T00:00:00+02:00",
        "2025-03-15T00:00:00Z",
        "2025-03-15T00:00:00.123456789Z",
        "2025-03-15T00:00:00.123456789+05:30",
    ] {
        let tdb = time(|| parse_datetime(black_box(s)).unwrap().posix_instant());
        let chr = time(|| DateTime::parse_from_rfc3339(black_box(s)).unwrap().timestamp());
        println!("{s:<44}{tdb:>10}{chr:>10}");
    }

    // chrono's format-item path — what parse_finite uses for the date part
    let d = "2025-03-15";
    let tdb = time(|| parse_date(black_box(d)).unwrap().leap_aware_start());
    let chr = time(|| {
        NaiveDate::parse_from_str(black_box(d), "%Y-%m-%d")
            .unwrap()
            .num_days_from_ce()
    });
    println!("{d:<44}{tdb:>10}{chr:>10}");

    let dt = "2025-03-15T00:00:00.123456";
    let tdb = time(|| parse_datetime(black_box(dt)).unwrap().posix_instant());
    let chr = time(|| {
        NaiveDateTime::parse_from_str(black_box(dt), "%Y-%m-%dT%H:%M:%S%.f")
            .unwrap()
            .and_utc()
            .timestamp()
    });
    println!("{dt:<44}{tdb:>10}{chr:>10}");

    // beyond chrono: resolution and span it cannot represent at all
    println!("\n=== beyond chrono's range (tdb only) ===");
    for s in [
        "2025-03-15T00:00:00.123456789012345678Z",
        "0300000-03-15T00:00:00Z",
        "2016-12-31T23:59:60Z",
    ] {
        let tdb = time(|| parse_datetime(black_box(s)).unwrap().posix_instant());
        println!("{s:<44}{tdb:>10}{:>10}", "n/a");
    }

    // i128 vs arbitrary-precision: same instant math, two domains.
    // rug allocates + calls GMP per op; i128 is register arithmetic.
    println!("\n=== instant domain: i128 vs rug Integer ===");
    use rug::Integer;
    for s in ["2025-03-15T00:00:00.5+02:00", "2025-03-15T00:00:00Z"] {
        let dt = parse_datetime(s).unwrap();
        let i128_t = time(|| {
            parse_datetime(black_box(s)).unwrap().posix_instant()
        });
        let rug_t = time(|| {
            let d = parse_datetime(black_box(s)).unwrap();
            let days = Integer::from(tdb_datetime::days_from_civil(d.year, d.month, d.day).unwrap());
            (days * 86400 + Integer::from(d.seconds_of_day())
                - Integer::from(d.tz.offset_minutes() * 60)) as Integer
        });
        let _ = dt;
        println!("{s:<44}{i128_t:>10}{rug_t:>10}");
    }
    // deep-time + planck-scale fraction — i128 span, bigint-precision frac verbatim
    println!("\n=== deep time (i128 seconds) ===");
    for s in [
        "-03000000000-03-15T00:00:00Z",
        "-03000000000-03-15T00:00:00.12345678901234567890123456789012345678901234Z",
    ] {
        let tdb = time(|| parse_datetime(black_box(s)).unwrap().posix_instant());
        println!("{s:<64}{tdb:>10}");
    }
}
