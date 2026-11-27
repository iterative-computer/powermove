//! Fault injection: stall one engine stage on purpose, to prove the UI never
//! waits on it (PM-699). Debug and test builds only; release builds compile
//! every call to nothing.
//!
//! `PM_STALL=render=2,decode=0.5` makes each frame render sleep 2 s and each
//! decode 0.5 s before it works (seconds, fractions allowed). Stage names:
//! `render`, `decode`, `shader`, `export`, `kernel`, `v8`.
//! `PM_STALL_FILE=/path` reads the same text from a file, again at most four
//! times a second, so one running app can be stalled stage by stage (an
//! empty file clears every stall). Tests set a stall in process with [`set`].

use std::time::Duration;

/// An engine stage a stall can hold up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Stage {
    /// One viewer frame render on the engine worker.
    Render,
    /// One media frame decode.
    Decode,
    /// One shader or pipeline compile.
    Shader,
    /// One export frame.
    Export,
    /// One kernel actor message (the work before a publication).
    Kernel,
    /// One V8 realm turn (panel or extension script work).
    V8,
}

impl Stage {
    pub const ALL: [Stage; 6] = [
        Stage::Render,
        Stage::Decode,
        Stage::Shader,
        Stage::Export,
        Stage::Kernel,
        Stage::V8,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Render => "render",
            Stage::Decode => "decode",
            Stage::Shader => "shader",
            Stage::Export => "export",
            Stage::Kernel => "kernel",
            Stage::V8 => "v8",
        }
    }
}

#[cfg(debug_assertions)]
mod imp {
    use super::Stage;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Once, OnceLock};
    use std::time::{Duration, Instant};

    static MICROS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
    static ENV: Once = Once::new();
    static FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
    /// When the file was last read, in ms after `EPOCH`.
    static READ_AT: AtomicU64 = AtomicU64::new(0);
    static EPOCH: OnceLock<Instant> = OnceLock::new();

    fn apply(spec: &str) {
        let mut micros = [0u64; 6];
        for part in spec.split(',') {
            let Some((name, secs)) = part.split_once('=') else {
                continue;
            };
            let (Some(stage), Ok(secs)) = (
                Stage::ALL.into_iter().find(|s| s.name() == name.trim()),
                secs.trim().parse::<f64>(),
            ) else {
                continue;
            };
            if secs.is_finite() && secs > 0. {
                micros[stage as usize] = (secs * 1e6) as u64;
            }
        }
        for (slot, us) in MICROS.iter().zip(micros) {
            slot.store(us, Ordering::Relaxed);
        }
    }

    fn read_env() {
        ENV.call_once(|| {
            if let Ok(spec) = std::env::var("PM_STALL") {
                apply(&spec);
            }
        });
        let Some(path) = FILE.get_or_init(|| std::env::var_os("PM_STALL_FILE").map(PathBuf::from))
        else {
            return;
        };
        let now = EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1;
        let last = READ_AT.load(Ordering::Relaxed);
        if (last == 0 || now >= last + 250)
            && READ_AT
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            apply(&std::fs::read_to_string(path).unwrap_or_default());
        }
    }

    pub fn set(stage: Stage, stall: Duration) {
        read_env();
        MICROS[stage as usize].store(stall.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn get(stage: Stage) -> Duration {
        read_env();
        Duration::from_micros(MICROS[stage as usize].load(Ordering::Relaxed))
    }
}

/// Sleeps for the stall set for `stage`, if any. Call it on the stage's own
/// thread, at the start of the work it stands for.
#[inline]
pub fn stall(stage: Stage) {
    #[cfg(debug_assertions)]
    {
        let d = imp::get(stage);
        if !d.is_zero() {
            std::thread::sleep(d);
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = stage;
}

/// Sets (or with `Duration::ZERO` clears) the stall for `stage`, over
/// `PM_STALL`. Does nothing in release builds.
pub fn set(stage: Stage, stall: Duration) {
    #[cfg(debug_assertions)]
    imp::set(stage, stall);
    #[cfg(not(debug_assertions))]
    let _ = (stage, stall);
}

/// The stall set for `stage` (zero in release builds).
pub fn get(stage: Stage) -> Duration {
    #[cfg(debug_assertions)]
    return imp::get(stage);
    #[cfg(not(debug_assertions))]
    {
        let _ = stage;
        Duration::ZERO
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn set_stalls_only_its_stage() {
        set(Stage::Shader, Duration::from_millis(30));
        let at = Instant::now();
        stall(Stage::Shader);
        assert!(at.elapsed() >= Duration::from_millis(30));
        let at = Instant::now();
        stall(Stage::Decode);
        assert!(at.elapsed() < Duration::from_millis(30));
        set(Stage::Shader, Duration::ZERO);
        assert!(get(Stage::Shader).is_zero());
    }
}
