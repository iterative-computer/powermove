//! `pm-cli`: the engine's headless binary (plan section 22, "The CLI"; section
//! 25, "Infrastructure").
//!
//! ```text
//! pm-cli render <project.pmv|.json> -o <out.mov|.mp4> [options]
//! pm-cli parity render [--capture <dir>]... [--only <prefix>] [--real <file.pmv>]
//!                      [--no-definitions] [--determinism] [--diffs <dir>] [--json <file>]
//! pm-cli parity eval [options]
//! pm-cli scope <project.pmv|.json> -o <dir> [options] | --frame <file> -o <dir>
//! ```
//!
//! `render` and `parity render` exit 0 done, 1 failed, 2 usage, 3 the engine
//! cannot render this yet (the reasons are printed; the desktop app routes
//! such projects to the current engine); `render` and `scope` exit 4 when a program
//! only the app's WebGL fallback runner draws makes the project not
//! headless-renderable (named per layer and definition). `parity eval` (evaluation parity,
//! section 25; docs/engine/phase1-gate.md) keeps its own statuses, listed in
//! `pm-cli parity eval --help`.

mod angle;
mod compare;
mod corpus;
mod fonts;
mod parameters;
mod parity;
mod render;
mod render_parity;
mod scope;

use std::path::PathBuf;
use std::process::ExitCode;

const EVAL_USAGE: &str = "usage: pm-cli parity eval [options]

Evaluates every project of a parity corpus capture at every captured sample
time and compares every dumped value with section 25's tolerances.

options:
  --corpus <dir>          capture directory (default: the newest v1-* capture,
                          else v0-*, under ~/Library/Application Support/
                          Powermove Engine Corpus)
  --only <prefix,...>     only projects whose <group>/<name> starts with a prefix
  --exclude <prefix,...>  leave projects out
  --fresh-font-cache      measure text as a fresh renderer instead of the
                          capture renderer's font cache state
  --wasm                  also compare through pm-wasm: the built module in Node
                          and the same engine natively (bit identity)
  --no-native             skip the native pm-eval comparison
  --wasm-module <file>    the module (default: apps/desktop/src/renderer/public/
                          engine/pm_wasm.wasm, from pm-wasm/scripts/build.mjs)
  --wasm-subsets          serve metrics-and-shaping subsets as the app does
                          (ink-dependent values are then not compared)
  --node <bin>            Node binary (default: node)
  --json <file>           write the full report as JSON
  --verbose               list every mismatch
  --samples <n>           mismatches listed per category (default 8)
  --help, -h              print this text

exit status: 0 every compared value within tolerance or allowed and native and
wasm identical; 1 a mismatch or an identity difference; 2 only tolerance
questions; 64 usage; 70 the run failed.";

