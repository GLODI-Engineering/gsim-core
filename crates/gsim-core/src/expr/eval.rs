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
        Expr::Call(name, args) => {
            let values: Result<Vec<Value>, EvalError> = args.iter().map(|a| eval(a, ctx)).collect();
            eval_call(name, values?)
        }
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
        BinOp::Eq => Ok(Value::from(l == r)),
        BinOp::Ne => Ok(Value::from(l != r)),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => eval_ordering(op, &l, &r),
        BinOp::And | BinOp::Or => unreachable!("handled by eval_binary's short-circuit path"),
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
