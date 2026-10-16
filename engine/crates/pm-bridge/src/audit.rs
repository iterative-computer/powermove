//! Process identity: audit tokens, pidversion, parent pid, and code signatures.

use crate::ffi::{self, audit_token_t};
use std::ffi::{CStr, c_void};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AuditToken(pub audit_token_t);

impl AuditToken {
    pub fn pid(&self) -> i32 {
        // SAFETY: pure accessor over a by-value struct.
        unsafe { ffi::audit_token_to_pid(self.0) }
    }
    /// Incremented by the kernel each time a pid is reused, so (pid, pidversion)
    /// names exactly one process instance.
    pub fn pidversion(&self) -> i32 {
        // SAFETY: pure accessor over a by-value struct.
        unsafe { ffi::audit_token_to_pidversion(self.0) }
    }
    pub fn euid(&self) -> u32 {
        // SAFETY: pure accessor over a by-value struct.
        unsafe { ffi::audit_token_to_euid(self.0) }
    }

    /// The audit token of a running process, read through its task name port. Task
    /// name ports carry no control rights and are available for same-user processes.
    pub fn of_pid(pid: i32) -> Result<Self, String> {
        let mut name: ffi::mach_port_t = 0;
        // SAFETY: out-pointer is valid; the returned send right is released below.
        let kr = unsafe { ffi::task_name_for_pid(ffi::task_self(), pid, &mut name) };
        if kr != ffi::KERN_SUCCESS {
            return Err(format!("task_name_for_pid({pid}): {}", ffi::kr_string(kr)));
        }
        let mut token = audit_token_t::default();
        let mut count = ffi::TASK_AUDIT_TOKEN_COUNT;
        // SAFETY: buffer is 8 natural_t words, matching the count passed.
        let kr = unsafe {
            ffi::task_info(
                name,
                ffi::TASK_AUDIT_TOKEN,
                (&mut token as *mut audit_token_t).cast(),
                &mut count,
            )
        };
        // SAFETY: releasing the send right acquired above.
        unsafe { ffi::mach_port_deallocate(ffi::task_self(), name) };
        if kr != ffi::KERN_SUCCESS {
            return Err(format!(
                "task_info(TASK_AUDIT_TOKEN, {pid}): {}",
                ffi::kr_string(kr)
            ));
        }
        let t = AuditToken(token);
        if t.pid() != pid {
            return Err(format!("task for pid {pid} reports pid {}", t.pid()));
        }
        Ok(t)
    }

    pub fn of_self() -> Result<Self, String> {
        Self::of_pid(std::process::id() as i32)
    }
}

/// Parent pid of a process, from libproc.
pub fn parent_pid(pid: i32) -> Result<i32, String> {
    let mut info = std::mem::MaybeUninit::<ffi::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<ffi::proc_bsdinfo>() as i32;
    // SAFETY: buffer has the size passed; proc_pidinfo writes at most that many bytes.
    let n = unsafe {
        ffi::proc_pidinfo(
            pid,
            ffi::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if n != size {
        return Err(format!("proc_pidinfo({pid}) returned {n}"));
    }
    // SAFETY: fully written by proc_pidinfo (n == size).
    Ok(unsafe { info.assume_init() }.pbi_ppid as i32)
}

// --- Code signatures ------------------------------------------------------------------

/// Owned CoreFoundation reference.
struct Cf(*const c_void);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own one reference.
            unsafe { ffi::CFRelease(self.0) };
        }
    }
}

fn cf_string(s: &str) -> Cf {
    // SAFETY: bytes/len describe a valid UTF-8 buffer.
    Cf(unsafe {
        ffi::CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as isize,
            ffi::kCFStringEncodingUTF8,
            0,
        )
    })
}

fn cf_to_string(s: *const c_void) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: type-checked before use as a CFString; buffer is sized and NUL-terminated.
    unsafe {
        if ffi::CFGetTypeID(s) != ffi::CFStringGetTypeID() {
            return None;
        }
        let mut buf = [0 as std::ffi::c_char; 256];
        if ffi::CFStringGetCString(
            s,
            buf.as_mut_ptr(),
            buf.len() as isize,
            ffi::kCFStringEncodingUTF8,
        ) == 0
        {
            return None;
        }
        Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

/// The dynamic code object for the exact process instance named by an audit token.
fn code_for_token(token: &AuditToken) -> Result<Cf, String> {
    // SAFETY: token bytes are a valid buffer of the given length.
    let data = Cf(unsafe {
        ffi::CFDataCreate(
            std::ptr::null(),
            (&token.0 as *const audit_token_t).cast(),
            std::mem::size_of::<audit_token_t>() as isize,
        )
    });
    // SAFETY: Security exports this constant; keys/values arrays have one element each.
    let dict = unsafe {
        let keys = [ffi::kSecGuestAttributeAudit];
        let values = [data.0];
        Cf(ffi::CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &ffi::kCFTypeDictionaryKeyCallBacks,
            &ffi::kCFTypeDictionaryValueCallBacks,
        ))
    };
    let mut code: ffi::SecCodeRef = std::ptr::null();
    // SAFETY: valid attributes dictionary and out-pointer.
    let st = unsafe { ffi::SecCodeCopyGuestWithAttributes(std::ptr::null(), dict.0, 0, &mut code) };
    if st != ffi::errSecSuccess {
        return Err(format!("SecCodeCopyGuestWithAttributes: OSStatus {st}"));
    }
    Ok(Cf(code))
}

