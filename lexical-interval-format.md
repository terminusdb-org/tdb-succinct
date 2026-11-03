# Lexical dateTimeInterval entry format

Status: implemented, with one pending pre-release change: the class-byte
renumbering (`0..=3` → `4..=7`) described under *Legacy dispatch*.
Encoding lives in `src/tfc/lexical_interval.rs`
(`encode_lexical_interval`, `decode_lexical_entry`). Datatype:
`xdd:dateTimeInterval`.

An interval dictionary entry is a self-delimiting ordering key. There is no
payload: the canonical spelling is reconstructed from the key fields on
decode. Byte order over entries is the value order — comparisons never
decode.

## Layout

```
start instant key
end   instant key
period bytes, 0x00 terminator
start meta   (finite components only)
end   meta   (finite components only)
flag  u8
```

### Instant key

```
class byte
if finite: bigint(int) + fraction
```

`bigint` is `bigint_to_storage` (`src/tfc/integer.rs`): sign-magnitude,
embedded size, arbitrary precision, order-preserving. `fraction` is the
centary encoding from `integer_and_fraction_to_storage`
(`src/tfc/decimal.rs`): canonical decimal digit pairs with a continuation
bit, bitwise-complemented for negative values; an empty fraction stores a
single `0x00` "false zero" byte so the field is never confused with a
present fraction.

Class bytes place the non-finite component kinds at their semantic
extremes, so plain byte order gives `−∞`-start first and `+∞`-end last.
The range starts at 4 so byte 0 also discriminates against the legacy
encodings, whose first byte is a component type in `0..=3`:

| class byte | start component | end component |
|------------|-----------------|---------------|
| 4          | Unbounded (−∞)  | Finite        |
| 5          | Unspecified     | Unknown       |
| 6          | Unknown         | Unspecified   |
| 7          | Finite          | Unbounded (+∞)|

### Period

Canonical ISO 8601 duration spelling, `0x00`-terminated. For
`FLAG_START_DURATION` / `FLAG_DURATION_END` it is the supplied duration
component verbatim — it is part of the value's identity
(`2025-01-01/P1M` ≠ `2025-01-01/P30D`). For `FLAG_EXPLICIT` it is the
*derived* period computed at encode time (`derived_period`), used purely
for ordering.

### Meta (finite components only)

```
kind      u8    KIND_DATE = 0, KIND_DATETIME = 1
tz_class  u8    see table
offset    i16   absolute minutes, only when tz_class is TZ_NEG or TZ_POS
frac_len  u8    supplied fraction digit count
```

`frac_len` preserves supplied precision without touching ordering of the
instant itself: `.5`, `.50`, `.500` are the same instant but distinct
literals. Invariant: `frac_len` ≥ canonical fraction length, and
`frac_len == 0` iff no fraction.

### tshift classes

| tz_class | byte | meaning |
|----------|------|---------|
| TZ_NEG      | 0 | negative offset, non-zero (`-hh:mm`) |
| TZ_NEG_ZERO | 1 | `-00:00` (unknown local offset) |
| TZ_Z        | 2 | `Z` |
| TZ_ZONELESS | 3 | no zone designator |
| TZ_POS_ZERO | 4 | `+00:00` |
| TZ_POS      | 5 | positive offset, non-zero (`+hh:mm`) |

Ordering on this axis is the tshift order: negative offsets, then Z, then
zone-less, then positive offsets.

### Flag

| flag | byte | form |
|------|------|------|
| FLAG_EXPLICIT       | 0 | `T/T` — two explicit instants |
| FLAG_START_DURATION | 1 | `T/P` — start + period |
| FLAG_DURATION_END   | 2 | `P/T` — period + end |

## Ordering

Lexicographic over the layout: start instant, end instant, period string,
start meta, end meta, flag. Instant order is numeric through the
sign-magnitude bigint and the centary fraction; the class byte ranges make
the special kinds land on the correct side of all finite values.

## Canonical spelling

Not stored. On decode, `reconstruct_component_canonical` rebuilds each
finite component's spelling from its key fields (wall clock from
`instant ± tshift`, fraction digits, zone marker from the tz class), and a
duration side is the stored period itself. Canonicalisation rules applied
at parse:

- `-` and `:` separators stripped (basic form);
- zone spellings normalised to the tshift classes above;
- fraction trailing zeros stripped into `frac_len`;
- duration sides keep the supplied spelling: the expression is the value.
  `P2W` and `P14D` are equal durations but different literals and never
  collapse; the same holds for `P0D` and `PT0S`. Producers (`derived_period`,
  `duration_parts_to_period`, Prolog `duration_string`) emit `PT0S` for a
  computed zero — the maximum-precision spelling — but that decree does
  not rewrite supplied spellings.

`render_lexical_interval` re-extends components to ISO extended form for
display.

## Legacy dispatch

Two fixed-width encodings predate this format:

| name | width | layout |
|------|-------|--------|
| `INTERVAL_LEGACY_ENCODED_WIDTH` | 48 | two 24-byte components |
| `INTERVAL_ENCODED_WIDTH` | 52 | two 26-byte components |

`upgrade_lexical_intervals` detects them by entry length and rewrites them
to the lexical encoding when a dictionary is rebuilt. Immutable layer
files are never rewritten in place — the upgrade happens when data flows
into a new dictionary (delta/rollup merge), and a dictionary is always
upgraded as a whole, so a type segment never mixes the two encodings.

Dispatch is per-entry and exact — each entry's first byte determines its
decoder, with no inference across the segment:

```
bytes[0] ≤ 3   → legacy decoder (entry must be 48 or 52 bytes, else malformed)
bytes[0] ≥ 4   → lexical decoder (this format)
```

Dictionaries are written atomically, so in practice a type segment is
homogeneous — all entries share one encoding. That is a property of
correct writers, not something decode depends on: per-entry dispatch
decodes a mixed segment correctly regardless. Segment scans (index
builds, upgrades) may additionally assert homogeneity cheaply — compare
the byte-0 class of the first and last entries — and treat a mismatch as
corruption rather than silently tolerating it.

Before the renumbering, class bytes were `0..=3` — identical to the
legacy component-type range — and dispatch was length-only, so a lexical
entry of exactly 48 or 52 bytes would have been misrouted. Moving the
class range above the legacy values removes the ambiguity at zero cost:
byte order within the type segment is unchanged (only relative order
matters), and a legacy entry's byte 0 can never legitimately exceed 3.
A legacy entry with a corrupt component-type byte ≥ 4 reaches the lexical
decoder and fails its field invariants — loud, not silent.

## Errors

Malformed entries — truncated fields, unknown class bytes, `frac_len`
inconsistent with the canonical fraction, trailing bytes after the flag —
return `LexicalIntervalError::MalformedEntry`. Decoders must not silently
coerce.
