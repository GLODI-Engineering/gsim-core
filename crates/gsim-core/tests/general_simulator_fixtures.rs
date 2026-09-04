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
//! - `converter.sig2phys` / `converter.phys2sig` — added after `general-mna` renamed
//!   `Sig2Voltage`/`Sig2Current` -> `Sig2Phys`/`Probe` -> `Phys2Sig` (2026-09-02..04 commits).
//!   `phys2sig` has a real mutually-exclusive-parameter (`node` XOR `branch`) validation rule
//!   and, faithfully, **zero input ports** (it reads circuit state directly, never wired) —
//!   the first fixture with an empty `ports:` list.
//! - `extensibility.octfunc` / `extensibility.octblock` — the two new Octave-hosted script
//!   kinds. First fixtures with **two independent variadic port groups** in one component
//!   (`input_names` and `output_names` each drive their own `for_each`, unrelated in length).
//! - `control.discrete_state_space` / `control.discrete_transfer_function` — confirmed
//!   structurally identical to `control.state_space`/`control.transfer_function` (same
//!   matrix-dimension / non-port-affecting-array patterns), plus one new required-and-positive
//!   `ts` (sample time) parameter each -- no new engine pattern, added for taxonomy
//!   completeness.
//! - `power.pwm1` / `power.pwm2` — fixed-port power-electronics blocks with real validated
//!   numeric bounds (`freq > 0`, dead times `>= 0`, and `pspwm`'s `f_min <= f_max`).
//! - `source.sinwave` / `.pulsewave` / `.expwave` / `.sffmwave` — the four real
//!   `general-simulator` waveform-source kinds (confirmed as four distinct `kind=` values in
//!   the component reference, not one kind with a `type=` selector), each with zero input
//!   ports (they read `t` directly, like `converter.phys2sig` reads circuit state directly).
//! - `math.table_lookup` — a `points=` array parameter; the real engine sorts by `x` rather
//!   than rejecting unsorted input (so only non-empty is a real validation rule -- there is no
//!   `all`/fold builtin in the closed expression grammar to check per-entry shape generically,
//!   a known, deliberate gap, same category as the pre-existing nested-array-equality one).
//! - `math.fn1` / `math.fn2` / `math.fn3` — the `MathFn1`/`MathFn2`/`MathFn3` waveform-
//!   arithmetic families, one enum-selected `fn=` parameter each plus fixed 1/2/3 input ports.
//! - `logic.flip_flop` — `FlipFlopKind` (dff/jkff/tff) enum-gated data-input ports via the
//!   `when` idiom (`control.counter`'s pattern), not the `if()`-in-`for_each`-source pattern,
//!   since each kind needs different port *ids*, not just a different count.
//! - `logic.sr_latch` — fixed `set`/`reset`/`q` ports plus a `priority` enum selector.
//! - `logic.hysteresis` — `low <= high` cross-parameter validation, fixed 1-in/1-out ports.
//! - `control.vco` — `f_min <= f_max` cross-parameter validation, fixed 1-in/1-out ports.
//! - `machine.pmsm` — the most parameter-heavy fixture: 7 independently validated scalar
//!   parameters (`r_s >= 0`, `l_d > 0`, `l_q > 0`, `pole_pairs > 0`, `inertia > 0`,
//!   `friction >= 0`), fixed 3-in/4-out ports.
//! - `control.transform.clarke` / `.clarke_inv` / `.park` / `.park_inv` / `.clarke_park` /
//!   `.clarke_park_inv` — the 6-member Clarke/Park family as 6 separate static components (per
//!   the 2026-08-29 audit's recommendation, confirmed against `CoordinateTransform`'s real
//!   `input_count()`/`output_names()`), each with real port labels, not generic `in0`/`out0`.
//!   `park`/`park_inv` use a `zero`/`zero_out` id pair where the real input and output both
//!   conventionally carry a `zero` label (port ids must be unique; labels don't).

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

    // `clamp_lo`/`clamp_hi` were added after the Phase 2 registry work found that
    // `general-mna`'s `"pid"` arm reads both with `get(...)` and no default -- a contract
    // without them compiles to a netlist that fails at load with `missing field 'clamp_lo'`.
    // Their defaults are the same "effectively unbounded" 1e30 sentinel `source.pulsewave`'s
    // `pw`/`per` already use, so a defaulted PID is an unclamped one.
    let names: Vec<&str> = c.parameters.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["kp", "ki", "kd", "n", "clamp_lo", "clamp_hi"]);
    assert!(validate_parameters(&c, &params(json!({}))).is_empty());
    assert!(
        validate_parameters(&c, &params(json!({"clamp_lo": -5.0, "clamp_hi": 5.0}))).is_empty()
    );
    let inverted = validate_parameters(&c, &params(json!({"clamp_lo": 5.0, "clamp_hi": -5.0})));
    assert_eq!(inverted.len(), 1);
    assert_eq!(inverted[0].parameter, "clamp_hi");
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

