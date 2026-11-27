//! A number carries the form the library declares for it, from the moment it enters a document.
//!
//! # The defect
//!
//! `save_document` is a parse followed by a serialize, so it writes back whatever number form
//! the document it was handed carried (#78/#79). That is right for a value this crate has no
//! opinion about, and wrong for a value its own library *declares*:
//!
//! ```text
//! sheet.add("passive.resistor", id="load", params={"R": 5})   # a Python int
//! sheet.save()                                                # writes "R": 5
//! ```
//!
//! `passive.resistor` declares `R` as `number` — real-valued. The document now says integer,
//! and a Python reader of it is told the author meant an integer, because `json.loads`
//! distinguishes the two and nothing on the way through did. Rust and JavaScript widen both to
//! `f64` and cannot tell, which is why this went unnoticed in two of the three writers.
//!
//! # Why at entry rather than at write
//!
//! The owner's decision, in his words: *"enforce from the beginning the type of the number so
//! that the type may be attached to it."*
//!
//! Both this crate and the editor used to decide a number's form **at write time**, which means
//! every writer has to remember to ask, and a writer that forgets is silently wrong — there
//! were three writers and one of them was always going to forget. Attach the form where the
//! value is introduced and every writer is correct by construction: `save_document` stops
//! having a policy about numbers, because the number already knows.
//!
//! The second gain is where a bad value is reported. A fractional value for a parameter
//! declared `integer` used to be discovered at compile, or not at all; now it is refused at the
//! call that introduced it, naming the block and the parameter.
//!
//! # No new type was needed, which is a correction
//!
//! I first wrote on #107 that a parameter value would have to stop being a bare
//! `serde_json::Value`, and that this was a breaking change to the document model. That was
//! wrong. `serde_json::Value`'s `Number` is already either an integer or an `f64`, and the two
//! serialise differently — `Number::from_f64(48.0)` writes `48.0`, `Number::from(48i64)` writes
//! `48`, and a reserialised document keeps whichever it parsed. The representation the owner
//! asked for existed all along; what was missing is the lookup at the boundary.
//!
//! So this module coerces `Value`s in place and the document model is unchanged.
//!
//! # Three forms, because two would force a lie
//!
//! [`DeclaredForm::AsGiven`] is not a fallback, it is an answer. The library declares a
//! *container* for the 13 `vector` and `matrix` parameters — a state matrix is real, but the
//! declaration says `matrix` and nothing says what its elements are — and 11 of the document's
//! 27 numeric sites were deliberately left undeclared when #268 audited them: a model-level
//! parameter is whatever its author meant, a sweep's bounds take the form of whatever is swept,
//! a `.SUBCKT`'s parameters live in SPICE text, and a `CodeDefine.literal` is a constant only
//! its author can type. A two-valued type would force an answer at all 24 of those. Carrying
//! them as given is what keeps the honest cases honest, and it is what the editor's
//! `number-form.ts` already does by answering `undefined`.
//!
//! # What is not touched, on purpose
//!
//! **Strings.** A parameter's value may be a §5.3 expression — `"{Rload * 2}"` — and
//! substitution happens on the compile path, never on load, because `save_document` would
//! otherwise write the resolved number into the user's file and destroy the parameterisation
//! that makes one sheet serve a sweep (see [`crate::params`]). A string reaching this module
//! leaves unchanged.
//!
//! **A block's `ic` map, its `ratings`, and model-level params.** None of them is a declared
//! component parameter, so none has a form to attach. A capacitor's `IC` *is* declared as a
//! parameter and no card reads it, which is a different defect:
//! `GLODI-Engineering/omni-schematics#283`.

use serde_json::{Map, Number, Value};

use crate::compile::library::find_component;

