//! Blender discovery and the supervised worker (contract 11 invariants 2
//! and 3, contract 14; plan section 19, security; appendix A items 29 and
//! 33). The CLI host and tests run Blender through this module; Electron
//! main's host services follow the same rules in TypeScript
//! (`apps/desktop/src/main/engine/blender.ts`).
//!
//! - **Discovery** as today's `blenderCandidates`: `POWERMOVE_BLENDER_PATH`,
//!   `/Applications` and `~/Applications` bundles (versioned names
//!   included, newest first), then `PATH`; the first Blender 4.5 or newer.
//! - **Least privilege**: `--background --factory-startup --disable-autoexec
//!   --offline-mode`, a scrubbed environment (a fixed `PATH`, `HOME`,
//!   `TMPDIR`, and `BLENDER_USER_*` in the worker's scratch directory,
//!   nothing inherited), its own process group, and assets copied into a
//!   per-request directory under extension rules.
//! - **Bounds**: one request at a time per worker, the 10-minute timeout,
//!   output files checked (safe names, the per-surface cap).
//! - **Cancel stops Blender**: the process group is killed.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_protocol::blender::{MIN_BLENDER_VERSION, limits};

use crate::BlenderError;
use crate::worker::{WORKER_SCRIPT, worker_args};

/// A usable Blender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub executable: PathBuf,
    /// `major.minor.patch`.
    pub version: String,
    /// The first line of `--version` ("Blender 4.5.14 LTS").
    pub banner: String,
}

/// `Blender 4.5.14 LTS` → `4.5.14` when it is at least 4.5.
pub fn parse_version(banner: &str) -> Option<(String, bool)> {
    let rest = banner.strip_prefix("Blender ")?;
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = version.split('.').map(|p| p.parse::<u16>().ok());
    let major = parts.next()??;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some((
        format!("{major}.{minor}.{patch}"),
        (major, minor) >= MIN_BLENDER_VERSION,
    ))
}

/// A Blender bundle or folder name and the version it names
/// (`Blender.app`, `Blender 4.5.app`, `Blender-4.5.14.app`).
fn bundle_version(name: &str) -> Option<Option<Vec<u32>>> {
    let stem = name.strip_suffix(".app")?;
    let rest = stem.strip_prefix("Blender")?;
    if rest.is_empty() {
        return Some(None);
    }
    let rest = rest.strip_prefix([' ', '_', '-'])?;
    let parts: Option<Vec<u32>> = rest.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty() && p.len() <= 4).map(Some)
}

fn bundles_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut found: Vec<(Option<Vec<u32>>, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            bundle_version(&name).map(|v| (v, name))
        })
        .collect();
    // Unversioned first, then newest first, then by name.
    found.sort_by(|(a, an), (b, bn)| match (a, b) {
        (None, None) => an.cmp(bn),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => b.cmp(a).then_with(|| an.cmp(bn)),
    });
    found
        .into_iter()
        .map(|(_, name)| dir.join(name).join("Contents/MacOS/Blender"))
        .collect()
}

/// Where Blender is looked for, in order.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("POWERMOVE_BLENDER_PATH") {
        out.push(PathBuf::from(p));
    }
    out.extend(bundles_in(Path::new("/Applications")));
    if let Some(home) = std::env::var_os("HOME") {
        out.extend(bundles_in(&Path::new(&home).join("Applications")));
    }
    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).map(|p| p.join("blender")));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// Runs `--version` (10 s at most).
pub fn probe(executable: &Path) -> Result<Install, BlenderError> {
    if !executable.is_absolute() || !executable.exists() {
        return Err(BlenderError::Missing(format!(
            "Blender not found at {}",
            executable.display()
        )));
    }
    let mut child = Command::new(executable)
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BlenderError::Missing(format!("Blender could not start: {e}")))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BlenderError::Missing("Blender detection timed out".into()));
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        use std::io::Read;
        let _ = s.read_to_string(&mut out);
    }
    let banner = out.lines().next().unwrap_or("").trim().to_owned();
    match parse_version(&banner) {
        Some((version, true)) => Ok(Install {
            executable: executable.to_owned(),
            version,
            banner,
        }),
        Some((_, false)) => Err(BlenderError::Missing(
            "Blender 4.5 or newer is required".into(),
        )),
        None => Err(BlenderError::Missing(format!(
            "{} is not Blender",
            executable.display()
        ))),
    }
}

