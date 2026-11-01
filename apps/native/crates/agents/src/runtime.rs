//! Native-only pinned npm runtime installer. Registry/network and executable
//! probes are injectable; OAuth homes are not inputs to this API.
use crate::error;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha512};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const CODEX_VERSION: &str = "0.159.1";
pub const CLAUDE_VERSION: &str = "2.1.280";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Codex,
    Claude,
}
impl Runtime {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Arch {
    Arm64,
    X64,
}
impl Arch {
    fn name(self) -> &'static str {
        match self {
            Self::Arm64 => "arm64",
            Self::X64 => "x64",
        }
    }
    fn triple(self) -> &'static str {
        match self {
            Self::Arm64 => "aarch64-apple-darwin",
            Self::X64 => "x86_64-apple-darwin",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Pin {
    pub provider: Runtime,
    pub arch: Arch,
    pub version: String,
    pub integrity: String,
}
impl Pin {
    pub fn package(&self) -> String {
        match self.provider {
            Runtime::Claude => format!("@anthropic-ai/claude-code-darwin-{}", self.arch.name()),
            Runtime::Codex => "@openai/codex".into(),
        }
    }
    pub fn package_version(&self) -> String {
        match self.provider {
            Runtime::Claude => self.version.clone(),
            Runtime::Codex => format!("{}-darwin-{}", self.version, self.arch.name()),
        }
    }
    pub fn registry_url(&self) -> io::Result<String> {
        validate_version(&self.version)?;
        Ok(format!(
            "https://registry.npmjs.org/{}/{}",
            self.package().replace('/', "%2F"),
            self.package_version()
        ))
    }
}
pub trait Registry {
    fn metadata(&mut self, url: &str) -> io::Result<Value>;
    fn download(&mut self, url: &str) -> io::Result<Box<dyn Read>>;
}
pub trait Probe {
    fn version(&mut self, binary: &Path) -> io::Result<String>;
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub version: String,
    pub app_version: String,
}
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
pub(crate) fn unique() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| error("Missing parent"))?;
    real_dir(parent)?;
    let temporary = parent.join(format!(".write-{}", unique()));
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut file = o.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()
}
pub(crate) fn real_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty() && *p != path)
    {
        real_dir(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(error("Refusing a linked or non-directory parent")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}
fn validate_version(v: &str) -> io::Result<()> {
    if v.is_empty()
        || v.len() > 80
        || !v
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        Err(error("Invalid pinned runtime version"))
    } else {
        Ok(())
    }
}
fn digest_file(file: &Path) -> io::Result<String> {
    let mut f = File::open(file)?;
    let mut hash = Sha512::new();
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("sha512-{}", STANDARD.encode(hash.finalize())))
}
/// A pin includes the exact npm SRI from the reviewed dependency lock. Neither
/// a mutable dist-tag nor a registry-supplied replacement checksum grants trust.
pub fn install(
    root: &Path,
    app_version: &str,
    pin: &Pin,
    registry: &mut impl Registry,
    probe: &mut impl Probe,
) -> io::Result<PathBuf> {
    if !pin.integrity.starts_with("sha512-")
        || STANDARD
            .decode(&pin.integrity[7..])
            .map_or(true, |v| v.len() != 64)
    {
        return Err(error("A sha512 npm pin is required"));
    }
    let meta = registry.metadata(&pin.registry_url()?)?;
    let url = meta["dist"]["tarball"]
        .as_str()
        .ok_or_else(|| error("Missing npm tarball"))?;
    if !url.starts_with("https://registry.npmjs.org/")
        || url.contains(['\\', '\r', '\n', '?', '#'])
        || meta["version"] != pin.package_version()
        || meta["dist"]["integrity"] != pin.integrity
    {
        return Err(error("Registry release does not match reviewed pin"));
    }
    real_dir(root)?;
    let root = fs::canonicalize(root)?;
    let versions = root.join(".versions");
    real_dir(&versions)?;
    // Never replace a previous version directory, even on same-version retries.
    let target = versions.join(format!(
        "{}-{}-{}",
        pin.provider.name(),
        pin.version,
        unique()
    ));
    fs::create_dir(&target)?;
    let archive = target.join("package.tgz");
    let mut source = registry.download(url)?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive)?;
    let n = io::copy(
        &mut Read::by_ref(&mut source).take(512 * 1024 * 1024 + 1),
        &mut out,
    )?;
    out.sync_all()?;
    if n > 512 * 1024 * 1024 || digest_file(&archive)? != pin.integrity {
        return Err(error(
            "The downloaded runtime didn’t match its published checksum.",
        ));
    }
    extract(&archive, &target, pin)?;
    let binary = target.join("bin").join(pin.provider.name());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    }
    let observed = probe.version(&binary)?;
    if !observed.split_whitespace().any(|v| v == pin.version) {
        return Err(error("Installed runtime reports a different version"));
    }
    atomic_write(
        &target.join("installed.json"),
        &serde_json::to_vec(&Installed {
            version: pin.version.clone(),
            app_version: app_version.into(),
        })?,
    )?;
    sync_tree(&target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let staged = root.join(format!(".{}-{}", pin.provider.name(), unique()));
        symlink(&target, &staged)?;
        fs::rename(staged, root.join(pin.provider.name()))?;
        File::open(&root)?.sync_all()?;
    }
    // Legacy metadata remains compatible; discovery validates metadata embedded
    // in the atomically swapped tree, surviving death before this sidecar write.
    atomic_write(
        &root.join(format!("{}.json", pin.provider.name())),
        &serde_json::to_vec(&Installed {
            version: pin.version.clone(),
            app_version: app_version.into(),
        })?,
    )?;
    Ok(binary)
}
/// Read only the managed versions tree. No caller-provided OAuth directory.
pub fn candidate(root: &Path, provider: Runtime, app_version: &str) -> io::Result<Option<PathBuf>> {
    let link = root.join(provider.name());
    let target = match fs::canonicalize(&link) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !target.starts_with(fs::canonicalize(root.join(".versions"))?) {
        return Err(error("Runtime link leaves managed versions"));
    }
    let installed: Installed = serde_json::from_slice(&fs::read(target.join("installed.json"))?)?;
    if installed.app_version != app_version {
        return Ok(None);
    }
    let bin = target.join("bin").join(provider.name());
    let m = fs::symlink_metadata(&bin)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(error("Runtime is not a regular native binary"));
    }
    Ok(Some(bin))
}
/// Extract the reviewed native vendor tree, including required helpers and
/// resources. Links, special files and path traversal are never followed.
fn extract(archive: &Path, target: &Path, pin: &Pin) -> io::Result<()> {
    let mut reader = GzDecoder::new(File::open(archive)?);
    let mut header = [0; 512];
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0u64;
    loop {
        reader.read_exact(&mut header)?;
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            String::from_utf8_lossy(&header[range])
                .trim_end_matches('\0')
                .to_string()
        };
        let name = field(0..100);
        let prefix = field(345..500);
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if name.starts_with('/') || name.split('/').any(|s| s == "..") || name.contains('\\') {
            return Err(error("Unsafe runtime tar path"));
        }
        let size = u64::from_str_radix(field(124..136).trim_matches([' ', '\0']), 8)
            .map_err(|_| error("Invalid tar size"))?;
        let expected = u64::from_str_radix(field(148..156).trim_matches([' ', '\0']), 8)
            .map_err(|_| error("Invalid tar checksum"))?;
        let sum = header
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    *b as u64
                }
            })
            .sum::<u64>();
        if expected != sum {
            return Err(error("Invalid tar checksum"));
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| error("Tar size overflow"))?;
        if total > 1024 * 1024 * 1024 {
            return Err(error("Runtime archive too large"));
        }
        let selected = match pin.provider {
            Runtime::Claude => (name == "package/claude").then(|| "bin/claude".to_owned()),
            Runtime::Codex => name
                .strip_prefix(&format!("package/vendor/{}/", pin.arch.triple()))
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        };
        if let Some(relative) = selected {
            let relative = relative.trim_end_matches('/').to_owned();
            if relative.is_empty()
                || relative
                    .split('/')
                    .any(|s| s.is_empty() || s == "." || s == "..")
                || !seen.insert(relative.clone())
            {
                return Err(error("Invalid or duplicate runtime path"));
            }
            let output = target.join(&relative);
            if header[156] == b'5' {
                if size != 0 {
                    return Err(error("Runtime directory carries data"));
                }
                real_dir(&output)?;
            } else {
                if !matches!(header[156], 0 | b'0') {
                    return Err(error("Nonregular runtime entry"));
                }
                real_dir(
                    output
                        .parent()
                        .ok_or_else(|| error("Missing runtime parent"))?,
                )?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output)?;
                if io::copy(&mut Read::by_ref(&mut reader).take(size), &mut file)? != size {
                    return Err(error("Truncated runtime tar"));
                }
                file.sync_all()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = u32::from_str_radix(field(100..108).trim_matches([' ', '\0']), 8)
                        .map_err(|_| error("Invalid tar mode"))?;
                    fs::set_permissions(
                        &output,
                        fs::Permissions::from_mode(if mode & 0o111 != 0 { 0o755 } else { 0o600 }),
                    )?;
                }
            }
        } else if io::copy(&mut Read::by_ref(&mut reader).take(size), &mut io::sink())? != size {
            return Err(error("Truncated runtime tar"));
        }
        let padding = (512 - size % 512) % 512;
        io::copy(
            &mut Read::by_ref(&mut reader).take(padding),
            &mut io::sink(),
        )?;
    }
    if !seen.contains(&format!("bin/{}", pin.provider.name())) {
        return Err(error("Runtime tarball lacks native executable"));
    }
    Ok(())
}
pub fn newer(candidate: &str, current: &str) -> bool {
    fn parts(s: &str) -> Vec<u64> {
        let mut p = s
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .take(3)
            .map(|s| s.parse().unwrap_or(0))
            .collect::<Vec<_>>();
        p.resize(3, 0);
        p
    }
    parts(candidate) > parts(current)
}