const USAGE: &str = "usage:
  pm-cli render <project.pmv|.json> -o <out.mov|.mp4|.webm|.png|folder> [options]
      --preset master|master-alpha|web|web-hevc|png   (default master: ProRes 422 HQ)
      --format prores-proxy|prores-lt|prores|prores-hq|prores-4444|prores-4444xq|
               h264|hevc|hevc10|vp9|vp8|png|png16|still|still16
               (HEVC goes in MOV when the output ends in .mov; PNG sequences
               are written to a new folder)
      --quality draft|high|max             H.264, HEVC, and WebM at 4, 16, or 40 Mbps
      --alpha none|straight|premultiplied  ProRes 4444, HEVC 8-bit, WebM, PNG
      --range all|work|START:END           seconds (default all)
      --fps N | N/D | 23.976 | 29.97 | ...  (default the composition's rate)
      --scale S | --size WxH               (default the composition's size)
      --audio pcm16|pcm24|float|aac        (default 24-bit PCM in MOV, AAC in MP4)
      --timecode HH:MM:SS:FF  --reel NAME  start timecode (`;` for drop-frame), MOV
      --no-audio  --no-captions  --color quicktime|broadcast|srgb|rec709
      --dither on|off  --precision 16|32
      --composition ID  --media ASSET=FILE  --force (replace an existing file)
      --definitions FILE  a definition set snapshot (contract 10, JSON) to draw
               effects, transitions, and extension layers with (default: the
               built-in definitions)
      --param NAME=JSON (repeatable; validates against the parameter definition)
      --ui-scale S  the editor's display scale factor, which fixes how its
                    renderer measures system-ui text (default 2)
      --approximate-svg  level 1 SVG images are rasterized by the Chromium
               helper (`powermove render` starts it; a bundled pm-cli starts
               native-angle-child.cjs). Without one, a project that draws
               them fails (svg-chromium-unavailable) unless this flag accepts
               resvg's approximation, named in a notice
  pm-cli parity render [--capture DIR]... [--only PREFIX] [--diffs DIR] [--json FILE]
                       [--real FILE.pmv] [--no-definitions] [--determinism]
                       [--svg-rasters DIR]
      compares engine renders with the legacy captures of the parity corpus
      (default: the newest v1-* and render-* captures). The renderer holds
      the built-in definitions as the kernel pushes them (--no-definitions:
      none, so effects and transitions route). --real adds each capture's
      real/<slug> project with its media unpacked from FILE.pmv (the file the
      capture opened, checked by SHA-256). --determinism renders every
      sample again with the renderer's caches dropped and requires
      bit-identical buffers. Level 1 SVG images are drawn from Chromium's
      rasters in --svg-rasters DIR (missing ones are listed in its
      needs.json for engine/crates/pm-render/tools/svg-rasters), else by
      resvg with an \"approximate SVG\" notice
  pm-cli parity eval [options]
      evaluation parity against the corpus (see pm-cli parity eval --help)
  pm-cli scope <project.pmv|.json> -o <dir> [options]
  pm-cli scope --frame <capture.rgba32f|frame.png> -o <dir>
      waveform, RGB parade, and vectorscope of a 32-bit working render (or a
      captured frame): waveform.png, parade.png, vectorscope.png, scope.json
      --time S  --composition ID  --size WxH | --scale S  --media ASSET=FILE
      --motion-blur off|export|N (default export)  --transparent  --columns N
      --definitions FILE  --approximate-svg (as for render: user GLSL programs
               the desktop pinned to its WebGL runner, or has not verified,
               run in the helper's Chromium, else exit 4)
      --ui-scale S (default 2)";

/// Exit 4 (`render` and `scope`): user programs only the app's WebGL
/// runner may draw here, and no Chromium helper is attached.
fn not_headless(reasons: &[String]) -> ExitCode {
    eprintln!(
        "pm-cli: not headless-renderable: only the app's WebGL fallback runner draws these \
         programs (the native pipeline cannot translate them, the desktop pinned them to its \
         runner, or they are not verified natively yet), and pm-cli has no Chromium helper \
         (`powermove render` and a bundled pm-cli start one):"
    );
    for r in reasons {
        eprintln!("  {r}");
    }
    ExitCode::from(4)
}

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What the `parity eval` arguments ask for.
enum Parsed {
    Run(Box<parity::Options>),
    Help,
}

fn parse_eval(args: &[String]) -> Result<Parsed, String> {
    let mut o = parity::Options {
        capture: None,
        only: Vec::new(),
        exclude: Vec::new(),
        fresh_font_cache: false,
        native: true,
        wasm: false,
        wasm_module: None,
        wasm_subsets: false,
        node: "node".into(),
        json: None,
        verbose: false,
        max_samples: 8,
    };
    let mut i = 0;
    let value = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("{name} needs a value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--corpus" => o.capture = Some(PathBuf::from(value(&mut i, "--corpus")?)),
            "--only" => o.only.extend(list(&value(&mut i, "--only")?)),
            "--exclude" => o.exclude.extend(list(&value(&mut i, "--exclude")?)),
            "--fresh-font-cache" => o.fresh_font_cache = true,
            "--wasm" => o.wasm = true,
            "--no-native" => o.native = false,
            "--wasm-module" => o.wasm_module = Some(PathBuf::from(value(&mut i, "--wasm-module")?)),
            "--wasm-subsets" => o.wasm_subsets = true,
            "--node" => o.node = value(&mut i, "--node")?,
            "--json" => o.json = Some(PathBuf::from(value(&mut i, "--json")?)),
            "--verbose" => o.verbose = true,
            "--help" | "-h" => return Ok(Parsed::Help),
            "--samples" => {
                o.max_samples = value(&mut i, "--samples")?
                    .parse()
                    .map_err(|_| "--samples needs a number".to_owned())?
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    if !o.native && !o.wasm {
        return Err("--no-native needs --wasm".into());
    }
    Ok(Parsed::Run(Box::new(o)))
}

fn fail_usage(message: &str) -> ExitCode {
    eprintln!("pm-cli: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

fn value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    args.get(*i)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_render(args: &[String]) -> Result<render::Options, String> {
    let mut o = render::Options::default();
    let mut input = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" | "--output" => o.output = PathBuf::from(value(args, &mut i, a)?),
            "--preset" => o.preset = Some(value(args, &mut i, a)?.into()),
            "--format" => o.format = Some(value(args, &mut i, a)?.into()),
            "--quality" => o.quality = Some(value(args, &mut i, a)?.into()),
            "--alpha" => o.alpha = Some(value(args, &mut i, a)?.into()),
            "--range" => o.range = Some(value(args, &mut i, a)?.into()),
            "--fps" => o.fps = Some(value(args, &mut i, a)?.into()),
            "--scale" => {
                let v = value(args, &mut i, a)?;
                o.scale = Some(v.parse().map_err(|_| format!("scale {v:?}"))?);
            }
            "--size" => {
                let v = value(args, &mut i, a)?;
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| format!("size {v:?} (WxH)"))?;
                let p = |s: &str| s.parse::<u32>().map_err(|_| format!("size {v:?}"));
                o.size = Some((p(w)?, p(h)?));
            }
            "--no-audio" => o.no_audio = true,
            "--color" => o.color = Some(value(args, &mut i, a)?.into()),
            "--dither" => {
                o.dither = Some(match value(args, &mut i, a)? {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("dither {v:?} (on or off)")),
                })
            }
            "--precision" => {
                let v = value(args, &mut i, a)?;
                o.precision = Some(v.parse().map_err(|_| format!("precision {v:?}"))?);
            }
            "--composition" => o.composition = Some(value(args, &mut i, a)?.into()),
            "--media" => {
                let v = value(args, &mut i, a)?;
                let (asset, file) = v
                    .split_once('=')
                    .ok_or_else(|| format!("media {v:?} (ASSET=FILE)"))?;
                o.media.push((asset.into(), PathBuf::from(file)));
            }
            "--force" => o.force = true,
            "--audio" => o.audio = Some(value(args, &mut i, a)?.into()),
            "--timecode" => o.timecode = Some(value(args, &mut i, a)?.into()),
            "--reel" => o.reel = Some(value(args, &mut i, a)?.into()),
            "--no-captions" => o.no_captions = true,
            "--approximate-svg" => o.approximate_svg = true,
            "--param" => o
                .parameters
                .push(parameters::parse(value(args, &mut i, a)?)?),
            "--definitions" => o.definitions = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--ui-scale" => o.ui_scale = Some(fonts::parse_ui_scale(value(args, &mut i, a)?)?),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => {
                if input.replace(PathBuf::from(other)).is_some() {
                    return Err("one project at a time".into());
                }
            }
        }
        i += 1;
    }
    o.input = input.ok_or("no project file")?;
    if o.output.as_os_str().is_empty() {
        return Err("no output (-o)".into());
    }
    Ok(o)
}

fn parse_scope(args: &[String]) -> Result<scope::Options, String> {
    let mut o = scope::Options::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" | "--output" => o.output = PathBuf::from(value(args, &mut i, a)?),
            "--frame" => o.frame = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--time" => {
                let v = value(args, &mut i, a)?;
                o.time = v.parse().map_err(|_| format!("time {v:?}"))?;
            }
            "--composition" => o.composition = Some(value(args, &mut i, a)?.into()),
            "--size" => {
                let v = value(args, &mut i, a)?;
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| format!("size {v:?} (WxH)"))?;
                let p = |s: &str| s.parse::<u32>().map_err(|_| format!("size {v:?}"));
                o.size = Some((p(w)?, p(h)?));
            }
            "--scale" => {
                let v = value(args, &mut i, a)?;
                o.scale = Some(v.parse().map_err(|_| format!("scale {v:?}"))?);
            }
            "--media" => {
                let v = value(args, &mut i, a)?;
                let (asset, file) = v
                    .split_once('=')
                    .ok_or_else(|| format!("media {v:?} (ASSET=FILE)"))?;
                o.media.push((asset.into(), PathBuf::from(file)));
            }
            "--motion-blur" => o.motion_blur = Some(value(args, &mut i, a)?.into()),
            "--transparent" => o.transparent = true,
            "--approximate-svg" => o.approximate_svg = true,
            "--definitions" => o.definitions = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--ui-scale" => o.ui_scale = Some(fonts::parse_ui_scale(value(args, &mut i, a)?)?),
            "--columns" => {
                let v = value(args, &mut i, a)?;
                o.columns = Some(v.parse().map_err(|_| format!("columns {v:?}"))?);
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => {
                if o.input.replace(PathBuf::from(other)).is_some() {
                    return Err("one project at a time".into());
                }
            }
        }
        i += 1;
    }
    if o.output.as_os_str().is_empty() {
        return Err("no output directory (-o)".into());
    }
    if o.input.is_some() == o.frame.is_some() {
        return Err("a project or --frame (not both)".into());
    }
    Ok(o)
}

