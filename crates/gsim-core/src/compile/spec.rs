//! The per-type-family emitter registry: what netlist line(s) each dotted `Block.type` renders
//! into, keyed by the same type strings GSF v2 already uses.
//!
//! # The two sub-grammars, and why one registry covers both
//!
//! `general-simulator`'s netlist format is two genuinely different grammars in one file
//! (`spice-lsp/docs/GRAMMAR.md` §3 and §12):
//!
//! 1. **Real SPICE element syntax** — `NAME node1 node2 [node3 node4] value_or_model_ref`.
//!    Positional node arguments resolving to shared net names.
//! 2. **The `kind=` block-graph syntax** — `NAME kind=<value> field=value ...`. No device
//!    letter, no node list; every `field=value` is either a literal or a **direct reference to
//!    another block's name**, never a net.
//!
//! A [`TypeSpec`] can declare either, or **both**: an ideal switch really is emitted as a pair
//! in the real reference circuit netlist —
//! `DEV d s idealswitchmodel` (the element card, supplying the nodes) plus
//! `DEV kind=ideal_switch r_on=... gate=block ctrl=GATE` (the fields card).
//!
//! # The device letter is load-bearing
//!
//! Verified directly against `general-mna/src/builder.rs`'s stamping loop, which dispatches on
//! `element.device_letter` — the **first character of the element's name** — with `'R'`, `'C'`,
//! `'L'`, `'V'`, `'I'`, `'G'`, `'E'`, `'F'`, `'H'`, `'K'`, `'D'` each stamping a different
//! device, and anything else rejected as unsupported. A resistor named `snub1` is therefore not
//! a resistor to this target at all. [`TypeSpec::device_letter`] records the required letter and
//! [`element_name`] prefixes an id that doesn't already start with it, rather than silently
//! emitting a line the simulator will refuse.

use serde_json::Value;

/// What follows an element card's positional nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementTail {
    /// One parameter rendered as the element's scalar value (`Rsnub d snub 20`).
    Value(&'static str),
    /// A fixed model-name token (`DEV d s idealswitchmodel`) — the `kind=` card carries the
    /// real parameters.
    Model(&'static str),
    /// A controlling *element name* parameter then a gain parameter (`Fxfmr PA PB Exfmr -3.5`).
    ControlThenValue {
        control: &'static str,
        gain: &'static str,
    },
    /// Nothing after the nodes.
    None,
}

/// One `field=value` token on a `kind=` block card.
#[derive(Debug, Clone, Copy)]
pub enum FieldSpec {
    /// `field=<the block's param value>`.
    Param {
        field: &'static str,
        param: &'static str,
        required: bool,
    },
    /// `field=<fixed text>`.
    Literal {
        field: &'static str,
        value: &'static str,
    },
    /// `field=<name of whatever drives this port>` — resolved through wire adjacency to the
    /// peer block's *output name*, which is what §12's `in=`/`ctrl=`/`gate=` fields take.
    Ref {
        field: &'static str,
        port: &'static str,
    },
    /// `field=<comma-joined names of whatever drives each of these ports, in order>` —
    /// `inputs=ctrl_freq,ctrl_phase,ctrl_duty`.
    Refs {
        field: &'static str,
        ports: &'static [&'static str],
    },
    /// `outputs=<comma-joined output names>`, emitted only when this block declares more than
    /// one output and is not content with the target's own defaults.
    Outputs { field: &'static str },
}

/// How a type's `kind=` string is chosen.
#[derive(Debug, Clone, Copy)]
pub enum KindName {
    /// One fixed `kind=` string for the whole type.
    Fixed(&'static str),
    /// **One `kind=` string per function/variant.** Confirmed against
    /// `general-mna/src/system_builder.rs`'s actual dispatch: the coordinate-transform family
    /// really is `kind=clarke` / `kind=park` / `kind=clarkepark` / ..., *not*
    /// `kind=transform type=clarke`. A type whose GSF v2 contract models the variant as a
    /// parameter therefore has to map that parameter's value onto a distinct `kind=` string
    /// here — this is per-family, not one shape for everything.
    PerVariant {
        param: &'static str,
        map: &'static [(&'static str, &'static str)],
    },
}

#[derive(Debug, Clone, Copy)]
pub struct KindSpec {
    pub kind: KindName,
    pub fields: &'static [FieldSpec],
}

/// How one dotted `Block.type` is rendered.
#[derive(Debug, Clone, Copy)]
pub struct TypeSpec {
    pub type_name: &'static str,
    /// `Some` => emit a SPICE element card. The letter is the required first character of the
    /// emitted name — see the module doc.
    pub device_letter: Option<char>,
    /// Which of the block's ports become the element card's positional nodes, in order.
    pub node_ports: &'static [&'static str],
    pub tail: ElementTail,
    /// `Some` => emit a `kind=` block card.
    pub kind: Option<KindSpec>,
    /// This type's output ports, in the order `general-mna`'s own `output_names` convention
    /// uses. `output_ports[0]` aliases the block's own name; later ones need explicit names.
    pub output_ports: &'static [&'static str],
    /// A structural block that emits no netlist line at all (a ground symbol only *names* a
    /// net; a subckt boundary only marks the def's interface).
    pub structural: bool,
}