/// The first supported Blender: `configured` alone when given, else the
/// [`candidates`] (an older Blender found first never hides a newer one).
pub fn discover(configured: Option<&Path>) -> Result<Install, BlenderError> {
    let list = match configured {
        Some(p) => vec![p.to_owned()],
        None => candidates(),
    };
    let mut too_old = None;
    for c in list {
        match probe(&c) {
            Ok(i) => return Ok(i),
            Err(BlenderError::Missing(m)) if m.contains("4.5 or newer") => too_old = Some(m),
            Err(_) => {}
        }
    }
    Err(BlenderError::Missing(too_old.unwrap_or_else(|| {
        crate::BLENDER_MISSING_MESSAGE.to_owned()
    })))
}

/// Asset extensions the worker accepts (today's list).
pub const ASSET_EXTENSIONS: [&str; 10] = [
    "blend", "glb", "gltf", "obj", "png", "jpg", "jpeg", "webp", "exr", "hdr",
];

/// `^[a-zA-Z][a-zA-Z0-9_-]{0,159}$` (today's `safeId`).
pub fn safe_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 160
        && b[0].is_ascii_alphabetic()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
}

/// One render request to the worker.
#[derive(Debug, Clone)]
pub struct WorkerRequest<'a> {
    /// A [`safe_id`].
    pub id: &'a str,
    /// The snapshot's canonical JSON.
    pub snapshot: &'a [u8],
    /// Asset id → file (copied in under `<id>.<ext>`).
    pub assets: &'a [(String, PathBuf)],
    /// The layers to render, in order.
    pub outputs: &'a [String],
}

/// What the worker wrote.
#[derive(Debug, Clone)]
pub struct WorkerOutput {
    /// Layer id → PNG file (inside the request directory; move it out
    /// before the next request).
    pub files: Vec<(String, PathBuf)>,
    pub manifest: serde_json::Value,
    /// The request directory (removed by the next request or on drop).
    pub dir: PathBuf,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

/// One serialized Blender process running the pinned worker.
pub struct Worker {
    install: Install,
    root: PathBuf,
    timeout: Duration,
    running: Option<Running>,
    last_dir: Option<PathBuf>,
    counter: u64,
}

impl Worker {
    /// A worker whose scratch directory is created under `scratch_parent`.
    pub fn new(
        install: Install,
        scratch_parent: &Path,
        timeout: Duration,
    ) -> Result<Worker, BlenderError> {
        let root = scratch_parent.join(format!(
            "powermove-blender-{}-{}",
            std::process::id(),
            unique()
        ));
        create_private_dir(&root)
            .map_err(|e| BlenderError::Failed(format!("Blender scratch directory: {e}")))?;
        Ok(Worker {
            install,
            root,
            timeout,
            running: None,
            last_dir: None,
            counter: 0,
        })
    }

    pub fn install(&self) -> &Install {
        &self.install
    }

    /// The scrubbed environment (nothing of the parent's).
    fn environment(&self) -> Vec<(String, PathBuf)> {
        let r = &self.root;
        vec![
            (
                "PATH".into(),
                PathBuf::from("/usr/bin:/bin:/usr/sbin:/sbin"),
            ),
            ("HOME".into(), r.join("home")),
            ("TMPDIR".into(), r.join("tmp")),
            ("BLENDER_USER_RESOURCES".into(), r.join("user")),
            ("BLENDER_USER_CONFIG".into(), r.join("user/config")),
            ("BLENDER_USER_SCRIPTS".into(), r.join("user/scripts")),
            ("BLENDER_USER_DATAFILES".into(), r.join("user/datafiles")),
            ("PYTHONUNBUFFERED".into(), PathBuf::from("1")),
            ("PYTHONNOUSERSITE".into(), PathBuf::from("1")),
            ("PYTHONDONTWRITEBYTECODE".into(), PathBuf::from("1")),
            ("LANG".into(), PathBuf::from("en_US.UTF-8")),
        ]
    }

