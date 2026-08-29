//! Shared `Value` -> `String` stringification for template interpolation and the `concat()`
//! builtin — one implementation so `"in${{ i }}"` and `concat(i, "x")` never disagree on how a
//! number prints.

use serde_json::Value;

pub fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            // `for_each`'s `index` binding, and most arithmetic results, are whole numbers
            // stored as f64 (`Value::from(0.0)`) -- print those as "0", not "0.0".
            Some(f) if f.is_finite() && f.fract() == 0.0 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_number_floats_print_without_a_decimal_point() {
        assert_eq!(stringify(&json!(0.0)), "0");
        assert_eq!(stringify(&json!(3.0)), "3");
        assert_eq!(stringify(&json!(3.5)), "3.5");
    }
}
