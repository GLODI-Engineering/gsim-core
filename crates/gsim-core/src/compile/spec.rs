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
//! by a real switching-converter netlist —
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

use crate::document::Block;
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

/// How many ports a family whose port *count* is set by a parameter actually has.
///
/// GSF v2 already models these families with `for_each`-generated ports
/// (`extensibility.cscript`'s `in${{ i }}` over `params.input_names`,
/// `control.state_space`'s `in${{ i }}` over `count(at(params.b, 0))`), so the count is a
/// property of the *instance*, not of the type — a `&'static [&'static str]` cannot express it.
#[derive(Debug, Clone, Copy)]
pub enum Arity {
    /// The number of entries in an array parameter (`input_names`, `output_names`).
    ParamLen(&'static str),
    /// The column count of a matrix parameter. `general-mna`'s
    /// `parse_matrix_or_vector_shorthand` accepts either a real matrix (`b=[[1,0],[0,1]]`,
    /// `n x p` for `p` inputs) or a **flat vector as SISO shorthand** (`b=[1,0]` means one
    /// input), so a flat array here counts as exactly one column, not as its own length.
    MatrixCols(&'static str),
}

impl Arity {
    pub fn count(&self, block: &Block) -> Option<usize> {
        match self {
            Arity::ParamLen(p) => block.params.get(*p)?.as_array().map(Vec::len),
            Arity::MatrixCols(p) => {
                let rows = block.params.get(*p)?.as_array()?;
                match rows.first() {
                    Some(Value::Array(first)) => Some(first.len()),
                    // Flat-vector SISO shorthand.
                    Some(_) => Some(1),
                    None => Some(0),
                }
            }
        }
    }
}

/// Output ports whose count is set by a parameter rather than fixed by the type.
#[derive(Debug, Clone, Copy)]
pub struct DynamicOutputs {
    /// Port ids are `<prefix><i>`, matching the GSF v2 fixture's own `for_each` emission.
    pub prefix: &'static str,
    pub arity: Arity,
    /// The array parameter holding each output's *declared* name, used as the emitted name's
    /// suffix so a generated `outputs=` list stays readable (`SCR,SCR_y2`).
    pub names_param: Option<&'static str>,
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
    /// `field=<comma-joined names of the drivers of ports `<prefix>0`..`<prefix>{n-1}`>`, where
    /// `n` comes from an [`Arity`] rather than being fixed by the type. Every one of those
    /// ports must be driven — an `inputs=` list is positional and cannot have a hole.
    RefsIndexed {
        field: &'static str,
        prefix: &'static str,
        arity: Arity,
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
    /// Set instead of [`Self::output_ports`] when the output count is per-instance.
    pub dynamic_outputs: Option<DynamicOutputs>,
    /// This family emits **one vector-valued signal** under the block's own name rather than
    /// one separately-addressable signal per output.
    ///
    /// True for `kind=statespace`/`discretestatespace`, verified against
    /// `general-mna/src/system_builder.rs`: those two arms build their `BlockInstance` with no
    /// `output_names` at all (there is no `outputs=` field to parse), and
    /// `BlockKind::StateSpace`'s own doc states `outputs() > 1` produces a
    /// `SignalValue::Vector`. The GSF v2 contract still draws `out0..out{q-1}` pins, so a
    /// document *can* wire the second one — and referencing it would silently mean "the whole
    /// vector" instead. [`super::Scope::ref_name`] rejects that rather than emitting a netlist
    /// that parses and means something else.
    pub vector_output: bool,
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
            dynamic_outputs: None,
            vector_output: false,
            structural: false,
        }
    }

    /// This block's output port ids in declaration order — the static list, or the
    /// `<prefix><i>` ids a [`DynamicOutputs`] arity generates for this particular instance.
    pub fn output_port_ids(&self, block: &Block) -> Vec<String> {
        match &self.dynamic_outputs {
            Some(dynamic) => {
                let n = dynamic.arity.count(block).unwrap_or(0);
                (0..n).map(|i| format!("{}{i}", dynamic.prefix)).collect()
            }
            None => self.output_ports.iter().map(|p| (*p).to_string()).collect(),
        }
    }

    /// The suffix used to build the emitted name of output `index` (never used for index 0,
    /// which always aliases the block's own card name): the declared output name where the
    /// family has a `names_param`, else the port id itself.
    pub fn output_suffix(&self, block: &Block, index: usize, port: &str) -> String {
        let declared = self
            .dynamic_outputs
            .as_ref()
            .and_then(|d| d.names_param)
            .and_then(|p| block.params.get(p))
            .and_then(Value::as_array)
            .and_then(|names| names.get(index))
            .map(render_value);
        match declared {
            Some(name) if !name.is_empty() => name,
            _ => port.to_string(),
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

/// `NAME kind=<kind> inputs=<in ports, in order> outputs=<the three output names>` — the
/// whole shape of every coordinate-transform card.
macro_rules! transform {
    ($type_name:literal, $kind:literal, $inputs:expr, $outputs:expr) => {
        TypeSpec {
            kind: Some(KindSpec {
                kind: KindName::Fixed($kind),
                fields: &[
                    FieldSpec::Refs {
                        field: "inputs",
                        ports: $inputs,
                    },
                    FieldSpec::Outputs { field: "outputs" },
                ],
            }),
            output_ports: $outputs,
            ..TypeSpec::empty($type_name)
        }
    };
}

/// A script-hosted block (`cscript`/`pyblock`/`pyfunc`/`octfunc`/`octblock`): the given
/// source-locating fields, then the `inputs=`/`outputs=` lists whose lengths are this
/// instance's own `input_names`/`output_names` parameters — exactly the arity the GSF v2
/// fixtures' `for_each` port emission uses.
macro_rules! script_block {
    ($type_name:literal, $kind:literal, [$($field:expr),* $(,)?]) => {
        TypeSpec {
            kind: Some(KindSpec {
                kind: KindName::Fixed($kind),
                fields: &[
                    $($field,)*
                    FieldSpec::RefsIndexed {
                        field: "inputs",
                        prefix: "in",
                        arity: Arity::ParamLen("input_names"),
                    },
                    FieldSpec::Outputs { field: "outputs" },
                ],
            }),
            dynamic_outputs: Some(DynamicOutputs {
                prefix: "out",
                arity: Arity::ParamLen("output_names"),
                names_param: Some("output_names"),
            }),
            ..TypeSpec::empty($type_name)
        }
    };
}

const fn req(field: &'static str, param: &'static str) -> FieldSpec {
    FieldSpec::Param {
        field,
        param,
        required: true,
    }
}

const fn opt(field: &'static str, param: &'static str) -> FieldSpec {
    FieldSpec::Param {
        field,
        param,
        required: false,
    }
}

const TWO_PIN: &[&str] = &["p1", "p2"];
const SOURCE_PINS: &[&str] = &["p", "n"];
const ABC: &[&str] = &["a", "b", "c"];
const ALPHA_BETA_ZERO: &[&str] = &["alpha", "beta", "zero"];
const DQ_ZERO_THETA: &[&str] = &["d", "q", "zero", "theta"];
const DQ_ZERO_OUT: &[&str] = &["d", "q", "zero_out"];

/// Every type the compiler knows how to emit.
///
/// # `in=` versus `inputs=`
///
/// These are **not** interchangeable in `general-mna`. Its `build_kind` reads one or the other
/// per family, from `fields` directly: `gain`/`vco`/`saturation`/`hysteresis`/`pwm`/`tf`/
/// `discretetf`/`sig2phys` take a single `in=<signal>`; `pspwm`/`pmsm`/the six coordinate
/// transforms/`product` take a comma-joined `inputs=`; `statespace`/`discretestatespace`/the
/// script-hosted families accept `inputs=` with `in=` as the one-input special case. Getting
/// this wrong produces `device '<name>' missing field 'in'` at load time, not a silent
/// mis-simulation -- but it is still a netlist that never runs, so each entry below names its
/// source arm.
///
/// Seeded with exactly the set a switching-converter acceptance circuit needs (that circuit
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
                // `kind=pwm` reads a *single* `in=` duty signal (only `pspwm` takes a
                // three-entry `inputs=` list), and the GSF v2 fixture's input pin is `in`.
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
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
                req("k", "k"),
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
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
                // A single *symmetric* bound (`|y| <= limit`), not a lower/upper pair --
                // `BlockKind::Saturation`'s netlist form is
                // `NAME kind=saturation in=<signal> limit=<f64>`.
                req("limit", "limit"),
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
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
                req("f_min", "f_min"),
                req("f_max", "f_max"),
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
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
                req("low", "low"),
                req("high", "high"),
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("logic.hysteresis")
    },
    // -- linear dynamics: state-space and transfer function ------------------
    //
    // Field spellings read out of `general-mna/src/system_builder.rs`'s own `"statespace"` /
    // `"tf"` / `"discretestatespace"` / `"discretetf"` arms, not from the fixtures alone:
    //   * matrices are Python list literals (`a=[[1,2],[3,4]]`) -- `parse_matrix_rows` strips
    //     one layer of brackets and splits on *top-level* commas, which is exactly what
    //     `render_value` already produces for a nested JSON array;
    //   * `b=`/`c=` also accept a flat vector as SISO shorthand, and `d=` is optional
    //     (defaulting to an all-zero `q x p` matrix), so neither is forced here;
    //   * inputs go through `inputs=<sig,...>`; `in=` is that same field's one-input special
    //     case, so the list form covers both;
    //   * there is deliberately **no** `outputs=` field on either state-space arm -- see
    //     `TypeSpec::vector_output`.
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("statespace"),
            fields: &[
                req("a", "a"),
                req("b", "b"),
                req("c", "c"),
                opt("d", "d"),
                FieldSpec::RefsIndexed {
                    field: "inputs",
                    prefix: "in",
                    arity: Arity::MatrixCols("b"),
                },
            ],
        }),
        output_ports: &["out0"],
        vector_output: true,
        ..TypeSpec::empty("control.state_space")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("discretestatespace"),
            fields: &[
                req("a", "a"),
                req("b", "b"),
                req("c", "c"),
                opt("d", "d"),
                // `parse_required_periodic_sample_time`: mandatory here, unlike the optional
                // `ts=` on cscript/pyblock/pyfunc.
                req("ts", "ts"),
                FieldSpec::RefsIndexed {
                    field: "inputs",
                    prefix: "in",
                    arity: Arity::MatrixCols("b"),
                },
            ],
        }),
        output_ports: &["out0"],
        vector_output: true,
        ..TypeSpec::empty("control.discrete_state_space")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("tf"),
            fields: &[
                req("num", "num"),
                req("den", "den"),
                // Genuinely SISO: the `"tf"` arm reads `in=` only, with no `inputs=` branch.
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("control.transfer_function")
    },
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("discretetf"),
            fields: &[
                req("num", "num"),
                req("den", "den"),
                req("ts", "ts"),
                FieldSpec::Ref {
                    field: "in",
                    port: "in",
                },
            ],
        }),
        output_ports: &["out"],
        ..TypeSpec::empty("control.discrete_transfer_function")
    },
    // -- coordinate transforms ----------------------------------------------
    //
    // `KindName::PerVariant` turns out **not** to be what this family needs: GSF v2 already
    // gives each transform its own dotted type (`control.transform.clarke`, ...) with its own
    // port names, and `system_builder.rs` matches
    // `"clarke" | "clarkeinv" | "park" | "parkinv" | "clarkepark" | "clarkeparkinv"` as six
    // distinct `kind=` strings with no selector parameter anywhere. So the mapping is one
    // fixed `kind=` per type -- a plain `KindName::Fixed`.
    //
    // Input order is `CoordinateTransform::call`'s own positional order
    // (`general-simulator/crates/continuous-blocks/src/coordinate_transforms.rs`), which
    // matches each fixture's left-side port order exactly. `outputs=` is always emitted (all
    // six have exactly three outputs), because the implicit default names come from
    // `CoordinateTransform::output_names()`, not from GSF v2's port ids -- and the two differ:
    // `control.transform.park`'s third *output* port is `zero_out`, its `zero` id being taken
    // by an input. Leaving the names implicit would make document port and netlist symbol
    // disagree.
    transform!("control.transform.clarke", "clarke", ABC, ALPHA_BETA_ZERO),
    transform!(
        "control.transform.clarke_inv",
        "clarkeinv",
        ALPHA_BETA_ZERO,
        ABC
    ),
    transform!(
        "control.transform.park",
        "park",
        &["alpha", "beta", "zero", "theta"],
        DQ_ZERO_OUT
    ),
    transform!(
        "control.transform.park_inv",
        "parkinv",
        DQ_ZERO_THETA,
        &["alpha", "beta", "zero_out"]
    ),
    transform!(
        "control.transform.clarke_park",
        "clarkepark",
        &["a", "b", "c", "theta"],
        &["d", "q", "zero"]
    ),
    transform!(
        "control.transform.clarke_park_inv",
        "clarkeparkinv",
        DQ_ZERO_THETA,
        ABC
    ),
    // -- machines -----------------------------------------------------------
    TypeSpec {
        kind: Some(KindSpec {
            kind: KindName::Fixed("pmsm"),
            fields: &[
                req("r_s", "r_s"),
                req("l_d", "l_d"),
                req("l_q", "l_q"),
                req("lambda_pm", "lambda_pm"),
                req("pole_pairs", "pole_pairs"),
                req("inertia", "inertia"),
                req("friction", "friction"),
                // Exactly three, in this order -- system_builder.rs rejects any other count
                // with "kind='pmsm' needs 3 inputs (vd,vq,t_load)".
                FieldSpec::Refs {
                    field: "inputs",
                    ports: &["vd", "vq", "t_load"],
                },
                FieldSpec::Outputs { field: "outputs" },
            ],
        }),
        output_ports: &["id", "iq", "omega_m", "theta_e"],
        ..TypeSpec::empty("machine.pmsm")
    },
    // -- script-hosted blocks -----------------------------------------------
    //
    // Every field re-read out of `system_builder.rs`'s own arms rather than carried over from
    // `.cir` files skimmed earlier. The one real surprise: **`kind=cscript` takes `lib=`, not
    // `path=`** -- it names a precompiled shared object, not a source file. The GSF v2 fixture
    // agrees; its parameter really is `lib`. Everything else in the family takes `path=`, and
    // `pyfunc`/`octfunc`/`octblock` additionally require `function=`. `xc_count=` exists only
    // on the three *stateful* arms (`cscript`/`pyblock`/`octblock`); `pyfunc`/`octfunc` never
    // parse it, so it is not emitted for them. `ts=` is optional throughout (`ts=variable` is
    // accepted by the stateful three and rejected by the stateless two -- a distinction the
    // simulator enforces on the value, which passes through here verbatim).
    script_block!(
        "extensibility.cscript",
        "cscript",
        [
            req("lib", "lib"),
            opt("ts", "ts"),
            opt("xc_count", "xc_count")
        ]
    ),
    script_block!(
        "extensibility.pyblock",
        "pyblock",
        [
            req("path", "path"),
            opt("ts", "ts"),
            opt("xc_count", "xc_count")
        ]
    ),
    script_block!(
        "extensibility.pyfunc",
        "pyfunc",
        [
            req("path", "path"),
            req("function", "function"),
            opt("ts", "ts")
        ]
    ),
    script_block!(
        "extensibility.octfunc",
        "octfunc",
        [
            req("path", "path"),
            req("function", "function"),
            opt("ts", "ts")
        ]
    ),
    script_block!(
        "extensibility.octblock",
        "octblock",
        [
            req("path", "path"),
            req("function", "function"),
            opt("ts", "ts"),
            opt("xc_count", "xc_count")
        ]
    ),
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
        // A switch instance conventionally named DEV/DEVA/DEVB already starts with 'D'.
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
