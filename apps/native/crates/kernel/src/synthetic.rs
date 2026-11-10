//! Synthetic large projects for scale benches and soak runs (feature
//! `synthetic`; never in a shipped build). Deterministic for a given layer
//! count: the open composition holds half the layers flat (shapes, text,
//! solids, nulls and groups with parenting, effects and keyframed
//! transforms), the other half sits in a chain of nested precomps reached
//! through precomp layers of the open composition.
//!
//! `cargo run -p pm-kernel --features synthetic --example scale_project -- 5000 out.pmv`

use std::fmt::Write as _;

/// Depth of the nested precomp chain.
pub const PRECOMP_DEPTH: usize = 8;
/// The open composition's id.
pub const MAIN: &str = "main";
const SECONDS: f64 = 60.0;

/// A project of `layers` layers in the stored (v1) form.
pub fn project(layers: usize) -> pm_model::Value {
    pm_model::json::parse(&project_json(layers)).expect("synthetic project is JSON")
}

/// Layer ids of the open composition, top to bottom.
pub fn main_layer_ids(layers: usize) -> Vec<String> {
    let (flat, _) = split(layers);
    let mut ids: Vec<String> = (0..PRECOMP_DEPTH).map(|d| format!("pc{d}")).collect();
    ids.extend((0..flat).map(|n| format!("L{n}")));
    ids
}

fn split(layers: usize) -> (usize, usize) {
    let nested = if layers >= 4 * PRECOMP_DEPTH {
        layers / 2 / PRECOMP_DEPTH
    } else {
        0
    };
    let flat = layers.saturating_sub(nested * PRECOMP_DEPTH + PRECOMP_DEPTH);
    (flat, nested)
}

/// Same project as [`project`], as JSON text.
pub fn project_json(layers: usize) -> String {
    let (flat, nested) = split(layers);
    let mut main = String::new();
    for d in 0..PRECOMP_DEPTH {
        comma(&mut main);
        precomp_layer(&mut main, &format!("pc{d}"), &format!("comp{d}"), d);
    }
    for n in 0..flat {
        comma(&mut main);
        layer(&mut main, &format!("L{n}"), n);
    }
    let mut comps = String::new();
    for d in 0..PRECOMP_DEPTH {
        let mut inner = String::new();
        if d + 1 < PRECOMP_DEPTH {
            precomp_layer(
                &mut inner,
                &format!("c{d}pc"),
                &format!("comp{}", d + 1),
                d + 1,
            );
        }
        for n in 0..nested.saturating_sub(usize::from(d + 1 < PRECOMP_DEPTH)) {
            comma(&mut inner);
            layer(&mut inner, &format!("c{d}L{n}"), n);
        }
        comma(&mut comps);
        let _ = write!(
            comps,
            r##""comp{d}":{{"name":"Nested {d}","w":1920,"h":1080,"fps":30,"dur":{SECONDS},"bg":"#000000","layers":[{inner}]}}"##
        );
    }
    format!(
        r##"{{"id":"scale{layers}","name":"Scale {layers}","w":1920,"h":1080,"fps":30,"dur":{SECONDS},"bg":"#000000","compId":"{MAIN}","compName":"Main","work":[0,{SECONDS}],"markers":[],"assets":{{}},"layers":[{main}],"comps":{{{comps}}}}}"##
    )
}

fn comma(out: &mut String) {
    if !out.is_empty() {
        out.push(',');
    }
}

