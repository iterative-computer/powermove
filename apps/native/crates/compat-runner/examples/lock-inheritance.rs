//! Deterministic diagnostic for clean profile reopen while a concurrent fork
//! still holds an inherited descriptor. Run before V8 initialization.
//! This returns failure until the kernel explicitly unlocks its stable inode.
use pm_kernel::locks::FileLock;
use std::{io, os::fd::RawFd};

fn pipe() -> io::Result<[RawFd; 2]> {
    let mut fds = [-1; 2];
    // SAFETY: two writable descriptor slots, checked before use.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fds)
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let profile = tempfile::tempdir()?;
    let path = profile.path().canonicalize()?.join("synthetic.lock");
    let lock = FileLock::acquire(&path)?;
    let fds = pipe()?;
    // SAFETY: the child uses only async-signal-safe read/close/_exit. It never
    // enters Rust allocation, V8, or inherited mutexes. Parent reaps it below.
    let child = unsafe { libc::fork() };
    if child == 0 {
        unsafe {
            libc::close(fds[1]);
            let mut byte = 0u8;
            libc::read(fds[0], (&mut byte as *mut u8).cast(), 1);
            libc::_exit(0);
        }
    }
    unsafe {
        libc::close(fds[0]);
    }
    if child < 0 {
        unsafe {
            libc::close(fds[1]);
        }
        return Err(io::Error::last_os_error().into());
    }
    drop(lock); // joins the heartbeat; its final record has PID 0.
    let reopened = FileLock::acquire(&path);
    let record = std::fs::read_to_string(&path);
    // Always release/reap the diagnostic child before reporting the assertion.
    unsafe {
        let byte = 1u8;
        libc::write(fds[1], (&byte as *const u8).cast(), 1);
        libc::close(fds[1]);
        libc::waitpid(child, std::ptr::null_mut(), 0);
    }
    match reopened {
        Ok(_) => {
            println!("clean_reopen_with_inherited_descriptor=pass");
            Ok(())
        }
        Err(error) => {
            eprintln!(
                "clean_reopen_with_inherited_descriptor=FAIL; kind={:?}; synthetic_record={record:?}",
                error.kind()
            );
            Err(error.into())
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
