//! Finding and launching the agent CLIs (port of `codex/env.ts`,
//! `codex/isolation.ts`, `claude/env.ts`, `claude/isolation.ts`).
//!
//! Each CLI keeps Powermove's sign-in private: Codex runs with
//! `CODEX_HOME=<userData>/codex-runtime` (the user's existing login is copied
//! in once), Claude Code with `CLAUDE_CONFIG_DIR=<userData>/claude-runtime`.
//! Disconnecting in Powermove never signs a terminal session out.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Result, bail};
use tokio::process::{Child, Command};

pub const CODEX_NOT_FOUND: &str = "Powermove's ChatGPT runtime (Codex) is missing. Install it with `npm install -g @openai/codex`, or set CODEX_BINARY to codex.exe.";
pub const CLAUDE_NOT_FOUND: &str = "Powermove's Claude runtime (Claude Code) is missing. Install Claude Code, or set CLAUDE_BINARY to claude.exe.";

const CODEX_HOME_NAME: &str = "codex-runtime";
const CLAUDE_HOME_NAME: &str = "claude-runtime";
const AUTH_OWNER_FILE: &str = ".powermove-auth-owned";

/// Where the CLIs live and run.
#[derive(Debug, Clone)]
pub struct Runtimes {
    pub user_data: PathBuf,
    /// Packaged runtimes (`<resources>/codex/bin/codex.exe`, `<resources>/claude/bin/claude.exe`).
    pub resources_dir: PathBuf,
    /// Overrides for tests; production discovers.
    pub codex_binary: Option<PathBuf>,
    pub claude_binary: Option<PathBuf>,
    /// Test seam: run Claude Code against this config directory instead of the private one.
    pub claude_home: Option<PathBuf>,
}

fn is_file(path: &Path) -> bool {
    path.is_file()
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|path| !path.as_os_str().is_empty())
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|value| std::env::split_paths(&value).collect()).unwrap_or_default()
}

const CODEX_TRIPLES: [(&str, &str); 2] = [("codex-win32-x64", "x86_64-pc-windows-msvc"), ("codex-win32-arm64", "aarch64-pc-windows-msvc")];