#[test]
fn sig2phys_bridges_signal_to_a_physical_electrical_port() {
    let c = load("converter.sig2phys");
    let evaluated = evaluate_component(&c, &params(json!({"domain": "current"}))).unwrap();
    assert_eq!(evaluated.ports.len(), 2);
    assert_eq!(evaluated.ports[0].kind, "signal");
    assert_eq!(evaluated.ports[1].kind, "physical-electrical");

    assert!(validate_parameters(&c, &params(json!({"domain": "voltage"}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"domain": "amps"}))).len(),
        1
    );
}

#[test]
fn phys2sig_has_zero_input_ports_and_enforces_node_xor_branch() {
    let c = load("converter.phys2sig");
    let evaluated = evaluate_component(&c, &params(json!({"node": "vout"}))).unwrap();
    assert_eq!(evaluated.ports.len(), 1);
    assert_eq!(
        evaluated.ports[0].direction,
        gsim_core::port::PortDirection::Out
    );

    // Exactly one of node/branch must be set.
    assert!(validate_parameters(&c, &params(json!({"node": "vout"}))).is_empty());
    assert!(validate_parameters(&c, &params(json!({"branch": "v1"}))).is_empty());
    // Neither set (both default to "") -- must fail.
    assert_eq!(validate_parameters(&c, &params(json!({}))).len(), 1);
    // Both set -- must also fail.
    assert_eq!(
        validate_parameters(&c, &params(json!({"node": "vout", "branch": "v1"}))).len(),
        1
    );
}

#[test]
fn octfunc_has_two_independently_variadic_port_groups() {
    let c = load("extensibility.octfunc");

    let default = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(
        default
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in0", "out0"]
    );

    // 3 inputs, 1 output -- the two groups vary independently.
    let evaluated = evaluate_component(
        &c,
        &params(json!({"input_names": ["a", "b", "c"], "output_names": ["y"]})),
    )
    .unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in0", "in1", "in2", "out0"]
    );
    assert_eq!(evaluated.ports[1].label.as_deref(), Some("b"));

    // 1 input, 2 outputs -- the reverse skew.
    let evaluated2 = evaluate_component(
        &c,
        &params(json!({"input_names": ["u"], "output_names": ["y1", "y2"]})),
    )
    .unwrap();
    assert_eq!(
        evaluated2
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in0", "out0", "out1"]
    );
}

#[test]
fn octblock_shares_octfunc_port_shape_plus_a_validated_xc_count() {
    let c = load("extensibility.octblock");
    let evaluated = evaluate_component(
        &c,
        &params(json!({"input_names": ["a", "b"], "output_names": ["y"], "xc_count": 2})),
    )
    .unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in0", "in1", "out0"]
    );

    assert!(validate_parameters(&c, &params(json!({"xc_count": 0}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"xc_count": -1}))).len(),
        1
    );
}

/// `general-simulator` has five code-integration kinds sharing this exact `inputs=`/`outputs=`
/// dual-variadic-port shape (`docs/journal/2026-09.md`): `CScript`, `PyFunction`, `PyBlock`,
/// `OctFunc`, `OctBlock`. `cscript`/`pyblock`/`octblock` additionally carry `xc_count`;
/// `pyfunc`/`octfunc` are stateless. These three assert the remaining ones are structurally
/// identical to `extensibility.octfunc`/`extensibility.octblock` (already covered above) --
/// deliberately not claiming any new engine coverage, just taxonomy completeness.
#[test]
fn cscript_pyfunc_pyblock_share_octfunc_octblocks_dual_variadic_shape() {
    for (name, has_xc) in [
        ("extensibility.cscript", true),
        ("extensibility.pyfunc", false),
        ("extensibility.pyblock", true),
    ] {
        let c = load(name);
        let evaluated = evaluate_component(
            &c,
            &params(json!({"input_names": ["a", "b"], "output_names": ["y1", "y2"]})),
        )
        .unwrap();
        assert_eq!(
            evaluated
                .ports
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            vec!["in0", "in1", "out0", "out1"],
            "{name} port shape"
        );
        if has_xc {
            assert_eq!(
                validate_parameters(&c, &params(json!({"xc_count": -1}))).len(),
                1,
                "{name} should validate xc_count"
            );
        }
    }
}