    fn start(&mut self) -> Result<(), BlenderError> {
        if self.running.is_some() {
            return Ok(());
        }
        let fail =
            |e: std::io::Error| BlenderError::Failed(format!("Blender could not start: {e}"));
        for d in [
            "home",
            "tmp",
            "user/config",
            "user/scripts",
            "user/datafiles",
        ] {
            create_private_dir(&self.root.join(d)).map_err(fail)?;
        }
        let script = self.root.join("worker.py");
        write_private(&script, WORKER_SCRIPT.as_bytes()).map_err(fail)?;
        let mut cmd = Command::new(&self.install.executable);
        cmd.args(worker_args(&script))
            .env_clear()
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in self.environment() {
            cmd.env(k, v);
        }
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(fail)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, lines) = channel();
        std::thread::Builder::new()
            .name("pm-blender-stdout".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(msg) = line.strip_prefix("PM_BLENDER:")
                        && tx.send(msg.to_owned()).is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(fail)?;
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let t = tail.clone();
        std::thread::Builder::new()
            .name("pm-blender-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else { break };
                    let mut q = t.lock().unwrap_or_else(|e| e.into_inner());
                    q.push_back(line);
                    while q.len() > 40 {
                        q.pop_front();
                    }
                }
            })
            .map_err(fail)?;
        self.running = Some(Running {
            child,
            stdin,
            lines,
            stderr: tail,
        });
        Ok(())
    }

    /// The process id of the running worker (tests check cancel stops it).
    pub fn pid(&self) -> Option<u32> {
        self.running.as_ref().map(|r| r.child.id())
    }

    /// Kills the worker's process group (cancel stops Blender).
    pub fn kill(&mut self) {
        if let Some(mut r) = self.running.take() {
            let pid = r.child.id() as i32;
            // SAFETY: signalling our own child's process group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    fn stderr_tail(&self) -> String {
        self.running
            .as_ref()
            .map(|r| {
                let q = r.stderr.lock().unwrap_or_else(|e| e.into_inner());
                q.iter().cloned().collect::<Vec<_>>().join("\n")
            })
            .unwrap_or_default()
    }

    /// Renders one request. `cancel` is polled while Blender works; setting
    /// it kills Blender and answers [`BlenderError::Cancelled`].
    pub fn render(
        &mut self,
        req: &WorkerRequest<'_>,
        cancel: &AtomicBool,
    ) -> Result<WorkerOutput, BlenderError> {
        if !safe_id(req.id) || req.outputs.iter().any(|o| !safe_id(o)) {
            return Err(BlenderError::Failed("Invalid Blender request".into()));
        }
        if req.assets.len() > limits::MAX_ASSETS {
            return Err(BlenderError::Failed(format!(
                "at most {} Blender assets",
                limits::MAX_ASSETS
            )));
        }
        if req.snapshot.len() as u64 > limits::MAX_SNAPSHOT_BYTES {
            return Err(BlenderError::Failed("Render snapshot is too large".into()));
        }
        if let Some(d) = self.last_dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
        self.start()?;
        self.counter += 1;
        let dir = self.root.join(format!("job-{}", self.counter));
        let io = |e: std::io::Error| BlenderError::Failed(format!("Blender request: {e}"));
        create_private_dir(&dir).map_err(io)?;
        self.last_dir = Some(dir.clone());
        // Assets: copied in as `<id>.<ext>` (no source-machine paths).
        let mut bytes = 0u64;
        let mut assets = serde_json::Map::new();
        for (id, path) in req.assets {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if !safe_id(id) || !ASSET_EXTENSIONS.contains(&ext.as_str()) {
                return Err(BlenderError::Failed(
                    "Unsupported Blender asset format".into(),
                ));
            }
            let target = dir.join(format!("{id}.{ext}"));
            bytes += std::fs::copy(path, &target).map_err(io)?;
            if bytes > limits::MAX_INPUT_BYTES {
                return Err(BlenderError::Failed("Blender inputs exceed 256 MB".into()));
            }
            assets.insert(
                id.clone(),
                serde_json::Value::String(target.to_string_lossy().into_owned()),
            );
        }
        let snapshot = std::str::from_utf8(req.snapshot)
            .map_err(|_| BlenderError::Failed("Render snapshot is not UTF-8".into()))?;
        let mut text = String::with_capacity(snapshot.len() + 512);
        text.push_str("{\"id\":");
        text.push_str(&serde_json::to_string(req.id).expect("string"));
        text.push_str(",\"operation\":\"render\",\"snapshot\":");
        text.push_str(snapshot);
        text.push_str(",\"assets\":");
        text.push_str(&serde_json::Value::Object(assets).to_string());
        text.push_str(",\"dir\":");
        text.push_str(&serde_json::to_string(&dir.to_string_lossy()).expect("string"));
        text.push_str(",\"outputs\":");
        text.push_str(&serde_json::to_string(req.outputs).expect("strings"));
        text.push('}');
        let request_path = dir.join("request.json");
        write_private(&request_path, text.as_bytes()).map_err(io)?;
        let envelope = serde_json::json!({"id": req.id, "path": request_path.to_string_lossy()});
        {
            let running = self.running.as_mut().expect("started");
            let line = format!("{envelope}\n");
            if let Err(e) = running
                .stdin
                .write_all(line.as_bytes())
                .and_then(|_| running.stdin.flush())
            {
                let tail = self.stderr_tail();
                self.kill();
                return Err(BlenderError::Failed(format!(
                    "Blender stopped: {e}. {tail}"
                )));
            }
        }
        let deadline = Instant::now() + self.timeout;
        let answer = loop {
            if cancel.load(Ordering::Relaxed) {
                self.kill();
                return Err(BlenderError::Cancelled);
            }
            if Instant::now() >= deadline {
                self.kill();
                return Err(BlenderError::Failed("Blender operation timed out".into()));
            }
            let running = self.running.as_mut().expect("started");
            match running.lines.recv_timeout(Duration::from_millis(50)) {
                Ok(msg) => {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) else {
                        continue;
                    };
                    if v.get("id").and_then(|i| i.as_str()) == Some(req.id) {
                        break v;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Ok(Some(status)) = running.child.try_wait() {
                        let tail = self.stderr_tail();
                        self.running = None;
                        return Err(BlenderError::Failed(format!(
                            "Blender stopped ({status}). {tail}"
                        )));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let tail = self.stderr_tail();
                    self.kill();
                    return Err(BlenderError::Failed(format!("Blender stopped. {tail}")));
                }
            }
        };
        if answer.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            let message = answer
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("Blender operation failed");
            return Err(BlenderError::Failed(message.to_owned()));
        }
        let manifest: serde_json::Value = std::fs::read(dir.join("manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| BlenderError::Failed("Invalid Blender output".into()))?;
        let mut files = Vec::new();
        for f in manifest
            .get("files")
            .and_then(|f| f.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let id = f.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if !safe_id(id)
                || name.is_empty()
                || name.len() > 240
                || Path::new(name)
                    .file_name()
                    .map(|n| n != name)
                    .unwrap_or(true)
            {
                return Err(BlenderError::Failed("Invalid Blender output".into()));
            }
            let path = dir.join(name);
            let size = std::fs::metadata(&path).map_err(io)?.len();
            if size > limits::MAX_OUTPUT_BYTES_PER_SURFACE {
                return Err(BlenderError::Failed(
                    "A Blender surface exceeds the per-surface output cap".into(),
                ));
            }
            files.push((id.to_owned(), path));
        }
        Ok(WorkerOutput {
            files,
            manifest,
            dir,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn unique() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{}", N.fetch_add(1, Ordering::Relaxed))
}

/// Creates `dir` (and parents) with mode 0700.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Writes `bytes` to a new owner-only file.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_bundles_parse_like_today() {
        assert_eq!(
            parse_version("Blender 4.5.14 LTS"),
            Some(("4.5.14".into(), true))
        );
        assert_eq!(parse_version("Blender 5.0"), Some(("5.0.0".into(), true)));
        assert_eq!(
            parse_version("Blender 4.4.3"),
            Some(("4.4.3".into(), false))
        );
        assert_eq!(parse_version("Python 3.11"), None);
        assert_eq!(bundle_version("Blender.app"), Some(None));
        assert_eq!(
            bundle_version("Blender-4.5.14.app"),
            Some(Some(vec![4, 5, 14]))
        );
        assert_eq!(bundle_version("Blender 4.5.app"), Some(Some(vec![4, 5])));
        assert_eq!(bundle_version("Blender_5.0.app"), Some(Some(vec![5, 0])));
        assert_eq!(bundle_version("BlenderKit.app"), None);
        assert_eq!(bundle_version("Blender"), None);
        assert!(safe_id("L001") && safe_id("texture_0") && !safe_id("0x") && !safe_id("a/b"));
    }

    #[test]
    fn bundles_sort_unversioned_then_newest() {
        let dir = std::env::temp_dir().join(format!("pm-blender-bundles-{}", unique()));
        for n in [
            "Blender-4.5.2.app",
            "Blender.app",
            "Blender 5.0.app",
            "Other.app",
        ] {
            std::fs::create_dir_all(dir.join(n)).unwrap();
        }
        let found: Vec<String> = bundles_in(&dir)
            .iter()
            .map(|p| {
                p.strip_prefix(&dir)
                    .unwrap()
                    .components()
                    .next()
                    .unwrap()
                    .as_os_str()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            found,
            ["Blender.app", "Blender 5.0.app", "Blender-4.5.2.app"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_blender_names_the_requirement() {
        let e = discover(Some(Path::new("/nonexistent/Blender"))).unwrap_err();
        assert_eq!(
            e,
            BlenderError::Missing(crate::BLENDER_MISSING_MESSAGE.into())
        );
    }
}
