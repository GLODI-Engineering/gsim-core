//! Pure evaluator for the `${{ ... }}` AST. No I/O, no mutation, no access to anything outside
//! the `Context` scope chain it is given — this is the entire trust boundary that makes it safe
//! to run an expression sourced from a third-party component definition.

use super::ast::{BinOp, Expr, UnOp};
use crate::fmt::stringify;
use regex::Regex;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct EvalError {
    pub message: String,
}

impl EvalError {
    fn new(msg: impl Into<String>) -> Self {
        EvalError {
            message: msg.into(),
        }
    }
}

/// A chain of variable scopes, innermost last. `for_each` pushes one scope per loop; the
/// outermost scope always holds `params`.
#[derive(Debug, Clone, Default)]
pub struct Context {
    scopes: Vec<Map<String, Value>>,
}

impl Context {
    pub fn new(params: Map<String, Value>) -> Self {
        let mut root = Map::new();
        root.insert("params".to_string(), Value::Object(params));
        Context { scopes: vec![root] }
    }

    /// Returns a new context with one additional scope layer bound on top — used for a single
    /// `for_each` iteration's `as`/`index` bindings. The parent's scopes remain visible.
    pub fn child(&self, bindings: Map<String, Value>) -> Context {
        let mut scopes = self.scopes.clone();
        scopes.push(bindings);
        Context { scopes }
    }

    fn lookup(&self, name: &str) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }
}

pub fn eval(expr: &Expr, ctx: &Context) -> Result<Value, EvalError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Var(path) => eval_var(path, ctx),
        Expr::Unary(op, inner) => eval_unary(op, eval(inner, ctx)?),
        Expr::Binary(op, lhs, rhs) => eval_binary(op, lhs, rhs, ctx),
        Expr::Call(name, args) if name == "if" => eval_if(args, ctx),
        Expr::Call(name, args) => {
            let values: Result<Vec<Value>, EvalError> = args.iter().map(|a| eval(a, ctx)).collect();
            eval_call(name, values?)
        }
    }
}

/// `if(cond, then, else)` short-circuits: only the taken branch is evaluated, so e.g.
/// `if(count(x) == 0, 0, at(x, 0))` never runs the out-of-bounds `at()` when `x` is empty. This
/// is why `if` is special-cased here rather than living in [`eval_call`], whose arguments are
/// always evaluated eagerly before it runs.
fn eval_if(args: &[Expr], ctx: &Context) -> Result<Value, EvalError> {
    let [cond, then_branch, else_branch] = args else {
        return Err(EvalError::new(format!(
            "if() expects 3 arguments (cond, then, else), got {}",
            args.len()
        )));
    };
    if as_bool(&eval(cond, ctx)?, "if() condition")? {
        eval(then_branch, ctx)
    } else {
        eval(else_branch, ctx)
    }
}

fn eval_var(path: &[String], ctx: &Context) -> Result<Value, EvalError> {
    let Some((head, rest)) = path.split_first() else {
        return Err(EvalError::new("empty variable path"));
    };
    let mut current = ctx
        .lookup(head)
        .ok_or_else(|| EvalError::new(format!("unknown variable '{head}'")))?
        .clone();
    for seg in rest {
        current = match current {
            Value::Object(map) => map
                .get(seg)
                .cloned()
                .ok_or_else(|| EvalError::new(format!("no field '{seg}' on {head}")))?,
            other => {
                return Err(EvalError::new(format!(
                    "cannot access field '{seg}' on non-object value {other}"
                )))
            }
        };
    }
    Ok(current)
}

fn as_number(v: &Value, ctx_desc: &str) -> Result<f64, EvalError> {
    v.as_f64()
        .ok_or_else(|| EvalError::new(format!("expected a number in {ctx_desc}, got {v}")))
}

fn as_bool(v: &Value, ctx_desc: &str) -> Result<bool, EvalError> {
    v.as_bool()
        .ok_or_else(|| EvalError::new(format!("expected a boolean in {ctx_desc}, got {v}")))
}

fn as_str(v: &Value, ctx_desc: &str) -> Result<String, EvalError> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| EvalError::new(format!("expected a string in {ctx_desc}, got {v}")))
}

fn eval_unary(op: &UnOp, v: Value) -> Result<Value, EvalError> {
    match op {
        UnOp::Neg => Ok(Value::from(-as_number(&v, "unary '-'")?)),
        UnOp::Not => Ok(Value::from(!as_bool(&v, "unary '!'")?)),
    }
}

fn eval_binary(op: &BinOp, lhs: &Expr, rhs: &Expr, ctx: &Context) -> Result<Value, EvalError> {
    // Logical operators short-circuit; every other operator evaluates both sides.
    match op {
        BinOp::And => {
            let l = as_bool(&eval(lhs, ctx)?, "'&&' left operand")?;
            if !l {
                return Ok(Value::from(false));
            }
            Ok(Value::from(as_bool(
                &eval(rhs, ctx)?,
                "'&&' right operand",
            )?))
        }
        BinOp::Or => {
            let l = as_bool(&eval(lhs, ctx)?, "'||' left operand")?;
            if l {
                return Ok(Value::from(true));
            }
            Ok(Value::from(as_bool(
                &eval(rhs, ctx)?,
                "'||' right operand",
            )?))
        }
        _ => {
            let l = eval(lhs, ctx)?;
            let r = eval(rhs, ctx)?;
            eval_binary_values(op, l, r)
        }
    }
}

