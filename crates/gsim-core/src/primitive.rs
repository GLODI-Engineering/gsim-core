//! The resolved (post-`for_each`/`when`/`${{ }}`) drawing primitive vocabulary. Field-for-field
//! identical to the `Primitive` union in `general-simulator-gui`'s
//! `apps/frontend/src/canvas/nodes/primitive-renderer.ts` (the frozen vocabulary GSF v1 already
//! committed to and GSF v2 does not change) — no new primitive kinds, so the existing frontend
//! renderer needs no changes to consume GSF v2 output.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Primitive {
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        lw: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        lw: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filled: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rotation: Option<f64>,
    },
    Rectf {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rotation: Option<f64>,
    },
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        lw: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filled: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Arc {
        cx: f64,
        cy: f64,
        r: f64,
        a0: f64,
        a1: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ccw: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lw: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Polygon {
        pts: Vec<(f64, f64)>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lw: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filled: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Polyline {
        pts: Vec<(f64, f64)>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lw: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Text {
        x: f64,
        y: f64,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        align: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        baseline: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bold: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rotation: Option<f64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_from_the_same_shape_the_frontend_emits() {
        let v = json!({"type": "circle", "cx": 30.0, "cy": 20.0, "r": 10.0});
        let p: Primitive = serde_json::from_value(v).unwrap();
        assert_eq!(
            p,
            Primitive::Circle {
                cx: 30.0,
                cy: 20.0,
                r: 10.0,
                lw: None,
                filled: None,
                color: None,
            }
        );
    }
}