impl TypeSpec {
    pub const fn empty(type_name: &'static str) -> TypeSpec {
        TypeSpec {
            type_name,
            device_letter: None,
            node_ports: &[],
            tail: ElementTail::None,
            kind: None,
            output_ports: &[],
            structural: false,
        }
    }
}

/// The one name a block is known by in emitted netlist text.
///
/// For a type that emits **both** an element card and a `kind=` card (an ideal switch/diode),
/// the two lines MUST carry the same name: `general-mna` joins them by name, and its diode
/// stamping introduces the symbol `<name>_G` that the `kind=` card's own entry has to resolve.
/// Emitting `Dlega_high d s idealswitchmodel` next to `lega_high kind=ideal_switch ...` fails at
/// solve time with `MissingSymbol("Dlega_high_G")` -- found for real by running a compiled
/// netlist, not by inspection.
pub fn card_name(id: &str, spec: &TypeSpec) -> String {
    match spec.device_letter {
        Some(letter) => element_name(id, letter),
        None => id.to_string(),
    }
}

/// The emitted element name for a block: its own id when that already starts with the required
/// device letter (case-insensitively), else the letter prefixed onto it.
pub fn element_name(id: &str, letter: char) -> String {
    match id.chars().next() {
        Some(first) if first.eq_ignore_ascii_case(&letter) => id.to_string(),
        _ => format!("{letter}{id}"),
    }
}

/// Renders an `f64` as a netlist number token: integral values without a spurious `.0`,
/// very small/large magnitudes in exponent form (`2.2e-9` rather than `0.0000000022`), never
/// in a locale-dependent form. Both forms are accepted by `spice-lsp/docs/GRAMMAR.md` §2.
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    if v == 0.0 {
        return "0".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    if v.abs() < 1e-4 || v.abs() >= 1e15 {
        return format!("{v:e}");
    }
    format!("{v}")
}

/// Renders a JSON param value as a netlist token. Numbers are rendered without a trailing
/// `.0` where they are integral, and never in a locale-dependent form; a JSON array becomes a
/// bracketed Python-style list literal, which is exactly what §12's grammar accepts for
/// matrix/vector fields (`a=[[1,2],[3,4]]`) — with **no whitespace**, since a field's value is
/// a single whitespace-free token.
pub fn render_value(value: &Value) -> String {
    match value {
        Value::Null => "0".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(render_value).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(_) => value.to_string().replace(' ', ""),
    }
}

macro_rules! rc {
    ($type_name:literal, $letter:literal, $ports:expr, $value:literal) => {
        TypeSpec {
            device_letter: Some($letter),
            node_ports: $ports,
            tail: ElementTail::Value($value),
            ..TypeSpec::empty($type_name)
        }
    };
}

const TWO_PIN: &[&str] = &["p1", "p2"];
const SOURCE_PINS: &[&str] = &["p", "n"];