#[test]
fn counter_gains_optional_ports_only_when_their_signal_param_is_set() {
    let c = load("control.counter");

    let bare = evaluate_component(&c, &params(json!({"clk": "sysclk"}))).unwrap();
    assert_eq!(
        bare.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["clk", "count"]
    );

    let full = evaluate_component(
        &c,
        &params(json!({"clk": "sysclk", "up_down": "dir", "reset": "rst"})),
    )
    .unwrap();
    assert_eq!(
        full.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["clk", "up_down", "reset", "count"]
    );

    // clk is required.
    assert_eq!(validate_parameters(&c, &params(json!({}))).len(), 1);
    assert!(validate_parameters(&c, &params(json!({"clk": "sysclk"}))).is_empty());
}

/// Proves the fix for the "position-derived port ids only survive append/remove-at-the-end"
/// limitation noted in `docs/decisions/0001-scope-and-naming.md`: when the driving parameter is
/// an array of *objects* that each carry their own persistent `id` (rather than a bare array of
/// scalars indexed by loop position), `for_each` binds directly to each object and `emit` can
/// reference `${{ operand.id }}` instead of a loop index -- so removing an element from the
/// *middle* of the array does not renumber or reassign any surviving port's id. No engine change
/// was needed for this; it's a recommended authoring pattern the existing DSL already supports
/// (dotted field access on a `for_each`-bound object was already exercised by
/// `state_space`/`counter`'s `params.x` lookups -- this just does the same on the loop variable).
#[test]
fn stable_id_pattern_survives_removing_an_interior_operand() {
    let c = load("control.sum.stable_ids");

    let three = evaluate_component(
        &c,
        &params(json!({"operands": [
            {"id": "s1", "sign": 1},
            {"id": "s2", "sign": 1},
            {"id": "s3", "sign": -1}
        ]})),
    )
    .unwrap();
    assert_eq!(
        three
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in_s1", "in_s2", "in_s3", "out"]
    );

    // Remove s2 -- the MIDDLE element -- not the tail.
    let after_removal = evaluate_component(
        &c,
        &params(json!({"operands": [
            {"id": "s1", "sign": 1},
            {"id": "s3", "sign": -1}
        ]})),
    )
    .unwrap();
    // s1 and s3 keep their exact original port ids -- this is the property position-based
    // for_each (see contract::tests::a_bare_string_param_only_gives_positional_not_true_identity_stability)
    // cannot provide.
    assert_eq!(
        after_removal
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in_s1", "in_s3", "out"]
    );
    assert_eq!(after_removal.ports[1].label.as_deref(), Some("-")); // s3's own sign, unchanged
}

#[test]
fn discrete_state_space_matches_state_space_plus_a_required_positive_ts() {
    let c = load("control.discrete_state_space");

    let evaluated = evaluate_component(
        &c,
        &params(json!({
            "a": [[1, 0], [0, 1]],
            "b": [[0.1, 0], [0, 0.1]],
            "c": [[1, 0], [0, 1], [1, 1]],
            "d": [[0, 0], [0, 0], [0, 0]],
            "ts": 0.05
        })),
    )
    .unwrap();
    let ids: Vec<&str> = evaluated.ports.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["in0", "in1", "out0", "out1", "out2"]);

    assert!(validate_parameters(&c, &params(json!({"ts": 0.1}))).is_empty());
    // ts=0 (and, by construction, any non-positive ts) must fail -- "a discrete block's own
    // sample period is not optional."
    assert_eq!(
        validate_parameters(&c, &params(json!({"ts": 0.0}))).len(),
        1
    );
}

#[test]
fn discrete_transfer_function_matches_transfer_function_plus_a_required_positive_ts() {
    let c = load("control.discrete_transfer_function");

    let evaluated = evaluate_component(
        &c,
        &params(json!({"num": [0.1], "den": [1, -1], "ts": 0.1})),
    )
    .unwrap();
    assert_eq!(evaluated.ports.len(), 2);

    assert!(validate_parameters(&c, &params(json!({}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"ts": -1.0}))).len(),
        1
    );
    // Same den[0]==0 rule as the continuous version.
    assert_eq!(
        validate_parameters(&c, &params(json!({"den": [0, 1]}))).len(),
        1
    );
}

