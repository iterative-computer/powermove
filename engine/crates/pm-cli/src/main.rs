//! Headless binary, parity runner, test sinks (docs/rust-engine.md section 22).
//!
//! ```text
//! pm-cli parity eval [options]
//! ```
//!
//! Evaluation parity against the corpus (section 25; docs/engine/phase1-gate.md).

mod parity;

use std::path::PathBuf;

const USAGE: &str = "usage: pm-cli parity eval [options]

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

fn parse(args: &[String]) -> Result<Parsed, String> {
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["parity", "eval", rest @ ..] => {
            let rest: Vec<String> = rest.iter().map(|s| (*s).to_owned()).collect();
            match parse(&rest) {
                Ok(Parsed::Help) => {
                    println!("{USAGE}");
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
                    eprintln!("pm-cli parity eval: {e}\n\n{USAGE}");
                    64
                }
            }
        }
        ["--help" | "-h" | "help", ..] | ["parity", "--help" | "-h" | "help", ..] | [] => {
            println!("{USAGE}");
            0
        }
        _ => {
            eprintln!("{USAGE}");
            64
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    fn options(s: &str) -> parity::Options {
        match parse(&args(s)).unwrap() {
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
        assert!(parse(&args("--no-native")).is_err());
        assert!(parse(&args("--corpus")).is_err());
        assert!(parse(&args("--bogus")).is_err());
        assert!(matches!(parse(&args("--wasm --help")), Ok(Parsed::Help)));
        assert!(matches!(parse(&args("-h")), Ok(Parsed::Help)));
    }
}