fn eval_binary_values(op: &BinOp, l: Value, r: Value) -> Result<Value, EvalError> {
    match op {
        BinOp::Add => {
            if matches!(l, Value::String(_)) || matches!(r, Value::String(_)) {
                Ok(Value::from(format!("{}{}", stringify(&l), stringify(&r))))
            } else {
                Ok(Value::from(
                    as_number(&l, "'+' left")? + as_number(&r, "'+' right")?,
                ))
            }
        }
        BinOp::Sub => Ok(Value::from(
            as_number(&l, "'-' left")? - as_number(&r, "'-' right")?,
        )),
        BinOp::Mul => Ok(Value::from(
            as_number(&l, "'*' left")? * as_number(&r, "'*' right")?,
        )),
        BinOp::Div => Ok(Value::from(
            as_number(&l, "'/' left")? / as_number(&r, "'/' right")?,
        )),
        BinOp::Rem => Ok(Value::from(
            as_number(&l, "'%' left")? % as_number(&r, "'%' right")?,
        )),
        BinOp::Eq => Ok(Value::from(values_equal(&l, &r))),
        BinOp::Ne => Ok(Value::from(!values_equal(&l, &r))),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => eval_ordering(op, &l, &r),
        BinOp::And | BinOp::Or => unreachable!("handled by eval_binary's short-circuit path"),
    }
}

/// `==`/`!=` compare two numbers *by value*, not by `serde_json::Number`'s internal
/// representation. This matters because expression literals always lex as `f64`
/// (`Value::from(5.0)`), while a parameter's actual value may arrive from external JSON as an
/// integer-backed `Number` (`Value::from(5i64)`) — those are numerically equal but not `==` as
/// raw `Value`s, which would otherwise make `${{ value == 5 }}` silently false for a perfectly
/// valid integer parameter.
fn values_equal(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Number(_), Value::Number(_)) => l.as_f64() == r.as_f64(),
        _ => l == r,
    }
}

fn eval_ordering(op: &BinOp, l: &Value, r: &Value) -> Result<Value, EvalError> {
    let ord = match (l, r) {
        (Value::Number(_), Value::Number(_)) => {
            as_number(l, "comparison")?.partial_cmp(&as_number(r, "comparison")?)
        }
        (Value::String(a), Value::String(b)) => a.partial_cmp(b),
        _ => return Err(EvalError::new(format!("cannot compare {l} and {r}"))),
    }
    .ok_or_else(|| EvalError::new("incomparable values (NaN?)"))?;
    let b = match op {
        BinOp::Lt => ord.is_lt(),
        BinOp::Le => ord.is_le(),
        BinOp::Gt => ord.is_gt(),
        BinOp::Ge => ord.is_ge(),
        _ => unreachable!(),
    };
    Ok(Value::from(b))
}

/// The closed function whitelist. Adding a function means adding a case here — there is no
/// mechanism for a component definition to introduce its own.
fn eval_call(name: &str, args: Vec<Value>) -> Result<Value, EvalError> {
    match name {
        "split" => {
            let [s, sep] = take2(&args, "split")?;
            let s = as_str(&s, "split(s, ..)")?;
            let sep = as_str(&sep, "split(.., sep)")?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::from(c.to_string())).collect()
            } else {
                s.split(sep.as_str())
                    .map(|p| Value::from(p.to_string()))
                    .collect()
            };
            Ok(Value::Array(parts))
        }
        "count" => {
            let [v] = take1(&args, "count")?;
            let n = match v {
                Value::Array(a) => a.len(),
                Value::String(s) => s.chars().count(),
                Value::Object(m) => m.len(),
                other => return Err(EvalError::new(format!("count() cannot measure {other}"))),
            };
            Ok(Value::from(n as f64))
        }
        "concat" => Ok(Value::from(args.iter().map(stringify).collect::<String>())),
        "range" => {
            let [n] = take1(&args, "range")?;
            let n = as_number(&n, "range(n)")?;
            if n < 0.0 || n.fract() != 0.0 {
                return Err(EvalError::new(format!(
                    "range(n) expects a non-negative integer, got {n}"
                )));
            }
            Ok(Value::Array(
                (0..n as i64).map(|i| Value::from(i as f64)).collect(),
            ))
        }
        "at" => {
            let [container, key] = take2(&args, "at")?;
            match container {
                Value::Array(items) => {
                    let idx = as_number(&key, "at(array, index)")?;
                    if idx < 0.0 || idx.fract() != 0.0 {
                        return Err(EvalError::new(format!(
                            "at(array, index) expects a non-negative integer index, got {idx}"
                        )));
                    }
                    items.get(idx as usize).cloned().ok_or_else(|| {
                        EvalError::new(format!(
                            "at(): index {idx} out of bounds for array of length {}",
                            items.len()
                        ))
                    })
                }
                Value::Object(map) => {
                    let key = as_str(&key, "at(object, key)")?;
                    map.get(&key)
                        .cloned()
                        .ok_or_else(|| EvalError::new(format!("at(): no field '{key}'")))
                }
                other => Err(EvalError::new(format!(
                    "at() expects an array or object, got {other}"
                ))),
            }
        }
        "max" => reduce_numbers(&args, "max", f64::max),
        "min" => reduce_numbers(&args, "min", f64::min),
        "matches" => {
            let [s, pattern] = take2(&args, "matches")?;
            let s = as_str(&s, "matches(s, ..)")?;
            let pattern = as_str(&pattern, "matches(.., pattern)")?;
            let re = Regex::new(&pattern)
                .map_err(|e| EvalError::new(format!("invalid regex '{pattern}': {e}")))?;
            Ok(Value::from(re.is_match(&s)))
        }
        other => Err(EvalError::new(format!("unknown function '{other}'"))),
    }
}