/// Deterministic pseudo-random sequence per layer.
fn mix(n: usize, salt: u64) -> u64 {
    let mut x = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

fn channel(out: &mut String, id: &str, name: &str, value: f64, keys: usize, n: usize) {
    let _ = write!(out, r#""{name}":{{"v":{value},"kf":["#);
    for k in 0..keys {
        if k > 0 {
            out.push(',');
        }
        let t = k as f64 * SECONDS / keys.max(1) as f64;
        let v = value + (mix(n * 31 + k, 7) % 200) as f64 - 100.0;
        let _ = write!(
            out,
            r#"{{"t":{t},"v":{v},"i":"k{id}_{name}_{k}","outInterp":"bezier","outEase":{{"speed":0,"influence":33.3}},"inInterp":"bezier","inEase":{{"speed":0,"influence":33.3}},"autoBezier":false,"continuous":false}}"#
        );
    }
    out.push_str(r#"],"expr":null}"#);
}

fn transform(out: &mut String, id: &str, n: usize) {
    const STATIC: [(&str, f64); 12] = [
        ("perspective", 50.0),
        ("position.z", 0.0),
        ("anchor.z", 0.0),
        ("scale.z", 100.0),
        ("rotation.x", 0.0),
        ("rotation.y", 0.0),
        ("orientation.x", 0.0),
        ("orientation.y", 0.0),
        ("orientation.z", 0.0),
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("skew", 0.0),
    ];
    out.push_str(r#""p":{"#);
    for (name, v) in STATIC {
        let _ = write!(out, r#""{name}":{{"v":{v},"kf":[],"expr":null}},"#);
    }
    // Four animated channels, 4–20 keys each (about 48 a layer).
    let animated = [
        ("position.x", (mix(n, 1) % 1920) as f64),
        ("position.y", (mix(n, 2) % 1080) as f64),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("rotation", 0.0),
        ("opacity", 100.0),
    ];
    for (i, (name, v)) in animated.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let keys = if i < 4 {
            4 + (mix(n, 3 + i as u64) % 17) as usize
        } else {
            0
        };
        channel(out, id, name, v, keys, n);
    }
    out.push('}');
}

fn common(out: &mut String, id: &str, kind: &str, n: usize, parent: Option<String>) {
    let from = (mix(n, 11) % 40) as f64 * 0.5;
    let dur = 5.0 + (mix(n, 12) % 30) as f64;
    let parent = parent.map_or("null".to_owned(), |p| format!(r#""{p}""#));
    let _ = write!(
        out,
        r##"{{"id":"{id}","type":"{kind}","name":"{kind} {n}","from":{from},"dur":{dur},"on":true,"lock":false,"shy":false,"collapsed":true,"color":"#6C7BE8","blend":"normal","mblur":false,"parent":{parent},"transitionIn":null,"transitionOut":null,"masks":[],"locked_intent":{{}},"solo":false,"track":{track},"##,
        track = n % 12,
    );
    transform(out, id, n);
}

fn layer(out: &mut String, id: &str, n: usize) {
    // Every 50th layer is a null the next 49 may parent to.
    let anchor = n - n % 50;
    let prefix = id.trim_end_matches(char::is_numeric);
    let parent =
        (!n.is_multiple_of(50) && n.is_multiple_of(3)).then(|| format!("{prefix}{anchor}"));
    let kind = match (n % 50, n % 7) {
        (0, _) => "null",
        (_, 0) => "text",
        (_, 1) => "solid",
        _ => "shape",
    };
    common(out, id, kind, n, parent);
    let fx = if n.is_multiple_of(4) {
        format!(
            r#"[{{"id":"fx{n}","type":"blur","on":true,"p":{{"amount":{{"v":{},"kf":[],"expr":null}}}}}}]"#,
            n % 20
        )
    } else {
        "[]".to_owned()
    };
    let content = match kind {
        "text" => format!(
            r##"{{"text":{{"v":"Title {n}","kf":[],"expr":null}},"font":"Helvetica","size":64,"color":"#FFFFFF"}}"##
        ),
        "solid" => r##"{"color":"#336699","w":1920,"h":1080}"##.to_owned(),
        "shape" => r##"{"shape":"rect","color":"#FFFFFF","w":200,"h":120,"radius":8,"stroke":0,"strokeColor":"#ffffff","points":5}"##.to_owned(),
        _ => "{}".to_owned(),
    };
    let _ = write!(out, r#","fx":{fx},"d":{content}}}"#);
}

fn precomp_layer(out: &mut String, id: &str, comp: &str, n: usize) {
    common(out, id, "precomp", n, None);
    let _ = write!(
        out,
        r#","fx":[],"d":{{"comp":"{comp}","w":1920,"h":1080}}}}"#
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn layer_counts_add_up() {
        for n in [100, 5000, 20_000] {
            let value = super::project(n);
            let count = |layers: Option<&pm_model::Value>| {
                layers.and_then(|l| l.as_array()).map_or(0, Vec::len)
            };
            let mut total = count(value.get("layers"));
            for (_, comp) in value.get("comps").unwrap().as_object().unwrap().iter() {
                total += count(comp.get("layers"));
            }
            assert_eq!(total, n);
            assert_eq!(super::main_layer_ids(n).len(), count(value.get("layers")));
        }
    }
}
