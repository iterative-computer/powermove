//! Robustness of the translation pipeline against untrusted GLSL (plan
//! section 11: "Store GLSL is untrusted input: the pipeline is fuzzed and
//! resource-limited").
//!
//! Two inputs:
//!
//! * Mutants: deterministic mutations of every corpus program's author lines
//!   (deletions, duplications, swaps, extreme literals, hostile tokens, splices
//!   across programs, byte flips). Mutant `i` of seed `s` is always the same
//!   source, so any finding reproduces from `(seed, i)`.
//! * Bombs: handwritten sources aimed at resource limits (macro expansion,
//!   nesting depth, huge arrays, huge sources, many functions, recursion).
//!
//! The parent never translates anything itself. It runs a worker process that
//! prints a `begin` line before each input and a result line after it, so a
//! crash (abort, segfault) in glslang, SPIRV-Cross, or naga is attributed to
//! the exact input, and a hang is cut off by a per-input deadline. Both back
//! ends run on every input; naga's MSL writer runs too (wgpu would run it).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::case::Corpus;
use crate::pipeline::{self, Path};

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
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const EXTREMES: &[&str] = &[
    "0.0", "-0.0", "1e38", "3.4028235e38", "1e39", "1e-45", "-1e38", "2147483647", "-2147483648", "4294967295u", "1000000000", "65536", "0", "(1.0/0.0)", "(0.0/0.0)",
    "1e-38",
];

const TOKENS: &[&str] = &[
    "{", "}", "(", ")", ";", ",", "[", "]", "for(int pm_i=0;pm_i<1000000;pm_i++){", "while(true){", "do{", "}while(true);", "#define X X X\n", "texture(u_tex, vec2(0.5))",
    "dFdx(", "dFdy(", "fwidth(", "float pm_a[1000000];", "struct S{float x;};", "return;", "discard;", "break;", "continue;", "switch(1){", "case 0:", "default:", "#if 1\n",
    "#endif\n", "#line 999999\n", "#line 0\n", "/*", "*/", "\\\n", "\u{e9}", "\u{202e}", "#pragma optimize(off)\n", "#extension GL_EXT_shader_texture_lod : enable\n",
    "#version 450\n", "#undef main\n", "#define main pm_main\n", "invariant ", "flat ", "centroid ", "uniform ", "in ", "out ", "inout ", "const ", "highp ", "lowp ",
    "mat4(", "vec4(", "ivec2(", "uint(", "bool(", "int(", ".xyzw", ".rgba", ".sss", "++", "--", "<<", ">>", "%", "^^", "?", ":", "=", "==", "!=", "&&", "||",
    "texelFetch(u_tex, ivec2(-1), 0)", "textureLod(u_tex, vec2(0.5), 1e9)", "textureSize(u_tex, 99)", "gl_FragCoord", "gl_FragDepth = 2.0;", "gl_PointCoord",
    "inverse(mat4(0.0))", "pow(-1.0, 0.5)", "sqrt(-1.0)", "log(0.0)", "1.0/0.0", "0/0", "ivec4(1)/ivec4(0)", "int(1e38)", "uint(-1.0)",
];

fn author_region(frag: &str, line_offset: u32) -> usize {
    frag.split_inclusive('\n').take(line_offset as usize).map(str::len).sum()
}

