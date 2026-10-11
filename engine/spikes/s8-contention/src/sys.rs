//! Thread and process scheduling controls (macOS).

use std::ffi::c_int;

/// Darwin quality-of-service classes (`<sys/qos.h>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qos {
    UserInteractive,
    UserInitiated,
    Default,
    Utility,
    Background,
}

impl Qos {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user-interactive" | "interactive" => Self::UserInteractive,
            "user-initiated" | "initiated" => Self::UserInitiated,
            "default" => Self::Default,
            "utility" => Self::Utility,
            "background" => Self::Background,
            _ => return None,
        })
    }

    fn raw(self) -> u32 {
        match self {
            Self::UserInteractive => 0x21,
            Self::UserInitiated => 0x19,
            Self::Default => 0x15,
            Self::Utility => 0x11,
            Self::Background => 0x09,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::UserInteractive => "user-interactive",
            Self::UserInitiated => "user-initiated",
            Self::Default => "default",
            Self::Utility => "utility",
            Self::Background => "background",
        }
    }
}

#[repr(C)]
struct ThreadTimeConstraintPolicy {
    period: u32,
    computation: u32,
    constraint: u32,
    preemptible: i32,
}

const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
const THREAD_TIME_CONSTRAINT_POLICY_COUNT: u32 = 4;
const PRIO_DARWIN_PROCESS: c_int = 4;
const PRIO_DARWIN_BG: c_int = 0x1000;

unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: c_int) -> c_int;
    fn mach_thread_self() -> u32;
    fn thread_policy_set(thread: u32, flavor: u32, policy_info: *mut i32, count: u32) -> i32;
}

/// Set the calling thread's QoS class.
pub fn set_thread_qos(qos: Qos) -> Result<(), String> {
    // SAFETY: plain call on the current thread.
    let rc = unsafe { pthread_set_qos_class_self_np(qos.raw(), 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("pthread_set_qos_class_self_np({}) = {rc}", qos.name()))
    }
}

/// Put the calling thread under the Mach time-constraint (real-time) policy, the way
/// CoreVideo's display-link thread runs. Times are in nanoseconds.
pub fn set_thread_realtime(period_ns: u64, computation_ns: u64, constraint_ns: u64) -> Result<(), String> {
    let mut policy = ThreadTimeConstraintPolicy {
        period: crate::clock::ns_to_ticks(period_ns) as u32,
        computation: crate::clock::ns_to_ticks(computation_ns) as u32,
        constraint: crate::clock::ns_to_ticks(constraint_ns) as u32,
        preemptible: 1,
    };
    // SAFETY: policy is a correctly sized THREAD_TIME_CONSTRAINT_POLICY struct.
    let kr = unsafe {
        thread_policy_set(
            mach_thread_self(),
            THREAD_TIME_CONSTRAINT_POLICY,
            &mut policy as *mut _ as *mut i32,
            THREAD_TIME_CONSTRAINT_POLICY_COUNT,
        )
    };
    if kr == 0 {
        Ok(())
    } else {
        Err(format!("thread_policy_set(TIME_CONSTRAINT) = {kr}"))
    }
}

/// Darwin background process state (`setpriority(PRIO_DARWIN_PROCESS, 0, PRIO_DARWIN_BG)`):
/// CPU, I/O, and network throttling for the whole process.
pub fn set_process_darwin_bg() -> Result<(), String> {
    // SAFETY: plain call on the current process.
    let rc = unsafe { libc::setpriority(PRIO_DARWIN_PROCESS, 0, PRIO_DARWIN_BG) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!(
            "setpriority(PRIO_DARWIN_BG) = {rc} errno {}",
            std::io::Error::last_os_error()
        ))
    }
}

static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_term(_sig: c_int) {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Install SIGTERM and SIGINT handlers that set a flag read by [`stop_requested`].
pub fn install_stop_handler() {
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
    }
}

pub fn stop_requested() -> bool {
    STOP.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_can_be_set_and_parsed() {
        for name in ["user-interactive", "utility", "background", "default"] {
            let q = Qos::parse(name).unwrap();
            assert_eq!(q.name(), name);
        }
        assert!(Qos::parse("nope").is_none());
        // Run in a scratch thread so the test runner's thread keeps its QoS.
        std::thread::spawn(|| set_thread_qos(Qos::Utility).unwrap())
            .join()
            .unwrap();
    }

    #[test]
    fn realtime_policy_is_accepted() {
        std::thread::spawn(|| set_thread_realtime(16_666_667, 2_000_000, 8_000_000).unwrap())
            .join()
            .unwrap();
    }
}