/// What the library says a parameter's value is, or that it says nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredForm {
    /// `type: "number"` — real-valued. 143 of the library's parameters.
    Real,
    /// `type: "integer"`. 8 of them.
    Integer,
    /// `vector`, `matrix`, `list`, `enum`, `string`, an unknown component, an unknown parameter
    /// — anything the library does not pin to one numeric form. Carried through untouched.
    AsGiven,
}

/// A value whose declared form it cannot have.
#[derive(Debug, Clone, PartialEq)]
pub struct WrongForm {
    pub block: String,
    pub param: String,
    pub said: String,
}

impl std::fmt::Display for WrongForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "block '{}' parameter '{}': {}",
            self.block, self.param, self.said
        )
    }
}

impl std::error::Error for WrongForm {}

/// The form the library declares for one parameter of a block.
///
/// `fidelity` is taken and not used, and that is deliberate rather than an oversight. A
/// component declares its parameters twice — once in its own `params` for an inspector to lay
/// out, and once per `fidelities[].params` for the tier a block is placed at — so the honest
/// signature asks for the tier. It then reads only `params`, because **every one of the
/// library's tier parameters is also in its component's own list, with the same `type`**:
/// measured, 0 tier-only parameters and 0 type disagreements across all 78 components.
///
/// A tier branch would therefore be a branch no test can reach, and an untestable branch in a
/// coercion rule is worse than none: `AsGiven` is the answer when a lookup finds nothing, so a
/// broken tier lookup would not fail, it would quietly stop coercing. The agreement is asserted
/// instead — `the_two_halves_of_a_declaration_agree_about_type` below fails the day a tier
/// declares a parameter its component does not, or declares one differently, which is the day
/// this function needs the branch. (`gsim-core#93` is the same class of hand-maintained
/// agreement, for a tier's port offsets.)
pub fn declared_form(component: &str, _fidelity: &str, param: &str) -> DeclaredForm {
    let Some(found) = find_component(component) else {
        return DeclaredForm::AsGiven;
    };
    let declared = found
        .presentation
        .get("params")
        .and_then(|p| p.as_array())
        .and_then(|list| {
            // Both spellings: a component's own `params` says `name`, a tier says `id`, and a
            // declaration that is only half-found silently becomes `AsGiven`.
            list.iter().find(|p| {
                let named = p.get("name").or_else(|| p.get("id"));
                named.and_then(|n| n.as_str()) == Some(param)
            })
        });
    match declared
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
    {
        Some("number") => DeclaredForm::Real,
        Some("integer") => DeclaredForm::Integer,
        _ => DeclaredForm::AsGiven,
    }
}

/// `value` carrying `form`, or [`WrongForm`] when it cannot.
///
/// Only numbers move. A string may be an expression, a list may be a matrix row, and neither is
/// this function's business — see the module docs.
pub fn attach(
    form: DeclaredForm,
    block: &str,
    param: &str,
    value: Value,
) -> Result<Value, WrongForm> {
    let Value::Number(number) = &value else {
        return Ok(value);
    };
    match form {
        DeclaredForm::AsGiven => Ok(value),
        DeclaredForm::Real => {
            if number.is_f64() {
                return Ok(value);
            }
            // `as_f64` is `None` only for a `u64` above 2^53's exact range, which no component
            // parameter is; refusing is still better than rounding one silently.
            let widened = number
                .as_f64()
                .and_then(Number::from_f64)
                .ok_or_else(|| WrongForm {
                    block: block.to_string(),
                    param: param.to_string(),
                    said: format!("the library declares this real and {number} cannot be one"),
                })?;
            Ok(Value::Number(widened))
        }
        DeclaredForm::Integer => {
            if !number.is_f64() {
                return Ok(value);
            }
            let given = number.as_f64().unwrap_or(f64::NAN);
            if given.fract() != 0.0 || !given.is_finite() {
                return Err(WrongForm {
                    block: block.to_string(),
                    param: param.to_string(),
                    said: format!("the library declares this an integer and {number} is not one"),
                });
            }
            // Truncating silently is the failure this module exists to end, so only an exactly
            // integral real narrows -- `4.0` becomes `4`, and `4.5` is refused above.
            Ok(Value::Number(Number::from(given as i64)))
        }
    }
}

