//! Replies to the renderer as JSON text.
//!
//! Numbers are written as JavaScript's `Number::toString` (shortest
//! round-trip digits), so `JSON.parse` gives back the exact `f64`. JSON has no
//! form for `NaN`, `±Infinity`, or `-0`; those are written as the marker
//! object `{"\u0000":"NaN"}` (`"Infinity"`, `"-Infinity"`, `"-0"`), and the
//! reply sets [`Reply::special`] so the renderer revives them only when
//! needed. No document object has a key made of a single NUL, and replies
//! never contain document objects with such a key unescaped.

use pm_eval::JsValue;
use pm_model::json::write::{push_quoted, to_canonical_string};
use pm_model::{JsString, Node, NodeKind, Value};

#[derive(Default)]
pub struct Reply {
    pub text: String,
    /// Whether a special number marker was written.
    pub special: bool,
}

impl Reply {
    pub fn new() -> Reply {
        Reply::default()
    }

    pub fn raw(&mut self, s: &str) {
        self.text.push_str(s);
    }

    pub fn num(&mut self, n: f64) {
        if n.is_finite() && !(n == 0.0 && n.is_sign_negative()) {
            let mut buf = ryu_js::Buffer::new();
            self.text.push_str(buf.format(n));
        } else {
            self.special = true;
            let name = if n.is_nan() {
                "NaN"
            } else if n == f64::INFINITY {
                "Infinity"
            } else if n == f64::NEG_INFINITY {
                "-Infinity"
            } else {
                "-0"
            };
            self.text.push_str("{\"\\u0000\":\"");
            self.text.push_str(name);
            self.text.push_str("\"}");
        }
    }

    pub fn nums(&mut self, values: &[f64]) {
        self.text.push('[');
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                self.text.push(',');
            }
            self.num(*v);
        }
        self.text.push(']');
    }

    pub fn bool(&mut self, b: bool) {
        self.text.push_str(if b { "true" } else { "false" });
    }

    pub fn str(&mut self, s: &str) {
        push_quoted(&JsString::from(s), &mut self.text);
    }

    pub fn js_str(&mut self, s: &JsString) {
        push_quoted(s, &mut self.text);
    }

    /// `undefined` is written as `null` with the marker `{"\u0000":"undefined"}`
    /// so the renderer can tell the two apart.
    pub fn js(&mut self, v: &JsValue) {
        match v {
            JsValue::Undefined => {
                self.special = true;
                self.text.push_str("{\"\\u0000\":\"undefined\"}");
            }
            JsValue::Null => self.text.push_str("null"),
            JsValue::Bool(b) => self.bool(*b),
            JsValue::Number(n) => self.num(*n),
            JsValue::String(s) => self.js_str(s),
            JsValue::Object(value) => self.value(value),
        }
    }

    pub fn value(&mut self, v: &Value) {
        self.text.push_str(&to_canonical_string(v));
    }

    pub fn node(&mut self, n: &Node) {
        match n.kind() {
            NodeKind::Number(x) => self.num(*x),
            _ => self.text.push_str(&to_canonical_string(n)),
        }
    }

    /// Starts an object member: `"key":` (with a comma when not first).
    pub fn key(&mut self, first: &mut bool, key: &str) {
        if !*first {
            self.text.push(',');
        }
        *first = false;
        self.str(key);
        self.text.push(':');
    }
}
