//! End-to-end fixtures authored from `general-simulator`'s real component surface (see
//! `docs/journal/2026-08.md`'s entry on the component-spec audit), not synthetic examples. Each
//! one exercises a real structural pattern found there:
//!
//! - `passive.diode.ideal` / `passive.switch.ideal` — plain static components (fixed ports,
//!   scalar params, a cross-parameter `validate` rule).
//! - `control.pid` — the simplest possible static reference case.
//! - `control.transfer_function` — a variable-*length* array parameter that does NOT drive port
//!   count (contrast with the next three), validated with `count`/`at`.
//! - `control.sum` — array-length-driven port count (`general-simulator`'s real `sum(inputs,
//!   signs)` — `signs` is a numeric ±1 array, not the char-string toy example from earlier
//!   design work).
//! - `control.product` — a simpler variant of the same pattern: port count driven by a plain
//!   integer parameter via `range()`, rather than the length of an existing array.
//! - `control.state_space` — matrix-*dimension*-driven port count (inputs = width of `b`'s first
//!   row, outputs = row count of `c`) — a structurally different variadic pattern from Sum.
//! - `logic.gate` — enum-selector-driven arity (`op == "not"` forces exactly one input
//!   regardless of `n_operands`), using `if()` inside a `for_each` source.

use gsim_core::contract::{evaluate_component, validate_parameters, ComponentContract};
use serde_json::{json, Map, Value};

fn load(name: &str) -> ComponentContract {
    let path = format!(
        "{}/tests/fixtures/general_simulator/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}

fn params(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

#[test]
fn ideal_diode_is_static_with_two_ports_and_a_cross_param_validation_rule() {
    let c = load("passive.diode.ideal");

    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(evaluated.ports.len(), 2);
    assert_eq!(evaluated.bbox, (40.0, 20.0));

    // Defaults (v_breakdown=-100 < v_th=0.6) pass.
    assert!(validate_parameters(&c, &params(json!({}))).is_empty());
    // v_breakdown >= v_th must fail -- and must see v_th's *default* even though only
    // v_breakdown was supplied, exercising the cross-parameter default-fill fix.
    let failures = validate_parameters(&c, &params(json!({"v_breakdown": 10.0})));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].parameter, "v_breakdown");
}

#[test]
fn ideal_switch_has_three_ports_including_the_gate() {
    let c = load("passive.switch.ideal");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    let ids: Vec<&str> = evaluated.ports.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["d", "s", "g"]);

    assert!(validate_parameters(&c, &params(json!({"r_on": 0.05}))).is_empty());
    let failures = validate_parameters(&c, &params(json!({"r_on": -1.0})));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].parameter, "r_on");
}

#[test]
fn pid_is_the_simplest_static_reference_case() {
    let c = load("control.pid");
    let evaluated = evaluate_component(&c, &params(json!({"kp": 2.5}))).unwrap();
    assert_eq!(evaluated.ports.len(), 2);
    assert!(validate_parameters(&c, &params(json!({"n": 50.0}))).is_empty());
    assert_eq!(validate_parameters(&c, &params(json!({"n": 0.0}))).len(), 1);
}

#[test]
fn transfer_function_validates_array_params_without_them_affecting_port_count() {
    let c = load("control.transfer_function");
    // Port count is fixed at 2 regardless of coefficient array length.
    let evaluated =
        evaluate_component(&c, &params(json!({"num": [1, 2, 3], "den": [1, 0, 0, 1]}))).unwrap();
    assert_eq!(evaluated.ports.len(), 2);

    assert!(validate_parameters(&c, &params(json!({"num": [1], "den": [1, 1]}))).is_empty());
    // den[0] == 0 must fail.
    assert_eq!(
        validate_parameters(&c, &params(json!({"den": [0, 1]}))).len(),
        1
    );
    // len(num) > len(den) must fail.
    assert_eq!(
        validate_parameters(&c, &params(json!({"num": [1, 2, 3], "den": [1]}))).len(),
        1
    );
}

#[test]
fn sum_grows_ports_from_a_real_numeric_signs_array_not_a_char_string() {
    let c = load("control.sum");

    let two = evaluate_component(&c, &params(json!({"signs": [1, 1]}))).unwrap();
    assert_eq!(two.ports.len(), 3); // in0, in1, out
    assert_eq!(two.ports[0].label.as_deref(), Some("+"));
    assert_eq!(two.ports[1].label.as_deref(), Some("+"));
    assert_eq!(two.bbox.1, 30.0); // max(30, 2*14=28) = 30

    let three = evaluate_component(&c, &params(json!({"signs": [1, -1, 1]}))).unwrap();
    assert_eq!(three.ports.len(), 4);
    assert_eq!(
        three
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in0", "in1", "in2", "out"]
    );
    assert_eq!(three.ports[1].label.as_deref(), Some("-"));
    assert_eq!(three.bbox.1, 42.0); // max(30, 3*14=42) = 42

    assert!(validate_parameters(&c, &params(json!({"signs": [1, 1, 1, 1]}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"signs": []}))).len(),
        1
    );
}

#[test]
fn product_grows_ports_from_a_plain_integer_count_via_range() {
    let c = load("control.product");

    let default = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(default.ports.len(), 3); // in0, in1, out (default n_inputs=2)

    let five = evaluate_component(&c, &params(json!({"n_inputs": 5}))).unwrap();
    assert_eq!(five.ports.len(), 6);
    assert_eq!(
        five.ports[..5]
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in0", "in1", "in2", "in3", "in4"]
    );

    assert!(validate_parameters(&c, &params(json!({"n_inputs": 8}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"n_inputs": 1}))).len(),
        1
    );
}

#[test]
fn state_space_ports_are_driven_by_matrix_dimensions_not_a_length_param() {
    let c = load("control.state_space");

    // 2 inputs (b has 2 columns), 3 outputs (c has 3 rows).
    let evaluated = evaluate_component(
        &c,
        &params(json!({
            "a": [[0, 1], [-1, -1]],
            "b": [[1, 0], [0, 1]],
            "c": [[1, 0], [0, 1], [1, 1]],
            "d": [[0, 0], [0, 0], [0, 0]]
        })),
    )
    .unwrap();
    let ids: Vec<&str> = evaluated.ports.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["in0", "in1", "out0", "out1", "out2"]);
    assert_eq!(evaluated.bbox.1, 60.0); // max(30, max(2,3)*20=60) = 60

    // Single-input single-output default.
    let siso = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(
        siso.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["in0", "out0"]
    );
}

#[test]
fn logic_gate_arity_depends_on_the_op_enum_not_just_n_operands() {
    let c = load("logic.gate");

    let and3 = evaluate_component(&c, &params(json!({"op": "and", "n_operands": 3}))).unwrap();
    assert_eq!(
        and3.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["in0", "in1", "in2", "out"]
    );

    // "not" forces exactly one input even though n_operands still says 3 -- proves the `when`/
    // `if`-in-for_each-source pattern actually overrides the length param, not just supplements
    // it.
    let not = evaluate_component(&c, &params(json!({"op": "not", "n_operands": 3}))).unwrap();
    assert_eq!(
        not.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["in0", "out"]
    );
    assert_eq!(not.bbox.1, 20.0);

    assert!(validate_parameters(&c, &params(json!({"op": "xor"}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"op": "maybe"}))).len(),
        1
    );
}