/// Every parameter of one block, carrying the form its component declares.
///
/// The one function the entry points call: `set_param` has a single parameter in hand, while
/// `add_block` and `load_document` have a whole table, and all three must agree about what the
/// library says.
pub fn attach_block_params(
    component: &str,
    fidelity: &str,
    block: &str,
    params: Map<String, Value>,
) -> Result<Map<String, Value>, WrongForm> {
    let mut attached = Map::new();
    for (name, value) in params {
        let form = declared_form(component, fidelity, &name);
        attached.insert(name.clone(), attach(form, block, &name, value)?);
    }
    Ok(attached)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_library_declares_a_resistance_real_and_a_count_integer() {
        // The two forms that exist, read out of the real library rather than a fixture: if the
        // catalogue stopped declaring types this would fail rather than quietly answer
        // `AsGiven` for everything, which is how a coercion gate becomes a no-op.
        assert_eq!(
            declared_form("passive.resistor", "ideal", "R"),
            DeclaredForm::Real
        );
        assert_eq!(
            declared_form("control.product", "ideal", "n_inputs"),
            DeclaredForm::Integer
        );
    }

    #[test]
    fn a_container_is_not_a_number_and_says_so() {
        // The 13 `vector`/`matrix` parameters. The declaration names the container; a state
        // matrix is real and the library does not say so, so inventing an answer here would be
        // the lie the third form exists to avoid.
        assert_eq!(
            declared_form("control.state_space", "ideal", "a"),
            DeclaredForm::AsGiven
        );
        assert_eq!(
            declared_form("extensibility.cscript", "ideal", "input_names"),
            DeclaredForm::AsGiven
        );
    }

    #[test]
    fn nothing_known_about_it_means_as_given() {
        assert_eq!(
            declared_form("no.such.component", "ideal", "R"),
            DeclaredForm::AsGiven
        );
        assert_eq!(
            declared_form("passive.resistor", "ideal", "no_such_param"),
            DeclaredForm::AsGiven
        );
    }

    #[test]
    fn the_two_halves_of_a_declaration_agree_about_type() {
        // Why `declared_form` reads only a component's own `params` and ignores the tier: today
        // the two halves never disagree, so a tier branch would be a branch no test can reach --
        // and an unreachable branch in a coercion rule does not fail when it breaks, it quietly
        // stops coercing, because `AsGiven` is what a failed lookup answers.
        //
        // This is the assertion that keeps that true. If a tier ever declares a parameter its
        // component does not, or declares one with a different type, this fails and whoever
        // added it is told that `declared_form` now needs to consult the tier.
        let mut tier_only = Vec::new();
        let mut disagree = Vec::new();
        let mut checked = 0;
        for component in crate::compile::library::library() {
            let own: std::collections::BTreeMap<&str, &str> = component
                .presentation
                .get("params")
                .and_then(|p| p.as_array())
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(|p| {
                    let name = p.get("name").or_else(|| p.get("id"))?.as_str()?;
                    Some((name, p.get("type").and_then(|t| t.as_str()).unwrap_or("")))
                })
                .collect();
            for tier in component
                .presentation
                .get("fidelities")
                .and_then(|f| f.as_array())
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let name = tier.get("tier").and_then(|t| t.as_str()).unwrap_or("?");
                for param in tier
                    .get("params")
                    .and_then(|p| p.as_array())
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                {
                    let Some(id) = param.get("id").or_else(|| param.get("name")) else {
                        continue;
                    };
                    let Some(id) = id.as_str() else { continue };
                    let declared = param.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    checked += 1;
                    match own.get(id) {
                        None => tier_only.push(format!("{} {name} {id}", component.id)),
                        Some(mine) if *mine != declared => disagree.push(format!(
                            "{} {name} {id}: {mine} vs {declared}",
                            component.id
                        )),
                        Some(_) => {}
                    }
                }
            }
        }
        // Before the two empty lists are believed.
        assert!(
            checked > 150,
            "only {checked} tier parameters were examined"
        );
        assert_eq!(
            tier_only,
            Vec::<String>::new(),
            "a tier declares its own parameter now"
        );
        assert_eq!(
            disagree,
            Vec::<String>::new(),
            "a tier disagrees about a type now"
        );
    }

    #[test]
    fn a_parameter_is_found_under_either_spelling() {
        // A component's own `params` says `name` and a tier says `id`; both are read, because a
        // declaration that is only half-found silently becomes `AsGiven`.
        assert_eq!(
            declared_form("passive.inductor", "ideal", "IC"),
            DeclaredForm::Real
        );
    }

    #[test]
    fn an_integer_written_for_a_real_parameter_becomes_a_real() {
        // The defect, in one line: a Python `5` for a parameter the library declares real.
        let got = attach(DeclaredForm::Real, "load", "R", json!(5)).unwrap();
        assert_eq!(serde_json::to_string(&got).unwrap(), "5.0");
    }

    #[test]
    fn a_real_stays_the_same_real_it_arrived_as() {
        for text in ["48.0", "1e-9", "0.1", "-0.0"] {
            let value: Value = serde_json::from_str(text).unwrap();
            let got = attach(DeclaredForm::Real, "b", "R", value.clone()).unwrap();
            assert_eq!(
                got.as_f64().unwrap().to_bits(),
                value.as_f64().unwrap().to_bits(),
                "{text} changed value, not just form"
            );
        }
    }

    #[test]
    fn an_exactly_integral_real_narrows_for_an_integer_parameter() {
        let got = attach(DeclaredForm::Integer, "g", "n_inputs", json!(4.0)).unwrap();
        assert_eq!(serde_json::to_string(&got).unwrap(), "4");
    }

    #[test]
    fn a_fractional_value_for_an_integer_parameter_is_refused_where_it_was_written() {
        // Truncating 4.5 to 4 is exactly the silent wrongness this module exists to end, and
        // the message carries the block and the parameter rather than a JSON pointer.
        let err = attach(DeclaredForm::Integer, "sum1", "n_inputs", json!(4.5)).unwrap_err();
        assert_eq!(err.block, "sum1");
        assert_eq!(err.param, "n_inputs");
        assert!(
            err.to_string()
                .contains("block 'sum1' parameter 'n_inputs'"),
            "{err}"
        );
        assert!(err.to_string().contains("4.5"), "{err}");
    }

    #[test]
    fn an_expression_is_left_alone_because_the_compile_path_resolves_it() {
        // Substituting on load would write the resolved number into the user's file and destroy
        // the parameterisation that makes one sheet serve a sweep (`crate::params`).
        for value in [
            json!("{Rload * 2}"),
            json!("1k"),
            json!(null),
            json!([1, 2]),
        ] {
            assert_eq!(
                attach(DeclaredForm::Real, "b", "R", value.clone()).unwrap(),
                value
            );
        }
    }

    #[test]
    fn as_given_moves_nothing_in_either_direction() {
        for value in [json!(5), json!(5.0), json!([[0]])] {
            assert_eq!(
                attach(DeclaredForm::AsGiven, "b", "a", value.clone()).unwrap(),
                value
            );
        }
    }

    #[test]
    fn a_whole_table_is_attached_at_once_and_untouched_keys_survive() {
        let mut params = Map::new();
        params.insert("R".to_string(), json!(5));
        params.insert("unknown".to_string(), json!(7));
        let got = attach_block_params("passive.resistor", "ideal", "load", params).unwrap();
        assert_eq!(serde_json::to_string(&got["R"]).unwrap(), "5.0");
        assert_eq!(serde_json::to_string(&got["unknown"]).unwrap(), "7");
    }
}
