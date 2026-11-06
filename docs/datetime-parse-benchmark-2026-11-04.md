# Datetime parse benchmark — 2026-11-04

`tdb-datetime` fixed-format parser vs `chrono` — 100,000 iterations, ns/op, release build.

```text
input                                              tdb    chrono
2025-03-15T00:00:00+02:00                           32        14   (parse_from_rfc3339)
2025-03-15T00:00:00Z                                27        12   (parse_from_rfc3339)
2025-03-15T00:00:00.123456789Z                      27        16   (parse_from_rfc3339)
2025-03-15T00:00:00.123456789+05:30                 32        19   (parse_from_rfc3339)
2025-03-15                                          19        76   (NaiveDate %Y-%m-%d)
2025-03-15T00:00:00.123456                          20       137   (NaiveDateTime %.f)

beyond chrono (tdb only):
2025-03-15T00:00:00.123456789012345678Z             29        n/a  (18 frac digits)
0300000-03-15T00:00:00Z                             29        n/a  (year 300000)
2016-12-31T23:59:60Z                                27        n/a  (leap second)

instant domain: i128 vs rug Integer (same math):
2025-03-15T00:00:00.5+02:00                         23        89
2025-03-15T00:00:00Z                                24        73

deep time (i64 fast path — ±2.9·10¹¹ years rides it):
-03000000000-03-15T00:00:00Z                        32
-03000000000-03-15T00:00:00.123…34 (44 digits)       44
```

## Reading it

- **Dual-width arithmetic.** Year accumulation, `days_from_civil`, and the
  instant chain all run in `i64` and spill to `i128` only when the literal
  exceeds i64 range. The i64 seconds domain covers ±~2.9·10¹¹ years, so
  every ordinary timestamp — and most "deep time" — pays chrono-width math.
- **Strict grammar, faster parse.** The grammar is exactly the
  `xsd:dateTime` lexical space — `T` mandatory, `HH:MM:SS` mandatory,
  `Z`/`±HH:MM` only, no `+` year sign, no basic formats. Removing the
  leniency branches shaved ~5–8 ns off every form.
- vs `NaiveDate(Time)::parse_from_str` — the format-item path `parse_finite`
  calls today — the parser is **4–6.9× faster** (19–20 vs 76–137 ns).
- vs `DateTime::parse_from_rfc3339` — chrono's hand-tuned fast path — the
  parser runs ~1.7–2.3× slower (27–32 vs 12–19 ns) while carrying strictly
  more: `i128` years (±10³⁶), verbatim arbitrary-precision fractions, all
  six timezone classes, and IERS-leap-day `23:59:60`.
- **i128 beats arbitrary-precision** for the instant domain: identical math
  is 23 vs 89 ns (rug allocates and calls GMP per op). i64/i128 dual width
  on the parse path; bigint conversion only at the storage-encode boundary
  where span beyond ~10³⁸ s would ever need it.
- Deep-time parse of a 44-digit-fraction, 11-digit-negative-year literal
  stays in the same ~45 ns envelope — no bigints touched anywhere.

Resolution floor this clears: nanosecond-only fractions, ±262,143-year span,
no leap seconds, format-string dispatch — chrono cannot express the values
in the second table at all.
