# ISO 8601 interval parse/serialise benchmark and storage-format considerations

Status: measurements taken on the **payload-free lexical implementation** (`src/tfc/lexical_interval.rs`, release build, `examples/interval_bench.rs`). The entry is a single ordered field sequence `(start.class, start.instant, end.class, end.instant, period␣0x00, start.kind, end.kind, start.tshift, end.tshift, flag)` — there is no stored payload: decode reconstructs the supplied spelling from these fields. Instants live in a leap-aware domain (`day_posix + leaps_before(day) + sod` over the closed 27-entry IERS table), so `23:59:60` is a real instant and duration seconds count leap seconds.

## What is being measured

Three pipeline stages of the lexical implementation, per lexical form, at 10,000 and 100,000 iterations:

- **parse**: `parse_lexical_interval(str) -> LexicalInterval` — lexical to in-memory struct (canonicalisation, component parsing, instant materialisation, incommensurability check).
- **parse+encode**: parse plus `encode_lexical_interval` producing the dictionary entry — the document-insert path.
- **decode+render**: `decode_lexical_entry` plus `render_lexical_interval` — entry back to extended ISO lexical — the document-read path.

`entry B` is the encoded dictionary-entry width — the variable-width replacement for the fixed 52-byte legacy format.

## Results (ns/op, release build, Apple Silicon)

| Variant | parse | parse+encode | decode+render | entry B |
|---|---:|---:|---:|---:|
| `../..` (forever) | 135 | 200 | 230 | 4 |
| `/` (both unknown) | 76 | 140 | 111 | 4 |
| `dateTime/dateTime` with `Z` | 721 | 1604 | 1063 | 29 |
| `dateTime/dateTime` localtime (no zone) | 746 | 1616 | 1039 | 29 |
| `dateTime/dateTime` offset zones | 843 | 1813 | 1270 | 34 |
| `dateTime/period` | 587 | 1088 | 913 | 23 |
| `period/dateTime` | 569 | 1073 | 901 | 23 |
| `date/date` | 517 | 1269 | 769 | 23 |
| `date/period` | 481 | 983 | 681 | 23 |
| `period/date` | 469 | 942 | 660 | 23 |

Absolute wall time for 100k of the slowest full pipeline (`tz-offset` at ~1.8µs encode): **~181ms parse+encode, ~127ms decode+render**. For the common `Z`-zone form: **~160ms parse+encode, ~106ms decode+render** per 100k values.

## Entry composition — where the bytes go

There is no payload. Each finite component contributes `class(1) + decimal instant` to the instants block and `kind(1) + tshift` to the meta block; `period` is a `0x00`-terminated string; `flag` is one byte. Decode rebuilds the supplied spelling from these fields (`reconstruct_component_canonical` + extended-form separators):

| Variant | text B | instants | period | meta+flag | total |
|---|---:|---:|---:|---:|---:|
| `../..` | 5 | 2 | 1 | 1 | 4 |
| `/` | 1 | 2 | 1 | 1 | 4 |
| `dateTime/dateTime` Z | 41 | 14 | 10 | 5 | 29 |
| `dateTime/dateTime` localtime | 39 | 14 | 10 | 5 | 29 |
| `dateTime/dateTime` tz-offset | 51 | 14 | 11 | 9 | 34 |
| `dateTime/period` | 24 | 14 | 4 | 5 | 23 |
| `period/dateTime` | 24 | 14 | 4 | 5 | 23 |
| `date/date` | 21 | 14 | 4 | 5 | 23 |
| `date/period` | 14 | 14 | 4 | 5 | 23 |
| `period/date` | 14 | 14 | 4 | 5 | 23 |

- **Instants are ~14B for two finite components** — `class(1) + decimal int(~5) + frac marker(1)` each; sub-second fractions and extreme magnitudes widen only the digits involved.
- **Period is the swing field**: `T/T` forms carry the derived canonical period (~10B, e.g. `P1M28DT5H15M`) — it is what makes them cluster adjacently with `T/P`/`P/T` at the same instant pair; duration forms store their 3-char supplied duration + terminator (4B).
- **Meta+flag is 5B base** (`kind`+`tshift class` per component + `flag`); each `±HH:MM` zone adds 2B of signed offset.
- Special endpoints cost 4B total — two class bytes, empty period, flag.

## Interpretation

