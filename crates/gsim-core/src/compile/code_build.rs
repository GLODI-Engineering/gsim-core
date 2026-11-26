//! Resolving a sheet's build plan against the sheet's own parameters — the half of
//! [`docs/decisions/0009`](../../../../docs/decisions/0009-a-document-carried-build-for-code-blocks.md)
//! that has teeth.
//!
//! The record's central claim is that a code block's `-D` constants and the numbers on its
//! circuit's element cards are *the same numbers*. That claim is only true if one pass over one
//! dictionary produces both, which is why this runs inside [`super::compile_with`] and why its
//! output is a field of [`super::CompileOutput`] rather than something a caller asks for
//! separately. A caller cannot hold a deck without holding the build that belongs to it.
//!
//! And the converse, which is the point of the whole feature: when a define cannot be derived,
//! this returns `Err`, `compile_with` propagates it, and **there is no netlist either**. The
//! refusal is not a build step declining to run — that can be skipped, retried past, or
//! softened into a warning. It is the deck failing to exist. A build that quietly omitted a
//! define would produce a block whose constants disagree with the circuit it is wired into, and
//! the run would then report a confident wrong answer, which is
//! `GLODI-Engineering/omni-schematics#235` caused by the feature meant to end it.
//!
//! Nothing here names a compiler, a flag spelling or a platform library suffix. Those are
//! statements about a machine and belong to whoever spawns the process (0009's "interpret the
//! binding, carry the toolchain").

use super::CompileError;
use crate::document::{CodeBuild, CodeDefine, Model};
use crate::fmt::stringify;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The one step kind this version implements. 0009 §10 keeps the field open; an unrecognised
/// kind is refused rather than skipped, because a skipped step leaves an artifact nobody built.
pub const SHARED_LIBRARY: &str = "shared-library";

/// The languages a `shared-library` step may declare. Not a compiler list — these say what the
/// *source* is, and which driver compiles it is the builder's business.
pub const LANGUAGES: [&str; 2] = ["c", "c++"];

/// The only block type that may name a build step in this version. A `pyblock` or an `octfunc`
/// is interpreted, not compiled; 0009 §10 records that its constants problem is real and is
/// answered by a future step kind rather than by pretending a Python file is compiled.
const CSCRIPT: &str = "extensibility.cscript";

/// One product of a build, with every path as the document wrote it and every define resolved
/// to the value the netlist carries for the same parameter.
///
/// Serialized for the bindings, so the editor receives exactly this and does not re-derive any
/// of it — the same rule `super::provenance` follows, and for the same reason.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedStep {
    pub id: String,
    pub kind: String,
    pub lang: String,
    /// Relative to the schematic's own directory, exactly as written (0005: a path is emitted
    /// as the user spelled it, because this crate does not know where the build will run).
    pub sources: Vec<String>,
    pub includes: Vec<String>,
    /// `NAME=VALUE` tokens in 0009 §7's canonical order. The builder prefixes its compiler's
    /// own define flag; this list is deliberately flagless.
    pub defines: Vec<String>,
    /// The names of `optional` defines whose parameter was absent, in the same order. Carried
    /// so that "built without this one on purpose" and "built without it because nobody
    /// noticed" are different values in the build record (0009 §7.5).
    pub omitted_defines: Vec<String>,
    pub flags: Vec<String>,
    pub libs: Vec<String>,
    pub needs: Vec<String>,
    /// The artifact, taken from the `lib` parameter of the block(s) this step serves — the one
    /// place a sheet names it (0009 §4). A stem with no platform suffix resolves to the
    /// platform's own library name, which `general-simulator#125` settled.
    pub artifact: String,
    /// Which drawn blocks this step builds for. More than one is legitimate and means they
    /// share a library.
    pub blocks: Vec<String>,
    /// `Some` exactly when one of this step's bound defines resolved to a structured value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<ResolvedHeader>,
}