fn char_boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Mutant `index` for `seed`: 1 to 4 stacked mutations of one corpus program.
pub fn mutant(corpus: &Corpus, seed: u64, index: u64) -> (String, String, u32) {
    let mut rng = Rng::new(seed, index);
    let case = &corpus.cases[rng.below(corpus.cases.len())];
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
            1 | 2 | 3 => {
                let mut lines: Vec<String> = body.split_inclusive('\n').map(str::to_string).collect();
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
                let numbers: Vec<usize> = (0..bytes.len()).filter(|&i| bytes[i].is_ascii_digit() && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_' || bytes[i - 1] == b'.'))).collect();
                if numbers.is_empty() {
                    continue;
                }
                let a = numbers[rng.below(numbers.len())];
                let mut b = a;
                while b < bytes.len() && (bytes[b].is_ascii_digit() || bytes[b] == b'.' || bytes[b] == b'e' || bytes[b] == b'u') {
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
                let other = &corpus.cases[rng.below(corpus.cases.len())];
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
    (format!("{head}{body}"), format!("{} ops {:?}", case.id, ops), case.line_offset)
}

const HEADER: &str = "#version 300 es\nprecision highp float;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n";

/// Handwritten resource bombs (name, source).
pub fn bombs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut macros = String::from("#define A0 1.0\n");
    for i in 1..=24 {
        macros.push_str(&format!("#define A{i} (A{p} + A{p})\n", p = i - 1));
    }
    out.push(("macro-expansion-2^24".into(), format!("{HEADER}{macros}void main(){{ o = vec4(A24); }}\n")));
    let depth = 20_000;
    out.push(("nested-parens-20000".into(), format!("{HEADER}void main(){{ float f = {}1.0{}; o = vec4(f); }}\n", "(".repeat(depth), ")".repeat(depth))));
    out.push(("nested-blocks-10000".into(), format!("{HEADER}void main(){{ {} o = vec4(1.0); {} }}\n", "{".repeat(10_000), "}".repeat(10_000))));
    out.push(("nested-ifs-2000".into(), format!("{HEADER}void main(){{ o = vec4(0.0); {} o = vec4(1.0); {} }}\n", "if (u_res.x > 0.0) {".repeat(2000), "}".repeat(2000))));
    out.push(("local-array-2^28".into(), format!("{HEADER}void main(){{ float a[268435456]; a[int(u_res.x)] = 1.0; o = vec4(a[0]); }}\n")));
    out.push(("uniform-array-2^27-vec4".into(), format!("{HEADER}uniform vec4 big[134217728];\nvoid main(){{ o = big[int(u_res.x)]; }}\n")));
    out.push(("const-array-100k".into(), format!("{HEADER}const float k[100000] = float[100000]({});\nvoid main(){{ o = vec4(k[int(u_res.x)]); }}\n", vec!["1.0"; 100_000].join(","))));
    out.push(("identifier-1MB".into(), format!("{HEADER}void main(){{ float {id} = 1.0; o = vec4({id}); }}\n", id = format!("x{}", "a".repeat(1 << 20)))));
    out.push(("source-10MB".into(), format!("{HEADER}void main(){{ float f = 0.0;\n{} o = vec4(f); }}\n", "f += sin(f) * 0.5 + texture(u_tex, v_st + f).r;\n".repeat(200_000))));
    let mut functions = String::new();
    for i in 0..20_000 {
        functions.push_str(&format!("float f{i}(float x){{ return x * {i}.0; }}\n"));
    }
    out.push(("functions-20000".into(), format!("{HEADER}{functions}void main(){{ o = vec4(f19999(1.0)); }}\n")));
    let mut chain = String::from("float g0(float x){ return x + 1.0; }\n");
    for i in 1..2000 {
        chain.push_str(&format!("float g{i}(float x){{ return g{p}(x) + g{p}(x * 0.5); }}\n", p = i - 1));
    }
    out.push(("call-tree-2^2000-inlined".into(), format!("{HEADER}{chain}void main(){{ o = vec4(g1999(u_res.x)); }}\n")));
    out.push(("recursion".into(), format!("{HEADER}float r(float x){{ return x > 0.0 ? r(x - 1.0) : 0.0; }}\nvoid main(){{ o = vec4(r(u_res.x)); }}\n")));
    out.push(("loop-1e9-texture".into(), format!("{HEADER}void main(){{ vec4 a = vec4(0.0); for (int i = 0; i < 1000000000; i++) a += texture(u_tex, v_st + float(i)); o = a; }}\n")));
    let mut loops = String::new();
    for i in 0..64 {
        loops.push_str(&format!("for (int i{i} = 0; i{i} < 4; i{i}++) "));
    }
    out.push(("nested-loops-64".into(), format!("{HEADER}void main(){{ float a = 0.0; {loops} a += 1.0; o = vec4(a); }}\n")));
    out.push(("constant-loop-unroll-1e6".into(), format!("{HEADER}void main(){{ float a = 0.0; for (int i = 0; i < 1000000; i++) a += sin(float(i)); o = vec4(a); }}\n")));
    out.push(("struct-nesting-500".into(), {
        let mut s = String::from("struct S0 { float x; };\n");
        for i in 1..500 {
            s.push_str(&format!("struct S{i} {{ S{p} a; S{p} b; }};\n", p = i - 1));
        }
        format!("{HEADER}{s}uniform S20 u_s;\nvoid main(){{ o = vec4(1.0); }}\n")
    }));
    out.push(("swizzle-chain-100k".into(), format!("{HEADER}void main(){{ vec4 v = vec4(u_res, 1.0, 1.0); o = v{}; }}\n", ".wzyx".repeat(100_000))));
    out
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Outcome {
    pub index: i64,
    pub naga: String,
    pub spvc: String,
    /// First line of the rejection message (naga path), for triage.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub ms: f64,
    pub max_rss_mb: f64,
}

fn max_rss_mb() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    usage.ru_maxrss as f64 / (1024.0 * 1024.0) // bytes on macOS
}

fn outcome_of(vertex: &str, source: &str, line_offset: u32, path: Path, gpu: Option<&crate::render::Gpu>) -> (String, String) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match pipeline::translate(vertex, source, line_offset, path) {
        Err(f) => (format!("reject:{}", f.stage), f.message.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().chars().take(200).collect()),
        Ok(translated) => {
            if path == Path::Naga {
                // wgpu runs naga's MSL writer at pipeline creation.
                for stage in [&translated.vertex, &translated.fragment] {
                    let module = stage.naga.as_ref().unwrap();
                    let info = match pipeline::naga_validate(module) {
                        Ok(info) => info,
                        Err(e) => return ("reject:naga-validate".into(), e.chars().take(200).collect()),
                    };
                    let options = naga::back::msl::Options { lang_version: (3, 0), ..Default::default() };
                    if let Err(e) = naga::back::msl::write_string(module, &info, &options, &Default::default()) {
                        return ("reject:naga-msl".into(), format!("{e:?}").chars().take(200).collect());
                    }
                }
            }
            // With `--metal`, also create the wgpu pipeline (the Metal compiler
            // runs out of process in MTLCompilerService).
            if let Some(gpu) = gpu {
                if let Err(f) = gpu.build(translated, path, "fuzz") {
                    return (format!("reject:{}", f.stage), f.message.chars().take(200).collect());
                }
            }
            ("ok".into(), String::new())
        }
    }));
    result.unwrap_or_else(|_| ("panic".into(), String::new()))
}

