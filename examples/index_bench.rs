//! Interval end-index benchmark at corpus scale.
//!
//! Corpus: 1,000,000 reporting-period-style intervals `2020-01-01/P{i}D`
//! (unique daily ends spanning ~2,738 years), plus a variant whose ends
//! exceed the `(i64, u32)` rank domain (sub-nanosecond fractions, deep
//! years) to exercise the wide index.
//!
//! Per check class (`le`, `ge`, `eq`, `range`) we measure:
//!   - brute force: decode every end key once, compare per item — the
//!     pre-index check model, 1M comparisons per check;
//!   - index: `positions_end_*` wavelet probes — the Allen-relation check
//!     path used by the engine.
//!
//! Both are then costed for 1,000 checks.

use bytes::{Bytes, BytesMut};
use std::hint::black_box;
use std::time::Instant;
use tdb_succinct::tfc::datatypes::Datatype;
use tdb_succinct::tfc::interval::IntervalEndIndex;
use tdb_succinct::tfc::lexical_interval::{
    cmp_end_key, decode_lexical_entry, encode_lexical_interval, end_key,
    parse_lexical_interval, posix_to_leap_aware_secs, probe_key, EndKey,
};
use tdb_succinct::tfc::typed::{TypedDict, TypedDictBufBuilder, TypedDictEntry};
use tdb_succinct::SizedDictEntry;

const N: usize = 1_000_000;
const CHECKS: usize = 1_000;
/// 2020-01-01T00:00:00Z as POSIX seconds.
const EPOCH: i64 = 1_577_836_800;

fn build_dict(entries: Vec<TypedDictEntry>) -> (TypedDict, std::time::Duration) {
    let t = Instant::now();
    let mut entries = entries;
    entries.sort();
    let mut b = TypedDictBufBuilder::new(
        BytesMut::new(),
        BytesMut::new(),
        BytesMut::new(),
        BytesMut::new(),
    );
    b.add_all(entries.into_iter());
    let (a, c, d, e) = b.finalize();
    (
        TypedDict::from_parts(a.freeze(), c.freeze(), d.freeze(), e.freeze()),
        t.elapsed(),
    )
}

fn iv_entry(s: &str) -> TypedDictEntry {
    let iv = parse_lexical_interval(s).unwrap();
    TypedDictEntry::new(
        Datatype::DateTimeInterval,
        SizedDictEntry::from(Bytes::from(encode_lexical_interval(&iv))),
    )
}

/// End keys in dictionary (segment) order.
fn end_keys(dict: &TypedDict) -> Vec<EndKey> {
    dict.lexical_interval_segment()
        .unwrap_or(0..0)
        .map(|id| {
            let entry = dict.entry(id as usize).expect("interval id in segment");
            let iv = decode_lexical_entry(&entry.to_bytes()).unwrap();
            end_key(&iv.end)
        })
        .collect()
}

/// Probe seconds for a day boundary `d` days after the epoch.
fn probe_secs(d: i64) -> i64 {
    posix_to_leap_aware_secs(EPOCH + d * 86_400)
}