- **Entry width is the win, now unconditionally.** Every variant is smaller than the 52-byte format — 4–34B vs 52B — and smaller than supplied text on every shape except the lightest duration/date forms (23B vs 14B text: the ~20B of ordering/probe/reconstruction fields costs more than a bare `2025-01-01/P3M` is worth in raw text, and buys exactly what text cannot do).
- **The payload elimination is what put the format under text.** The stored instant is no longer duplicated — the supplied spelling is *reconstructed* from the ordering fields on decode, made lossless by the canonicalisation rules (`Z`/`-00:00`/`+00:00`/offsets ride the `tshift` axis, wall digits ride the instant, `23:59:60` is recovered from `sod == 86400` on leap days, inclusive `date` ends from kind+exclusive-bound minus one calendar day).
- **Parse is the expensive stage** (~0.5–0.85µs): canonicalisation, calendar work, fraction handling, leap validation and incommensurability checks all run at insert. Encode adds ~0.4–1.0µs (decimal instant + period derivation).
- **Decode+render rose to ~0.65–1.3µs** — reconstruction does a leap-aware day search plus wall/zone formatting; the old `unpack5` path was cheaper but cost ~40% of entry size. Still far below layer-read cost.
- **Special endpoints early-out**: `../..` and `/` skip all finite-component work — ~140–230ns end to end, 4-byte entries.
- **Bare dates have no zone spelling** in ISO 8601 — the "with Z, localtime, timezone" variants apply to `dateTime` endpoints only.
- **The legacy 52-byte row for comparison** (same bench, earlier runs): parse+encode ~0.5–0.9µs, decode+render ~0.1–1.1µs, 52B/entry. The new format trades ~2× parse cost (canonicalisation + arbitrary-precision keys) for full lexical fidelity, instant-major ordering, leap-second correctness and arbitrary precision — at 4–34B vs the fixed 52B.

## Ordering-domain cost: rational math vs decimal keys