/// Where the inputs come from.
#[derive(Clone)]
enum Inputs {
    /// Deterministic mutants of the corpus for a seed.
    Mutants(u64),
    /// The handwritten resource bombs.
    Bombs,
    /// `<index>.frag` files in a directory (probes, dumped inputs).
    Dir(PathBuf),
}

impl Inputs {
    fn from_args(get: &dyn Fn(&str) -> Option<String>, args: &[String]) -> Self {
        if let Some(dir) = get("--dir") {
            Inputs::Dir(PathBuf::from(dir))
        } else if args.iter().any(|a| a == "--bombs") {
            Inputs::Bombs
        } else {
            Inputs::Mutants(get("--seed").map_or(1, |s| s.parse().unwrap()))
        }
    }

    fn args(&self) -> Vec<String> {
        match self {
            Inputs::Mutants(seed) => vec!["--seed".into(), seed.to_string()],
            Inputs::Bombs => vec!["--bombs".into()],
            Inputs::Dir(dir) => vec!["--dir".into(), dir.display().to_string()],
        }
    }

    /// One worker per input for bombs and probes, so peak memory is per input.
    fn isolated(&self) -> bool {
        !matches!(self, Inputs::Mutants(_))
    }

    fn count(&self, iterations: i64) -> anyhow::Result<i64> {
        Ok(match self {
            Inputs::Mutants(_) => iterations,
            Inputs::Bombs => bombs().len() as i64,
            Inputs::Dir(dir) => std::fs::read_dir(dir)?.filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "frag"))).count() as i64,
        })
    }

    /// Source, a note for abnormal outcomes, and the line offset.
    fn get(&self, corpus: &Corpus, index: i64) -> anyhow::Result<(String, String, u32)> {
        Ok(match self {
            Inputs::Mutants(seed) => mutant(corpus, *seed, index as u64),
            Inputs::Bombs => {
                let (name, source) = bombs().swap_remove(index as usize);
                (source, name, 0)
            }
            Inputs::Dir(dir) => {
                let source = std::fs::read_to_string(dir.join(format!("{index}.frag")))?;
                let name = std::fs::read_to_string(dir.join(format!("{index}.name"))).unwrap_or_default();
                (source, name, 0)
            }
        })
    }

    fn results_name(&self, metal: &str) -> String {
        match self {
            Inputs::Mutants(seed) => format!("mutants-seed{seed}{metal}.jsonl"),
            Inputs::Bombs => format!("bombs{metal}.jsonl"),
            Inputs::Dir(_) => format!("engine{metal}.jsonl"),
        }
    }
}