fn bench(
    name: &str,
    _dict: &TypedDict,
    keys: &[EndKey],
    idx: &IntervalEndIndex,
    seg: std::ops::Range<u64>,
) {
    // 1000 probe instants spread uniformly across the corpus span.
    let probes: Vec<i64> = (1..=CHECKS)
        .map(|i| probe_secs((i as i64 * N as i64) / CHECKS as i64))
        .collect();
    println!("--- {} ({} ends) ---", name, seg.end - seg.start);

    // Brute-force baseline: per-item key compare, one full pass per check.
    let classes: &[(&str, Box<dyn Fn(&EndKey, &EndKey) -> bool>)] = &[
        (
            "le",
            Box::new(|k, p| {
                matches!(k, EndKey::Finite { .. }) && cmp_end_key(k, p) != std::cmp::Ordering::Greater
            }),
        ),
        (
            "ge",
            Box::new(|k, p| cmp_end_key(k, p) != std::cmp::Ordering::Less),
        ),
        (
            "eq",
            Box::new(|k, p| cmp_end_key(k, p) == std::cmp::Ordering::Equal),
        ),
    ];
    for (label, f) in classes {
        let t = Instant::now();
        let mut hits = 0u64;
        for secs in probes.iter().take(3) {
            let p = probe_key(*secs, 0);
            hits += keys.iter().filter(|k| f(k, &p)).count() as u64;
        }
        let per_check = t.elapsed() / 3;
        println!(
            "  brute {:>2}: {:>7.2} ms/check, ~{} hits — x{} checks = {:.1} s",
            label,
            per_check.as_secs_f64() * 1e3,
            hits / 3,
            CHECKS,
            per_check.as_secs_f64() * CHECKS as f64
        );
    }

    // Indexed: probes, up to 1000 checks per class — capped at 5s, the
    // ×1000 figure extrapolated from the measured rate.
    let fns: [(&str, fn(&IntervalEndIndex, std::ops::Range<u64>, i64, u32) -> Vec<u64>); 3] = [
        ("le", IntervalEndIndex::positions_end_le),
        ("ge", IntervalEndIndex::positions_end_ge),
        ("eq", IntervalEndIndex::positions_end_eq),
    ];
    for (label, f) in fns {
        let start = Instant::now();
        let (mut done, mut hits) = (0u64, 0u64);
        while done < CHECKS as u64 && start.elapsed().as_secs() < 5 {
            let secs = probes[black_box(done as usize % probes.len())];
            hits += f(idx, seg.clone(), secs, 0).len() as u64;
            done += 1;
        }
        let per = start.elapsed().as_secs_f64() / done as f64;
        println!(
            "  index {:>2}: {:>7.2} µs/check, ~{} hits — x{} checks = {:.1} ms ({} measured)",
            label,
            per * 1e6,
            hits / done,
            CHECKS,
            per * CHECKS as f64 * 1e3,
            done
        );
    }

    // Range checks: ends inside a 90-day window (a fiscal quarter).
    let start = Instant::now();
    let (mut done, mut hits) = (0u64, 0u64);
    while done < CHECKS as u64 && start.elapsed().as_secs() < 5 {
        let d = (done * 90) as i64;
        let r = idx.positions_end_range(
            seg.clone(),
            (probe_secs(d), 0),
            (probe_secs(d + 90), 0),
        );
        hits += r.len() as u64;
        done += 1;
    }
    let per = start.elapsed().as_secs_f64() / done as f64;
    println!(
        "  index range-90d: {:>7.2} µs/check, ~{} hits — x{} checks = {:.1} ms ({} measured)",
        per * 1e6,
        hits / done,
        CHECKS,
        per * CHECKS as f64 * 1e3,
        done
    );

    // Brute range for scale: single pass.
    let t = Instant::now();
    let lo = probe_key(probe_secs(10), 0);
    let hi = probe_key(probe_secs(100), 0);
    let hits: usize = keys
        .iter()
        .filter(|k| {
            matches!(k, EndKey::Finite { .. })
                && cmp_end_key(k, &lo) != std::cmp::Ordering::Less
                && cmp_end_key(k, &hi) != std::cmp::Ordering::Greater
        })
        .count();
    let per_check = t.elapsed();
    println!(
        "  brute range-90d: {:>7.2} ms/check, ~{} hits — x{} checks = {:.1} s",
        per_check.as_secs_f64() * 1e3,
        hits,
        CHECKS,
        per_check.as_secs_f64() * CHECKS as f64
    );
}

fn main() {
    // Corpus A: 1M duration-form intervals with unique daily ends —
    // 2020-01-01/P1D .. 2020-01-01/P1000000D (~year 4718), all rankable.
    let t = Instant::now();
    let entries: Vec<TypedDictEntry> = (1..=N)
        .map(|i| iv_entry(&format!("2020-01-01/P{}D", i)))
        .collect();
    let (dict, dict_t) = build_dict(entries);
    println!(
        "corpus A: encode {} in {:.2}s, dict build {:.2}s",
        N,
        t.elapsed().as_secs_f64() - dict_t.as_secs_f64(),
        dict_t.as_secs_f64()
    );

    let t = Instant::now();
    let keys = end_keys(&dict);
    println!("  decode+keys: {:.2}s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let idx = IntervalEndIndex::build_lexical(&dict).unwrap();
    println!("  index build: {:.3}s", t.elapsed().as_secs_f64());
    let seg = dict.lexical_interval_segment().unwrap();
    bench("ranked", &dict, &keys, &idx, seg);

    // Corpus B: same corpus plus a handful of unrepresentable ends —
    // one sub-nanos or deep-year end moves the whole index to the wide form.
    let mut entries: Vec<TypedDictEntry> = (1..=N)
        .map(|i| iv_entry(&format!("2020-01-01/P{}D", i)))
        .collect();
    entries.push(iv_entry("2025-01-01/2025-06-01T00:00:00.0000000000001Z"));
    entries.push(iv_entry("-15000000000-01-01/-13800000000-01-01"));
    entries.push(iv_entry("2025-01-01/2025-06-01T00:00:00.5Z"));
    let (dict, _) = build_dict(entries);
    let keys = end_keys(&dict);
    let t = Instant::now();
    let idx = IntervalEndIndex::build_lexical(&dict).unwrap();
    println!("wide index build: {:.3}s", t.elapsed().as_secs_f64());
    let seg = dict.lexical_interval_segment().unwrap();
    bench("wide", &dict, &keys, &idx, seg);
}
