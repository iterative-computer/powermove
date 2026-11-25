//! Builds the vendored libfx core (apps/native/vendor/fx) with the pinned Zig
//! and links it statically. Never downloads anything: the pinned Zig is
//! installed by vendor/fx/install-zig.sh. Without it (or on a target the core
//! is not built for) the crate compiles a Rust stub (`src/sys.rs`; there is
//! no C stub library) whose every call returns `Unavailable`, with one cargo
//! warning, so the workspace still builds.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The only target the core is built for so far.
const CORE_TARGET: &str = "aarch64-apple-darwin";
const ZIG_TARGET: &str = "aarch64-macos";
/// The macOS the app supports, as Zig names it.
const ZIG_TRIPLE: &str = "aarch64-macos.13.0";

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest.join("../../vendor/fx");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    println!("cargo::rustc-check-cfg=cfg(aero_core, values(\"unavailable\"))");
    println!("cargo::rerun-if-env-changed=PM_ZIG_HOME");
    for path in [
        "src",
        "include",
        "test-support",
        "build.zig",
        "build.zig.zon",
        "toolchain.toml",
    ] {
        println!("cargo::rerun-if-changed={}", vendor.join(path).display());
    }

    let target = env::var("TARGET").unwrap_or_default();
    if target != CORE_TARGET {
        return stub(&format!(
            "the libfx core is only built for {CORE_TARGET}, not {target}"
        ));
    }
    let version = match pinned_version(&vendor.join("toolchain.toml")) {
        Some(version) => version,
        None => panic!("vendor/fx/toolchain.toml has no [zig] version"),
    };
    let zig_home = match env::var_os("PM_ZIG_HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => match env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(".cache/powermove/zig"),
            None => return stub("neither PM_ZIG_HOME nor HOME is set"),
        },
    };
    let zig = zig_home.join(format!("zig-{ZIG_TARGET}-{version}/zig"));
    // Tracked whether or not it exists, so installing the pinned Zig into
    // the same place after a stub build rebuilds the real core (Sol's item
    // 10); see the missing case below for the folder.
    println!("cargo::rerun-if-changed={}", zig.display());
    match Command::new(&zig).arg("version").output() {
        Ok(output)
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == version => {}
        Ok(output) => {
            return stub(&format!(
                "{} is Zig {}, not the pinned {version}",
                zig.display(),
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
        Err(_) => {
            // The binary a later install brings keeps the tarball's old
            // mtime, so its own path cannot tell cargo it arrived. The
            // folder it lands in changes when it does (or, missing, keeps
            // this script rerunning until it exists).
            println!("cargo::rerun-if-changed={}", zig_home.display());
            return stub(&format!(
                "the pinned Zig {version} is not at {} (run apps/native/vendor/fx/install-zig.sh)",
                zig.display()
            ));
        }
    }

    // Local cache and install prefix inside OUT_DIR, so build folders never
    // share artifacts; the global cache (package fetches, the standard
    // library's compiled C) lives with the pinned toolchain.
    let prefix = out.join("zig-out");
    let status = Command::new(&zig)
        .current_dir(&vendor)
        .env("ZIG_GLOBAL_CACHE_DIR", zig_home.join("zig-cache"))
        .args(["build", "capi", "fake-server"])
        .arg(format!("-Dtarget={ZIG_TRIPLE}"))
        .arg("--cache-dir")
        .arg(out.join("zig-cache"))
        .arg("--prefix")
        .arg(&prefix)
        .arg("--summary")
        .arg("none")
        .status()
        .unwrap_or_else(|error| panic!("could not run {}: {error}", zig.display()));
    if !status.success() {
        panic!("zig build of the libfx core failed ({status})");
    }
    let lib = prefix.join("lib");
    if !lib.join("libfx_core.a").exists() {
        panic!(
            "zig build did not produce {}",
            lib.join("libfx_core.a").display()
        );
    }
    println!("cargo::rustc-link-search=native={}", lib.display());
    println!("cargo::rustc-link-lib=static=fx_core");
    println!(
        "cargo::rustc-env=PM_AERO_FAKE_SERVER={}",
        prefix.join("bin/fake-inference-server").display()
    );
    println!(
        "cargo::metadata=include={}",
        vendor.join("include").display()
    );
}

fn stub(reason: &str) {
    // A build that must carry the core (the owner's trial Sandbox: pm-native-app's
    // `aero-trial` feature turns on `require-core`) fails here instead of shipping
    // the stub: a trial build once went out without the core.
    if env::var_os("CARGO_FEATURE_REQUIRE_CORE").is_some() {
        panic!(
            "pm-aero-core: this build requires the libfx core, but it would be the stub ({reason}). \
             Install the pinned Zig with apps/native/vendor/fx/install-zig.sh and set PM_ZIG_HOME."
        );
    }
    println!(
        "cargo::warning=pm-aero-core: built without the libfx core ({reason}); every call returns Unavailable"
    );
    println!("cargo::rustc-cfg=aero_core=\"unavailable\"");
    println!("cargo::rustc-env=PM_AERO_FAKE_SERVER=");
}

/// `[zig] version = "…"` from toolchain.toml.
fn pinned_version(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_zig = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_zig = line == "[zig]";
            continue;
        }
        if !in_zig {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version") {
            let value = rest
                .trim_start()
                .strip_prefix('=')?
                .trim()
                .trim_matches('"');
            return Some(value.to_string());
        }
    }
    None
}
