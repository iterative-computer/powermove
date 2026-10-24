//! Thread QoS classes for engine processes (contract 13, invariant 10).
//!
//! Job infrastructure and agent work run at utility QoS. A user-started
//! export uses default QoS until playback starts, then utility until it stops.
//! The interactive engine keeps the default class; its render and audio
//! threads raise their own.
//!
//! On macOS a thread does not inherit its creator's QoS class: a thread
//! made with `std::thread::spawn` starts at the default class even when its
//! creator runs at utility. Engine threads are therefore made with
//! [`spawn_engine_thread`], which applies the process's class first.
//! Threads Chromium starts in the utility process itself (its IO and IPC
//! threads) keep their own classes.

use std::io;
use std::thread::{Builder, JoinHandle};

use pm_protocol::error::EngineProcess;

/// The two QoS classes used by job-engine work. Darwin's background class
/// slows the response to playback deadline signals and has no GPU effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobQos {
    Default,
    Utility,
}

/// Sets the calling thread's class. A thread can only set its own Darwin QoS;
/// workers refresh at work boundaries when playback state changes.
pub fn set_job_qos(class: JobQos) -> Result<(), String> {
    #[cfg(target_vendor = "apple")]
    {
        let native = match class {
            JobQos::Default => libc::qos_class_t::QOS_CLASS_DEFAULT,
            JobQos::Utility => libc::qos_class_t::QOS_CLASS_UTILITY,
        };
        // SAFETY: sets the calling thread's QoS class; no pointers involved.
        let status = unsafe { libc::pthread_set_qos_class_self_np(native, 0) };
        if status != 0 {
            return Err(format!("pthread_set_qos_class_self_np failed ({status})"));
        }
    }
    #[cfg(not(target_vendor = "apple"))]
    let _ = class;
    Ok(())
}

/// Sets the calling thread's QoS class for `process`.
pub fn apply_process_qos(process: EngineProcess) -> Result<(), String> {
    if process != EngineProcess::Job {
        return Ok(());
    }
    set_job_qos(JobQos::Utility)
}

/// Starts an engine thread at `process`'s QoS class. Every long-lived
/// thread an engine starts (render lanes, decoders, readers) goes through
/// this.
pub fn spawn_engine_thread<F, T>(
    process: EngineProcess,
    name: impl Into<String>,
    work: F,
) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().name(name.into()).spawn(move || {
        // Only the job engine changes class, and the call only fails for
        // an invalid class; a failure leaves the default class.
        let _ = apply_process_qos(process);
        work()
    })
}

/// The calling thread's QoS class as text (for status and tests).
pub fn current_qos() -> &'static str {
    #[cfg(target_vendor = "apple")]
    {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut priority = 0;
        // SAFETY: both out-pointers are valid for writes.
        let status = unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority)
        };
        if status != 0 {
            return "unknown";
        }
        match class {
            libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE => "userInteractive",
            libc::qos_class_t::QOS_CLASS_USER_INITIATED => "userInitiated",
            libc::qos_class_t::QOS_CLASS_DEFAULT => "default",
            libc::qos_class_t::QOS_CLASS_UTILITY => "utility",
            libc::qos_class_t::QOS_CLASS_BACKGROUND => "background",
            _ => "unspecified",
        }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        "unspecified"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_do_not_inherit_qos_but_engine_threads_get_their_class() {
        let (parent, plain, engine, interactive) = std::thread::spawn(|| {
            apply_process_qos(EngineProcess::Job).unwrap();
            let plain = std::thread::spawn(current_qos).join().unwrap();
            let engine = spawn_engine_thread(EngineProcess::Job, "pm-test-job", current_qos)
                .unwrap()
                .join()
                .unwrap();
            let interactive =
                spawn_engine_thread(EngineProcess::Interactive, "pm-test-int", current_qos)
                    .unwrap()
                    .join()
                    .unwrap();
            (current_qos(), plain, engine, interactive)
        })
        .join()
        .unwrap();
        if cfg!(target_vendor = "apple") {
            assert_eq!(parent, "utility");
            assert_eq!(plain, "default", "macOS threads do not inherit QoS");
            assert_eq!(engine, "utility");
            assert_eq!(interactive, "default");
        }
    }
}
