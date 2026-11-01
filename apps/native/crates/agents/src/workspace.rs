//! Compatible provider workspace. The native host owns sandboxed commands,
//! compilation and publication; these are explicit capabilities, not JS hooks.
use crate::{
    Access, Provider,
    approvals::Authority,
    error,
    runtime::{atomic_write, real_dir, unique},
    valid_id,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

pub fn safe_component(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let value = if out.is_empty() { "project" } else { out };
    value.chars().take(120).collect()
}
pub fn session_path(
    profile: &Path,
    project: &str,
    provider: Provider,
    authority: Access,
    thread: Option<&str>,
) -> io::Result<PathBuf> {
    if authority == Access::Editor {
        return Err(error("Editor sessions are ephemeral"));
    }
    let part = if provider == Provider::Chatgpt {
        String::new()
    } else {
        format!("-{}", provider.name())
    };
    let authority = if authority == Access::Computer {
        "computer"
    } else {
        "project"
    };
    let mut dir = profile.join("Agent State").join(safe_component(project));
    if let Some(id) = thread {
        if !valid_id(id, 120) {
            return Err(error("Invalid agent thread id"));
        }
        dir = dir.join("threads").join(id);
    }
    Ok(dir.join(format!("session-v2{part}-{authority}.txt")))
}
pub fn read_session(path: &Path) -> io::Result<Option<String>> {
    let v = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let v = v.trim();
    Ok((valid_id(v, 200) && v.as_bytes()[0].is_ascii_alphanumeric()).then(|| v.into()))
}
pub fn write_session(path: &Path, id: &str) -> io::Result<()> {
    if !valid_id(id, 200) || !id.as_bytes()[0].is_ascii_alphanumeric() {
        return Err(error("Invalid session id"));
    }
    if path.exists() {
        let previous = fs::read(path)?;
        if previous != id.as_bytes() {
            atomic_write(
                &path.with_extension(format!("{}.archived", unique())),
                &previous,
            )?;
        }
    }
    atomic_write(path, id.as_bytes())
}

pub trait Host {
    /// A Project host MUST enforce filesystem confinement in the OS. Linux tests
    /// inject a fake. Do not implement this by running an unrestricted shell.
    fn run_command(
        &mut self,
        root: &Path,
        authority: &Authority,
        command: &str,
        timeout_ms: u64,
        background: bool,
    ) -> io::Result<Value>;
    fn command_status(&mut self, id: &str, wait_ms: u64, stop: bool) -> io::Result<Value>;
    fn stop_all(&mut self) -> io::Result<()>;
    fn compile(&mut self, id: &str) -> io::Result<Value>;
    fn publish(&mut self, result: &Value) -> io::Result<Value>;
}
pub struct Workspace<H: Host> {
    root: PathBuf,
    archive: PathBuf,
    authority: Authority,
    host: H,
    app_context: bool,
    jobs: BTreeMap<String, bool>,
}
impl<H: Host> Workspace<H> {
    pub fn new(
        root: &Path,
        archive: &Path,
        authority: Authority,
        host: H,
        app_context: bool,
    ) -> io::Result<Self> {
        real_dir(root)?;
        real_dir(archive)?;
        let root = fs::canonicalize(root)?;
        let archive = fs::canonicalize(archive)?;
        if archive.starts_with(&root) {
            return Err(error(
                "Main-owned archive must live outside agent workspace",
            ));
        }
        Ok(Self {
            root,
            archive,
            authority,
            host,
            app_context,
            jobs: BTreeMap::new(),
        })
    }
    pub fn call(&mut self, name: &str, args: &Value) -> io::Result<Value> {
        match name {
            "list_files" => {
                let file = self.open_path(args["path"].as_str().unwrap_or(""), true)?;
                let mut entries = list_dir(&file)?
                    .into_iter()
                    .map(|(name, directory)| json!({"name": name, "directory": directory}))
                    .collect::<Vec<_>>();
                entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                let total = entries.len();
                entries.truncate(1000);
                Ok(json!({"entries":entries,"total":total}))
            }
            "read_file" => {
                let mut file = self.open_path(required(args, "path")?, false)?;
                if !file.metadata()?.is_file() || file.metadata()?.len() > 8 * 1024 * 1024 {
                    return Err(error(
                        "Read a regular file no larger than 8 MB; use run_command to inspect larger files.",
                    ));
                }
                let mut bytes = vec![];
                Read::by_ref(&mut file)
                    .take(8 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(error("File grew past read limit"));
                }
                let path = required(args, "path")?;
                if path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg") {
                    return Ok(
                        json!([{"type":"image","mimeType":if path.ends_with(".png"){"image/png"}else{"image/jpeg"},"data":STANDARD.encode(bytes)}]),
                    );
                }
                let source = String::from_utf8_lossy(&bytes);
                let chars = source.encode_utf16().collect::<Vec<_>>();
                let offset = integer(args, "offset", 0, 0, chars.len() as u64)? as usize;
                let limit = integer(args, "limit", 30000, 1, 100000)? as usize;
                Ok(
                    json!({"text":String::from_utf16_lossy(&chars[offset..(offset+limit).min(chars.len())]),"offset":offset,"totalChars":chars.len()}),
                )
            }
            "write_file" => {
                let text = required(args, "text")?;
                if text.encode_utf16().count() > 1_000_000 {
                    return Err(error("Provide text no larger than 1,000,000 characters."));
                }
                let path = required(args, "path")?;
                self.write_file(path, text.as_bytes())?;
                Ok(json!({"path":self.logical(path)?,"bytes":text.len()}))
            }
            "run_command" => {
                let command = required(args, "command")?;
                if command.trim().is_empty() || command.len() > 100000 {
                    return Err(error(
                        "Provide a nonempty command, at most 100,000 characters.",
                    ));
                }
                let background = boolean(args, "background")?;
                if background && self.jobs.values().filter(|v| **v).count() >= 4 {
                    return Err(error("At most 4 background jobs can run at once."));
                }
                let timeout = integer(
                    args,
                    "timeoutMs",
                    if background { 600000 } else { 30000 },
                    1,
                    600000,
                )?;
                let result = self.host.run_command(
                    &self.root,
                    &self.authority,
                    command,
                    timeout,
                    background,
                )?;
                if background {
                    let id = result["jobId"]
                        .as_str()
                        .ok_or_else(|| error("Host omitted job id"))?;
                    self.jobs.insert(id.into(), true);
                }
                Ok(result)
            }
            "command_status" => {
                let id = required(args, "jobId")?;
                if !self.jobs.contains_key(id) {
                    return Err(error(
                        "Unknown background job. Use a jobId returned by run_command.",
                    ));
                }
                let result = self.host.command_status(
                    id,
                    integer(args, "waitMs", 0, 0, 120000)?,
                    boolean(args, "stop")?,
                )?;
                if result["state"] != "running" {
                    self.jobs.insert(id.into(), false);
                }
                Ok(result)
            }
            "compile_extension" => {
                let id = required(args, "id")?;
                if !(2..=64).contains(&id.len())
                    || !id.as_bytes()[0].is_ascii_lowercase() && !id.as_bytes()[0].is_ascii_digit()
                    || !id
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                {
                    return Err(error("Provide a valid extension id."));
                }
                self.host.compile(id)
            }
            "complete_task" => {
                self.host.stop_all()?;
                self.jobs.clear();
                if !args["summary"].is_string()
                    || !args["commands"]
                        .as_array()
                        .is_some_and(|a| a.len() <= 80 && a.iter().all(Value::is_string))
                    || args["extensions"].as_array().is_none_or(|a| a.len() > 32)
                    || !["artifacts", "notes", "externalActions"]
                        .iter()
                        .all(|k| args[k].is_array())
                {
                    return Err(error(
                        "complete_task requires summary, commands, artifacts, extensions, notes and externalActions matching its schema.",
                    ));
                }
                if self.app_context
                    && (!args["commands"].as_array().unwrap().is_empty()
                        || args["artifacts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|a| a["importToTimeline"] == true))
                {
                    return Err(error(
                        "No project is attached. Return commands: [] and do not import artifacts to a timeline.",
                    ));
                }
                self.host.publish(args)
            }
            _ => Err(error(format!("Unknown workspace tool: {name}"))),
        }
    }
    fn logical(&self, path: &str) -> io::Result<PathBuf> {
        if path.is_empty() || path.contains('\0') {
            return Err(error("Provide a valid file path."));
        }
        let p = Path::new(path);
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };
        let mut normalized = PathBuf::new();
        for component in p.components() {
            match component {
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }
        if !self.authority.computer_allowed() && !normalized.starts_with(&self.root) {
            return Err(error("This path is outside the project workspace."));
        }
        Ok(normalized)
    }
    fn base(&self, path: &Path) -> (PathBuf, PathBuf) {
        if let Ok(relative) = path.strip_prefix(&self.root) {
            (self.root.clone(), relative.to_path_buf())
        } else {
            // Explicit computer paths: resolve system links above the target (macOS
            // `/var` -> `/private/var`, `/tmp`) once, then walk with O_NOFOLLOW, so a
            // link planted later still fails closed.
            let resolved = match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) => fs::canonicalize(parent)
                    .map(|p| p.join(name))
                    .unwrap_or_else(|_| path.to_path_buf()),
                _ => path.to_path_buf(),
            };
            let relative = resolved
                .strip_prefix("/")
                .expect("absolute computer path")
                .to_path_buf();
            (PathBuf::from("/"), relative)
        }
    }
    fn open_path(&self, path: &str, directory: bool) -> io::Result<File> {
        let logical = self.logical(path)?;
        let (root, relative) = self.base(&logical);
        open_relative(&root, &relative, directory, false)
    }
    fn write_file(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        let logical = self.logical(path)?;
        let (base, rel) = self.base(&logical);
        let name = rel.file_name().ok_or_else(|| error("File path required"))?;
        let parent = open_relative(&base, rel.parent().unwrap_or(Path::new("")), true, true)?;
        #[cfg(unix)]
        {
            use std::{
                ffi::CString,
                os::{
                    fd::{AsRawFd, FromRawFd},
                    unix::ffi::OsStrExt,
                },
            };
            let name = CString::new(name.as_bytes()).map_err(|_| error("Invalid path"))?;
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            let existed = fd >= 0;
            if existed {
                let old = unsafe { File::from_raw_fd(fd) };
                if !old.metadata()?.is_file() {
                    return Err(error("Cannot replace a nonregular file"));
                }
            } else if io::Error::last_os_error().kind() != io::ErrorKind::NotFound {
                return Err(io::Error::last_os_error());
            }
            let archive = File::open(&self.archive)?;
            use std::os::unix::fs::MetadataExt;
            if parent.metadata()?.dev() != archive.metadata()?.dev() {
                return Err(error("Workspace archive must be on the same volume"));
            }
            let temp = CString::new(format!(".agent-write-{}", unique())).unwrap();
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    temp.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            file.write_all(bytes)?;
            file.sync_all()?;
            // Swap atomically so the exact displaced inode survives, even if
            // another writer changed it after the preflight. New files use an
            // exclusive rename, so an arriving authored file is never clobbered.
            let backup = format!("replaced-{}", unique());
            let receipt = self.archive.join(format!("{backup}.json"));
            let mut record = json!({"version":1,"kind":"replacement","path":logical,"staged":temp.to_string_lossy(),"backup":backup,"status":"prepared"});
            if existed {
                atomic_write(&receipt, &serde_json::to_vec(&record)?)?;
            }
            atomic_rename(parent.as_raw_fd(), &temp, &name, existed)?;
            if existed {
                let archived = CString::new(backup).unwrap();
                if unsafe {
                    libc::renameat(
                        parent.as_raw_fd(),
                        temp.as_ptr(),
                        archive.as_raw_fd(),
                        archived.as_ptr(),
                    )
                } < 0
                {
                    return Err(error(
                        "Replacement published; original retained in workspace staging for recovery",
                    ));
                }
                archive.sync_all()?;
                record["status"] = json!("archived");
                atomic_write(&receipt, &serde_json::to_vec(&record)?)?;
            }
            parent.sync_all()?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (parent, name, bytes);
            Err(error("Descriptor-confined writes require Unix"))
        }
    }
}
impl<H: Host> Drop for Workspace<H> {
    fn drop(&mut self) {
        let _ = self.host.stop_all();
    }
}
fn required<'a>(v: &'a Value, key: &str) -> io::Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| error(format!("Provide {key}")))
}
fn boolean(v: &Value, key: &str) -> io::Result<bool> {
    match v.get(key) {
        None => Ok(false),
        Some(Value::Bool(v)) => Ok(*v),
        _ => Err(error(format!("{key} must be true or false."))),
    }
}
fn integer(v: &Value, key: &str, default: u64, min: u64, max: u64) -> io::Result<u64> {
    let n = match v.get(key) {
        None => default,
        Some(v) => v.as_u64().ok_or_else(|| error(format!("Invalid {key}")))?,
    };
    Ok(n.clamp(min, max))
}
/// Lists an already-opened directory descriptor without re-resolving its path.
fn list_dir(file: &File) -> io::Result<Vec<(String, bool)>> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    {
        fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))?
            .map(|v| {
                v.map(|v| {
                    (
                        v.file_name().to_string_lossy().into_owned(),
                        v.file_type().is_ok_and(|t| t.is_dir()),
                    )
                })
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS /dev/fd/N is not a directory link; read the descriptor itself.
        let fd = unsafe { libc::dup(file.as_raw_fd()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let dir = unsafe { libc::fdopendir(fd) };
        if dir.is_null() {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        unsafe { libc::rewinddir(dir) };
        let mut out = Vec::new();
        loop {
            let entry = unsafe { libc::readdir(dir) };
            if entry.is_null() {
                break;
            }
            let entry = unsafe { &*entry };
            let name = unsafe { std::ffi::CStr::from_ptr(entry.d_name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            if name == "." || name == ".." {
                continue;
            }
            out.push((name, entry.d_type == libc::DT_DIR));
        }
        unsafe { libc::closedir(dir) };
        Ok(out)
    }
}
/// Every ancestor is opened relative to an owned directory descriptor. A rename
/// or symlink swap cannot redirect later operations outside that directory.
fn open_relative(root: &Path, relative: &Path, directory: bool, create: bool) -> io::Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let mut options = fs::OpenOptions::new();
    use std::os::unix::fs::OpenOptionsExt;
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut current = options.open(root)?;
    let parts = relative
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect::<Vec<_>>();
    for (i, part) in parts.iter().enumerate() {
        let Component::Normal(part) = part else {
            return Err(error("Invalid workspace path"));
        };
        let name = CString::new(part.as_bytes()).map_err(|_| error("Invalid path"))?;
        let is_dir = i + 1 < parts.len() || directory;
        if create && is_dir {
            let result = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
        }
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if is_dir { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    Ok(current)
}

#[cfg(unix)]
fn atomic_rename(
    dir: std::os::fd::RawFd,
    from: &std::ffi::CStr,
    to: &std::ffi::CStr,
    swap: bool,
) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            dir,
            from.as_ptr(),
            dir,
            to.as_ptr(),
            if swap {
                libc::RENAME_EXCHANGE
            } else {
                libc::RENAME_NOREPLACE
            },
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            dir,
            from.as_ptr(),
            dir,
            to.as_ptr(),
            if swap {
                libc::RENAME_SWAP
            } else {
                libc::RENAME_EXCL
            },
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(error("Atomic archive rename unsupported on this platform"));
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