/// Worker: translate inputs `start..start+count`, printing `begin <i>` then
/// one JSON result line per input.
pub fn worker(args: &[String]) -> anyhow::Result<()> {
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned());
    let corpus: Corpus = serde_json::from_slice(&std::fs::read(get("--corpus").unwrap_or_else(|| "work/corpus.json".into()))?)?;
    let inputs = Inputs::from_args(&get, args);
    let start: i64 = get("--start").map_or(0, |s| s.parse().unwrap());
    let count: i64 = get("--count").map_or(1000, |s| s.parse().unwrap());
    // Panics are recorded as outcomes; keep their messages off stderr.
    std::panic::set_hook(Box::new(|_| {}));
    let stdout = std::io::stdout();
    let gpu = if args.iter().any(|a| a == "--metal") { Some(crate::render::Gpu::new()?) } else { None };
    for index in start..start + count {
        let (source, _, offset) = inputs.get(&corpus, index)?;
        writeln!(stdout.lock(), "begin {index}")?;
        stdout.lock().flush()?;
        let started = Instant::now();
        // `path`/`done` lines let the parent attribute a crash or hang to the
        // back end that was running.
        writeln!(stdout.lock(), "path naga")?;
        stdout.lock().flush()?;
        let (naga, detail) = outcome_of(&corpus.vertex, &source, offset, Path::Naga, gpu.as_ref());
        writeln!(stdout.lock(), "done naga {naga}")?;
        writeln!(stdout.lock(), "path spvc")?;
        stdout.lock().flush()?;
        let (spvc, _) = outcome_of(&corpus.vertex, &source, offset, Path::SpirvCross, gpu.as_ref());
        let outcome = Outcome { index, naga, spvc, detail, ms: started.elapsed().as_secs_f64() * 1000.0, max_rss_mb: max_rss_mb() };
        writeln!(stdout.lock(), "{}", serde_json::to_string(&outcome)?)?;
        stdout.lock().flush()?;
    }
    Ok(())
}

/// Parent: run workers over the inputs, restarting after crashes and hangs.
pub fn run(args: &[String]) -> anyhow::Result<()> {
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned());
    let corpus_path = get("--corpus").unwrap_or_else(|| "work/corpus.json".into());
    let iterations: i64 = get("--iterations").map_or(2000, |s| s.parse().unwrap());
    let deadline = Duration::from_secs(get("--deadline").map_or(30, |s| s.parse().unwrap()));
    let inputs = Inputs::from_args(&get, args);
    let out = match (&inputs, get("--out")) {
        (_, Some(out)) => PathBuf::from(out),
        (Inputs::Dir(dir), None) => dir.clone(),
        _ => PathBuf::from("work/fuzz"),
    };
    std::fs::create_dir_all(&out)?;
    let corpus: Corpus = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    let exe = std::env::current_exe()?;
    let total = inputs.count(iterations)?;
    let metal_flag = args.iter().any(|a| a == "--metal");
    let results_path = out.join(inputs.results_name(if metal_flag { "-metal" } else { "" }));
    let mut results = std::fs::File::create(&results_path)?;
    let mut next = 0i64;
    while next < total {
        let count = if inputs.isolated() { 1 } else { total - next };
        let mut child = Command::new(&exe)
            .args(["fuzz-worker", "--corpus", &corpus_path, "--start", &next.to_string(), "--count", &count.to_string()])
            .args(inputs.args())
            .args(if metal_flag { vec!["--metal"] } else { vec![] })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap_or_default()).is_err() {
                    break;
                }
            }
        });
        let mut begun: Option<i64> = None;
        let mut begun_at = Instant::now();
        let mut progress = Progress::default();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if let Some(i) = line.strip_prefix("begin ") {
                        begun = Some(i.parse()?);
                        begun_at = Instant::now();
                        progress = Progress::default();
                    } else if let Some(path) = line.strip_prefix("path ") {
                        progress.running = path.to_string();
                    } else if let Some(rest) = line.strip_prefix("done ") {
                        if let Some(("naga", outcome)) = rest.split_once(' ') {
                            progress.naga = Some(outcome.to_string());
                        }
                    } else if !line.is_empty() {
                        writeln!(results, "{line}")?;
                        begun = None;
                        next += 1;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if begun.is_some() && begun_at.elapsed() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let i = begun.unwrap();
                        record_abnormal(&mut results, &out, &corpus, &inputs, i, &progress, "timeout", begun_at.elapsed())?;
                        next = i + 1;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let status = child.wait()?;
                    if let Some(i) = begun {
                        record_abnormal(&mut results, &out, &corpus, &inputs, i, &progress, &format!("crash ({status})"), begun_at.elapsed())?;
                        next = i + 1;
                    } else if next < total && !inputs.isolated() {
                        anyhow::bail!("worker exited without output ({status})");
                    }
                    break;
                }
            }
        }
    }
    eprintln!("wrote {}", results_path.display());
    Ok(())
}

