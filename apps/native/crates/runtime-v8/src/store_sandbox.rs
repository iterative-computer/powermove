//! Confinement is applied after trusted V8 initialization, before extension input.
use crate::{Error, Result};
use serde_json::{Value, json};
pub const STORE_SEATBELT: &str = include_str!("../store.sb");
pub trait StoreSandbox {
    fn confine(&self) -> Result<Value>;
}
pub struct PlatformStoreSandbox;
#[cfg(target_os = "macos")]
impl StoreSandbox for PlatformStoreSandbox {
    fn confine(&self) -> Result<Value> {
        use std::ffi::{CStr, CString};
        #[link(name = "sandbox")]
        unsafe extern "C" {
            fn sandbox_init(profile: *const i8, flags: u64, error: *mut *mut i8) -> i32;
            fn sandbox_free_error(error: *mut i8);
        }
        let profile = CString::new(STORE_SEATBELT).unwrap();
        let mut error = std::ptr::null_mut();
        // SAFETY: owned NUL-terminated profile and a valid out pointer.
        if unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) } != 0 {
            let reason = if error.is_null() {
                "seatbelt failed".into()
            } else {
                // SAFETY: sandbox_init returned this C string; release with its matching API.
                let text = unsafe { CStr::from_ptr(error) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { sandbox_free_error(error) };
                text
            };
            return Err(Error::Host(reason));
        }
        Ok(json!({"platform":"macos", "seatbelt":true}))
    }
}
/// Real seccomp denial plus best-effort namespace creation for Linux tests.
/// It is deliberately not a macOS seatbelt equivalence claim.
#[cfg(target_os = "linux")]
impl StoreSandbox for PlatformStoreSandbox {
    fn confine(&self) -> Result<Value> {
        let namespaces;
        // SAFETY: child-only setup; no pointers are passed to unshare/prctl.
        unsafe {
            namespaces =
                libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET | libc::CLONE_NEWNS) == 0;
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(Error::Host(std::io::Error::last_os_error().to_string()));
            }
        }
        fn stmt(code: u16, k: u32) -> libc::sock_filter {
            libc::sock_filter {
                code,
                jt: 0,
                jf: 0,
                k,
            }
        }
        let arch = if cfg!(target_arch = "x86_64") {
            0xc000003e
        } else if cfg!(target_arch = "aarch64") {
            0xc00000b7
        } else {
            return Err(Error::Invalid("Unsupported seccomp test architecture"));
        };
        let mut filter = vec![
            stmt(0x20, 4),
            libc::sock_filter {
                code: 0x15,
                jt: 1,
                jf: 0,
                k: arch,
            },
            stmt(0x06, 0x80000000),
            stmt(0x20, 0),
        ];
        // Deny the x32 syscall ABI as well as native syscall numbers.
        if cfg!(target_arch = "x86_64") {
            filter.extend([
                libc::sock_filter {
                    code: 0x45,
                    jt: 0,
                    jf: 1,
                    k: 0x40000000,
                },
                stmt(0x06, 0x80000000),
            ]);
        }
        for syscall in [
            libc::SYS_openat,
            libc::SYS_openat2,
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_ptrace,
            libc::SYS_mount,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_unlinkat,
            libc::SYS_renameat,
            libc::SYS_renameat2,
            libc::SYS_mkdirat,
            libc::SYS_linkat,
            libc::SYS_symlinkat,
            libc::SYS_readlinkat,
        ] {
            filter.extend([
                libc::sock_filter {
                    code: 0x15,
                    jt: 0,
                    jf: 1,
                    k: syscall as u32,
                },
                stmt(0x06, 0x00050000 | libc::EPERM as u32),
            ]);
        }
        #[cfg(target_arch = "x86_64")]
        for syscall in [
            libc::SYS_open,
            libc::SYS_creat,
            libc::SYS_unlink,
            libc::SYS_rename,
            libc::SYS_mkdir,
            libc::SYS_rmdir,
            libc::SYS_link,
            libc::SYS_symlink,
            libc::SYS_readlink,
            libc::SYS_fork,
            libc::SYS_vfork,
        ] {
            filter.extend([
                libc::sock_filter {
                    code: 0x15,
                    jt: 0,
                    jf: 1,
                    k: syscall as u32,
                },
                stmt(0x06, 0x00050000 | libc::EPERM as u32),
            ]);
        }
        filter.push(stmt(0x06, 0x7fff0000));
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_mut_ptr(),
        };
        // SAFETY: kernel copies the bounded filter. TSYNC covers existing V8 workers.
        if unsafe { libc::syscall(libc::SYS_seccomp, 1, 1, &program) } != 0 {
            return Err(Error::Host(std::io::Error::last_os_error().to_string()));
        }
        Ok(json!({"platform":"linux-test-double","seccomp":true,"namespaces":namespaces}))
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl StoreSandbox for PlatformStoreSandbox {
    fn confine(&self) -> Result<Value> {
        Err(Error::Invalid("Unsupported Store platform"))
    }
}