/// Checksums are copied from the frozen repository's public npm lock, never
/// discovered from runtime credentials or accepted from a mutable dist-tag.
pub fn bundled_pin(provider: Runtime, arch: Arch) -> Pin {
    let pins: Value = serde_json::from_str(include_str!("../fixtures/runtime-pins.json"))
        .expect("reviewed runtime pins");
    let package = match provider {
        Runtime::Claude => format!("@anthropic-ai/claude-code-darwin-{}", arch.name()),
        Runtime::Codex => format!("@openai/codex-darwin-{}", arch.name()),
    };
    let integrity = pins["pins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["package"] == package)
        .unwrap()["integrity"]
        .as_str()
        .unwrap()
        .to_owned();
    Pin {
        provider,
        arch,
        version: if provider == Runtime::Codex {
            CODEX_VERSION
        } else {
            CLAUDE_VERSION
        }
        .into(),
        integrity,
    }
}

/// Public npm HTTPS adapter using the OS curl executable. Invoked on an agent
/// worker, never on the UI thread. No auth commands, cookies or credential files.
#[derive(Default)]
pub struct NpmRegistry;
struct Download {
    child: std::process::Child,
    stdout: std::process::ChildStdout,
    finished: bool,
}
impl Read for Download {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.stdout.read(buf)?;
        if n == 0 && !self.finished {
            self.finished = true;
            if !self.child.wait()?.success() {
                return Err(error("npm runtime download failed"));
            }
        }
        Ok(n)
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
fn curl(url: &str, max_bytes: u64) -> io::Result<std::process::Child> {
    if !url.starts_with("https://registry.npmjs.org/") || url.contains(['\r', '\n', '\\']) {
        return Err(error("Only public npm HTTPS URLs are supported"));
    }
    std::process::Command::new("/usr/bin/curl")
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "180",
            "--max-filesize",
            &max_bytes.to_string(),
            "--",
            url,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
}
impl Registry for NpmRegistry {
    fn metadata(&mut self, url: &str) -> io::Result<Value> {
        let mut child = curl(url, 1024 * 1024)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("Missing curl stdout"))?;
        let mut download = Download {
            stdout,
            child,
            finished: false,
        };
        let mut bytes = vec![];
        Read::by_ref(&mut download)
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(error("Oversized npm metadata"));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
    fn download(&mut self, url: &str) -> io::Result<Box<dyn Read>> {
        let mut child = curl(url, 512 * 1024 * 1024)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("Missing curl stdout"))?;
        Ok(Box::new(Download {
            child,
            stdout,
            finished: false,
        }))
    }
}
#[derive(Default)]
pub struct NativeProbe;
impl Probe for NativeProbe {
    fn version(&mut self, binary: &Path) -> io::Result<String> {
        let mut child = std::process::Command::new(binary)
            .arg("--version")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let result = stdout.take(4097).read_to_end(&mut bytes).map(|_| bytes);
            let _ = tx.send(result);
        });
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait()? {
                let bytes = rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .map_err(|_| error("Runtime version pipe did not close"))??;
                if !status.success() || bytes.len() > 4096 {
                    return Err(error("Runtime version probe failed"));
                }
                return String::from_utf8(bytes)
                    .map_err(|_| error("Runtime version was not UTF-8"));
            }
            if std::time::Instant::now() >= end {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error("Runtime version probe timed out"));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
pub fn install_if_newer(
    root: &Path,
    app_version: &str,
    pin: &Pin,
    current_version: &str,
    registry: &mut impl Registry,
    probe: &mut impl Probe,
) -> io::Result<Option<PathBuf>> {
    if !newer(&pin.version, current_version) {
        return Ok(None);
    }
    install(root, app_version, pin, registry, probe).map(Some)
}

fn sync_tree(root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_tree(&entry.path())?;
        }
    }
    File::open(root)?.sync_all()
}