/// The generated constants header for a step whose bound defines include a structured value —
/// a lookup table, an array, a struct initialiser — which `-D` carries badly or not at all
/// (`docs/decisions/0009` §3).
///
/// **A build output, not a source.** It sits beside the artifact, is rewritten every run and is
/// meant to be gitignored; it must never be written into the user's own source folder, where it
/// would look like something they wrote and would be stale the moment a parameter changed.
///
/// This crate says what it contains and where it goes *relative to the schematic*. It does not
/// write the file, and it does not know which of `-include` and `/FI` forces it onto a compile
/// line — that is a statement about a compiler, and the builder's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedHeader {
    /// Beside the artifact, derived from the block's own `lib` so that no machine path is baked
    /// into anything.
    pub path: String,
    /// Whole `#define NAME { ... }` lines, in the canonical order of §7. Compared by **content**
    /// rather than by the file's timestamp: the header is rewritten every run, so an mtime
    /// check would be vacuous and a changed lookup table would be an invisible staleness.
    pub defines: Vec<String>,
}

/// A sheet's build, in dependency order: a step appears after everything it `needs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedCodeBuild {
    pub steps: Vec<ResolvedStep>,
}

/// Resolves `model.code_build`, or refuses.
///
/// `Ok(None)` means the sheet carries no plan — every document written before 0009, and every
/// sheet whose code blocks the user builds by hand.
pub fn resolve(model: &Model) -> Result<Option<ResolvedCodeBuild>, CompileError> {
    let plan = match &model.code_build {
        Some(plan) => plan,
        None => {
            // A block naming a step on a sheet with no plan is the one thing that must not pass
            // silently here: it is a user who wrote half the feature, and leaving it alone
            // means building nothing while the document says otherwise.
            if let Some(block) = model.blocks.iter().find(|b| b.build.is_some()) {
                return Err(CompileError::BadCodeBuild {
                    at: format!("block '{}'", block.id),
                    reason: format!(
                        "names build step '{}', but this sheet has no 'code_build' plan to find it in",
                        block.build.as_deref().unwrap_or_default()
                    ),
                });
            }
            return Ok(None);
        }
    };

    let mut seen = BTreeSet::new();
    for step in &plan.steps {
        if !seen.insert(step.id.as_str()) {
            return Err(CompileError::BadCodeBuild {
                at: format!("build step '{}'", step.id),
                reason: "two steps claim this id, so a block naming it names both".to_string(),
            });
        }
    }

    // Which blocks each step serves, and what artifact they agree on.
    let mut claims: BTreeMap<&str, (Vec<String>, String)> = BTreeMap::new();
    for block in &model.blocks {
        let Some(step_id) = block.build.as_deref() else {
            continue;
        };
        let resolved_type = super::spec::lookup(&block.r#type).map(|s| s.type_name);
        if resolved_type != Some(CSCRIPT) {
            return Err(CompileError::BadCodeBuild {
                at: format!("block '{}'", block.id),
                reason: format!(
                    "has type '{}', which is not compiled -- only an {CSCRIPT} block may name a build step",
                    block.r#type
                ),
            });
        }
        let step = plan.step(step_id).ok_or_else(|| CompileError::BadCodeBuild {
            at: format!("block '{}'", block.id),
            reason: format!(
                "names build step '{step_id}', which this sheet's plan does not define (it defines: {})",
                step_ids(plan)
            ),
        })?;
        let lib = block
            .params
            .get("lib")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if lib.is_empty() {
            return Err(CompileError::BadCodeBuild {
                at: format!("block '{}'", block.id),
                reason: format!(
                    "names build step '{step_id}' but has no 'lib', so the step has nowhere to write its artifact"
                ),
            });
        }
        check_path(&lib, &format!("block '{}' lib", block.id))?;
        if let Some(source) = block.source.as_deref() {
            check_path(source, &format!("block '{}' source", block.id))?;
            if !step.sources.iter().any(|s| s == source) {
                return Err(CompileError::BadCodeBuild {
                    at: format!("block '{}'", block.id),
                    reason: format!(
                        "its source '{source}' is not among the sources build step '{step_id}' compiles ({}), so one of the two is out of date",
                        step.sources.join(", ")
                    ),
                });
            }
        }
        match claims.get_mut(step_id) {
            Some((blocks, agreed)) => {
                if *agreed != lib {
                    return Err(CompileError::BadCodeBuild {
                        at: format!("build step '{step_id}'"),
                        reason: format!(
                            "blocks '{}' and '{}' both name it but want its artifact in different places ('{agreed}' and '{lib}')",
                            blocks.join("', '"),
                            block.id
                        ),
                    });
                }
                blocks.push(block.id.clone());
            }
            None => {
                claims.insert(step_id, (vec![block.id.clone()], lib));
            }
        }
    }

    let order = order_steps(plan)?;
    let mut steps = Vec::with_capacity(order.len());
    for step_id in order {
        let step = plan
            .step(step_id)
            .expect("order_steps only yields real ids");
        let Some((blocks, artifact)) = claims.get(step_id) else {
            // A step nothing builds for and nothing depends on is a step that would never run.
            // Refused rather than dropped: a user who wrote it meant something by it.
            return Err(CompileError::BadCodeBuild {
                at: format!("build step '{step_id}'"),
                reason:
                    "no block names it and no other step needs it, so nothing would ever run it"
                        .to_string(),
            });
        };
        if step.kind != SHARED_LIBRARY {
            return Err(CompileError::BadCodeBuild {
                at: format!("build step '{step_id}'"),
                reason: format!(
                    "has kind '{}', which this version does not build -- it knows only '{SHARED_LIBRARY}'. It is refused rather than skipped, because skipping it would leave an artifact nobody built",
                    step.kind
                ),
            });
        }
        let lang = step.lang.as_deref().unwrap_or("").trim();
        if !LANGUAGES.contains(&lang) {
            return Err(CompileError::BadCodeBuild {
                at: format!("build step '{step_id}'"),
                reason: format!(
                    "declares lang '{lang}', which is not one of {} -- and it has to say, because guessing it from a file extension is a guess in a feature whose subject is not guessing",
                    LANGUAGES.join(", ")
                ),
            });
        }
        if step.sources.is_empty() {
            return Err(CompileError::BadCodeBuild {
                at: format!("build step '{step_id}'"),
                reason: "has no sources, so there is nothing to compile".to_string(),
            });
        }
        for source in &step.sources {
            check_path(source, &format!("build step '{step_id}' source"))?;
        }
        let includes = joined(&plan.includes, &step.includes);
        for include in &includes {
            check_path(include, &format!("build step '{step_id}' include"))?;
        }
        let Resolved {
            defines,
            omitted_defines,
            header_defines,
        } = resolve_defines(model, plan, step_id, &step.defines)?;
        let header = (!header_defines.is_empty()).then(|| ResolvedHeader {
            path: header_path(artifact),
            defines: header_defines,
        });
        steps.push(ResolvedStep {
            id: step.id.clone(),
            kind: step.kind.clone(),
            lang: lang.to_string(),
            sources: step.sources.clone(),
            includes,
            defines,
            omitted_defines,
            flags: joined(&plan.flags, &step.flags),
            libs: joined(&plan.libs, &step.libs),
            needs: step.needs.clone(),
            artifact: artifact.clone(),
            blocks: blocks.clone(),
            header,
        });
    }

    Ok(Some(ResolvedCodeBuild { steps }))
}

fn step_ids(plan: &CodeBuild) -> String {
    if plan.steps.is_empty() {
        return "none".to_string();
    }
    plan.steps
        .iter()
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn joined(plan_level: &[String], step_level: &[String]) -> Vec<String> {
    let mut out = plan_level.to_vec();
    out.extend(step_level.iter().cloned());
    out
}

/// Plan-level `needs` are not a thing, so this is a topological sort over the step list alone.
/// Order matters to a caller: a step that generates a header has to run before the step that
/// includes it, and the one thing a format can do about that is hand the steps over already
/// sorted rather than leaving every consumer to sort them the same way.
fn order_steps(plan: &CodeBuild) -> Result<Vec<&str>, CompileError> {
    let known: BTreeSet<&str> = plan.steps.iter().map(|s| s.id.as_str()).collect();
    for step in &plan.steps {
        for need in &step.needs {
            if !known.contains(need.as_str()) {
                return Err(CompileError::BadCodeBuild {
                    at: format!("build step '{}'", step.id),
                    reason: format!(
                        "needs '{need}', which this sheet's plan does not define (it defines: {})",
                        step_ids(plan)
                    ),
                });
            }
        }
    }

    let mut done: Vec<&str> = Vec::with_capacity(plan.steps.len());
    let mut settled: BTreeSet<&str> = BTreeSet::new();
    while done.len() < plan.steps.len() {
        let mut progressed = false;
        for step in &plan.steps {
            if settled.contains(step.id.as_str()) {
                continue;
            }
            if step
                .needs
                .iter()
                .all(|n| settled.contains(n.as_str()) || n == &step.id)
            {
                // A step needing itself is a cycle of one and is caught below, not here.
                if step.needs.iter().any(|n| n == &step.id) {
                    continue;
                }
                settled.insert(step.id.as_str());
                done.push(step.id.as_str());
                progressed = true;
            }
        }
        if !progressed {
            let stuck: Vec<&str> = plan
                .steps
                .iter()
                .map(|s| s.id.as_str())
                .filter(|id| !settled.contains(id))
                .collect();
            return Err(CompileError::BadCodeBuild {
                at: "this sheet's build plan".to_string(),
                reason: format!(
                    "these steps depend on each other in a circle, so none of them can run first: {}",
                    stuck.join(", ")
                ),
            });
        }
    }
    Ok(done)
}

/// 0009 §7: plan-level defines in document order, then the step's own, a repeated `name`
/// replacing the earlier entry in place rather than adding a second `-D`.
/// What one step's `defines` resolve to: the `-D` tokens, the names the author declared absent,
/// and the `#define` lines a structured constant needs instead of a `-D`.
struct Resolved {
    defines: Vec<String>,
    omitted_defines: Vec<String>,
    header_defines: Vec<String>,
}

fn resolve_defines(
    model: &Model,
    plan: &CodeBuild,
    step_id: &str,
    step_defines: &[CodeDefine],
) -> Result<Resolved, CompileError> {
    reject_duplicates(&plan.defines, "this sheet's build plan")?;
    reject_duplicates(step_defines, &format!("build step '{step_id}'"))?;

    let mut order: Vec<&str> = Vec::new();
    let mut effective: BTreeMap<&str, &CodeDefine> = BTreeMap::new();
    for define in plan.defines.iter().chain(step_defines.iter()) {
        if effective.insert(define.name.as_str(), define).is_none() {
            order.push(define.name.as_str());
        }
    }

    let mut defines = Vec::new();
    let mut omitted = Vec::new();
    let mut header_defines = Vec::new();
    for name in order {
        let define = effective[name];
        // The shape of the resolved value is the *only* thing that decides which transport
        // carries it (0009 §3). One list, one pass, one set of refusals -- so an author's
        // choice of shape cannot make their build less safe, because they never chose a
        // transport at all.
        match resolve_one(model, define)? {
            Some(Carried::Scalar(value)) => defines.push(format!("{}={value}", define.name)),
            Some(Carried::Structured(initialiser)) => {
                header_defines.push(format!("#define {} {initialiser}", define.name));
            }
            None => omitted.push(define.name.clone()),
        }
    }
    Ok(Resolved {
        defines,
        omitted_defines: omitted,
        header_defines,
    })
}

/// How a resolved value gets to the block: on the compile line, or through the header.
enum Carried {
    Scalar(String),
    Structured(String),
}

/// The generated header sits beside the artifact and is named after it, so a sheet with two
/// built blocks gets two headers and neither has to be told where to go. Derived from `lib`
/// rather than declared, because a second place to write the path is a second place for it to
/// drift -- the same reason a step has no `output` (§4).
fn header_path(artifact: &str) -> String {
    let stem = artifact
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(artifact)
        .trim_end_matches(".so")
        .trim_end_matches(".dylib")
        .trim_end_matches(".dll");
    match artifact.rfind(['/', '\\']) {
        Some(cut) => format!("{}/{stem}_gsim_constants.h", &artifact[..cut]),
        None => format!("{stem}_gsim_constants.h"),
    }
}

/// A JSON array becomes a C brace initialiser, nesting as it goes — which is what an array, a
/// 2-D table and a struct initialiser all want. A non-scalar *inside* an array (an object, or
/// null) has no C spelling and is refused by the caller rather than rendered as something.
fn brace_initialiser(values: &[Value], define: &CodeDefine) -> Result<String, CompileError> {
    let mut parts = Vec::with_capacity(values.len());
    for value in values {
        match value {
            Value::Array(inner) => parts.push(brace_initialiser(inner, define)?),
            Value::Number(_) | Value::Bool(_) | Value::String(_) => parts.push(stringify(value)),
            other => {
                return Err(CompileError::BadCodeBuild {
                    at: format!("define '{}'", define.name),
                    reason: format!(
                        "contains {other}, which has no C spelling -- a structured constant is a number, or an array of them"
                    ),
                })
            }
        }
    }
    Ok(format!("{{ {} }}", parts.join(", ")))
}

fn reject_duplicates(defines: &[CodeDefine], at: &str) -> Result<(), CompileError> {
    let mut seen = BTreeSet::new();
    for define in defines {
        if !seen.insert(define.name.as_str()) {
            return Err(CompileError::BadCodeBuild {
                at: at.to_string(),
                reason: format!(
                    "defines '{}' twice in one list; a step may override a plan-level define, but a list may not disagree with itself",
                    define.name
                ),
            });
        }
    }
    Ok(())
}

/// `Ok(None)` means an `optional` define whose parameter is absent — the author's declared
/// absence, and the only absence this function will report without refusing.
fn resolve_one(model: &Model, define: &CodeDefine) -> Result<Option<Carried>, CompileError> {
    if !is_identifier(&define.name) {
        return Err(CompileError::BadCodeBuild {
            at: format!("define '{}'", define.name),
            reason: "is not an identifier, so it cannot be a macro name".to_string(),
        });
    }
    match (&define.from, &define.literal) {
        (Some(_), Some(_)) => Err(CompileError::BadCodeBuild {
            at: format!("define '{}'", define.name),
            reason: "carries both 'from' and 'literal'; a define has one source of truth or it has the drift this format exists to prevent".to_string(),
        }),
        (None, None) => Err(CompileError::BadCodeBuild {
            at: format!("define '{}'", define.name),
            reason: "carries neither 'from' (a model parameter) nor 'literal' (a constant the author writes out), so it has no value".to_string(),
        }),
        (None, Some(literal)) => carry(literal, define).map(Some),
        (Some(param), None) => resolve_from_param(model, define, param).map(Some),
    }
    .or_else(|err| {
        // An `optional` define forgives exactly one thing: its parameter not being there.
        // Everything else about it is still wrong, and still refused.
        match (&err, define.optional) {
            (CompileError::UnderivableDefine { reason, .. }, true)
                if reason.starts_with(ABSENT) =>
            {
                Ok(None)
            }
            _ => Err(err),
        }
    })
}

/// The prefix on the one reason `optional` forgives, and nothing else does. A prefix rather
/// than the whole string, because the sentence goes on to list the parameters the sheet *does*
/// declare — which is what turns a misspelt `from` from a dead end into a one-line fix, and is
/// the same courtesy `docs/decisions/0005` settled for a capture whose column is not there.
const ABSENT: &str = "absent";

fn resolve_from_param(
    model: &Model,
    define: &CodeDefine,
    param: &str,
) -> Result<Carried, CompileError> {
    if param.contains('.') {
        return Err(CompileError::BadCodeBuild {
            at: format!("define '{}'", define.name),
            reason: format!(
                "derives from '{param}', and a dotted name is reserved for binding a define to a drawn component's own parameter, which no version supports yet. A define derives from a model parameter, by its bare name"
            ),
        });
    }
    let Some(entry) = model.params.iter().find(|p| p.name == param) else {
        let declared = if model.params.is_empty() {
            "this sheet declares no parameters at all".to_string()
        } else {
            format!(
                "this sheet declares {}, named {}",
                model.params.len(),
                model
                    .params
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(CompileError::UnderivableDefine {
            define: define.name.clone(),
            param: param.to_string(),
            reason: format!("{ABSENT} -- {declared}"),
        });
    };
    match &entry.value {
        Value::Number(_) | Value::Bool(_) | Value::Array(_) => carry(&entry.value, define),
        // Unreachable through `compile`, which resolves §5.1 references and §5.2 expressions
        // before any build is resolved (#94) -- so by the time this runs, a parameter's value is
        // a number, a bool or a structured constant. Kept, and kept fatal, for a caller that
        // reaches `resolve` with an unsubstituted model: emitting the text would put
        // `-DR={Rload * 2}` on a compile line, and emitting nothing would be the silent omission
        // this whole module exists to prevent.
        Value::String(text) => Err(CompileError::UnderivableDefine {
            define: define.name.clone(),
            param: param.to_string(),
            reason: format!(
                "its value is still the unresolved text '{text}'; a build is resolved from a model whose §5.2 expressions have been evaluated"
            ),
        }),
        other => Err(CompileError::UnderivableDefine {
            define: define.name.clone(),
            param: param.to_string(),
            reason: format!("its value is {other}, which is not a value a macro can expand to"),
        }),
    }
}

/// Decides a resolved value's transport from its shape, and only from its shape.
fn carry(value: &Value, define: &CodeDefine) -> Result<Carried, CompileError> {
    match value {
        Value::Number(_) | Value::Bool(_) | Value::String(_) => {
            Ok(Carried::Scalar(stringify(value)))
        }
        Value::Array(values) => brace_initialiser(values, define).map(Carried::Structured),
        other => Err(CompileError::BadCodeBuild {
            at: format!("define '{}'", define.name),
            reason: format!("has the value {other}, which is not a value a macro can expand to"),
        }),
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A build description is the one part of a document that makes something *run*, so a path in
/// it may not leave the sheet's own tree. Refused rather than normalised: a sheet that meant
/// `../shared/law.c` should say so in a plan whose root the user chose, and silently rewriting
/// it would be this crate guessing where the build will happen (0005).
fn check_path(path: &str, at: &str) -> Result<(), CompileError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(CompileError::BadCodeBuild {
            at: at.to_string(),
            reason: "is empty".to_string(),
        });
    }
    let absolute = trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || trimmed
            .as_bytes()
            .get(1)
            .is_some_and(|c| *c == b':' && trimmed.as_bytes()[0].is_ascii_alphabetic());
    if absolute {
        return Err(CompileError::BadCodeBuild {
            at: at.to_string(),
            reason: format!(
                "'{trimmed}' is an absolute path, which breaks the moment the project is cloned, opened over a remote or shared -- a build path is relative to the schematic"
            ),
        });
    }
    if trimmed.split(['/', '\\']).any(|segment| segment == "..") {
        return Err(CompileError::BadCodeBuild {
            at: at.to_string(),
            reason: format!(
                "'{trimmed}' climbs out of the schematic's own directory, and a build description may not reach outside the tree it travels in"
            ),
        });
    }
    Ok(())
}