/// Every type the compiler knows how to emit.
///
/// Seeded with exactly the set the real multi-leg power-electronics reference circuit needs (that circuit
/// compiling and running is the acceptance bar), then extended across the existing
/// `tests/fixtures/general_simulator/*.json` families whose netlist shape is unambiguous from
/// `general-mna`'s own dispatch. Families needing shapes not yet exercised by a real circuit
/// are deliberately left out rather than guessed — an unknown type is a hard
/// `CompileError::UnknownType`, never a silently-dropped block.
pub const REGISTRY: &[TypeSpec] = &[
    // -- structural ---------------------------------------------------------
    TypeSpec {
        structural: true,
        ..TypeSpec::empty("routing.ground")
    },
    TypeSpec {
        structural: true,
        ..TypeSpec::empty("spice-subckt.boundary")
    },
    // -- passives -----------------------------------------------------------
    rc!("passive.resistor", 'R', TWO_PIN, "R"),
    rc!("passive.capacitor", 'C', TWO_PIN, "C"),
    rc!("passive.inductor", 'L', TWO_PIN, "L"),
    // -- independent sources ------------------------------------------------
    rc!("source.voltage.dc", 'V', SOURCE_PINS, "V"),
    rc!("source.current.dc", 'I', SOURCE_PINS, "I"),
    // -- dependent sources (the E/F pair a true ideal transformer is built from) --
    TypeSpec {
        device_letter: Some('E'),
        node_ports: &["p", "n", "cp", "cn"],
        tail: ElementTail::Value("gain"),
        ..TypeSpec::empty("source.dependent.vcvs")
    },
    TypeSpec {
        device_letter: Some('G'),
        node_ports: &["p", "n", "cp", "cn"],
        tail: ElementTail::Value("gain"),
        ..TypeSpec::empty("source.dependent.vccs")
    },
    TypeSpec {
        device_letter: Some('F'),
        node_ports: SOURCE_PINS,
        tail: ElementTail::ControlThenValue {
            control: "control",
            gain: "gain",
        },
        ..TypeSpec::empty("source.dependent.cccs")
    },
    TypeSpec {
        device_letter: Some('H'),
        node_ports: SOURCE_PINS,
        tail: ElementTail::ControlThenValue {
            control: "control",
            gain: "gain",
        },
        ..TypeSpec::empty("source.dependent.ccvs")
    },
    // -- switching devices: element card + kind= card, exactly as the real .cir writes them --
    TypeSpec {
        device_letter: Some('D'),
        node_ports: &["d", "s"],
        tail: ElementTail::Model("idealswitchmodel"),
        kind: Some(KindSpec {
            kind: KindName::Fixed("ideal_switch"),
            fields: &[
                FieldSpec::Param {
                    field: "r_on",
                    param: "r_on",
                    required: true,
                },
                FieldSpec::Param {
                    field: "g_breakdown",
                    param: "g_breakdown",
                    required: true,
                },
                FieldSpec::Param {
                    field: "v_breakdown",
                    param: "v_breakdown",
                    required: true,
                },
                FieldSpec::Param {
                    field: "g_off",
                    param: "g_off",
                    required: true,
                },
                FieldSpec::Param {
                    field: "v_th",
                    param: "v_th",
                    required: true,
                },
                FieldSpec::Param {
                    field: "g_on",
                    param: "g_on",
                    required: true,
                },
                // Every gate is block-driven -- general-mna rejects any other gate= spelling.
                FieldSpec::Literal {
                    field: "gate",
                    value: "block",
                },
                FieldSpec::Ref {
                    field: "ctrl",
                    port: "g",
                },
            ],
        }),
        ..TypeSpec::empty("passive.switch.ideal")
    },
    TypeSpec {
        device_letter: Some('D'),
        node_ports: &["a", "k"],
        tail: ElementTail::Model("idealdiodemodel"),
        kind: Some(KindSpec {
            kind: KindName::Fixed("ideal_diode"),
            fields: &[
                FieldSpec::Param {
                    field: "g_breakdown",
                    param: "g_breakdown",
                    required: true,
                },
                FieldSpec::Param {
                    field: "v_breakdown",
                    param: "v_breakdown",
                    required: true,
                },
                FieldSpec::Param {
                    field: "g_off",
                    param: "g_off",
                    required: true,
                },
                FieldSpec::Param {
                    field: "v_th",
                    param: "v_th",
                    required: true,
                },
                FieldSpec::Param {
                    field: "g_on",
                    param: "g_on",
                    required: true,
                },
            ],
        }),
        ..TypeSpec::empty("passive.diode.ideal")
    },
    // -- converters ---------------------------------------------------------
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("sig2phys"),
            fields: &[
                FieldSpec::Param {
                    field: "domain",
                    param: "domain",
                    required: true,
                },
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("converter.sig2phys")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("phys2sig"),
            fields: &[
                FieldSpec::Param {
                    field: "node",
                    param: "node",
                    required: false,
                },
                FieldSpec::Param {
                    field: "branch",
                    param: "branch",
                    required: false,
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("converter.phys2sig")
    },
    // -- signal sources -----------------------------------------------------
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("const"),
            fields: &[FieldSpec::Param {
                field: "value",
                param: "value",
                required: true,
            }],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("source.const")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("time"),
            fields: &[],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("source.time")
    },
    // -- PWM modulators -----------------------------------------------------
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("pspwm"),
            fields: &[
                FieldSpec::Param {
                    field: "f_min",
                    param: "f_min",
                    required: true,
                },
                FieldSpec::Param {
                    field: "f_max",
                    param: "f_max",
                    required: true,
                },
                FieldSpec::Param {
                    field: "red",
                    param: "red",
                    required: false,
                },
                FieldSpec::Param {
                    field: "fed",
                    param: "fed",
                    required: false,
                },
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["freq", "phase", "duty"],
                },
                FieldSpec::Outputs { field: "outputs" },
            ],
        }),
        // general-mna's own default is [name, "{name}_comp"] -- matched by the emitter.
        output_ports: &["main", "comp"],
        ..TypeSpec::empty("power.pwm2")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("pwm"),
            fields: &[
                FieldSpec::Param {
                    field: "freq",
                    param: "freq",
                    required: true,
                },
                FieldSpec::Param {
                    field: "red",
                    param: "red",
                    required: false,
                },
                FieldSpec::Param {
                    field: "fed",
                    param: "fed",
                    required: false,
                },
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["duty"],
                },
                FieldSpec::Outputs { field: "outputs" },
            ],
        }),
        output_ports: &["main", "comp"],
        ..TypeSpec::empty("power.pwm1")
    },
    // -- plain signal-flow blocks (one implicit output aliased to the block's own name) --
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("gain"),
            fields: &[
                FieldSpec::Param {
                    field: "k",
                    param: "k",
                    required: true,
                },
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["in"],
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("control.gain")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("saturation"),
            fields: &[
                FieldSpec::Param {
                    field: "lower",
                    param: "lower",
                    required: true,
                },
                FieldSpec::Param {
                    field: "upper",
                    param: "upper",
                    required: true,
                },
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["in"],
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("control.saturation")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("vco"),
            fields: &[
                FieldSpec::Param {
                    field: "f_min",
                    param: "f_min",
                    required: true,
                },
                FieldSpec::Param {
                    field: "f_max",
                    param: "f_max",
                    required: true,
                },
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["in"],
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("control.vco")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("hysteresis"),
            fields: &[
                FieldSpec::Param {
                    field: "low",
                    param: "low",
                    required: true,
                },
                FieldSpec::Param {
                    field: "high",
                    param: "high",
                    required: true,
                },
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["in"],
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("logic.hysteresis")
    },
];

pub fn lookup(type_name: &str) -> Option<&'static TypeSpec> {
    REGISTRY.iter().find(|s| s.type_name == type_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_name_prefixes_only_when_the_letter_is_missing() {
        assert_eq!(element_name("Rsnub", 'R'), "Rsnub");
        assert_eq!(element_name("rsnub", 'R'), "rsnub");
        assert_eq!(element_name("snub", 'R'), "Rsnub");
        // The real reference circuit's switch instances are named DEV/DEVA/DEVB -- already 'D'.
        assert_eq!(element_name("DEV", 'D'), "DEV");
        assert_eq!(element_name("sw1", 'D'), "Dsw1");
    }

    #[test]
    fn registry_type_names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for spec in REGISTRY {
            assert!(seen.insert(spec.type_name), "duplicate {}", spec.type_name);
        }
    }

    #[test]
    fn renders_values_without_locale_or_whitespace_surprises() {
        assert_eq!(render_value(&serde_json::json!(20)), "20");
        assert_eq!(render_value(&serde_json::json!(2.2e-9)), "2.2e-9");
        assert_eq!(render_value(&serde_json::json!(300000.0)), "300000");
        assert_eq!(render_value(&serde_json::json!(-1e6)), "-1000000");
        assert_eq!(render_value(&serde_json::json!("voltage")), "voltage");
        assert_eq!(
            render_value(&serde_json::json!([[1, 2], [3, 4]])),
            "[[1,2],[3,4]]"
        );
    }
}