The ordering domain is **rational seconds from the Unix epoch**: an instant is `secs + frac/10^len` where `secs` is an arbitrary-precision integer (universe-scale, beyond chrono's ~±262k-year calendar range) and `frac` an arbitrary-precision decimal fraction (planck scale is ~44 digits). Three representations of the same key were measured — materialisation (`make`), one comparison (`cmp`, adjacent values; `cmp=`, equal values), landing position via `binary_search` inside a sorted 100k-key set (`land`), and a full 100k-key sort.

- **rational** — `rug::Rational` (GMP `mpq`), canonicalised by GCD at construction; comparison cross-multiplies numerators/denominators.
- **deckey** — the codebase's existing `integer_and_fraction_to_storage` (tfc/decimal.rs): sortable bytes, memcmp order — exact rational ordering with no arithmetic.
- **lean** — today's `(i64 secs, u32 nanos)` tuple, shown only where representable; the fixed-width fast path for normal time.

| Tier | repr | make ns | cmp ns | cmp= ns | land@100k ns | sort 100k |
|---|---|---|---:|---:|---:|---:|---:|
| nanos (lean-representable) | rational | 336 | 13 | 13 | 231 | 1.2 ms |
| nanos | deckey | 235 | 2 | 3 | 89 | 0.3 ms |
| nanos | lean | 30 | 0 | 0 | 30 | 0.1 ms |
| frac30 | rational | 651 | 25 | 21 | 404 | 2.1 ms |
| frac30 | deckey | 312 | 1 | 1 | 56 | 0.2 ms |
| planck44 | rational | 722 | 25 | 27 | 427 | 3.0 ms |
| planck44 | deckey | 329 | 1 | 1 | 136 | 0.2 ms |
| universe-age (4.35e17 s) + frac44 | rational | 778 | 27 | 31 | 453 | 2.7 ms |
| universe-age + frac44 | deckey | 342 | 1 | 1 | 51 | 0.2 ms |
| deep-time 1e30 s + frac44 | rational | 779 | 28 | 29 | 417 | 2.3 ms |
| deep-time 1e30 + frac44 | deckey | 432 | 2 | 1 | 50 | 0.2 ms |
| pre-epoch negative + frac30 | rational | 663 | 20 | 18 | 368 | 2.0 ms |
| pre-epoch + frac30 | deckey | 307 | 1 | 1 | 49 | 0.2 ms |

### What the numbers say

- **Decimal keys are the ordering representation.** `deckey` compares in 1–2ns (memcmp early-outs on the first differing byte) vs 13–31ns for `mpq` cross-multiplication — and it is *already implemented* in the tree for `xsd:decimal`. A decimal fraction is a rational with denominator 10^k; comparing digit strings is exact rational comparison specialised to decimal denominators. `rug::Rational` remains the right tool for *computed* edges — duration arithmetic on fractions, midpoints, derived bounds — but ordering never needs it.
- **Precision and magnitude cost little once the format is right.** Going from nanos to a 74-significant-digit deep-time value moves `deckey` make from ~300ns to ~430ns and leaves cmp at ~1ns. "Crazy precision or distance from now is possible but expensive" — and the expense is a few hundred nanoseconds, not a redesign.
- **Normal time is lean.** The fixed-width `(i64, u32)` path makes/compares ~10× cheaper than `deckey` at nanos tier. A two-tier key — fixed-width for representable values, decimal-encoded for extremes — preserves today's speed for the common case; the decimal path is the escape hatch, not the toll booth.
- **Landing is never the cost.** `land` is 17 comparisons deep in a 100k set: ~50–140ns for deckey, ~230–450ns for rational. Dictionary placement, probes and index order are unaffected in shape — only the key bytes widen.
- **No fraction canonicalisation needed:** bytewise decimal keys order equal-instant fractions *adjacently* without stripping — `.5`, `.50`, `.500` are the same instant and sort together, in spelling order. Trailing zeros are supplied precision, kept verbatim in key and spelling; the earlier "strip at encode" requirement is retired.

### Very large years — deferred, format-ready

Ordering works in *seconds*, so the bench keys the extreme tiers directly. **Decision: beyond-chrono range is out of scope for now.** `chrono`/`NaiveDateTime` caps at roughly ±262,143 years; expanded-year forms (`Y+1000000-01-01`, ISO 8601-2) or deep-time dates cannot pass through a calendar library and will need bigint Gregorian arithmetic when implemented.

What matters is that the *key format needs no change to support it later*: `deckey` accepts arbitrary-magnitude seconds today, so values stored once the feature lands will order correctly with no re-keying, no migration and no format version bump. The future work is just a parser — proleptic-Gregorian arithmetic on bigints turning expanded-year lexicals into epoch seconds — not a calendar library and not a storage change.

## What the 52-byte format bought (legacy baseline)

The old entry led with a 24-byte order-preserving prefix `(start_secs, start_nanos, end_secs, end_nanos)`. What the storage design needed from it — and how the lexical format keeps each property:

- **Dictionary order = instant order.** Kept: the class+decimal instant key preserves the same probe pattern (`lexical_ids_start_*`), now additionally ordered by the mirrored-tail special classes and the timezone axis.
- **The end index is positional.** Kept via `IntervalEndIndex::build_lexical` — dense-rank end ordering over the same `(i64, u32)` rank domain, so the wavelet query machinery is unchanged.
- **Dedup.** Changed by decision: literal dedup after light canonicalisation (separators stripped). `07:00Z` vs `09:00+02:00` are now *distinct* entries sharing a sort-key prefix — required for XBRL lexical fidelity.
- **Fixed width.** Given up: entries are variable-width (4–34B measured). Id↔position arithmetic relies on the dictionary's block structure, which already handles variable-width strings.

The legacy format's real cost was lexical loss: `+00:00` rendered back as `Z`, offsets survived only as values, and `date`-only ends re-rendered as the exclusive `dateTime` bump — the reason this format exists.

## Endpoint-notation convention (writing rule — always applies)

In this document and every related spec, test and example:

- **dateTime endpoints denote half-open intervals** `[start, end)` — the end instant is exclusive. Write `2025-12-01T00:00:00Z/2026-01-01T00:00:00Z` for the interval covering December.
- **date endpoints denote inclusive semantics** — a date-only endpoint covers its whole day. `P1M/2025-12-31` reads as "the month whose last covered day is December 31"; the date end materialises to the exclusive bound `2026-01-01T00:00:00`.
- **Never mix the two within one interval.** `2025-12-01T00:00:00Z/2025-12-31` (dateTime start, date end) and `2025-12-01/2026-01-01T00:00:00Z` are exactly the class of mistakes this rule exists to prevent: the reader cannot tell whether the end is inclusive without checking which side carries the date spelling. A mixed example is a bug in the document, not a permitted shorthand.

Durations (`P1M`, `PT24H`) may appear on either side with either endpoint notation — the anchoring rules below apply whichever notation the anchored endpoint uses.

## Lexicographic ordering of stored entries

Entries sort as a tuple, byte fields in layout order:

```
( start.class, start.instant, end.class, end.instant, period␣0x00,
  start.kind, end.kind, start.tshift, end.tshift, flag )
```

Instant-major: the two instants order first; spelling metadata only breaks ties between identical instant pairs.

1. **Start class** — mirrored-tail ordering: `unbounded(0) < unspecified(1) < unknown(2) < finite(3)`. Special classes never mix inside one interval, so a finite start only ever faces `END_*` on the end side.
2. **Start instant** — the order-preserving decimal encoding: sign flag, arbitrary-precision ceiling-integer seconds, verbatim fraction digits, over the **leap-aware instant domain** (`day_posix + leaps_before(day) + sod`). Negative values use the ceiling convention (`-1.5` stores int `-1`, frac `5`); bytewise order is instant order at arbitrary precision, and `23:59:60` occupies its own real instant strictly between `23:59:59` and `00:00:00`.
3. **End class, end instant** — mirrored tail `finite(0) < unknown(1) < unspecified(2) < unbounded(3)`, same decimal encoding.
4. **Period** — the canonical duration bytes followed by a `0x00` terminator, so the field orders exactly like the period string a human reads. `T/T` intervals store the **derived** canonical period (coarsest-unit decomposition, `PT0S` for zero, `W` never emitted): `2026-01-01/2026-03-31` gets `P3M`, which is why it clusters next to `2026-01-01/P3M`. Supplied durations keep their own spelling — `P90D` ≠ `P3M` even at the same resolved span, because calendar-relative units are anchor-dependent.
5. **Start kind, end kind** — `date(0) < dateTime(1)`, finite components only.
6. **Start tshift, end tshift** — timezone axis `neg(0) < -00:00(1) < Z(2) < zone-less(3) < +00:00(4) < pos(5)`, nonzero offsets carrying signed i16 minutes so `-14:00 < -05:00`.
7. **Flag** — `T/T(0) < T/P(1) < P/T(2)`: among identical instant pairs and periods, explicit forms sort first, then `start/P`, then `P/end`. Final field and dedup key: entries are byte-identical iff every canonical distinction agrees — the supplied spelling is *deterministic in* the fields (it is reconstructed on decode), so no trailing text is needed to separate spellings.

Consequence: **every spelling of the same instant pair is adjacent** — the instant pair fixes the entry prefix through `end.instant`; spellings differ only in `period`/`kind`/`tshift`/`flag`. Instant-boundary probes never split a spelling group.

## The leap-aware instant domain

The instant is `day_posix + leaps_before(day) + sod`, where `sod` counts seconds of day *including* a leap second and `leaps_before` counts the day's entries in the closed 27-row IERS table (1972-06-30 … 2016-12-31 — the practice ends by 2035, so the table never grows). Each listed day has 86401 seconds; `23:59:60` is its 86400th — a real instant strictly between `23:59:59` and the next `00:00:00`, valid only on a listed day (the Feb-29 rule applied to times).

The map is order-preserving and never extrapolated backwards: pre-1972 instants sit at pure POSIX, matching every official fixpoint (POSIX, chrono, `java.time` all define history as 86400 s/day — the leap mechanism did not exist). Post-leap instants shift by a uniform +27. Duration arithmetic uses the split the units imply: `S` counts real elapsed seconds (`PT1S` from `23:59:60` lands on `00:00:00`; minute 23:59 of a leap day holds 61 real seconds), while `Y`/`M`/`D`/`H`/`Min` are wall units anchored in the component's own frame. An anchor *on* `23:59:60` carries its `+1 s` position through wall-unit shifts — `PT1M` lands on `00:01:00`, the normalised 61st-second position of the next minute.

## Derived period canonicalisation — coarsest spec wins

When an interval arrives as `dateTime/dateTime` or `date/date`, the resolved span is a period between two endpoints. That period is materialised explicitly in **canonical coarsest form**: among all equivalent period specifications, choose the one with the fewest numbers and the lowest multipliers.

The rule resolves to greedy decomposition into the largest units first, anchored at the start (calendar-relative units only have meaning against an anchor):

```
years → months → weeks → days → hours → minutes → seconds(+fraction)
```

| Resolved span | Canonical period | Rejected equivalents |
|---|---|---|
| `[2026-01-01, 2026-04-01)` | `P3M` | `P90D`, `PT2160H`, `PT129600M` |
| `[2025-01-01, 2026-04-01)` | `P1Y3M` | `P15M`, `P455D` |
| `[2026-01-01, 2026-03-03)` | `P2M2D` | `P61D` |
| `[2026-01-01, 2026-01-15)` | `P2W` | `P14D` (if weeks admitted — see Q below) |
| `[t, t)` | `PT0S` | — |

Why `P3M` always wins: multipliers 3 < 90 < 2160 — fewest and lowest. `P1Y3M` beats `P15M` (numbers 1+3=4 vs 15) and `P2M2D` beats `P61D` (2+1=3 vs 61): canonical decomposition minimises every multiplier at once because each unit is taken as large as it can be.

Rules:

- **Anchor at start.** `P1M` is 31 days from January 1 but 28 from February 1 — the canonical form is computed against the resolved interval's own start.
- **Exact decomposition only.** A span that is not a whole number of months keeps its remainder in the next-finer unit (`P2M2D`); a span that lands mid-second keeps fraction.
- **Explicit-form input keeps its supplied flag** — `s/e` stays `explicit`; the canonical period is a derived component, not a flag change. The supplied duration in `s/P` and `P/e` forms is stored verbatim (it *is* the supplied period).
- **Leap seconds count only in `S`.** `Y`/`M`/`D`/`H`/`Min` decompose on the wall clock — a wall hour spanning a leap second is still `PT1H` (the leap is absorbed like a leap day inside a year); the `S` field is the real elapsed-second residue. `[2016-12-31T23:00Z, 2017-01-01T01:00Z)` derives `PT2H`, which is `PT7201S` counted in seconds — both resolve to the same interval. An end *on* `23:59:60` can sit under a picked wall unit, leaving an honest `60S` residue (`[23:00, 23:59:60)` → `PT59M60S`) that cannot fold into a wall minute. Duration arithmetic follows the same split: `PT60S` from `23:59:00` lands on `23:59:60`, `PT61S` on `00:00:00`.

## The decided direction: ordered fields + spelling reconstruction

**Decided**: each entry *is* its ordering tuple — a single field sequence whose byte order is the spec's chronological order — with **no stored payload**. Every meaning-carrying distinction of the supplied spelling is a field (`tshift` axis carries `Z`/`-00:00`/`+00:00`/offsets, the instant carries wall digits at resolved position, `kind` carries date vs dateTime, `period`+`flag` carry the duration form), so decode reconstructs the canonical spelling deterministically. Ordering, probes, index and query behaviour match the 52-byte design's instant ordering, now leap-aware.

### Light canonicalisation — decided rules

Canonicalisation runs at *parse*, before fields are materialised: **date/time separators `-` and `:` are dropped from the stored form and restored at render** (rendering always emits extended ISO form). Fraction digits are kept **verbatim** — `.50` and `.5` are the same instant but different supplied precisions, so they are different literals. Everything meaning-carrying survives as a distinct field value — `Z`, `+00:00`, `-00:00`, offsets, fractions at supplied precision, `..`, empty endpoints, `X*`, duration spellings including `P2W`.

Two spellings differing only by separators (`2025-01-31` vs `20250131`) mean the same thing and share the stored entry; every other distinction — including fraction precision — stays a distinct stored value. Literal dedup.

### Entry layout

```
| start.class start.instant | end.class end.instant | period␣0x00 | kind tshift ×2 | flag |
   class + decimal secs+frac   same                    string order   finite only    T/T T/P P/T
```

- **Instant**: the decimal storage encoding (`integer_and_fraction_to_storage`) over the leap-aware second domain — succinct, order-preserving, unbounded magnitude and precision. Fixed-width fast path may front it later; the format is the escape hatch for extremes.
- **Period**: supplied canonical duration for `T/P`/`P/T`; derived canonical for `T/T` (so `2026-01-01/2026-03-31` and `2026-01-01/P3M` share the period bytes and cluster); empty otherwise.
- Ordering is computed on the materialised instants — `09:00+02:00` and `07:00Z` order by the same instant regardless of spelling, then by the timezone axis (`neg < -00:00 < Z < zone-less < +00:00 < pos`).
- Instant-equality queries still work **across spellings**: every spelling of the same instant pair shares the entry prefix.
- `date` endpoints are stored **inclusive under local-time management** (kind `date`, instant materialised to the exclusive-day bound); probes and index behaviour unchanged.
- Precision is future-proofed: fractions beyond nanoseconds keep full lexical precision and the instant field carries it (decimal-encoded); calendar-relative durations and expanded-year forms can land later without format change.
- Storage: variable-width entries, 4–34 bytes on every measured shape vs fixed 52 — smaller than supplied text for all but the lightest duration/date forms.

## Canonicalisation — the actual challenge

With literal storage, **the encode-time canonicalisation function is the equality rule**: two spellings collide in the dictionary iff they normalise to the same fields. The design decision is which equivalence classes collapse.

### Spelling-equivalence classes (same instant or same value, different text)

| Class | Examples | Same instant? |
|---|---|---|
| Zero-offset spellings | `Z` vs `+00:00` vs `-00:00` | yes |
| Offset-equivalent wall times | `09:00+02:00` vs `07:00Z` | yes |
| Trailing fractional zeros | `10:30:00.500` vs `10:30:00.5` | same instant, different precision — **no** |
| Date vs dateTime start | `2025-01-01` vs `2025-01-01T00:00:00` | yes |
| Date-only vs exclusive-end | end `2025-03-31` vs end `2025-04-01T00:00:00` | yes |
| Form equivalence | `2025-01-01/2025-04-01` vs `2025-01-01/P3M` | yes — but only because P3M anchored at Jan 1 happens to resolve identically; `start/P3M` is *not* generally equivalent to a fixed end |
| Duration equivalence | `PT24H` vs `P1D` | yes as durations; `P3M` vs `P90D` **no** — months are calendar-relative |
| Case variants | `t`/`z` lowercase | yes, if accepted at all |
| Local-vs-zoned | `10:30:00` vs `10:30:00Z` | **no** — a zone-less instant is not the UTC instant; these must never merge |

### Dedup model options

- **(a) Literal storage** — store each spelling as a distinct value; equality is exact-spelling. Maximum fidelity; every spelling variant is a distinct entry. Query behaviour unchanged (instant prefix groups them).
- **(b) Normalised storage** — apply a small canonicalisation at encode (zero-offset → `Z`, `t`/`z` → upper, date components → dateTime). Most spelling noise collapses; offset-equivalent wall times and form equivalences (`s/e` vs `s/P`) stay distinct.
- **(c) Instant+form equality** — same instant + same form + same kind = one value regardless of spelling. `09:00+02:00` vs `07:00Z` merge; `s/e` vs `s/P` stay distinct (different form). Cheapest dictionary, but the *kept* spelling is whichever arrived first — fidelity depends on insertion order, which is ugly.

For reference, the **current 52-byte format is effectively (c) with lexical-normalised tz fields** — `+00:00` and `Z` already merge today; `s/P3M` and `s/e` already stay distinct (flag + duration bytes differ).

### Decision: literal with light canonicalisation

**Decided (a)+**: literal dedup — every meaning-carrying spelling is a distinct stored value — combined with the light canonicalisation above (separator strip only). `Z`, `+00:00`, `-00:00` stay distinct because they carry distinct meaning for XBRL round-trips; fraction precision is meaning too — `.5` and `.50` are different literals that order adjacent. (c) stays rejected: merging same-instant spellings makes the kept spelling nondeterministic.

The invariant holds: **normalise, then compare** — never let the equality rule be "whatever bytes happened to arrive".

## Decisions on the format questions

1. **Dedup**: literal + light canonicalisation (separators stripped; meaning-carrying spellings all distinct).
2. **`date` endpoints**: stored verbatim and inclusive under local-time management; the key materialises timestamp bounds (day-bump for ends).
3. **Entry layout**: ordered field tuple — `class`, decimal instant, `period␣0x00`, `kind`, `tshift`, `flag` — with no stored payload; the supplied spelling is reconstructed from the fields on decode.
4. **Weeks**: `P2W` stored verbatim; canonical decomposition never emits `W` (ladder `Y M D H Min S` — `W` can't mix with other units in ISO anyway).
5. **Migration**: only 48-byte layers exist in the field — the 52-byte format never shipped. The compat story is 48-byte → lexical only: one legacy read path to keep, one less format to migrate.
6. **Key precision**: widened — arbitrary-precision decimal keys (~300–430ns make, ~1–2ns cmp) carry full supplied precision; a fixed-width fast path can keep lean-representable values at ~30ns.
7. **Leap seconds**: supported via the leap-aware instant domain (`day_posix + leaps_before(day) + sod` over the closed 27-entry IERS table). `23:59:60` is a real instant — `23:59:59 < 23:59:60 < 00:00:00` falls out of instant order alone — valid only on listed leap days; `S` units and instant arithmetic count it, wall units (`Y M D H Min`) absorb it. Pre-1972 instants sit at pure POSIX, matching every official fixpoint.
8. **Spellings**: all component forms stored verbatim as field values (`s/P`, `P/e`, `..`, empty, `X*`); the canonicalised spelling is reconstructed, never duplicated.

### Duration-end anchoring (settled semantics)

`P1M/2025-12-31` is *the month ending on December 31*: the date-only end covers the whole day, so the exclusive bound is `2026-01-01` and the duration subtracts from that bound → `[2025-12-01, 2026-01-01)` — December exactly. The same rule applied uniformly: `P1M/2026-01-01T00:00:00Z`, `2025-12-01T00:00:00Z/P1M`, `2025-12-01T00:00:00Z/2026-01-01T00:00:00Z` and `P1M/2025-12-31` all resolve to the same interval while remaining four distinct stored values (different `flag`/duration components). The Rust parser always anchored this way; the Prolog `duration_end` clauses subtracted from the unbumped end (`[Nov 30, Jan 1)` — a 32-day "month") and have been corrected to match. This is exactly the class of semantic fix that lexical storage makes safe: the supplied form is retained, only the derived key changes.

## 100k dataset: storage size and index vs scan

A generated set of 100,000 distinct common-case intervals (datetime starts spread minute-wise across the year, `Z` timezone, explicit `T/T` form) measured end to end:

- **parse+encode**: ~1,588 ns/op. **decode+render**: ~1,253 ns/op.

| Storage form | Bytes | B/entry |
|---|---:|---:|
| lexical entries (new format) | 2,900,000 | 29.0 |
| supplied text verbatim | 5,100,000 | 51.0 |
| 52-byte format | 5,200,000 | 52.0 |
| lexical entries, front-coded blocks | 2,682,892 | 26.8 |

Payload removal is what turned this dataset — previously the worst case at 56.0 B/entry — into a ~43% win over text and ~44% under the 52-byte baseline, on the *uniform* `T/T` shape that used to exceed both. Front-coding still shaves ~7.5% on top (`BLOCK_SIZE` 8; each entry's `period` bytes are largely shared between neighbours). Realistic mixed datasets skew smaller — `date/date`, duration forms and specials are 4–23B — and the derived period is what makes `T/T` cluster correctly with `T/P`/`P/T` at the same instant pair; it is the price of that ordering, not filler.

- **Dictionary start probe** (`start ≤ t`, binary segment + class-boundary search): ~1.1 µs, returns the id-range pair `{specials} ∪ {finite ≤ t}` — ~17µs of per-100k cost regardless of how many ids match. Callers pass POSIX epoch seconds; the probe maps into the leap-aware domain internally.
- **End index build** (`IntervalEndIndex::build_lexical` → dense-rank wavelet): ~152 ms for 100k entries — a one-off per-layer cost.

### Index vs decode-scan — selectivity is the whole story

| Query | Wavelet index | Naive decode scan | Verdict |
|---|---:|---:|---|
| `end ≤ May 1` (~83% of entries match) | ~120.5 ms | ~98.4 ms | index loses — result enumeration dominates |
| `end` in a 1-hour window (~60 matches) | **~62.5 µs** | ~98.3 ms | index wins ~1,570× |

Two findings, one fix:

- `WaveletTree::positions_in_range` originally discovered in-range values via a **full `decode()` pass** over the sequence — O(n) bound-search regardless of selectivity, which made the index strictly slower than scanning. Fixed with a width-based strategy: when the value window is narrower than the position window it probes each candidate value via `lookup` (O(width·log σ + k log σ)); broad windows keep the decode pass. This is the "find different starting times with an end date" case — now sub-100µs instead of scan-bound.
- Even with optimal bound-search, **the index pays off only on selective queries**: enumerating k results costs ~k·log σ vs the scan's flat ~980ns/entry decode+reconstruct. Rule of thumb from these numbers: use the index when expected selectivity is under roughly half the segment; prefer scan (or scan-in-parallel) for broad sweeps. Query planning should pick the path per-bound, not assume the index always wins.

## Reproducing

```
cargo build --release --example interval_bench
./target/release/examples/interval_bench
```
