# Lexical duration entry format

Status: proposed for TerminusDB 12.1. Replaces the 2022 fixed-width
`xsd:duration` encoding. Covers `xsd:duration`, `xsd:yearMonthDuration`
and `xsd:dayTimeDuration` — all three share one storage format; the
dictionary's datatype tag discriminates.

## Requirements

- Arbitrary-precision components. Day counts beyond `u8` (the 2022 format
  silently dropped `P300D`), year counts into the billions, and second
  fractions at arbitrary decimal precision.
- Byte order = value order. Dictionary entries are sorted; ordering must
  not require decode.
- Unambiguous dispatch against the legacy 21-byte format at any length.
- Canonical ISO 8601 spelling derived on decode, never stored.

## Layout

```
offset  field
0       sign     0x00 = negative | 0x01 = non-negative
1..     payload  vint(week) vint(year) vint(month) vint(day) vint(hour)
                 vint(minute) vint(second int) [fraction] frac_len
```

`week` carries the exclusive ISO 8601 `PnW` form; it is nonzero only
when every other component is zero, keeping `P2W` and `P14D` distinct
values (equal durations, different denominations).

`vint` is `bigint_to_storage` (`src/tfc/integer.rs`) applied to a
non-negative magnitude: embedded size, arbitrary precision,
order-preserving. `fraction` is the centary encoding from
`integer_and_fraction_to_storage` (`src/tfc/decimal.rs`); an absent
fraction stores the `0x00` "false zero". `frac_len` is a `u8` holding the
supplied fraction digit count, preserving precision the way interval
components do: `PT1.5S` and `PT1.50S` are the same length but distinct
literals.

For a negative duration every payload byte is bitwise-complemented. The
complement is order-reversing, so within the negative class a larger
magnitude sorts earlier — byte order stays numeric order throughout.

Zero is canonical `PT0S`: sign `0x01`, all magnitudes zero, no fraction,
`frac_len` 0. Encoders must not emit a negative sign on an all-zero
magnitude; readers encountering one treat the value as zero.

## Ordering

Byte order is a total order over (sign class, then the magnitude tuple
week, year, month, day, hour, minute, second-int, fraction, `frac_len`).
Negatives sort before non-negatives; within each sign class, order is
numeric.

This is a deterministic component-tuple order, not an elapsed-time order.
Calendar components have no universal physical length — `P1M` and `P30D`
have no defined relationship without a reference instant — so ordering is
by component, matching the XSD model where such comparisons are
indeterminate.

## Dispatch

The sign byte doubles as the format discriminator — no tag byte is
needed. The legacy first byte is the order-encoded `i8` sign
(`0x80 ^ sign`), so valid legacy entries can only start `0x7F`, `0x80` or
`0x81` — disjoint from this format's `{0x00, 0x01}`.

Dispatch is per-entry and exact — each entry determines its own decoder,
with no inference across the segment:

```
len == 21 && bytes[0] ∈ {0x7F, 0x80, 0x81}   → legacy decoder (Duration::from_lexical)
bytes[0] ∈ {0x00, 0x01}                       → lexical decoder (this format)
otherwise                                     → malformed: error
```

Dictionaries are written atomically, so in practice a type segment is
homogeneous — all entries share one encoding. That is a property of
correct writers, not something decode depends on: per-entry dispatch
decodes a mixed segment correctly regardless. Segment scans may
additionally assert homogeneity cheaply (first and last entries share a
byte-0 class) and treat a mismatch as corruption.

The rule is exact at every length, including a new-format entry of
exactly 21 bytes. Two consequences, both acceptable and loud: a legacy
entry with an out-of-contract sign byte is now malformed rather than
silently decoded; and a corrupt 21-byte entry whose sign byte happens to
be `0x00`/`0x01` reaches the lexical decoder, where the strict field
invariants (self-delimiting varints, `frac_len` consistency, no trailing
bytes) make a valid-looking misdecode vanishingly unlikely — and still an
error, not a silent value change, in the remaining cases.

Versioning: future format revisions take sign-byte values outside both
sets (`0x02`…), preserving the same dispatch shape.

## Legacy format (read-only)

Entries written before this format — 21 bytes:

```
i8  sign    order-encoded: 0x80 ^ sign
i64 year    order-encoded: 0x8000… ^ bits, big-endian
u8  month, day, hour, minute
f64 second  order-preserving IEEE transform
```

decode through `Duration::from_lexical`. Immutable layer files are never
rewritten in place: legacy entries are upgraded to the lexical encoding
only when a dictionary is rebuilt (delta/rollup merge), via an
`upgrade_lexical_durations` pass mirroring `upgrade_lexical_intervals`.

## Canonical spelling

`[-]P(nW | [nY][nM][nD][T[nH][nM][n[.f]S]])` — the ISO week form is
exclusive and never combines with other fields. Zero components omitted,
all-zero → `PT0S`, negative → `-P` prefix. Seconds render the canonical
fraction digits, re-padded to `frac_len` supplied precision. Produced by
`duration_parts_to_period`, shared with the interval marshal path so the
period field of `xdd:dateTimeInterval` entries and standalone durations
spell identically. Supplied spellings are values: a stored `P2W` never
canonicalises to `P14D`.

Type scoping is enforced at the term layer, not storage:
`xsd:yearMonthDuration` carries year/month only, `xsd:dayTimeDuration`
day/hour/minute/second only.

## Errors

Encode rejects: a sign that is not a single class, a negative component
magnitude, a nonzero week alongside any other nonzero component, a
fraction inconsistent with `frac_len`, and non-canonical
input that would produce a second byte-spelling of one value. Decode
rejects: a sign byte outside `{0x00, 0x01}`, truncated fields, `frac_len` smaller than
the canonical fraction length, trailing bytes. Failures raise errors at
the boundary — a value that cannot be represented must not be silently
dropped (the failure mode of the 2022 format this replaces).
