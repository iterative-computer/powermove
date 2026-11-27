//! The OS boundary for runs that must fail closed (replays and the `none`
//! harness): every component runs under a Seatbelt profile (macOS
//! `sandbox-exec`, the mechanism `pm-runtime-v8`'s Store sandbox uses through
//! `sandbox_init`) whose only outbound network is TCP to the replay's port
//! (none at all without a replay). Seatbelt's one host filter, `localhost`,
//! matches loopback and this Mac's own addresses alike, so that port is held
//! on every interface by the replay server ([`crate::replay::hold`]): the
//! only listener the run can reach is the replay. Every other port and host
//! is refused before a packet leaves. The profile reads nothing under
//! the owner's home except the run folder and the binaries it was given,
//! writes only inside the run folder, and cannot reach the Keychain daemons.
//! The rules are enforced by the kernel, not by convention; the crate's
//! canary tests prove each one is refused.
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Keychain and credential services a confined run may not look up.
const DENIED_SERVICES: [&str; 6] = [
    "com.apple.SecurityServer",
    "com.apple.securityd",
    "com.apple.securityd.xpc",
    "com.apple.security.agent",
    "com.apple.secd",
    "com.apple.security.authhost",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Confinement {
    /// The run folder: writable and readable.
    pub root: PathBuf,
    /// Files or folders that stay readable under a protected root (the host
    /// and harness binaries).
    pub read: Vec<PathBuf>,
    /// Roots nothing may be read from, except `root` and `read`: the owner's
    /// home and every other user's.
    pub protected: Vec<PathBuf>,
    /// The one TCP port the run may connect to (the replay's, held on every
    /// interface); None allows no outbound connection at all.
    pub tcp_port: Option<u16>,
}

/// Resolve a path the way the kernel matches it (`/tmp` is `/private/tmp`),
/// refusing anything a profile string cannot carry.
fn real(p: &Path) -> Result<PathBuf, String> {
    let r = std::fs::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let s = r
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", r.display()))?;
    if s.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        return Err(format!(
            "{s}: a confined path cannot contain quotes or backslashes"
        ));
    }
    Ok(r)
}

fn quoted(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

impl Confinement {
    /// A run's boundary: `root` plus the given binaries, with the owner's home
    /// (`$HOME` of the runner) and `/Users` protected, and `extra_protected`.
    pub fn for_run(
        root: &Path,
        binaries: &[&Path],
        extra_protected: &[PathBuf],
    ) -> Result<Self, String> {
        if !cfg!(target_os = "macos") {
            return Err(
                "replay runs fail closed under an OS sandbox, and only macOS (Seatbelt) has one here: refusing"
                    .into(),
            );
        }
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(format!(
                "{SANDBOX_EXEC} is missing: refusing to run unconfined"
            ));
        }
        let mut protected = vec![PathBuf::from("/Users")];
        if let Some(h) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            protected.push(PathBuf::from(h));
        }
        protected.extend(extra_protected.iter().cloned());
        Ok(Self {
            root: real(root)?,
            read: binaries.iter().map(|b| real(b)).collect::<Result<_, _>>()?,
            protected: protected
                .iter()
                .filter(|p| p.exists())
                .map(|p| real(p))
                .collect::<Result<_, _>>()?,
            tcp_port: None,
        })
    }

    /// Allow outbound TCP to this one port (the caller holds it on every
    /// interface).
    pub fn allow_tcp(mut self, port: u16) -> Self {
        self.tcp_port = Some(port);
        self
    }

    /// Prove the profile applies here: a trivial program must run under it.
    /// A boundary that cannot be set up refuses the run.
    pub fn verify(&self) -> Result<(), String> {
        let out = self
            .command(Path::new("/usr/bin/true"))
            .env_clear()
            .output()
            .map_err(|e| format!("{SANDBOX_EXEC}: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the sandbox profile could not be applied ({}): {}; refusing to run unconfined",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }

    /// The Seatbelt profile. Later rules win, so each deny is followed by the
    /// narrow allows it keeps.
    pub fn profile(&self) -> String {
        let root = quoted(&self.root);
        let reads: Vec<String> = self
            .read
            .iter()
            .map(|p| {
                if p.is_dir() {
                    format!("(subpath {})", quoted(p))
                } else {
                    format!("(literal {})", quoted(p))
                }
            })
            .collect();
        // Path lookups stat the folders above the root and the binaries.
        let mut ancestors: Vec<String> = std::iter::once(&self.root)
            .chain(&self.read)
            .flat_map(|p| {
                p.ancestors()
                    .skip(1)
                    .map(Path::to_path_buf)
                    .collect::<Vec<_>>()
            })
            .map(|a| format!("(literal {})", quoted(&a)))
            .collect();
        ancestors.sort();
        ancestors.dedup();
        let protected: Vec<String> = self
            .protected
            .iter()
            .map(|p| format!("(subpath {})", quoted(p)))
            .collect();
        let services: Vec<String> = DENIED_SERVICES
            .iter()
            .map(|s| format!("(global-name \"{s}\")"))
            .collect();
        // An empty filter list would make a rule match everything.
        let rule = |head: &str, items: &[String]| {
            if items.is_empty() {
                String::new()
            } else {
                format!("({head} {})\n", items.join(" "))
            }
        };
        format!(
            "(version 1)\n\
             (allow default)\n\
             ; network: outbound TCP to the replay's held port only (no DNS, no other port or host)\n\
             (deny network*)\n\
             (allow network-bind network-inbound (local ip \"localhost:*\"))\n\
             {outbound}\
             (allow network* (local unix-socket (subpath {root})) (remote unix-socket (subpath {root})))\n\
             ; reads: nothing under a protected root but the run folder and the binaries\n\
             {protected}\
             (allow file-read* (subpath {root}) {reads})\n\
             {ancestors}\
             ; writes: the run folder and devices only\n\
             (deny file-write*)\n\
             (allow file-write* (subpath {root}) (subpath \"/dev\"))\n\
             ; no Keychain\n\
             (deny mach-lookup {services})\n",
            protected = rule("deny file-read*", &protected),
            reads = reads.join(" "),
            ancestors = rule("allow file-read-metadata", &ancestors),
            services = services.join(" "),
            outbound = match self.tcp_port {
                Some(p) => format!("(allow network-outbound (remote tcp \"localhost:{p}\"))\n"),
                None => String::new(),
            },
        )
    }

    /// `sandbox-exec -p <profile> <program>`: add the program's arguments to
    /// the returned command. Children inherit the profile.
    pub fn command(&self, program: &Path) -> Command {
        let mut c = Command::new(SANDBOX_EXEC);
        c.arg("-p").arg(self.profile()).arg(program);
        c
    }
}

/// A command for `program`, confined when a boundary is given.
pub fn command(program: &Path, confine: Option<&Confinement>) -> Command {
    match confine {
        Some(c) => c.command(program),
        None => Command::new(program),
    }
}