fn take1(args: &[Value], fn_name: &str) -> Result<[Value; 1], EvalError> {
    match args {
        [a] => Ok([a.clone()]),
        _ => Err(EvalError::new(format!(
            "{fn_name}() expects 1 argument, got {}",
            args.len()
        ))),
    }
}

fn take2(args: &[Value], fn_name: &str) -> Result<[Value; 2], EvalError> {
    match args {
        [a, b] => Ok([a.clone(), b.clone()]),
        _ => Err(EvalError::new(format!(
            "{fn_name}() expects 2 arguments, got {}",
            args.len()
        ))),
    }
}

fn reduce_numbers(
    args: &[Value],
    fn_name: &str,
    f: impl Fn(f64, f64) -> f64,
) -> Result<Value, EvalError> {
    if args.is_empty() {
        return Err(EvalError::new(format!(
            "{fn_name}() expects at least 1 argument"
        )));
    }
    let mut nums = args.iter().map(|v| as_number(v, fn_name));
    let first = nums.next().unwrap()?;
    let result = nums.try_fold(first, |acc, n| n.map(|n| f(acc, n)))?;
    Ok(Value::from(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::parse;
    use serde_json::json;

    fn eval_expr(src: &str) -> Result<Value, EvalError> {
        let expr = parse(src).expect("parse");
        eval(&expr, &Context::new(Map::new()))
    }

    #[test]
    fn range_produces_zero_indexed_integers() {
        assert_eq!(eval_expr("range(3)").unwrap(), json!([0.0, 1.0, 2.0]));
        assert_eq!(eval_expr("range(0)").unwrap(), json!([]));
    }

    #[test]
    fn at_indexes_arrays_and_looks_up_object_fields() {
        assert_eq!(eval_expr("at(range(5), 2)").unwrap(), json!(2.0));
        let ctx = Context::new(
            json!({"b": [[1, 2, 3], [4, 5, 6]]})
                .as_object()
                .unwrap()
                .clone(),
        );
        let row0 = eval(&parse("at(params.b, 0)").unwrap(), &ctx).unwrap();
        // `at()` passes the stored value through unchanged (no numeric coercion), and
        // serde_json's `Number` distinguishes integer-backed from float-backed representations
        // for equality -- match the literal ints the fixture's `json!` macro actually produced.
        assert_eq!(row0, json!([1, 2, 3]));
        assert_eq!(
            eval(&parse("count(at(params.b, 0))").unwrap(), &ctx).unwrap(),
            json!(3.0)
        );
    }

    #[test]
    fn at_out_of_bounds_is_an_error_not_a_panic() {
        assert!(eval_expr("at(range(2), 5)").is_err());
    }

    #[test]
    fn if_picks_the_matching_branch() {
        assert_eq!(eval_expr("if(true, 1, 2)").unwrap(), json!(1.0));
        assert_eq!(eval_expr("if(false, 1, 2)").unwrap(), json!(2.0));
    }

    #[test]
    fn equality_compares_numbers_by_value_not_by_json_representation() {
        // `5` in source always lexes as f64; `value` may arrive integer-backed from external
        // JSON (a parameter's `default`/supplied value). Both must compare equal.
        let ctx = Context::new(json!({"n": 5}).as_object().unwrap().clone());
        assert_eq!(
            eval(&parse("params.n == 5").unwrap(), &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            eval(&parse("params.n != 5").unwrap(), &ctx).unwrap(),
            json!(false)
        );
    }

    #[test]
    fn if_short_circuits_the_untaken_branch() {
        // at(range(1), 5) would error if evaluated -- it must not be, since the condition
        // selects the other branch. This is exactly the shape a real gated for_each source
        // needs: `if(params.op == 'not', [], range(count(params.operands)))`.
        assert_eq!(
            eval_expr("if(true, 0, at(range(1), 5))").unwrap(),
            json!(0.0)
        );
        assert_eq!(
            eval_expr("if(false, at(range(1), 5), 0)").unwrap(),
            json!(0.0)
        );
    }
}
