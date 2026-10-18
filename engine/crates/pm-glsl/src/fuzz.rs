//! Robustness of the translation pipeline against untrusted GLSL (plan
//! section 11: "Store GLSL is untrusted input: the pipeline is fuzzed and
//! resource-limited"). Ported from spike S3 (`docs/engine/spikes/S3.md`,
//! "Fuzzing and compile parity").
//!
//! Two inputs:
//!
//! * Mutants: deterministic mutations of seed programs' author lines
//!   (deletions, duplications, swaps, extreme literals, hostile tokens, splices
//!   across programs, byte flips). Mutant `i` of seed `s` is always the same
//!   source, so any finding reproduces from `(seed, i)`.
//! * Bombs: handwritten sources aimed at resource limits (macro expansion,
//!   nesting depth, huge arrays, huge sources, many functions, recursion).
//!
//! The `pm-glsl-fuzz` binary runs them in a worker process with a per-input
//! deadline, so a crash or hang in glslang or naga is attributed to the exact
//! input; [`outcome`] is what each input is judged by.

use serde::{Deserialize, Serialize};

use crate::pipeline;

/// A seed program: wrapped GLSL and the lines before the author's first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seed {
    pub id: String,
    pub frag: String,
    pub line_offset: u32,
}

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);
impl Rng {
    fn new(seed: u64, index: u64) -> Self {
        let mut r = Rng(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03);
        for _ in 0..4 {
            r.next();
        }
        r
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

const EXTREMES: &[&str] = &[
    "0.0",
    "-0.0",
    "1e38",
    "3.4028235e38",
    "1e39",
    "1e-45",
    "-1e38",
    "2147483647",
    "-2147483648",
    "4294967295u",
    "1000000000",
    "65536",
    "0",
    "(1.0/0.0)",
    "(0.0/0.0)",
    "1e-38",
];

const TOKENS: &[&str] = &[
    "{",
    "}",
    "(",
    ")",
    ";",
    ",",
    "[",
    "]",
    "for(int pm_i=0;pm_i<1000000;pm_i++){",
    "while(true){",
    "do{",
    "}while(true);",
    "#define X X X\n",
    "texture(u_tex, vec2(0.5))",
    "dFdx(",
    "dFdy(",
    "fwidth(",
    "float pm_a[1000000];",
    "struct S{float x;};",
    "return;",
    "discard;",
    "break;",
    "continue;",
    "switch(1){",
    "case 0:",
    "default:",
    "#if 1\n",
    "#endif\n",
    "#line 999999\n",
    "#line 0\n",
    "/*",
    "*/",
    "\\\n",
    "\u{e9}",
    "\u{202e}",
    "#pragma optimize(off)\n",
    "#extension GL_EXT_shader_texture_lod : enable\n",
    "#version 450\n",
    "#undef main\n",
    "#define main pm_main\n",
    "invariant ",
    "flat ",
    "centroid ",
    "uniform ",
    "in ",
    "out ",
    "inout ",
    "const ",
    "highp ",
    "lowp ",
    "mat4(",
    "vec4(",
    "ivec2(",
    "uint(",
    "bool(",
    "int(",
    ".xyzw",
    ".rgba",
    ".sss",
    "++",
    "--",
    "<<",
    ">>",
    "%",
    "^^",
    "?",
    ":",
    "=",
    "==",
    "!=",
    "&&",
    "||",
    "texelFetch(u_tex, ivec2(-1), 0)",
    "textureLod(u_tex, vec2(0.5), 1e9)",
    "textureSize(u_tex, 99)",
    "gl_FragCoord",
    "gl_FragDepth = 2.0;",
    "gl_PointCoord",
    "inverse(mat4(0.0))",
    "pow(-1.0, 0.5)",
    "sqrt(-1.0)",
    "log(0.0)",
    "1.0/0.0",
    "0/0",
    "ivec4(1)/ivec4(0)",
    "int(1e38)",
    "uint(-1.0)",
];

fn author_region(frag: &str, line_offset: u32) -> usize {
    frag.split_inclusive('\n')
        .take(line_offset as usize)
        .map(str::len)
        .sum()
}

fn char_boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Mutant `index` for `seed`: 1 to 4 stacked mutations of one corpus program.
pub fn mutant(seeds: &[Seed], seed: u64, index: u64) -> (String, String, u32) {
    let mut rng = Rng::new(seed, index);
    let case = &seeds[rng.below(seeds.len())];
    let start = author_region(&case.frag, case.line_offset);
    let (head, body) = case.frag.split_at(start);
    let mut body = body.to_string();
    let count = 1 + rng.below(4);
    let mut ops = Vec::new();
    for _ in 0..count {
        let op = rng.below(8);
        ops.push(op);
        let len = body.len().max(1);
        match op {
            0 => {
                let a = char_boundary(&body, rng.below(len));
                let b = char_boundary(&body, a + 1 + rng.below(20));
                body.replace_range(a..b, "");
            }
            1..=3 => {
                let mut lines: Vec<String> =
                    body.split_inclusive('\n').map(str::to_string).collect();
                if lines.is_empty() {
                    continue;
                }
                let i = rng.below(lines.len());
                match op {
                    1 => {
                        let line = lines[i].clone();
                        lines.insert(i, line);
                    }
                    2 => {
                        lines.remove(i);
                    }
                    _ => {
                        let j = rng.below(lines.len());
                        lines.swap(i, j);
                    }
                }
                body = lines.concat();
            }
            4 => {
                // Replace a numeric literal with an extreme value.
                let bytes = body.as_bytes();
                let numbers: Vec<usize> = (0..bytes.len())
                    .filter(|&i| {
                        bytes[i].is_ascii_digit()
                            && (i == 0
                                || !(bytes[i - 1].is_ascii_alphanumeric()
                                    || bytes[i - 1] == b'_'
                                    || bytes[i - 1] == b'.'))
                    })
                    .collect();
                if numbers.is_empty() {
                    continue;
                }
                let a = numbers[rng.below(numbers.len())];
                let mut b = a;
                while b < bytes.len()
                    && (bytes[b].is_ascii_digit()
                        || bytes[b] == b'.'
                        || bytes[b] == b'e'
                        || bytes[b] == b'u')
                {
                    b += 1;
                }
                body.replace_range(a..b, EXTREMES[rng.below(EXTREMES.len())]);
            }
            5 => {
                let at = char_boundary(&body, rng.below(len));
                body.insert_str(at, TOKENS[rng.below(TOKENS.len())]);
            }
            6 => {
                // Splice a span from another program's author lines.
                let other = &seeds[rng.below(seeds.len())];
                let other_body = &other.frag[author_region(&other.frag, other.line_offset)..];
                if other_body.is_empty() {
                    continue;
                }
                let a = char_boundary(other_body, rng.below(other_body.len()));
                let b = char_boundary(other_body, a + rng.below(400));
                let at = char_boundary(&body, rng.below(len));
                let end = char_boundary(&body, at + rng.below(200));
                body.replace_range(at..end, &other_body[a..b]);
            }
            _ => {
                let at = char_boundary(&body, rng.below(len));
                if at < body.len() {
                    let ch = body[at..].chars().next().unwrap();
                    let replacement = (b' ' + rng.below(95) as u8) as char;
                    body.replace_range(at..at + ch.len_utf8(), &replacement.to_string());
                }
            }
        }
    }
    (
        format!("{head}{body}"),
        format!("{} ops {:?}", case.id, ops),
        case.line_offset,
    )
}

const HEADER: &str = "#version 300 es\nprecision highp float;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n";

/// Handwritten resource bombs (name, source).
pub fn bombs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut macros = String::from("#define A0 1.0\n");
    for i in 1..=24 {
        macros.push_str(&format!("#define A{i} (A{p} + A{p})\n", p = i - 1));
    }
    out.push((
        "macro-expansion-2^24".into(),
        format!("{HEADER}{macros}void main(){{ o = vec4(A24); }}\n"),
    ));
    let depth = 20_000;
    out.push((
        "nested-parens-20000".into(),
        format!(
            "{HEADER}void main(){{ float f = {}1.0{}; o = vec4(f); }}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        ),
    ));
    out.push((
        "nested-blocks-10000".into(),
        format!(
            "{HEADER}void main(){{ {} o = vec4(1.0); {} }}\n",
            "{".repeat(10_000),
            "}".repeat(10_000)
        ),
    ));
    out.push((
        "nested-ifs-2000".into(),
        format!(
            "{HEADER}void main(){{ o = vec4(0.0); {} o = vec4(1.0); {} }}\n",
            "if (u_res.x > 0.0) {".repeat(2000),
            "}".repeat(2000)
        ),
    ));
    out.push((
        "local-array-2^28".into(),
        format!(
            "{HEADER}void main(){{ float a[268435456]; a[int(u_res.x)] = 1.0; o = vec4(a[0]); }}\n"
        ),
    ));
    out.push((
        "uniform-array-2^27-vec4".into(),
        format!("{HEADER}uniform vec4 big[134217728];\nvoid main(){{ o = big[int(u_res.x)]; }}\n"),
    ));
    out.push(("const-array-100k".into(), format!("{HEADER}const float k[100000] = float[100000]({});\nvoid main(){{ o = vec4(k[int(u_res.x)]); }}\n", vec!["1.0"; 100_000].join(","))));
    out.push((
        "identifier-1MB".into(),
        format!(
            "{HEADER}void main(){{ float {id} = 1.0; o = vec4({id}); }}\n",
            id = format!("x{}", "a".repeat(1 << 20))
        ),
    ));
    out.push((
        "source-10MB".into(),
        format!(
            "{HEADER}void main(){{ float f = 0.0;\n{} o = vec4(f); }}\n",
            "f += sin(f) * 0.5 + texture(u_tex, v_st + f).r;\n".repeat(200_000)
        ),
    ));
    let mut functions = String::new();
    for i in 0..20_000 {
        functions.push_str(&format!("float f{i}(float x){{ return x * {i}.0; }}\n"));
    }
    out.push((
        "functions-20000".into(),
        format!("{HEADER}{functions}void main(){{ o = vec4(f19999(1.0)); }}\n"),
    ));
    let mut chain = String::from("float g0(float x){ return x + 1.0; }\n");
    for i in 1..2000 {
        chain.push_str(&format!(
            "float g{i}(float x){{ return g{p}(x) + g{p}(x * 0.5); }}\n",
            p = i - 1
        ));
    }
    out.push((
        "call-tree-2^2000-inlined".into(),
        format!("{HEADER}{chain}void main(){{ o = vec4(g1999(u_res.x)); }}\n"),
    ));
    out.push(("recursion".into(), format!("{HEADER}float r(float x){{ return x > 0.0 ? r(x - 1.0) : 0.0; }}\nvoid main(){{ o = vec4(r(u_res.x)); }}\n")));
    out.push(("loop-1e9-texture".into(), format!("{HEADER}void main(){{ vec4 a = vec4(0.0); for (int i = 0; i < 1000000000; i++) a += texture(u_tex, v_st + float(i)); o = a; }}\n")));
    let mut loops = String::new();
    for i in 0..64 {
        loops.push_str(&format!("for (int i{i} = 0; i{i} < 4; i{i}++) "));
    }
    out.push((
        "nested-loops-64".into(),
        format!("{HEADER}void main(){{ float a = 0.0; {loops} a += 1.0; o = vec4(a); }}\n"),
    ));
    out.push(("constant-loop-unroll-1e6".into(), format!("{HEADER}void main(){{ float a = 0.0; for (int i = 0; i < 1000000; i++) a += sin(float(i)); o = vec4(a); }}\n")));
    out.push(("struct-nesting-500".into(), {
        let mut s = String::from("struct S0 { float x; };\n");
        for i in 1..500 {
            s.push_str(&format!("struct S{i} {{ S{p} a; S{p} b; }};\n", p = i - 1));
        }
        format!("{HEADER}{s}uniform S20 u_s;\nvoid main(){{ o = vec4(1.0); }}\n")
    }));
    out.push((
        "swizzle-chain-100k".into(),
        format!(
            "{HEADER}void main(){{ vec4 v = vec4(u_res, 1.0, 1.0); o = v{}; }}\n",
            ".wzyx".repeat(100_000)
        ),
    ));
    out
}

/// What one input did.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Outcome {
    /// `ok`, `reject:<stage>`, `reject:naga-msl`, or `panic`.
    pub result: String,
    /// First line of the rejection message, for triage.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// Translates `source` and writes naga's MSL for both stages (wgpu runs the
/// same writer at pipeline creation). Panics are outcomes, not crashes.
pub fn outcome(source: &str, line_offset: u32) -> Outcome {
    let result = std::panic::catch_unwind(|| match pipeline::translate(source, line_offset) {
        Err(f) => Outcome {
            result: format!("reject:{}", f.stage.name()),
            detail: f
                .message
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect(),
        },
        Ok(program) => {
            for module in [&program.vertex.module, &program.fragment.module] {
                if let Err(e) = pipeline::msl(module) {
                    return Outcome {
                        result: "reject:naga-msl".into(),
                        detail: e.chars().take(400).collect(),
                    };
                }
            }
            Outcome {
                result: "ok".into(),
                detail: String::new(),
            }
        }
    });
    result.unwrap_or_else(|_| Outcome {
        result: "panic".into(),
        detail: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<Seed> {
        serde_json::from_value(serde_json::json!([
            { "id": "a", "frag": "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){ o = vec4(0.25, 0.5, 1.0, 1.0); }\n", "lineOffset": 3 },
            { "id": "b", "frag": "#version 300 es\nprecision highp float;\nout vec4 o;\nfloat f(float x){ return x * 2.0; }\nvoid main(){ o = vec4(f(3.0)); }\n", "lineOffset": 3 }
        ]))
        .unwrap()
    }

    #[test]
    fn mutants_are_deterministic_and_keep_the_preamble() {
        let corpus = corpus();
        for i in 0..200 {
            let (a, note_a, _) = mutant(&corpus, 7, i);
            let (b, note_b, _) = mutant(&corpus, 7, i);
            assert_eq!(a, b);
            assert_eq!(note_a, note_b);
            assert!(
                a.starts_with("#version 300 es\nprecision highp float;\nout vec4 o;\n"),
                "{a}"
            );
        }
        assert_ne!(mutant(&corpus, 7, 0).0, mutant(&corpus, 8, 0).0);
    }

    #[test]
    fn bombs_are_well_formed_es_programs() {
        for (name, source) in bombs() {
            assert!(source.starts_with("#version 300 es\n"), "{name}");
        }
    }
}
