//! Explicit native registration, never launch-time setup. Review is read-only;
//! Claude Desktop changes require a host consent decision before any write.
use crate::{Error, Result};
use serde_json::{Value, json};
use std::os::{
    fd::AsRawFd,
    unix::fs::{OpenOptionsExt, PermissionsExt},
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
const FLAG: &str = "--powermove-mcp";
const LIMIT: u64 = 16 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Connect,
    Disconnect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Servers,
    Vscode,
    Opencode,
    Codex,
    ClaudeCode,
}
struct Spec {
    id: &'static str,
    name: &'static str,
    detail: &'static str,
    config: &'static str,
    format: Format,
    installed: &'static [&'static str],
    restart: &'static str,
}
const SPECS: &[Spec] = &[
    Spec {
        id: "claude-code",
        name: "Claude Code",
        detail: "Your Claude plan, in the terminal or your editor.",
        config: ".claude.json",
        format: Format::ClaudeCode,
        installed: &[".claude.json", ".claude"],
        restart: "Start a new Claude Code session to use it.",
    },
    Spec {
        id: "codex",
        name: "Codex",
        detail: "Your ChatGPT plan, in the Codex app or CLI.",
        config: ".codex/config.toml",
        format: Format::Codex,
        installed: &[".codex", "/Applications/Codex.app"],
        restart: "Start a new Codex session to use it.",
    },
    Spec {
        id: "claude-desktop",
        name: "Claude",
        detail: "Your Claude plan, in the Claude app.",
        config: "Library/Application Support/Claude/claude_desktop_config.json",
        format: Format::Servers,
        installed: &[
            "/Applications/Claude.app",
            "Applications/Claude.app",
            "Library/Application Support/Claude",
        ],
        restart: "Quit and reopen Claude to use it.",
    },
    Spec {
        id: "gemini-cli",
        name: "Gemini CLI",
        detail: "Your Google account or Gemini plan.",
        config: ".gemini/settings.json",
        format: Format::Servers,
        installed: &[".gemini"],
        restart: "Start a new Gemini CLI session to use it.",
    },
    Spec {
        id: "cursor",
        name: "Cursor",
        detail: "Your Cursor plan.",
        config: ".cursor/mcp.json",
        format: Format::Servers,
        installed: &["/Applications/Cursor.app", ".cursor"],
        restart: "Restart Cursor to use it.",
    },
    Spec {
        id: "vscode",
        name: "VS Code",
        detail: "GitHub Copilot’s agent mode.",
        config: "Library/Application Support/Code/User/mcp.json",
        format: Format::Vscode,
        installed: &[
            "/Applications/Visual Studio Code.app",
            "Library/Application Support/Code/User",
        ],
        restart: "Reload VS Code to use it.",
    },
    Spec {
        id: "windsurf",
        name: "Windsurf",
        detail: "Your Windsurf plan.",
        config: ".codeium/windsurf/mcp_config.json",
        format: Format::Servers,
        installed: &["/Applications/Windsurf.app", ".codeium/windsurf"],
        restart: "Restart Windsurf to use it.",
    },
    Spec {
        id: "lm-studio",
        name: "LM Studio",
        detail: "Models on this Mac, in LM Studio’s chat.",
        config: ".lmstudio/mcp.json",
        format: Format::Servers,
        installed: &["/Applications/LM Studio.app", ".lmstudio"],
        restart: "Turn Powermove on under Integrations in LM Studio’s chat.",
    },
    Spec {
        id: "opencode",
        name: "OpenCode",
        detail: "Any provider you use in OpenCode.",
        config: ".config/opencode/opencode.json",
        format: Format::Opencode,
        installed: &[".config/opencode"],
        restart: "Start a new OpenCode session to use it.",
    },
];
/// A safe review record contains no existing configuration or credentials.
#[derive(Clone, Debug)]
pub struct Review {
    pub client_id: String,
    pub path: PathBuf,
    pub command: PathBuf,
    pub action: Action,
    pub changes: bool,
}
pub enum Consent {
    Approved,
    Denied,
}
pub trait RegistrationConsent: Send + Sync {
    fn ask(&self, review: &Review) -> Result<Consent>;
}
pub trait ClaudeCli: Send + Sync {
    fn run_mcp(&self, args: &[String]) -> Result<()>;
}
pub struct Registration {
    home: PathBuf,
    profile: PathBuf,
    executable: PathBuf,
    applications: PathBuf,
    pub claude_cli: Option<Arc<dyn ClaudeCli>>,
}
pub struct Prepared {
    review: Review,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    custom: bool,
    connected: bool,
    issuer: String,
}
impl Prepared {
    pub fn review(&self) -> &Review {
        &self.review
    }
}
#[derive(Debug)]
pub struct Applied {
    pub changed: bool,
    pub archive: Option<PathBuf>,
    pub next: String,
}
impl Registration {
    pub fn new(
        home: PathBuf,
        profile: PathBuf,
        executable: PathBuf,
        applications: PathBuf,
    ) -> Result<Self> {
        if [&home, &profile, &executable, &applications]
            .iter()
            .any(|p| !p.is_absolute())
        {
            return Err(Error::invalid("Powermove’s location is unknown."));
        }
        Ok(Self {
            home: home.canonicalize()?,
            profile: profile.canonicalize()?,
            executable,
            applications,
            claude_cli: None,
        })
    }
    fn issuer(&self) -> String {
        pm_model::hash::sha256_hex(
            format!(
                "{}\0{}\0{}",
                self.home.display(),
                self.profile.display(),
                self.executable.display()
            )
            .as_bytes(),
        )
    }
    fn spec(id: &str) -> Result<&'static Spec> {
        SPECS
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::invalid("Unknown agent app."))
    }
    fn file(&self, spec: &Spec) -> PathBuf {
        self.home.join(spec.config)
    }
    pub fn inspect(&self) -> Value {
        json!({"clients":SPECS.iter().map(|s|{
   let installed=s.installed.iter().any(|p|{
    let file=if let Some(name)=p.strip_prefix("/Applications/"){self.applications.join(name)}else{self.home.join(p)};
    fs::symlink_metadata(file).is_ok()
   });
   let (state,custom)=if !installed {("not-installed",false)}else{match read(&self.file(s)).and_then(|v|state(s,v.as_deref())){Ok((true,custom))=>("connected",custom),Ok((false,_))=>("available",false),Err(_)=>("unreadable",false)}};
   json!({"id":s.id,"name":s.name,"detail":s.detail,"state":state,"custom":custom,"configPath":self.file(s)})
  }).collect::<Vec<_>>(),"command":self.executable,"args":[FLAG]})
    }
    pub fn prepare(&self, id: &str, action: Action) -> Result<Prepared> {
        let spec = Self::spec(id)?;
        let path = self.file(spec);
        let before = read(&path).map_err(|_| unreadable(spec, &path))?;
        let (connected, custom) =
            state(spec, before.as_deref()).map_err(|_| unreadable(spec, &path))?;
        if action == Action::Disconnect && custom {
            return Err(Error::invalid(format!(
                "This connection was set up by hand. Remove it in {}’s settings.",
                spec.name
            )));
        }
        let changes = match action {
            Action::Connect => !custom,
            Action::Disconnect => connected,
        };
        let after = if !changes || spec.format == Format::ClaudeCode {
            before.clone()
        } else {
            Some(
                transform(spec, before.as_deref(), &self.executable, action)
                    .map_err(|_| unreadable(spec, &path))?,
            )
        };
        Ok(Prepared {
            review: Review {
                client_id: id.into(),
                path,
                command: self.executable.clone(),
                action,
                changes,
            },
            before,
            after,
            custom,
            connected,
            issuer: self.issuer(),
        })
    }
    /// The caller must bind a real native confirmation host for Claude Desktop.
    /// A missing/denied consent never creates directories, archives or receipts.
    pub fn apply(
        &self,
        plan: Prepared,
        consent: Option<&dyn RegistrationConsent>,
    ) -> Result<Applied> {
        if plan.issuer != self.issuer() {
            return Err(Error::invalid(
                "Registration review belongs to another native host",
            ));
        }
        let spec = Self::spec(&plan.review.client_id)?;
        if !plan.review.changes {
            return Ok(Applied {
                changed: false,
                archive: None,
                next: if plan.custom {
                    "Already set up.".into()
                } else {
                    spec.restart.into()
                },
            });
        }
        if spec.id == "claude-desktop" && plan.review.action == Action::Connect {
            let consent = consent.ok_or_else(|| {
                Error::unavailable("explicit native Claude Desktop registration consent (S070)")
            })?;
            if matches!(consent.ask(&plan.review)?, Consent::Denied) {
                return Err(Error::invalid(
                    "Claude Desktop registration was not approved",
                ));
            }
        }
        // Other clients arrive only from an explicit connect/disconnect action.
        // Re-check after consent: a dialog can remain open while settings change.
        if read(&plan.review.path)? != plan.before {
            return Err(conflict(
                "Client settings changed. Review registration again before changing them.",
            ));
        }
        let lock_parent = directory(&self.profile.join("mcp"), true)?;
        let lock = open_at(
            &lock_parent,
            "client-registration.lock",
            libc::O_CREAT | libc::O_RDWR,
            0o600,
        )?;
        lock.lock()?;
        if read(&plan.review.path)? != plan.before {
            return Err(conflict(
                "Client settings changed. Review registration again before changing them.",
            ));
        }
        if spec.format == Format::ClaudeCode {
            let cli = self
                .claude_cli
                .as_deref()
                .ok_or_else(|| Error::unavailable("native Claude Code CLI registration adapter"))?;
            let archive = archive_copy(&self.profile, &plan.before)?;
            if plan.review.action == Action::Disconnect || plan.connected {
                cli.run_mcp(&["remove", "--scope", "user", "powermove"].map(str::to_owned))?;
            }
            if plan.review.action == Action::Connect {
                cli.run_mcp(&[
                    "add".into(),
                    "--scope".into(),
                    "user".into(),
                    "powermove".into(),
                    "--".into(),
                    self.executable.to_string_lossy().into_owned(),
                    FLAG.into(),
                ])?;
            }
            return Ok(Applied {
                changed: true,
                archive,
                next: spec.restart.into(),
            });
        }
        let after = plan.after.as_ref().expect("file-format plan");
        let archive = publish(&plan.review.path, plan.before.as_deref(), after)?;
        if spec.id == "claude-desktop" {
            // Explicit disconnect/connection remains intentional across launch/update.
            let receipt = self.profile.join("mcp/claude-desktop-registration.json");
            let before = read(&receipt)?;
            publish(&receipt, before.as_deref(), b"{\"registered\":true}\n")?;
        }
        Ok(Applied {
            changed: true,
            archive,
            next: spec.restart.into(),
        })
    }
}
fn unreadable(spec: &Spec, file: &Path) -> Error {
    Error::invalid(format!(
        "Powermove can’t safely edit {}’s settings ({}), for example because they contain comments. Use Copy setup and add it there yourself.",
        spec.name,
        file.display()
    ))
}
fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match crate::creative::bounded_fs::open(path, false) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > LIMIT {
        return Err(Error::invalid("Unsupported configuration"));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(Error::invalid("Unsupported configuration"));
    }
    Ok(Some(bytes))
}
fn object(bytes: Option<&[u8]>) -> Result<Value> {
    let Some(bytes) = bytes else {
        return Ok(json!({}));
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| Error::invalid("Invalid configuration"))?;
    if !value.is_object() {
        return Err(Error::invalid("Invalid configuration"));
    }
    Ok(value)
}
fn key(format: Format) -> &'static str {
    match format {
        Format::Vscode => "servers",
        Format::Opencode => "mcp",
        _ => "mcpServers",
    }
}
fn command_owned(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        regex::Regex::new(
            r"/Powermove(?: [^/]*)?\.app/Contents/MacOS/(?:Powermove|powermove-native)$",
        )
        .unwrap()
        .is_match(s)
    })
}
fn own(format: Format, entry: &Value) -> bool {
    let Some(map) = entry.as_object() else {
        return false;
    };
    if format == Format::Opencode {
        return entry["command"]
            .as_array()
            .is_some_and(|a| a.len() == 2 && command_owned(&a[0]) && a[1] == FLAG);
    }
    let allowed = if matches!(format, Format::Vscode | Format::ClaudeCode) {
        &["type", "command", "args", "env"][..]
    } else {
        &["command", "args"][..]
    };
    command_owned(&entry["command"])
        && entry["args"] == json!([FLAG])
        && map.keys().all(|k| allowed.contains(&k.as_str()))
        && entry
            .get("env")
            .is_none_or(|e| e.as_object().is_some_and(|m| m.is_empty()))
}
fn state(spec: &Spec, bytes: Option<&[u8]>) -> Result<(bool, bool)> {
    if spec.format == Format::Codex {
        let text = std::str::from_utf8(bytes.unwrap_or_default())
            .map_err(|_| Error::invalid("Unsupported configuration"))?;
        let table = codex_table(text);
        return Ok(if codex_mentioned(text) {
            (true, table.is_none_or(|(_, _, b)| !codex_own(b)))
        } else {
            (false, false)
        });
    }
    let config = object(bytes)?;
    let Some(servers) = config.get(key(spec.format)) else {
        return Ok((false, false));
    };
    let servers = servers
        .as_object()
        .ok_or_else(|| Error::invalid("Invalid configuration"))?;
    Ok(servers
        .get("powermove")
        .map_or((false, false), |entry| (true, !own(spec.format, entry))))
}
fn transform(spec: &Spec, before: Option<&[u8]>, exe: &Path, action: Action) -> Result<Vec<u8>> {
    if spec.format == Format::Codex {
        let text = std::str::from_utf8(before.unwrap_or_default())
            .map_err(|_| Error::invalid("Unsupported configuration"))?;
        let without = if let Some((start, end, _)) = codex_table(text) {
            format!("{}{}", &text[..start], &text[end..])
        } else {
            text.into()
        };
        let result = if action == Action::Connect {
            let base = without.trim_end();
            format!(
                "{base}{}[mcp_servers.powermove]\ncommand = {}\nargs = [\"{FLAG}\"]\n# The hidden editor takes a while to start the first time.\nstartup_timeout_sec = 60\n",
                if base.is_empty() { "" } else { "\n\n" },
                serde_json::to_string(&exe.to_string_lossy()).unwrap()
            )
        } else {
            format!(
                "{}\n",
                regex::Regex::new(r"\n{3,}")
                    .unwrap()
                    .replace_all(&without, "\n\n")
                    .trim_end()
            )
        };
        return Ok(result.into_bytes());
    }
    let mut config = object(before)?;
    let name = key(spec.format);
    if config.get(name).is_none() {
        config[name] = json!({})
    }
    let servers = config[name]
        .as_object_mut()
        .ok_or_else(|| Error::invalid("Invalid configuration"))?;
    if action == Action::Disconnect {
        servers.remove("powermove");
    } else {
        servers.insert(
            "powermove".into(),
            match spec.format {
                Format::Vscode => json!({"type":"stdio","command":exe,"args":[FLAG]}),
                Format::Opencode => json!({"type":"local","command":[exe,FLAG],"enabled":true}),
                _ => json!({"command":exe,"args":[FLAG]}),
            },
        );
    }
    let mut out = serde_json::to_vec_pretty(&config).unwrap();
    out.push(b'\n');
    Ok(out)
}
fn codex_header() -> regex::Regex {
    regex::Regex::new(
        r#"(?m)^[ \t]*\[mcp_servers\.(?:powermove|"powermove"|'powermove')\][ \t]*(?:#.*)?$"#,
    )
    .unwrap()
}
fn codex_table(text: &str) -> Option<(usize, usize, &str)> {
    let found = codex_header().find(text)?;
    let next = regex::Regex::new(r"(?m)^[ \t]*\[")
        .unwrap()
        .find(&text[found.end()..]);
    let end = next.map_or(text.len(), |m| found.end() + m.start());
    Some((found.start(), end, &text[found.start()..end]))
}
fn codex_mentioned(text: &str) -> bool {
    codex_header().is_match(text)
        || regex::Regex::new(r#"mcp_servers\.["']?powermove["']?\."#)
            .unwrap()
            .is_match(text)
        || regex::Regex::new(r#"(?ms)^\s*\[mcp_servers\].*?^\s*["']?powermove["']?\s*="#)
            .unwrap()
            .is_match(text)
}
fn codex_own(body: &str) -> bool {
    let Some(command) =
        regex::Regex::new(r#"(?m)^\s*command\s*=\s*("[^"\\]*(?:\\.[^"\\]*)*")\s*$"#)
            .unwrap()
            .captures(body)
    else {
        return false;
    };
    let Ok(command) = serde_json::from_str::<Value>(&command[1]) else {
        return false;
    };
    command_owned(&command)
        && regex::Regex::new(r#"(?m)^\s*args\s*=\s*\[\s*"--powermove-mcp"\s*,?\s*\]\s*$"#)
            .unwrap()
            .is_match(body)
        && body
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .all(|l| {
                ["command", "args", "startup_timeout_sec"]
                    .contains(&l.split('=').next().unwrap().trim())
            })
}
fn archive_copy(profile: &Path, before: &Option<Vec<u8>>) -> Result<Option<PathBuf>> {
    let Some(bytes) = before else { return Ok(None) };
    let path = profile
        .join("mcp")
        .join(format!("client-archive-{}", pm_kernel::new_id("config")));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(Some(path))
}
fn directory(path: &Path, create: bool) -> Result<fs::File> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let mut parent = fs::File::open("/")?;
    for part in path.components() {
        let std::path::Component::Normal(part) = part else {
            continue;
        };
        let part = std::ffi::CString::new(part.as_bytes())
            .map_err(|_| Error::invalid("Invalid settings directory"))?;
        if create
            && unsafe { libc::mkdirat(parent.as_raw_fd(), part.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(parent)
}
fn open_at(parent: &fs::File, name: &str, flags: i32, mode: u32) -> Result<fs::File> {
    use std::os::fd::FromRawFd;
    let name =
        std::ffi::CString::new(name).map_err(|_| Error::invalid("Invalid settings filename"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            // Keep the u32 argument promoted: Darwin's mode_t is only u16.
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}
fn publish(path: &Path, before: Option<&[u8]>, after: &[u8]) -> Result<Option<PathBuf>> {
    let parent_path = path
        .parent()
        .ok_or_else(|| Error::invalid("Invalid settings path"))?;
    let parent = directory(parent_path, true)?;
    let target = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::invalid("Invalid settings filename"))?;
    let temp = format!(".powermove-archive-{}", pm_kernel::new_id("registration"));
    let mut file = open_at(
        &parent,
        &temp,
        libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY,
        0o600,
    )?;
    if before.is_some() {
        let mode = crate::creative::bounded_fs::open(path, false)?
            .metadata()?
            .permissions()
            .mode()
            & 0o777;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    file.write_all(after)?;
    file.sync_all()?;
    let src = std::ffi::CString::new(temp.as_str()).unwrap();
    let dest = std::ffi::CString::new(target).unwrap();
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            parent.as_raw_fd(),
            src.as_ptr(),
            parent.as_raw_fd(),
            dest.as_ptr(),
            if before.is_some() {
                libc::RENAME_EXCHANGE
            } else {
                libc::RENAME_NOREPLACE
            },
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            parent.as_raw_fd(),
            src.as_ptr(),
            parent.as_raw_fd(),
            dest.as_ptr(),
            if before.is_some() {
                libc::RENAME_SWAP
            } else {
                libc::RENAME_EXCL
            },
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let archive = if before.is_some() {
        let displaced = open_at(&parent, &temp, libc::O_RDONLY, 0)?;
        displaced.set_permissions(fs::Permissions::from_mode(0o600))?;
        let archive_path = parent_path.join(&temp);
        let actual = read(&archive_path)?;
        if actual.as_deref() != before {
            return Err(conflict(
                "Client settings changed during publication. Both configurations were retained; inspect settings before retrying.",
            ));
        }
        Some(archive_path)
    } else {
        None
    };
    parent.sync_all()?;
    Ok(archive)
}

fn conflict(message: &str) -> Error {
    Error {
        code: "configuration_conflict".into(),
        message: message.into(),
        backing: Some("native external-client registration".into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn late_foreign_configuration_is_displaced_into_a_private_archive_not_lost() {
        let root = std::env::temp_dir().join(pm_kernel::new_id("registration-race"));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let config = root.join("settings.json");
        std::fs::write(&config, b"{\"foreign\":true}\n").unwrap();
        let error = super::publish(
            &config,
            Some(b"{\"earlier\":true}\n"),
            b"{\"planned\":true}\n",
        )
        .unwrap_err();
        assert_eq!(error.code, "configuration_conflict");
        assert_eq!(std::fs::read(&config).unwrap(), b"{\"planned\":true}\n");
        let archives = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(".powermove-archive-")
            })
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        assert_eq!(
            std::fs::read(&archives[0]).unwrap(),
            b"{\"foreign\":true}\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
