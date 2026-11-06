//! Lexical `xsd:duration` dictionary entries — see
//! `lexical-duration-format.md` for the byte-level format this module
//! implements, and terminusdb-enterprise/docs/specifications/xsd-duration.md for the
//! datatype contract. Byte layout: a sign byte, then the magnitude tuple
//! (week, year, month, day, hour, minute, second-int, fraction, `frac_len`)
//! as `bigint_to_storage`/`integer_and_fraction_to_storage` components,
//! payload bitwise-complemented for negative durations.

#[cfg(test)]
mod tests {
    use super::super::{decimal::integer_and_fraction_to_storage, integer::bigint_to_storage};
    use rug::Integer;

    /// Spec-layout encoder for the ordering contract. Commit 2 replaces
    /// this with the real entry encoder, which must produce the same byte
    /// order: sign class first, then the component tuple — so `P2W` and
    /// `P14D` are distinct entries and `-P2Y` sorts before `-P1Y`.
    fn spec_entry(
        sign: i8,
        week: i64,
        year: i64,
        month: i64,
        day: i64,
        hour: i64,
        minute: i64,
        sec_int: i64,
        frac: Option<&str>,
    ) -> Vec<u8> {
        let mut bytes = vec![if sign < 0 { 0x00 } else { 0x01 }];
        for m in [week, year, month, day, hour, minute] {
            bytes.extend(bigint_to_storage(Integer::from(m)));
        }
        bytes.extend(integer_and_fraction_to_storage(
            false,
            Integer::from(sec_int),
            frac,
        ));
        bytes.push(frac.map_or(0, |f| f.len() as u8));
        if sign < 0 {
            for b in bytes[1..].iter_mut() {
                *b = !*b;
            }
        }
        bytes
    }

    /// Component-tuple order in expected sort sequence: negatives first
    /// (sign 0x00, payload complemented so a larger magnitude sorts
    /// earlier), then non-negatives in tuple order. Not an elapsed-time
    /// order — `PT2S` precedes `P14D` because day dominates seconds.
    #[test]
    fn entries_sort_by_component_tuple() {
        // (expected-position comment, spec_entry args)
        let table: &[(&str, Vec<u8>)] = &[
            ("-P2W", spec_entry(-1, 2, 0, 0, 0, 0, 0, 0, None)),
            ("-P2Y", spec_entry(-1, 0, 2, 0, 0, 0, 0, 0, None)),
            ("-P1Y3D", spec_entry(-1, 0, 1, 0, 3, 0, 0, 0, None)),
            ("-P1Y2D", spec_entry(-1, 0, 1, 0, 2, 0, 0, 0, None)),
            ("-P14D", spec_entry(-1, 0, 0, 0, 14, 0, 0, 0, None)),
            ("-PT2S", spec_entry(-1, 0, 0, 0, 0, 0, 0, 2, None)),
            ("-PT1.5S", spec_entry(-1, 0, 0, 0, 0, 0, 0, 1, Some("5"))),
            ("PT0S", spec_entry(1, 0, 0, 0, 0, 0, 0, 0, None)),
            ("PT1.5S", spec_entry(1, 0, 0, 0, 0, 0, 0, 1, Some("5"))),
            ("PT1.50S", spec_entry(1, 0, 0, 0, 0, 0, 0, 1, Some("50"))),
            ("PT2S", spec_entry(1, 0, 0, 0, 0, 0, 0, 2, None)),
            ("P14D", spec_entry(1, 0, 0, 0, 14, 0, 0, 0, None)),
            ("P1Y", spec_entry(1, 0, 1, 0, 0, 0, 0, 0, None)),
            ("P1Y2D", spec_entry(1, 0, 1, 0, 2, 0, 0, 0, None)),
            ("P1Y3D", spec_entry(1, 0, 1, 0, 3, 0, 0, 0, None)),
            ("P2W", spec_entry(1, 2, 0, 0, 0, 0, 0, 0, None)),
        ];
        for w in table.windows(2) {
            assert!(w[0].1 < w[1].1, "{} should sort before {}", w[0].0, w[1].0);
        }
    }

    /// Equal durations with different denominations stay distinct
    /// entries; supplied fraction precision is part of the value.
    #[test]
    fn denominations_are_distinct_values() {
        assert_ne!(
            spec_entry(1, 2, 0, 0, 0, 0, 0, 0, None),
            spec_entry(1, 0, 0, 0, 14, 0, 0, 0, None),
            "P2W and P14D must differ"
        );
        assert_ne!(
            spec_entry(1, 0, 0, 0, 0, 0, 0, 1, Some("5")),
            spec_entry(1, 0, 0, 0, 0, 0, 0, 1, Some("50")),
            "PT1.5S and PT1.50S must differ"
        );
    }
}
