//! The computation whose result must be bit-identical natively and in WASM
//! (plan section 2, determinism): every keyframe track of the parity vectors
//! at every time, every easing and spring case, and a sweep of the `libm`
//! functions, folded into one FNV-1a digest of the result bits.

use pm_eval::easing::{self, SpringConfig, SpringTables};
use pm_eval::keyframes;
use pm_eval::{JsValue, math};
use pm_model::json::parse;
use pm_model::{Node, Value};

pub struct Digest(u64, u64);

impl Digest {
    pub fn new() -> Digest {
        Digest(0xcbf2_9ce4_8422_2325, 0)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn number(&mut self, x: f64) {
        self.bytes(&x.to_bits().to_le_bytes());
        self.1 += 1;
    }

    fn js(&mut self, v: &Result<JsValue, pm_eval::JsThrow>) {
        match v {
            Ok(JsValue::Number(n)) => self.number(*n),
            Ok(other) => {
                self.bytes(format!("{other:?}").as_bytes());
                self.1 += 1;
            }
            Err(e) => self.bytes(e.kind.as_bytes()),
        }
    }

    pub fn value(&self) -> u64 {
        self.0
    }

    pub fn count(&self) -> u64 {
        self.1
    }
}

fn f(hex: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(hex.as_str().unwrap(), 16).unwrap())
}

/// Digests the evaluation of `vectors` (the text of `eval-vectors.json`).
pub fn digest(vectors: &str) -> Digest {
    let vectors = parse(vectors).expect("vectors");
    let mut d = Digest::new();
    let mut springs = SpringTables::new();
    for case in vectors.get("tracks").unwrap().as_array().unwrap() {
        let kf = Node::from_value(parse(case.get("kf").unwrap().as_str().unwrap()).unwrap(), 0);
        let color = case.get("color").unwrap().as_bool().unwrap();
        for t in case.get("times").unwrap().as_array().unwrap() {
            d.js(&keyframes::evaluate(Some(&kf), f(t), color, &mut springs));
        }
    }
    for case in vectors.get("easing").unwrap().as_array().unwrap() {
        let c: Vec<f64> = case
            .get("curve")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(f)
            .collect();
        let curve = easing::bezier(c[0], c[1], c[2], c[3]);
        for x in case.get("xs").unwrap().as_array().unwrap() {
            d.number(curve.eval(f(x)));
        }
    }
    for case in vectors.get("springs").unwrap().as_array().unwrap() {
        let cfg = Node::from_value(
            parse(case.get("cfg").unwrap().as_str().unwrap()).unwrap(),
            0,
        );
        let cfg = SpringConfig::from_node(Some(&cfg));
        for t in case.get("ts").unwrap().as_array().unwrap() {
            d.number(springs.spring(f(t), cfg));
        }
        d.number(easing::spring_duration(cfg));
    }
    // libm sweep: 20,000 arguments per function, from tiny to huge.
    let mut x = 1.0e-9f64;
    for i in 0..20_000 {
        let a = if i % 2 == 0 { x } else { -x * 0.73 };
        for v in [
            math::sin(a),
            math::cos(a),
            math::tan(a),
            math::atan(a),
            math::atan2(a, 0.5 - a),
            math::exp(a.min(700.0)),
            math::log(a.abs()),
            math::pow(a.abs(), 0.37),
            math::sinh(a.clamp(-700.0, 700.0)),
            math::cbrt(a),
            math::hypot(a, 3.0),
        ] {
            d.number(v);
        }
        x *= 1.0021;
    }
    d
}