/// The native codex.exe inside an `@openai/codex` npm install rooted at `node_modules`.
fn codex_in_node_modules(node_modules: &Path) -> Option<PathBuf> {
    for (package, triple) in CODEX_TRIPLES {
        for base in [node_modules.join("@openai"), node_modules.join("@openai/codex/node_modules/@openai")] {
            let candidate = base.join(package).join("vendor").join(triple).join("bin").join("codex.exe");
            if is_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

impl Runtimes {
    /// `discoverCodexBinary`: CODEX_BINARY, the packaged runtime, the repo's
    /// node_modules, then `codex.exe` on PATH or the native binary behind an
    /// npm `codex` shim on PATH. The npm shim itself needs Node, so it is
    /// never launched directly.
    pub fn codex(&self) -> Result<PathBuf> {
        if let Some(binary) = self.codex_binary.clone().or_else(|| env_path("CODEX_BINARY")).filter(|path| is_file(path)) {
            return Ok(binary);
        }
        let packaged = self.resources_dir.join("codex").join("bin").join("codex.exe");
        if is_file(&packaged) {
            return Ok(packaged);
        }
        if let Some(repo) = self.resources_dir.parent().and_then(Path::parent) {
            if let Some(found) = codex_in_node_modules(&repo.join("apps/desktop/node_modules")) {
                return Ok(found);
            }
        }
        for directory in path_dirs() {
            let native = directory.join("codex.exe");
            if is_file(&native) {
                return Ok(native);
            }
            if is_file(&directory.join("codex.cmd")) || is_file(&directory.join("codex")) {
                if let Some(found) = codex_in_node_modules(&directory.join("node_modules")) {
                    return Ok(found);
                }
            }
        }
        if let Some(appdata) = env_path("APPDATA") {
            if let Some(found) = codex_in_node_modules(&appdata.join("npm").join("node_modules")) {
                return Ok(found);
            }
        }
        bail!(CODEX_NOT_FOUND)
    }

    /// `discoverClaudeBinary`: CLAUDE_BINARY, the packaged runtime, the native
    /// installer's `~/.local/bin/claude.exe`, then `claude.exe` on PATH.
    pub fn claude(&self) -> Result<PathBuf> {
        if let Some(binary) = self.claude_binary.clone().or_else(|| env_path("CLAUDE_BINARY")).filter(|path| is_file(path)) {
            return Ok(binary);
        }
        let mut candidates = vec![self.resources_dir.join("claude").join("bin").join("claude.exe")];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local").join("bin").join("claude.exe"));
        }
        candidates.extend(path_dirs().into_iter().map(|directory| directory.join("claude.exe")));
        candidates.into_iter().find(|candidate| is_file(candidate)).ok_or_else(|| anyhow::anyhow!(CLAUDE_NOT_FOUND))
    }

    /// `prepareIsolatedCodexHome`: a private CODEX_HOME whose auth.json is
    /// imported from the user's Codex login once, then owned by Powermove.
    pub async fn codex_home(&self) -> Result<PathBuf> {
        let source = env_path("CODEX_HOME").or_else(|| dirs::home_dir().map(|home| home.join(".codex")));
        prepare_codex_home(&self.user_data, source).await
    }

    /// Environment for any Codex process: the private home, plus the npm
    /// package's `codex-path` tools (rg.exe) that its Node shim would add to PATH.
    pub async fn codex_env(&self, binary: &Path) -> Result<Vec<(&'static str, OsString)>> {
        let mut env = vec![("CODEX_HOME", self.codex_home().await?.into_os_string())];
        if let Some(tools) = binary.parent().and_then(Path::parent).map(|vendor| vendor.join("codex-path")).filter(|tools| tools.is_dir()) {
            let mut paths = vec![tools];
            paths.extend(path_dirs());
            if let Ok(joined) = std::env::join_paths(paths) {
                env.push(("PATH", joined));
            }
        }
        Ok(env)
    }

    pub async fn claude_home(&self) -> Result<PathBuf> {
        if let Some(home) = &self.claude_home {
            return Ok(home.clone());
        }
        let home = self.user_data.join(CLAUDE_HOME_NAME);
        tokio::fs::create_dir_all(&home).await?;
        Ok(home)
    }
}

async fn prepare_codex_home(user_data: &Path, source: Option<PathBuf>) -> Result<PathBuf> {
        let mut home = user_data.join(CODEX_HOME_NAME);
        if source.as_deref() == Some(home.as_path()) {
            home = user_data.join(format!("{CODEX_HOME_NAME}-isolated"));
        }
        tokio::fs::create_dir_all(&home).await?;
        let config = home.join("config.toml");
        let current = tokio::fs::read_to_string(&config).await.unwrap_or_default();
        let mut next = current.clone();
        for (key, line) in [("cli_auth_credentials_store", "cli_auth_credentials_store = \"file\""), ("suppress_unstable_features_warning", "suppress_unstable_features_warning = true")] {
            let setting = regex::Regex::new(&format!(r"(?m)^[ \t]*{key}[ \t]*=.*$")).expect("static pattern");
            next = if setting.is_match(&next) { setting.replace(&next, line).into_owned() } else { format!("{line}\n{next}") };
        }
        if next != current {
            tokio::fs::write(&config, next).await?;
        }
        let owner = home.join(AUTH_OWNER_FILE);
        let auth = home.join("auth.json");
        let initialized = tokio::fs::try_exists(&owner).await.unwrap_or(false);
        if !initialized {
            // Import an existing login only on first use; afterwards a missing
            // auth.json means the person disconnected Powermove.
            if let Some(source_auth) = source.map(|source| source.join("auth.json")) {
                if !tokio::fs::try_exists(&auth).await.unwrap_or(false) && tokio::fs::try_exists(&source_auth).await.unwrap_or(false) {
                    tokio::fs::copy(&source_auth, &auth).await?;
                }
            }
            tokio::fs::write(&owner, "powermove\n").await?;
        }
        Ok(home)
}

/// A CLI process with piped stdio, no console window, and its own process
/// tree so cancellation can stop everything it started.
pub fn command(binary: &Path, args: &[String], cwd: Option<&Path>, env: &[(&str, OsString)], remove_env: &[&str]) -> Command {
    let mut command = Command::new(binary);
    command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for name in remove_env {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    command
}

/// Stop a CLI and everything it launched (MCP servers, shells).
pub async fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/T", "/F", "/PID", &pid.to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        taskkill.creation_flags(0x0800_0000);
        let _ = taskkill.status().await;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Run a short CLI command and collect its output.
pub async fn output(mut command: Command, timeout: std::time::Duration) -> Result<std::process::Output> {
    command.stdin(Stdio::null());
    let child = command.spawn()?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => Ok(result?),
        Err(_) => bail!("The agent runtime took too long to respond."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn codex_home_imports_login_once() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("user-codex");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("auth.json"), "{}").unwrap();
        let user_data = directory.path().join("data");
        let home = prepare_codex_home(&user_data, Some(source.clone())).await.unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("cli_auth_credentials_store = \"file\""));
        assert!(home.join("auth.json").is_file());
        // Disconnecting removes the private copy; it is not imported again.
        std::fs::remove_file(home.join("auth.json")).unwrap();
        prepare_codex_home(&user_data, Some(source)).await.unwrap();
        assert!(!home.join("auth.json").exists());
        assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), config);
    }
}