fn team_of(code: &Cf) -> Result<Option<String>, String> {
    let mut info: ffi::CFDictionaryRef = std::ptr::null();
    // SAFETY: valid code object and out-pointer; info is released by Cf.
    let st = unsafe {
        ffi::SecCodeCopySigningInformation(code.0, ffi::kSecCSSigningInformation, &mut info)
    };
    if st != ffi::errSecSuccess {
        return Err(format!("SecCodeCopySigningInformation: OSStatus {st}"));
    }
    let info = Cf(info);
    // SAFETY: info is a CFDictionary; the value is borrowed (Get rule).
    let team = unsafe { ffi::CFDictionaryGetValue(info.0, ffi::kSecCodeInfoTeamIdentifier) };
    Ok(cf_to_string(team))
}

/// Team ID of the calling process, if it is signed with a certificate.
pub fn own_team_id() -> Result<Option<String>, String> {
    let mut code: ffi::SecCodeRef = std::ptr::null();
    // SAFETY: out-pointer is valid.
    let st = unsafe { ffi::SecCodeCopySelf(0, &mut code) };
    if st != ffi::errSecSuccess {
        return Err(format!("SecCodeCopySelf: OSStatus {st}"));
    }
    team_of(&Cf(code))
}

/// How the bridge authenticates the engine's code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignaturePolicy {
    /// The sender must be validly signed by an Apple-issued certificate whose leaf
    /// OU is this Team ID (Developer ID or Apple Development).
    TeamId(String),
    /// Unsigned development builds only: main itself has no Team ID. The sender's
    /// signature (ad hoc or none) is not checked; pid, pidversion, parent, and uid
    /// still are. [`SignaturePolicy::for_this_process`] refuses this when main is signed.
    UnsignedDevBypass,
}

impl SignaturePolicy {
    /// Policy derived from main's own signature: signed builds always require the
    /// Team ID; the bypass is available only when main is unsigned and the caller
    /// explicitly asked for it.
    pub fn for_this_process(allow_unsigned_dev: bool) -> Result<Self, String> {
        Self::resolve(own_team_id()?, allow_unsigned_dev)
    }

    pub fn resolve(own_team: Option<String>, allow_unsigned_dev: bool) -> Result<Self, String> {
        match own_team {
            Some(team) => Ok(SignaturePolicy::TeamId(team)),
            None if allow_unsigned_dev => Ok(SignaturePolicy::UnsignedDevBypass),
            None => Err(
                "main has no Team ID and the unsigned development bypass was not requested".into(),
            ),
        }
    }
}

