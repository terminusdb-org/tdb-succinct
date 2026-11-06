//! Verifies the literals quoted in docs/xdd-datetimeinterval-tour — every
//! spelling must parse, encode, decode and render verbatim. Run:
//! `cargo run --release --example story_check`
use tdb_succinct::tfc::lexical_interval::{
    decode_lexical_entry, encode_lexical_interval, parse_lexical_interval, render_lexical_interval,
};

const STORY: &[(&str, &str)] = &[
    ("big-bang day", "-13800000000-01-01"),
    (
        "planck epoch",
        "-13800000000-01-01T00:00:00.00000000000000000000000000000000000000000000Z/\
         -13800000000-01-01T00:00:00.00000000000000000000000000000000000000000539Z",
    ),
    (
        "inflation",
        "-13800000000-01-01T00:00:00.000000000000000000000000000000000001Z/\
         -13800000000-01-01T00:00:00.00000000000000000000000000000001Z",
    ),
    (
        "quark soup",
        "-13800000000-01-01T00:00:00.000001Z/-13800000000-01-01T00:00:01Z",
    ),
    ("recombination", "-13800000000-01-01/P380000Y"),
    (
        "age of the universe",
        "-13800000000-01-01T00:00:00/2026-11-05T00:00:00",
    ),
    ("stelliferous ends", "100000000000000-01-01/.."),
    (
        "the cold forever",
        "1000000000000000000000000000000-01-01/..",
    ),
];

fn main() {
    for (name, lit) in STORY {
        let lit: String = lit.chars().filter(|c| !c.is_whitespace()).collect();
        let parsed = parse_lexical_interval(&lit).unwrap_or_else(|e| {
            panic!("{name}: {lit} failed to parse: {e:?}")
        });
        let entry = encode_lexical_interval(&parsed);
        let decoded = decode_lexical_entry(&entry).expect("decode");
        let rendered = render_lexical_interval(&decoded);
        assert_eq!(rendered, lit, "{name}: render mismatch");
        println!("{name:22} {lit}");
    }
    println!("all {} story literals round-trip", STORY.len());
}