fn parse_parity(args: &[String]) -> Result<render_parity::Options, String> {
    let mut o = render_parity::Options {
        captures: vec![],
        only: None,
        diffs: None,
        json: None,
        real: None,
        definitions: true,
        determinism: false,
        svg_rasters: None,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--capture" => o.captures.push(PathBuf::from(value(args, &mut i, a)?)),
            "--only" => o.only = Some(value(args, &mut i, a)?.into()),
            "--diffs" => o.diffs = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--json" => o.json = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--real" => o.real = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--no-definitions" => o.definitions = false,
            "--determinism" => o.determinism = true,
            "--svg-rasters" => o.svg_rasters = Some(PathBuf::from(value(args, &mut i, a)?)),
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    if o.captures.is_empty() {
        o.captures = corpus::default_captures();
    }
    Ok(o)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("render") => {
            let options = match parse_render(&args[1..]) {
                Ok(o) => o,
                Err(e) => return fail_usage(&e),
            };
            match render::run(&options) {
                Ok(summary) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summary).unwrap_or_default()
                    );
                    ExitCode::SUCCESS
                }
                Err(render::Failure::Usage(m)) => fail_usage(&m),
                Err(render::Failure::Unsupported(reasons)) => {
                    eprintln!("pm-cli: the new engine cannot render this project yet:");
                    for r in reasons {
                        eprintln!("  {r}");
                    }
                    ExitCode::from(3)
                }
                Err(render::Failure::NotHeadless(reasons)) => not_headless(&reasons),
                Err(render::Failure::Failed(m)) => {
                    eprintln!("pm-cli: {m}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("parity") => match args.get(1).map(String::as_str) {
            Some("render") => {
                let options = match parse_parity(&args[2..]) {
                    Ok(o) => o,
                    Err(e) => return fail_usage(&e),
                };
                match render_parity::run(&options) {
                    Ok(report)
                        if report.compared > 0
                            && report.failed == 0
                            && report.determinism_failed == 0 =>
                    {
                        ExitCode::SUCCESS
                    }
                    Ok(report) => {
                        if report.compared == 0 {
                            eprintln!("pm-cli: no supported corpus project was compared");
                        }
                        ExitCode::FAILURE
                    }
                    Err(e) => {
                        eprintln!("pm-cli: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Some("eval") => {
                let code = match parse_eval(&args[2..]) {
                    Ok(Parsed::Help) => {
                        println!("{EVAL_USAGE}");
                        0
                    }
                    Ok(Parsed::Run(opts)) => match parity::run(&opts) {
                        Ok(report) => {
                            parity::print_report(&report);
                            parity::status(&report)
                        }
                        Err(e) => {
                            eprintln!("pm-cli parity eval: {e}");
                            70
                        }
                    },
                    Err(e) => {
                        eprintln!("pm-cli parity eval: {e}\n\n{EVAL_USAGE}");
                        64
                    }
                };
                ExitCode::from(u8::try_from(code).unwrap_or(70))
            }
            Some("-h" | "--help" | "help") => {
                println!("{USAGE}");
                ExitCode::SUCCESS
            }
            _ => fail_usage("parity takes `render` or `eval`"),
        },
        Some("scope") => {
            let options = match parse_scope(&args[1..]) {
                Ok(o) => o,
                Err(e) => return fail_usage(&e),
            };
            match scope::run(&options) {
                Ok(stats) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&stats).unwrap_or_default()
                    );
                    ExitCode::SUCCESS
                }
                Err(render::Failure::Usage(m)) => fail_usage(&m),
                Err(render::Failure::NotHeadless(reasons)) => not_headless(&reasons),
                Err(render::Failure::Unsupported(reasons)) => {
                    eprintln!("pm-cli: the new engine cannot render this project yet:");
                    for r in reasons {
                        eprintln!("  {r}");
                    }
                    ExitCode::from(3)
                }
                Err(render::Failure::Failed(m)) => {
                    eprintln!("pm-cli: {m}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        None => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => fail_usage("no command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    fn options(s: &str) -> parity::Options {
        match parse_eval(&args(s)).unwrap() {
            Parsed::Run(o) => *o,
            Parsed::Help => panic!("{s}: help, not a run"),
        }
    }

    #[test]
    fn options_parse() {
        let o = options("--only text/,captions/ --exclude real/ --wasm --wasm-subsets --samples 3");
        assert_eq!(o.only, ["text/", "captions/"]);
        assert_eq!(o.exclude, ["real/"]);
        assert!(o.wasm && o.wasm_subsets && o.native);
        assert_eq!(o.max_samples, 3);
        assert!(parse_eval(&args("--no-native")).is_err());
        assert!(parse_eval(&args("--corpus")).is_err());
        assert!(parse_eval(&args("--bogus")).is_err());
        assert!(matches!(
            parse_eval(&args("--wasm --help")),
            Ok(Parsed::Help)
        ));
        assert!(matches!(parse_eval(&args("-h")), Ok(Parsed::Help)));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parity_render_options_parse() {
        let o = parse_parity(&s(&[
            "--capture",
            "/c/v1",
            "--real",
            "/c/perf/a.pmv",
            "--determinism",
        ]))
        .unwrap();
        assert_eq!(o.captures, vec![PathBuf::from("/c/v1")]);
        assert_eq!(o.real, Some(PathBuf::from("/c/perf/a.pmv")));
        assert!(o.definitions, "the built-in definitions by default");
        assert!(o.determinism);
        let o = parse_parity(&s(&["--capture", "/c/v1", "--no-definitions"])).unwrap();
        assert!(!o.definitions && !o.determinism && o.real.is_none());
        assert!(parse_parity(&s(&["--real"])).is_err(), "no file");
        assert!(parse_parity(&s(&["--bogus"])).is_err());
    }

    #[test]
    fn render_options_parse_strictly() {
        let o = parse_render(&s(&[
            "p.pmv",
            "-o",
            "out.mov",
            "--preset",
            "master-alpha",
            "--fps",
            "29.97",
            "--size",
            "1280x720",
            "--media",
            "a1=/tmp/x.mov",
            "--dither",
            "on",
            "--force",
        ]))
        .unwrap();
        assert_eq!(o.input, PathBuf::from("p.pmv"));
        assert_eq!(o.size, Some((1280, 720)));
        assert_eq!(
            o.media,
            vec![("a1".to_string(), PathBuf::from("/tmp/x.mov"))]
        );
        assert_eq!(o.dither, Some(true));
        assert!(o.force);
        assert!(parse_render(&s(&["p.pmv"])).is_err(), "no output");
        assert!(parse_render(&s(&["-o", "x.mov"])).is_err(), "no input");
        assert!(parse_render(&s(&["p.pmv", "-o", "x.mov", "--bogus"])).is_err());
        assert!(parse_render(&s(&["p.pmv", "-o", "x.mov", "--size", "12"])).is_err());
        let o = parse_render(&s(&[
            "p.pmv",
            "-o",
            "x.mov",
            "--format",
            "hevc10",
            "--audio",
            "float",
            "--timecode",
            "01:00:00;00",
            "--reel",
            "A001",
            "--no-captions",
        ]))
        .unwrap();
        assert_eq!(o.format.as_deref(), Some("hevc10"));
        assert_eq!(o.audio.as_deref(), Some("float"));
        assert_eq!(o.timecode.as_deref(), Some("01:00:00;00"));
        assert_eq!(o.reel.as_deref(), Some("A001"));
        assert!(o.no_captions);
    }

    #[test]
    fn scope_options_parse() {
        let o = parse_scope(&s(&[
            "p.pmv",
            "-o",
            "/tmp/s",
            "--time",
            "1.5",
            "--size",
            "640x360",
            "--motion-blur",
            "off",
            "--columns",
            "320",
        ]))
        .unwrap();
        assert_eq!(o.input, Some(PathBuf::from("p.pmv")));
        assert_eq!(o.time, 1.5);
        assert_eq!(o.size, Some((640, 360)));
        assert_eq!(o.columns, Some(320));
        let f = parse_scope(&s(&["--frame", "t1.rgba32f", "-o", "/tmp/s"])).unwrap();
        assert_eq!(f.frame, Some(PathBuf::from("t1.rgba32f")));
        assert!(parse_scope(&s(&["p.pmv"])).is_err(), "no output");
        assert!(parse_scope(&s(&["-o", "/tmp/s"])).is_err(), "no input");
        assert!(parse_scope(&s(&["p.pmv", "--frame", "f.png", "-o", "/tmp/s"])).is_err());
        assert!(parse_scope(&s(&["p.pmv", "-o", "/tmp/s", "--bogus"])).is_err());
    }

    #[test]
    fn parity_options_parse() {
        let o = parse_parity(&s(&[
            "--capture",
            "/a",
            "--capture",
            "/b",
            "--only",
            "render/",
        ]))
        .unwrap();
        assert_eq!(o.captures, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(o.only.as_deref(), Some("render/"));
        assert!(parse_parity(&s(&["--nope"])).is_err());
    }
}