/// Team IDs are 10 uppercase alphanumerics; anything else could inject into the
/// requirement language.
pub fn is_valid_team_id(team: &str) -> bool {
    team.len() == 10
        && team
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Verifies the code signature of the process named by `token`.
pub fn check_signature(token: &AuditToken, policy: &SignaturePolicy) -> Result<(), String> {
    let team = match policy {
        SignaturePolicy::UnsignedDevBypass => return Ok(()),
        SignaturePolicy::TeamId(t) => t,
    };
    if !is_valid_team_id(team) {
        return Err(format!("invalid Team ID {team:?}"));
    }
    let code = code_for_token(token)?;
    let text = format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
    let text = cf_string(&text);
    let mut req: ffi::SecRequirementRef = std::ptr::null();
    // SAFETY: valid CFString and out-pointer.
    let st = unsafe { ffi::SecRequirementCreateWithString(text.0, 0, &mut req) };
    if st != ffi::errSecSuccess {
        return Err(format!("SecRequirementCreateWithString: OSStatus {st}"));
    }
    let req = Cf(req);
    // SAFETY: valid code and requirement objects.
    let st = unsafe { ffi::SecCodeCheckValidity(code.0, ffi::kSecCSDefaultFlags, req.0) };
    if st != ffi::errSecSuccess {
        let actual = team_of(&code).ok().flatten();
        return Err(format!(
            "code signature does not satisfy Team ID {team} (OSStatus {st}, sender team {})",
            actual.as_deref().unwrap_or("none")
        ));
    }
    Ok(())
}

/// The engine instance main expects to check in: recorded right after spawn.
#[derive(Clone, Debug)]
pub struct ExpectedPeer {
    pub pid: i32,
    /// pidversion observed at spawn; `None` only if it could not be read, in which
    /// case the parent-pid check still binds the sender to a child of main.
    pub pidversion: Option<i32>,
    pub parent_pid: i32,
    pub uid: u32,
    pub signature: SignaturePolicy,
}

impl ExpectedPeer {
    /// Records the identity of a freshly spawned child of the calling process.
    pub fn for_child(pid: i32, signature: SignaturePolicy) -> Result<Self, String> {
        let token = AuditToken::of_pid(pid)?;
        Ok(Self {
            pid,
            pidversion: Some(token.pidversion()),
            parent_pid: std::process::id() as i32,
            // SAFETY: getuid has no preconditions.
            uid: unsafe { libc::getuid() },
            signature,
        })
    }

    /// Full verification of a check-in sender.
    pub fn verify(&self, token: &AuditToken) -> Result<(), String> {
        if token.pid() != self.pid {
            return Err(format!(
                "sender pid {} is not the engine pid {}",
                token.pid(),
                self.pid
            ));
        }
        if let Some(v) = self.pidversion
            && token.pidversion() != v
        {
            return Err(format!(
                "sender pidversion {} differs from spawn-time {v}",
                token.pidversion()
            ));
        }
        if token.euid() != self.uid {
            return Err(format!("sender uid {} is not {}", token.euid(), self.uid));
        }
        let ppid = parent_pid(token.pid())?;
        if ppid != self.parent_pid {
            return Err(format!(
                "sender parent {ppid} is not main {}",
                self.parent_pid
            ));
        }
        // The signature check is bound to the audit token (pid and pidversion), so it
        // examines exactly the process that sent the message.
        check_signature(token, &self.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_token_is_consistent() {
        let t = AuditToken::of_self().unwrap();
        assert_eq!(t.pid(), std::process::id() as i32);
        assert!(t.pidversion() > 0);
        assert_eq!(t.euid(), unsafe { libc::getuid() });
    }

    #[test]
    fn team_id_syntax() {
        assert!(is_valid_team_id("7FRXF46ZSN"));
        assert!(!is_valid_team_id("7frxf46zsn"));
        assert!(!is_valid_team_id("ABC\" or 1"));
        assert!(!is_valid_team_id("ABCDEFGHIJK"));
    }

    #[test]
    fn policy_resolution() {
        assert_eq!(
            SignaturePolicy::resolve(Some("ABCDE12345".into()), true).unwrap(),
            SignaturePolicy::TeamId("ABCDE12345".into())
        );
        assert_eq!(
            SignaturePolicy::resolve(None, true).unwrap(),
            SignaturePolicy::UnsignedDevBypass
        );
        assert!(SignaturePolicy::resolve(None, false).is_err());
    }

    #[test]
    fn test_binary_has_no_team_and_fails_team_policy() {
        // cargo test binaries are linker-signed ad hoc: no Team ID.
        assert_eq!(own_team_id().unwrap(), None);
        let t = AuditToken::of_self().unwrap();
        let err = check_signature(&t, &SignaturePolicy::TeamId("7FRXF46ZSN".into())).unwrap_err();
        assert!(err.contains("does not satisfy"), "{err}");
    }

    /// Positive Team ID check against a real Developer ID signature: bun (Oven,
    /// Team 7FRXF46ZSN) is the repository's package manager.
    #[test]
    fn developer_id_process_passes_its_team_and_fails_another() {
        let bun = which("bun").expect("bun must be on PATH (repository package manager)");
        let mut child = std::process::Command::new(bun)
            .args(["-e", "console.log('ready'); setTimeout(() => {}, 3000)"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // Take the token once bun runs: before its exec the child is still
        // this test binary (a fixed sleep raced exec on a loaded machine).
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().unwrap()),
            &mut line,
        )
        .unwrap();
        assert_eq!(line.trim(), "ready");
        let token = AuditToken::of_pid(child.id() as i32).unwrap();
        let ok = check_signature(&token, &SignaturePolicy::TeamId("7FRXF46ZSN".into()));
        let wrong = check_signature(&token, &SignaturePolicy::TeamId("ABCDE12345".into()));
        let peer = ExpectedPeer::for_child(
            child.id() as i32,
            SignaturePolicy::TeamId("7FRXF46ZSN".into()),
        )
        .unwrap();
        let full = peer.verify(&token);
        child.kill().ok();
        child.wait().ok();
        ok.unwrap();
        assert!(wrong.unwrap_err().contains("sender team 7FRXF46ZSN"));
        full.unwrap();
    }

    fn which(bin: &str) -> Option<std::path::PathBuf> {
        std::env::var_os("PATH")?
            .to_str()?
            .split(':')
            .map(|d| std::path::Path::new(d).join(bin))
            .find(|p| p.is_file())
    }
}
