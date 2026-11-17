//! A finite `f64` parameter must come back bit-identical from a document round trip.
//!
//! It arrived as a one-ULP difference, which sounds like nothing: a relative error near 9e-17
//! that cannot move a waveform. The reason it mattered is the contract. This project verifies a
//! trace against an analytically derived expression, and that only works if the constants in the
//! netlist are the ones that were written down. A value that quietly changes in the last bit
//! between the script and the card breaks that chain without reporting anything.
//!
//! It also had many chances to compound: every `gsim-tools` operation threads the whole model
//! through JSON -> Model -> JSON again, so a long build re-parses each parameter repeatedly.
//!
//! The cause was not where it looked. Emission was already exact — `serde_json::to_string`
//! writes `0.011067758972493853` — and so is Rust's own `str::parse::<f64>()`. It was
//! `serde_json`'s *parser*: without the `float_roundtrip` feature it uses a faster algorithm
//! that is not always correctly rounded, and it read that literal one ULP low. The feature is
//! enabled in each crate that parses a document, and these cases pin it.

use serde_json::Value;

/// The value the defect was found on, diffing a `gsim-tools`-built netlist against the
/// hand-written reference it reproduces: 45 of 47 `kind=` cards matched field for field, and
/// this gain was one of the two that did not.
const FOUND_ON: f64 = 0.011067758972493853;

fn reparse(v: f64) -> f64 {
    let text = serde_json::to_string(&Value::from(v)).expect("serialising an f64");
    serde_json::from_str::<Value>(&text)
        .expect("parsing it back")
        .as_f64()
        .expect("still a number")
}

#[test]
fn the_value_the_defect_was_found_on_round_trips_bit_for_bit() {
    let back = reparse(FOUND_ON);
    assert_eq!(
        back.to_bits(),
        FOUND_ON.to_bits(),
        "{FOUND_ON} came back as {back} ({:016x} vs {:016x})",
        back.to_bits(),
        FOUND_ON.to_bits()
    );
}

#[test]
fn a_literal_parses_to_the_same_bits_as_rust_itself_reads_it() {
    // `str::parse` is correctly rounded, so it is the reference for what the literal means.
    // This is the comparison that located the bug: the two disagreed.
    for lit in [
        "0.011067758972493853",
        "1e-320",                  // subnormal
        "2.2250738585072014e-308", // smallest normal
        "1.7976931348623157e308",  // f64::MAX
        "0.1",
        "1234567890.1234567",
    ] {
        let rust: f64 = lit.parse().expect("Rust parses the literal");
        let json = serde_json::from_str::<Value>(lit)
            .expect("serde_json parses the literal")
            .as_f64()
            .expect("as a number");
        assert_eq!(
            json.to_bits(),
            rust.to_bits(),
            "{lit}: serde_json read {json:e} ({:016x}), Rust reads {rust:e} ({:016x})",
            json.to_bits(),
            rust.to_bits()
        );
    }
}

#[test]
fn a_spread_of_values_survives_many_round_trips() {
    // `gsim-tools` re-parses the whole model on every operation, so one pass is not the
    // interesting case; a drift of one ULP per pass is.
    let values = [
        FOUND_ON,
        0.1,
        1.0 / 3.0,
        std::f64::consts::PI,
        1e-9,
        4.7e-6,
        6.8e3,
        -2.5,
        0.0,
        -0.0,
    ];
    for v in values {
        let mut x = v;
        for pass in 0..32 {
            x = reparse(x);
            assert_eq!(
                x.to_bits(),
                v.to_bits(),
                "{v} drifted on pass {pass}: {:016x} vs {:016x}",
                x.to_bits(),
                v.to_bits()
            );
        }
    }
}
