//! A development build started without a profile of its own (as macOS
//! reopening it, or an activation by bundle id, starts it) exits at once,
//! before it creates or opens anything: run in an empty HOME, the HOME stays
//! empty. The same for a profile that is the product's own.
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn run(home: &Path, args: &[&std::ffi::OsStr]) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_powermove-native"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("the bare build kept running");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    (status.code().unwrap_or(-1), stderr)
}

fn entries(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(entries(&path));
        }
        out.push(path);
    }
    out
}

#[test]
fn a_development_build_started_bare_opens_nothing() {
    let home = tempfile::tempdir().unwrap();
    let (code, stderr) = run(home.path(), &[]);
    assert_eq!(code, 64, "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("--profile"), "{stderr}");
    assert!(
        entries(home.path()).is_empty(),
        "{:?}",
        entries(home.path())
    );

    // Named, but the product's own profile (it does not exist yet: still
    // nothing is made).
    let product = home
        .path()
        .join("Library/Application Support/com.zellzoi.powermove.native-beta");
    let (code, stderr) = run(home.path(), &["--profile".as_ref(), product.as_os_str()]);
    assert_eq!(code, 64, "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        entries(home.path()).is_empty(),
        "{:?}",
        entries(home.path())
    );
}