#[test]
fn pwm1_has_fixed_ports_and_validates_freq_and_dead_times() {
    let c = load("power.pwm1");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in", "main", "comp"]
    );

    assert!(validate_parameters(&c, &params(json!({"freq": 20000.0}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"freq": 0.0}))).len(),
        1
    );
    assert_eq!(
        validate_parameters(&c, &params(json!({"red": -1e-6}))).len(),
        1
    );
}

#[test]
fn pwm2_has_a_frequency_phase_duty_input_triple_and_validates_f_min_le_f_max() {
    let c = load("power.pwm2");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["freq", "phase", "duty", "main", "comp"]
    );

    assert!(
        validate_parameters(&c, &params(json!({"f_min": 1000.0, "f_max": 50000.0}))).is_empty()
    );
    assert_eq!(
        validate_parameters(&c, &params(json!({"f_min": 5000.0, "f_max": 1000.0}))).len(),
        1
    );
}

#[test]
fn waveform_sources_are_zero_input_signal_references_with_their_real_parameter_sets() {
    let cases: &[(&str, Value)] = &[
        ("source.sinwave", json!({"va": 10.0, "freq": 1000.0})),
        ("source.pulsewave", json!({"v1": 0.0, "v2": 10.0})),
        ("source.expwave", json!({"v1": 0.0, "v2": 1.0})),
        (
            "source.sffmwave",
            json!({"va": 1.0, "fc": 1000.0, "fs": 100.0}),
        ),
    ];
    for (name, args) in cases {
        let c = load(name);
        let evaluated = evaluate_component(&c, &params(args.clone())).unwrap();
        assert_eq!(
            evaluated
                .ports
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            vec!["out"],
            "{name} must have exactly one zero-input output port"
        );
        assert!(
            validate_parameters(&c, &params(args.clone())).is_empty(),
            "{name} defaults plus its own required fields should validate"
        );
    }
}

#[test]
fn table_lookup_validates_only_non_emptiness_since_the_real_engine_sorts_rather_than_rejects() {
    let c = load("math.table_lookup");
    let evaluated = evaluate_component(
        &c,
        &params(json!({"points": [[0, 0], [1, 10], [2, 10], [3, 0]]})),
    )
    .unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["in", "out"]
    );

    assert!(validate_parameters(&c, &params(json!({"points": [[0, 0]]}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"points": []}))).len(),
        1
    );
}

#[test]
fn math_fn_families_have_the_right_arity_and_enum_membership_per_family() {
    let cases: &[(&str, &[&str], &str, &str)] = &[
        ("math.fn1", &["in", "out"], "sin", "cosx"),
        ("math.fn2", &["in1", "in2", "out"], "hypot", "nope"),
        ("math.fn3", &["in1", "in2", "in3", "out"], "limit", "nope"),
    ];
    for (name, expected_ports, good_fn, bad_fn) in cases {
        let c = load(name);
        let evaluated = evaluate_component(&c, &params(json!({"fn": good_fn}))).unwrap();
        assert_eq!(
            evaluated
                .ports
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            expected_ports.to_vec(),
            "{name} port shape"
        );
        assert!(validate_parameters(&c, &params(json!({"fn": good_fn}))).is_empty());
        assert_eq!(
            validate_parameters(&c, &params(json!({"fn": bad_fn}))).len(),
            1,
            "{name} should reject an unknown fn name"
        );
    }
}

#[test]
fn flip_flop_gains_kind_specific_data_ports_via_when_not_for_each() {
    let c = load("logic.flip_flop");

    let dff = evaluate_component(&c, &params(json!({"kind": "dff"}))).unwrap();
    assert_eq!(
        dff.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["clk", "d", "q"]
    );

    let tff = evaluate_component(&c, &params(json!({"kind": "tff"}))).unwrap();
    assert_eq!(
        tff.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["clk", "t", "q"]
    );

    // jkff needs both j and k -- neither d nor t.
    let jkff = evaluate_component(&c, &params(json!({"kind": "jkff"}))).unwrap();
    assert_eq!(
        jkff.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["clk", "j", "k", "q"]
    );

    // reset is optional and orthogonal to kind.
    let dff_reset =
        evaluate_component(&c, &params(json!({"kind": "dff", "reset": "rst"}))).unwrap();
    assert_eq!(
        dff_reset
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["clk", "d", "reset", "q"]
    );

    assert_eq!(
        validate_parameters(&c, &params(json!({"kind": "srff"}))).len(),
        1
    );
}

