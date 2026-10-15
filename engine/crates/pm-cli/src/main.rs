//! `pm-cli`: the engine's headless binary (plan section 22, "The CLI"; section
//! 25, "Infrastructure").
//!
//! ```text
//! pm-cli render <project.pmv|.json> -o <out.mov|.mp4> [options]
//! pm-cli parity render [--capture <dir>]... [--only <prefix>] [--diffs <dir>] [--json <file>]
//! ```
//!
//! Exit status: 0 done, 1 failed, 2 usage, 3 the engine cannot render this
//! yet (the reasons are printed; the desktop app routes such projects to the
//! current engine).

mod compare;
mod corpus;
mod parity;
mod render;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  pm-cli render <project.pmv|.json> -o <out.mov|.mp4> [options]
      --preset master|master-alpha|web     (default master: ProRes 422 HQ)
      --format prores-proxy|prores-lt|prores|prores-hq|prores-4444|prores-4444xq|h264
      --quality draft|high|max             H.264 at 4, 16, or 40 Mbps
      --alpha none|straight|premultiplied  ProRes 4444 only
      --range all|work|START:END           seconds (default all)
      --fps N | N/D | 23.976 | 29.97 | ...  (default the composition's rate)
      --scale S | --size WxH               (default the composition's size)
      --no-audio  --color quicktime|broadcast  --dither on|off  --precision 16|32
      --composition ID  --media ASSET=FILE  --force (replace an existing file)
  pm-cli parity render [--capture DIR]... [--only PREFIX] [--diffs DIR] [--json FILE]
      compares engine renders with the legacy captures of the parity corpus
      (default: the newest v1-* and render-* captures)";

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

fn parse_parity(args: &[String]) -> Result<parity::Options, String> {
    let mut o = parity::Options {
        captures: vec![],
        only: None,
        diffs: None,
        json: None,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--capture" => o.captures.push(PathBuf::from(value(args, &mut i, a)?)),
            "--only" => o.only = Some(value(args, &mut i, a)?.into()),
            "--diffs" => o.diffs = Some(PathBuf::from(value(args, &mut i, a)?)),
            "--json" => o.json = Some(PathBuf::from(value(args, &mut i, a)?)),
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
                match parity::run(&options) {
                    Ok(report) if report.compared > 0 && report.failed == 0 => ExitCode::SUCCESS,
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
            _ => fail_usage(
                "parity takes `render` (evaluation parity runs in pm-eval's corpus tests)",
            ),
        },
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => fail_usage("no command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
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
