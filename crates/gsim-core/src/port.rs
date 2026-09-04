//! The resolved port shape, matching the `side`/`offset`/`kind`/`direction` convention already
//! used by `general-simulator-gui`'s GSF v1 (`gsim_model.symbol.SymbolBuilder.port()` /
//! `gsim_model.layout.PortGeometry`) — GSF v2 does not introduce a new port-positioning scheme,
//! only a way to generate a *list* of these dynamically (see `crate::resolve`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortDirection {
    In,
    Out,
    Inout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    /// Stable, never reassigned to a different logical port even if sibling ports are added or
    /// removed by a `for_each` re-evaluation — see
    /// `docs/decisions/0001-scope-and-naming.md`'s "port identity" section.
    pub id: String,
    pub side: PortSide,
    #[serde(default)]
    pub offset: f64,
    /// Port-kind catalogue (`docs/specs/gsim-sch-format.md` §8) is open-ended; kept as a plain
    /// string rather than a closed Rust enum so a new kind never requires a schema-breaking
    /// release of this crate.
    pub kind: String,
    pub direction: PortDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Resolved signal dimension, e.g. `[3]` for a 3-wide vector signal or `[2, 3]` for a
    /// matrix — the same `number[]` shape `general-simulator-gui`'s `PortDesc.dim` carries
    /// (`apps/frontend/src/canvas/port-compat.ts`), and what its `dim-mismatch` rule compares.
    ///
    /// Optional, and absent means *unknown*, never "scalar": a contract that declares no `dim`
    /// makes the dimension check skip that endpoint rather than assume `[1]`, matching
    /// `canConnect`'s own `src.dim !== undefined && dst.dim !== undefined` guard.
    ///
    /// Deserialized leniently because the `${{ }}` interpreter produces **float-backed**
    /// numbers (`range(3)` is `[0.0, 1.0, 2.0]`), so a `"dim": "${{ [count(params.x)] }}"`
    /// contract would fail a plain `Vec<i64>` deserialization — see [`deserialize_dim`].
    #[serde(
        default,
        deserialize_with = "deserialize_dim",
        skip_serializing_if = "Option::is_none"
    )]
    pub dim: Option<Vec<i64>>,
}

/// Accepts each dimension element as either an integer- or a float-backed JSON number, rejecting
/// anything that is not a whole, non-negative count. A resolved GSF v2 expression always yields
/// `f64`-backed numbers, while a hand-written static contract yields integers; both spellings
/// must mean the same dimension.
fn deserialize_dim<'de, D>(de: D) -> Result<Option<Vec<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let Some(values) = Option::<Vec<serde_json::Value>>::deserialize(de)? else {
        return Ok(None);
    };
    let mut dim = Vec::with_capacity(values.len());
    for v in values {
        let n = v
            .as_f64()
            .ok_or_else(|| D::Error::custom(format!("dim element must be a number, got {v}")))?;
        if n < 0.0 || n.fract() != 0.0 {
            return Err(D::Error::custom(format!(
                "dim element must be a whole, non-negative count, got {n}"
            )));
        }
        dim.push(n as i64);
    }
    Ok(Some(dim))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn port(value: serde_json::Value) -> Port {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_port_without_a_dim_is_unknown_not_scalar() {
        let p = port(json!({"id": "in", "side": "left", "kind": "signal", "direction": "in"}));
        assert_eq!(p.dim, None);
        // ...and round-trips without inventing the field, so no existing document changes.
        let back = serde_json::to_value(&p).unwrap();
        assert!(back.get("dim").is_none(), "{back}");
    }

    #[test]
    fn a_dim_deserializes_from_both_integer_and_float_backed_numbers() {
        // Integer-backed: a hand-written static contract.
        let a = port(
            json!({"id": "in", "side": "left", "kind": "signal", "direction": "in", "dim": [3]}),
        );
        // Float-backed: what the `${{ }}` interpreter actually produces.
        let b = port(
            json!({"id": "in", "side": "left", "kind": "signal", "direction": "in", "dim": [3.0]}),
        );
        assert_eq!(a.dim, Some(vec![3]));
        assert_eq!(a.dim, b.dim);
    }

    #[test]
    fn a_non_whole_or_negative_dim_is_refused_rather_than_truncated() {
        for bad in [json!([2.5]), json!([-1]), json!(["3"])] {
            let value = json!({"id": "in", "side": "left", "kind": "signal", "direction": "in", "dim": bad});
            assert!(
                serde_json::from_value::<Port>(value).is_err(),
                "{bad} should not deserialize"
            );
        }
    }
}