#[test]
fn sr_latch_has_fixed_ports_and_a_priority_enum() {
    let c = load("logic.sr_latch");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["set", "reset", "q"]
    );

    assert!(validate_parameters(&c, &params(json!({"priority": "reset"}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"priority": "first"}))).len(),
        1
    );
}

#[test]
fn hysteresis_validates_low_le_high() {
    let c = load("logic.hysteresis");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(evaluated.ports.len(), 2);

    assert!(validate_parameters(&c, &params(json!({"high": 1.0, "low": -1.0}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"high": -1.0, "low": 1.0}))).len(),
        1
    );
}

#[test]
fn vco_validates_f_min_le_f_max() {
    let c = load("control.vco");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(evaluated.ports.len(), 2);

    assert!(validate_parameters(&c, &params(json!({"f_min": 1.0, "f_max": 1000.0}))).is_empty());
    assert_eq!(
        validate_parameters(&c, &params(json!({"f_min": 1000.0, "f_max": 1.0}))).len(),
        1
    );
}

#[test]
fn pmsm_has_the_real_3_in_4_out_port_shape_and_validates_every_scalar_independently() {
    let c = load("machine.pmsm");
    let evaluated = evaluate_component(&c, &params(json!({}))).unwrap();
    assert_eq!(
        evaluated
            .ports
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["vd", "vq", "t_load", "id", "iq", "omega_m", "theta_e"]
    );

    assert!(validate_parameters(&c, &params(json!({}))).is_empty());
    for (bad_param, bad_value) in [
        ("r_s", json!(-0.1)),
        ("l_d", json!(0.0)),
        ("l_q", json!(0.0)),
        ("pole_pairs", json!(0.0)),
        ("inertia", json!(0.0)),
        ("friction", json!(-1e-6)),
    ] {
        let failures = validate_parameters(&c, &params(json!({bad_param: bad_value})));
        assert_eq!(
            failures.len(),
            1,
            "{bad_param}={bad_value} should fail exactly one validation rule"
        );
        assert_eq!(failures[0].parameter, bad_param);
    }
}

/// The 6-member Clarke/Park family, confirmed against `CoordinateTransform::input_count()`/
/// `output_names()` in `general-simulator`'s own source rather than guessed: each is a pure,
/// parameter-free static component whose *port count and labels* differ by variant (3-in/3-out
/// for `clarke`/`clarke_inv`, 4-in/3-out for the other four).
#[test]
fn clarke_park_family_has_real_per_variant_port_labels_not_generic_ones() {
    let clarke = load("control.transform.clarke");
    let e = evaluate_component(&clarke, &params(json!({}))).unwrap();
    assert_eq!(
        e.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b", "c", "alpha", "beta", "zero"]
    );

    let clarke_inv = load("control.transform.clarke_inv");
    let e = evaluate_component(&clarke_inv, &params(json!({}))).unwrap();
    assert_eq!(
        e.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta", "zero", "a", "b", "c"]
    );

    let park = load("control.transform.park");
    let e = evaluate_component(&park, &params(json!({}))).unwrap();
    assert_eq!(
        e.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta", "zero", "theta", "d", "q", "zero_out"]
    );
    // Both the input and output "zero" carry the same real label despite the id split needed
    // for uniqueness.
    assert_eq!(e.ports[2].label.as_deref(), Some("zero"));
    assert_eq!(e.ports[6].label.as_deref(), Some("zero"));

    let park_inv = load("control.transform.park_inv");
    let e = evaluate_component(&park_inv, &params(json!({}))).unwrap();
    assert_eq!(
        e.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["d", "q", "zero", "theta", "alpha", "beta", "zero_out"]
    );

    let clarke_park = load("control.transform.clarke_park");
    let e = evaluate_component(&clarke_park, &params(json!({}))).unwrap();
    assert_eq!(
        e.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b", "c", "theta", "d", "q", "zero"]
    );

    let clarke_park_inv = load("control.transform.clarke_park_inv");
    let e = evaluate_component(&clarke_park_inv, &params(json!({}))).unwrap();
    assert_eq!(
        e.ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["d", "q", "zero", "theta", "a", "b", "c"]
    );
}