/// Write the inputs (mutants `0..iterations` of `--seed`, or every bomb) as
/// `<index>.frag` files so `scripts/webgl-compile.ts` can ask WebGL whether
/// it links each one: the differential oracle for compile parity.
pub fn dump(args: &[String]) -> anyhow::Result<()> {
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned());
    let corpus: Corpus = serde_json::from_slice(&std::fs::read(get("--corpus").unwrap_or_else(|| "work/corpus.json".into()))?)?;
    let iterations: i64 = get("--iterations").map_or(2000, |s| s.parse().unwrap());
    let inputs = Inputs::from_args(&get, args);
    let out = PathBuf::from(get("--out").unwrap_or_else(|| match &inputs {
        Inputs::Mutants(seed) => format!("work/fuzz/mutants-seed{seed}"),
        _ => "work/fuzz/bombs".into(),
    }));
    std::fs::create_dir_all(&out)?;
    for index in 0..inputs.count(iterations)? {
        let (source, note, _) = inputs.get(&corpus, index)?;
        std::fs::write(out.join(format!("{index}.frag")), source)?;
        std::fs::write(out.join(format!("{index}.name")), note)?;
    }
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// Which back end a worker is running for the current input, and the naga
/// outcome once it is known.
#[derive(Default)]
struct Progress {
    running: String,
    naga: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn record_abnormal(results: &mut std::fs::File, out: &std::path::Path, corpus: &Corpus, inputs: &Inputs, index: i64, progress: &Progress, what: &str, elapsed: Duration) -> anyhow::Result<()> {
    let (source, note, _) = inputs.get(corpus, index)?;
    // The reproducer stays in work/ (gitignored): mutants embed corpus sources.
    if !matches!(inputs, Inputs::Dir(_)) {
        std::fs::write(out.join(format!("{}-{index}.frag", if matches!(inputs, Inputs::Bombs) { "bomb" } else { "mutant" })), &source)?;
    }
    let naga = progress.naga.clone().unwrap_or_else(|| what.to_string());
    let spvc = if progress.running == "spvc" { what.to_string() } else { "not-run".to_string() };
    writeln!(results, "{}", serde_json::json!({ "index": index, "naga": naga, "spvc": spvc, "ms": elapsed.as_secs_f64() * 1000.0, "note": note }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Corpus {
        serde_json::from_value(serde_json::json!({
            "vertex": "#version 300 es\nin vec2 a_pos; void main(){ gl_Position = vec4(a_pos, 0.0, 1.0); }\n",
            "images": [],
            "cases": [
                { "id": "a", "group": "g", "kind": "effect", "frag": "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){ o = vec4(0.25, 0.5, 1.0, 1.0); }\n", "lineOffset": 3, "variants": [] },
                { "id": "b", "group": "g", "kind": "effect", "frag": "#version 300 es\nprecision highp float;\nout vec4 o;\nfloat f(float x){ return x * 2.0; }\nvoid main(){ o = vec4(f(3.0)); }\n", "lineOffset": 3, "variants": [] }
            ]
        }))
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
            assert!(a.starts_with("#version 300 es\nprecision highp float;\nout vec4 o;\n"), "{a}");
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
