//! Regenerates the checked-in TypeScript types and the golden wire fixtures.
//!
//! ```sh
//! source engine/env.sh && cd engine && cargo run -p pm-protocol --bin pm-protocol-gen
//! ```
//!
//! With `--check`, writes nothing and exits non-zero when either output is
//! out of date (the same check the `generated_outputs_are_current` test runs).

use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().skip(1).any(|arg| arg == "--check");
    let ts_files = match pm_protocol::ts::generate() {
        Ok(files) => files,
        Err(error) => {
            eprintln!("TypeScript generation failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let fixtures = match pm_protocol::samples::fixture_files() {
        Ok(files) => files,
        Err(error) => {
            eprintln!("fixture encoding failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let ts_dir = pm_protocol::ts::output_dir();
    let fixture_dir = pm_protocol::samples::fixture_dir();
    let result = if check {
        check_outputs(&ts_files, &ts_dir, &fixtures, &fixture_dir)
    } else {
        write_outputs(&ts_files, &ts_dir, &fixtures, &fixture_dir)
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

type TsFiles = std::collections::BTreeMap<std::path::PathBuf, String>;
type Fixtures = std::collections::BTreeMap<String, Vec<u8>>;

fn check_outputs(ts_files: &TsFiles, ts_dir: &std::path::Path, fixtures: &Fixtures, fixture_dir: &std::path::Path) -> std::io::Result<bool> {
    let ts = pm_protocol::ts::drift(ts_files, ts_dir)?;
    let wire = fixture_drift(fixtures, fixture_dir)?;
    if ts.is_empty() && wire.is_empty() {
        println!("TypeScript types and wire fixtures are current.");
        return Ok(true);
    }
    eprintln!("Out of date. TypeScript: {ts:?}. Fixtures: {wire:?}");
    eprintln!("Run: source engine/env.sh && cd engine && cargo run -p pm-protocol --bin pm-protocol-gen");
    Ok(false)
}

fn write_outputs(ts_files: &TsFiles, ts_dir: &std::path::Path, fixtures: &Fixtures, fixture_dir: &std::path::Path) -> std::io::Result<bool> {
    let before = pm_protocol::ts::write(ts_files, ts_dir)?;
    println!(
        "TypeScript: {} files in {} ({} new, {} changed, {} removed)",
        ts_files.len(),
        ts_dir.display(),
        before.missing.len(),
        before.changed.len(),
        before.stale.len()
    );
    std::fs::create_dir_all(fixture_dir)?;
    let stale = fixture_drift(fixtures, fixture_dir)?;
    for name in &stale.stale {
        std::fs::remove_file(fixture_dir.join(name))?;
    }
    for (name, bytes) in fixtures {
        let path = fixture_dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(&path, bytes)?;
        }
    }
    println!("Fixtures: {} files in {}", fixtures.len(), fixture_dir.display());
    Ok(true)
}

#[derive(Debug, Default)]
struct FixtureDrift {
    missing: Vec<String>,
    changed: Vec<String>,
    stale: Vec<String>,
}

impl FixtureDrift {
    fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.changed.is_empty() && self.stale.is_empty()
    }
}

fn fixture_drift(fixtures: &Fixtures, dir: &std::path::Path) -> std::io::Result<FixtureDrift> {
    let mut drift = FixtureDrift::default();
    for (name, bytes) in fixtures {
        match std::fs::read(dir.join(name)) {
            Ok(existing) if existing == *bytes => {}
            Ok(_) => drift.changed.push(name.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => drift.missing.push(name.clone()),
            Err(e) => return Err(e),
        }
    }
    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.ends_with(".msgpack") && !fixtures.contains_key(&name) {
                drift.stale.push(name);
            }
        }
    }
    drift.stale.sort();
    Ok(drift)
}
