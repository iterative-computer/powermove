//! Specializes GLSL functions that take sampler parameters (three.js's
//! `getShadow`, `getPointShadow`, `texture2DCompare`, and the like) into
//! one copy per sampler they are called with, so no function takes an
//! opaque argument. `pm_gpu::glsl` refuses such functions (naga cannot take
//! them and SPIRV-Tools' opaque inliner is not linked); three.js unrolls
//! its light loops, so every call passes a literal sampler (`name` or
//! `name[ 3 ]`) and the specialization is exact: the copy is the original
//! with the parameter replaced by the argument, which is what inlining does.
//! Copies are inserted after the original, inside the same preprocessor
//! region; the originals are removed.

use std::sync::OnceLock;

use regex::Regex;

struct Def {
    name: String,
    /// Byte range of the whole definition (header and body).
    start: usize,
    end: usize,
    /// Byte range of the body (`{` … `}` inclusive).
    body: (usize, usize),
    ret: String,
    /// Parameter declarations (`sampler2D shadowMap`, `vec2 uv`).
    params: Vec<String>,
}

fn header() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?m)^[ \t]*((?:(?:highp|mediump|lowp)\s+)?\w+)\s+(\w+)\s*\(").unwrap()
    })
}

/// The index after the bracket matching the one at `open`.
fn matching(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (o, c) = match b[open] {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0;
    for (i, &ch) in b.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Splits `a, f(b, c), d[ 1 ]` at top-level commas.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_owned());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

fn is_sampler(param: &str) -> bool {
    param.split_whitespace().any(|t| t.contains("sampler"))
}

fn param_name(param: &str) -> &str {
    param.split_whitespace().last().unwrap_or("")
}

/// Definitions whose parameters include a sampler.
fn sampler_defs(s: &str) -> Vec<Def> {
    let mut out = Vec::new();
    for c in header().captures_iter(s) {
        let m = c.get(0).unwrap();
        let ret = c[1].to_owned();
        if matches!(ret.as_str(), "return" | "else" | "if" | "for" | "while") {
            continue;
        }
        let open = m.end() - 1;
        let Some(close) = matching(s, open) else {
            continue;
        };
        let rest = &s[close..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with('{') {
            continue;
        }
        let brace = close + (rest.len() - trimmed.len());
        let Some(end) = matching(s, brace) else {
            continue;
        };
        let params: Vec<String> = split_args(&s[open + 1..close - 1]);
        if !params.iter().any(|p| is_sampler(p)) {
            continue;
        }
        out.push(Def {
            name: c[2].to_owned(),
            start: m.start(),
            end,
            body: (brace, end),
            ret,
            params,
        });
    }
    out
}

fn mangle(arg: &str) -> String {
    let mut out = String::new();
    let mut last = '_';
    for ch in arg.chars() {
        let c = if ch.is_ascii_alphanumeric() { ch } else { '_' };
        if c == '_' && last == '_' {
            continue;
        }
        out.push(c);
        last = c;
    }
    out.trim_end_matches('_').to_owned()
}

fn replace_word(body: &str, word: &str, with: &str) -> String {
    let re = Regex::new(&format!(r"\b{}\b", regex::escape(word))).unwrap();
    re.replace_all(body, regex::NoExpand(with)).into_owned()
}

/// The source with every sampler-taking function specialized per call.
pub fn specialize(source: &str) -> Result<String, String> {
    let mut s = source.to_owned();
    let mut made: Vec<String> = Vec::new();
    for _ in 0..10_000 {
        let defs = sampler_defs(&s);
        if defs.is_empty() {
            return Ok(s);
        }
        // The first call outside any original definition.
        let mut call: Option<(usize, usize, usize, Vec<String>)> = None;
        'search: for (di, d) in defs.iter().enumerate() {
            let re = Regex::new(&format!(r"\b{}\s*\(", regex::escape(&d.name))).unwrap();
            for m in re.find_iter(&s) {
                if defs
                    .iter()
                    .any(|g| m.start() >= g.start && m.start() < g.end)
                {
                    continue;
                }
                let open = m.end() - 1;
                let close = matching(&s, open).ok_or("unbalanced call")?;
                call = Some((di, m.start(), close, split_args(&s[open + 1..close - 1])));
                break 'search;
            }
        }
        let Some((di, at, close, args)) = call else {
            // Only uncalled originals remain.
            let mut out = s.clone();
            for d in defs.iter().rev() {
                out.replace_range(d.start..d.end, "");
            }
            return Ok(out);
        };
        let d = &defs[di];
        if args.len() != d.params.len() {
            return Err(format!(
                "{}: {} arguments for {} parameters",
                d.name,
                args.len(),
                d.params.len()
            ));
        }
        let mut name = d.name.clone();
        let mut body = s[d.body.0..d.body.1].to_owned();
        let mut params = Vec::new();
        let mut rest_args = Vec::new();
        for (p, a) in d.params.iter().zip(&args) {
            if is_sampler(p) {
                if !a
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_[] ".contains(c))
                {
                    return Err(format!(
                        "{}: sampler argument {a} is not a literal sampler",
                        d.name
                    ));
                }
                name.push_str("_pm_");
                name.push_str(&mangle(a));
                body = replace_word(&body, param_name(p), a);
            } else {
                params.push(p.clone());
                rest_args.push(a.clone());
            }
        }
        let call_text = format!("{name}( {} )", rest_args.join(", "));
        let end = d.end;
        let definition = format!(
            "\n{} {name}( {} ) {body}\n",
            d.ret.trim(),
            params.join(", ")
        );
        // Replace the call first (it lies after the definition).
        s.replace_range(at..close, &call_text);
        if !made.contains(&name) {
            s.insert_str(end, &definition);
            made.push(name);
        }
    }
    Err("sampler specialization did not converge".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_get_their_own_copies_and_originals_go() {
        let src = "float cmp( sampler2D d, vec2 uv, float c ) {\n\treturn step( c, texture( d, uv ).r );\n}\nfloat shadow( sampler2D m, vec2 uv ) {\n\treturn cmp( m, uv, 0.5 ) + cmp( m, uv + 1.0, 0.5 );\n}\nvoid main() {\n\tfloat a = shadow( dirMap[ 0 ], vUv );\n\tfloat b = shadow( dirMap[ 1 ], vUv );\n\tfloat c = shadow( dirMap[ 0 ], vUv * 2.0 );\n}\n";
        let out = specialize(src).unwrap();
        assert!(!out.contains("sampler2D"), "{out}");
        assert!(out.contains("float shadow_pm_dirMap_0( vec2 uv )"), "{out}");
        assert!(out.contains("shadow_pm_dirMap_1( vUv )"));
        assert!(out.contains("cmp_pm_dirMap_0( uv, 0.5 )"), "{out}");
        assert!(out.contains("texture( dirMap[ 0 ], uv )"));
        assert_eq!(out.matches("float shadow_pm_dirMap_0(").count(), 1);
    }

    #[test]
    fn three_shadow_programs_specialize() {
        use crate::three::materials::{LightCounts, Maps, Side, Standard, standard};
        let lights = LightCounts {
            directional: 2,
            point: 1,
            spot: 1,
            hemi: 1,
            directional_shadows: 2,
            point_shadows: 1,
            spot_shadows: 1,
            shadow_map: true,
            ..Default::default()
        };
        let p = standard(&Standard {
            name: "Surface".into(),
            side: Side::Double,
            transparent: false,
            maps: Maps::default(),
            lights,
        })
        .unwrap();
        let (_, fs) = crate::three::program::sources(&p).unwrap();
        let out = specialize(&fs).unwrap();
        assert!(!out.contains("sampler2D shadowMap"));
        assert!(out.contains("getShadow_pm_directionalShadowMap_1("));
        assert!(out.contains("getPointShadow_pm_pointShadowMap_0("));
    }
}
