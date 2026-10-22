//! First-use ANGLE parity decisions. No source or pixels are persisted.
//!
//! Decisions are immutable, content-addressed files, so independent viewer
//! and job processes cannot overwrite each other's pins. A damaged entry is
//! unknown and is probed again. Unknown programs always present ANGLE's frame.
//!
//! A reference the runner can never draw (a lasting failure,
//! [`FallbackFailure::is_final`]: WebGL refused the program, its draw passed
//! the runner's deadline, or its target is past the WebGL size limit) pins
//! the program to the runner, so the runner's own failure handling
//! reproduces today's ANGLE outcome (skipped and reported, turned off, or a
//! named job error). A transient failure leaves it unknown; the runner's
//! retry and backoff decide when it is asked for again.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::shader_layers::{FallbackFailure, FallbackPixels};

/// How often a cached native verification looks for a pin another process
/// wrote (one `stat`-sized read per program per period, not per draw).
const PIN_CHECK: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Unknown,
    NativeVerified,
    Runner,
}

#[derive(Debug, Default)]
pub struct Parity {
    environment: Option<String>,
    directory: Option<PathBuf>,
    decisions: HashMap<String, Verdict>,
    /// Native first-frame results stay paired with their request even if
    /// playback advances before ANGLE replies. Same bound as the transport.
    pending: HashMap<String, (String, FallbackPixels)>,
    pending_order: VecDeque<String>,
    present: HashMap<String, String>,
    /// When each cached native verification last looked for a pin on disk.
    checked: HashMap<String, Instant>,
    /// Decision PROBE-1: the frame being drawn presents an unknown
    /// program's native result and probes it in the background (viewer
    /// frames); otherwise it holds for the runner's result (exports,
    /// snapshots, renderers without the policy).
    background: bool,
    /// Programs pinned to the runner so far (by any decision, here or read
    /// from another process's marker): a host compares it around answers
    /// and frames and draws its views again when it grows.
    pins: u64,
}

impl Parity {
    /// `environment` includes the engine, GPU/driver, OS and browser versions.
    /// None disables probing (standalone renderers without a browser reference).
    pub fn configure(&mut self, environment: Option<String>, directory: Option<PathBuf>) {
        self.environment = environment;
        self.directory = directory;
        self.decisions.clear();
        self.pending.clear();
        self.pending_order.clear();
        self.present.clear();
        self.checked.clear();
    }

    pub fn environment(&self) -> Option<&str> {
        self.environment.as_deref()
    }
    pub fn directory(&self) -> Option<&std::path::Path> {
        self.directory.as_deref()
    }

    pub fn enabled(&self) -> bool {
        self.environment.is_some()
    }

    /// Decision PROBE-1 for the frames drawn from now on (see `background`).
    pub fn set_background(&mut self, background: bool) {
        self.background = background;
    }

    pub fn background(&self) -> bool {
        self.background
    }

    /// How many programs have been pinned to the runner (monotonic).
    pub fn pins(&self) -> u64 {
        self.pins
    }

    fn key(&self, source: &str) -> String {
        let mut hash = blake3::Hasher::new();
        hash.update(b"pm ANGLE parity 1\0");
        hash.update(self.environment.as_deref().unwrap_or_default().as_bytes());
        hash.update(&[0]);
        hash.update(pm_glsl::VERTEX.as_bytes());
        hash.update(&[0]);
        hash.update(source.as_bytes());
        hash.finalize().to_hex().to_string()
    }

    fn marker(&self, key: &str, suffix: &str, payload: &[u8]) -> bool {
        self.directory.as_ref().is_some_and(|dir| {
            std::fs::read(dir.join(format!("{key}.{suffix}")))
                .ok()
                .as_deref()
                == Some(payload)
        })
    }

    pub fn verdict(&mut self, source: &str) -> Verdict {
        if !self.enabled() {
            return Verdict::NativeVerified;
        }
        let key = self.key(source);
        match self.decisions.get(&key) {
            Some(Verdict::Runner) => return Verdict::Runner,
            Some(Verdict::NativeVerified) => {
                // Another renderer/process may have pinned a concurrent
                // first use. A cached native verification must never
                // conceal that pin for long; it is looked for at most once
                // per `PIN_CHECK` (it was looked for when first cached).
                let now = Instant::now();
                if self
                    .checked
                    .get(&key)
                    .is_some_and(|at| now.duration_since(*at) < PIN_CHECK)
                {
                    return Verdict::NativeVerified;
                }
                self.checked.insert(key.clone(), now);
                if self.marker(&key, "runner", b"runner\n") {
                    self.decisions.insert(key, Verdict::Runner);
                    self.pins += 1;
                    return Verdict::Runner;
                }
                return Verdict::NativeVerified;
            }
            Some(Verdict::Unknown) | None => {}
        }
        let verdict = if self.marker(&key, "runner", b"runner\n") {
            Verdict::Runner
        } else if self.marker(&key, "native", b"native-verified\n") {
            self.checked.insert(key.clone(), Instant::now());
            Verdict::NativeVerified
        } else {
            Verdict::Unknown
        };
        self.decisions.insert(key, verdict);
        verdict
    }

    /// Diagnostic corpus sweeps exercise every parameter set. Re-probe a
    /// verified program there; a pin is monotonic. Production uses first use.
    pub fn recheck(&mut self, source: &str) {
        if self.verdict(source) == Verdict::NativeVerified {
            let key = self.key(source);
            self.present.remove(&key);
            self.decisions.insert(key, Verdict::Unknown);
        }
    }

    pub fn pending(&self, source: &str) -> bool {
        self.pending.values().any(|(code, _)| code == source)
    }

    pub fn has_present(&self, source: &str) -> bool {
        self.present.contains_key(&self.key(source))
    }

    /// The probed frame still presents ANGLE even after its answer verified
    /// the native program. A newer frame may use the verification directly.
    pub fn take_present(&mut self, source: &str) -> Option<String> {
        self.present.remove(&self.key(source))
    }

    pub fn start(&mut self, request: &str, source: &str, native: FallbackPixels) {
        if self.pending(source) {
            return;
        }
        while self.pending.len() >= 32 {
            if let Some(old) = self.pending_order.pop_front() {
                self.pending.remove(&old);
            }
        }
        self.pending_order.push_back(request.to_owned());
        self.pending
            .insert(request.to_owned(), (source.to_owned(), native));
    }

    pub fn forget(&mut self, request: &str) {
        self.pending.remove(request);
        self.pending_order.retain(|key| key != request);
    }

    /// The runner could not draw a request. If it was a probe's reference
    /// and the failure is lasting, the program is pinned to the runner (the
    /// runner's failure handling then decides what today's app shows);
    /// otherwise the probe is forgotten and asked for again once the
    /// runner's retry is due.
    pub fn failed(&mut self, request: &str, failure: &FallbackFailure) {
        let probed = self.pending.remove(request);
        self.pending_order.retain(|key| key != request);
        if let Some((source, _)) = probed
            && failure.is_final()
        {
            self.pin(&source);
        }
    }

    /// Pins a program to the runner: its WebGL reference can never be
    /// compared (a lasting runner failure). Pins are never cleared.
    pub fn pin(&mut self, source: &str) {
        if self.enabled() {
            self.decide(source, Verdict::Runner);
        }
    }

    /// Judge the actual first frame when its answer lands, rather than
    /// waiting for playback to request that exact time/input a second time.
    /// Returns the verdict when `request` was a probe. A frame that held
    /// for the probe presents ANGLE's pixels for it; a background probe's
    /// frame already presented the native result, so nothing is kept.
    pub fn answer(&mut self, request: &str, angle: &FallbackPixels) -> Option<Verdict> {
        let (source, native) = self.pending.remove(request)?;
        self.pending_order.retain(|key| key != request);
        let verdict = self.judge(&source, &native, angle);
        if verdict == Verdict::NativeVerified && !self.background {
            self.present.insert(self.key(&source), request.to_owned());
        }
        Some(verdict)
    }

    /// Any channel over one code value pins the whole program. This is an
    /// absolute bound, also for HDR; non-finite disagreements pin too.
    pub fn judge(
        &mut self,
        source: &str,
        native: &FallbackPixels,
        angle: &FallbackPixels,
    ) -> Verdict {
        let verdict = if self.verdict(source) == Verdict::Runner {
            Verdict::Runner
        } else if matches(native, angle) {
            Verdict::NativeVerified
        } else {
            Verdict::Runner
        };
        self.decide(source, verdict);
        verdict
    }

    fn decide(&mut self, source: &str, verdict: Verdict) {
        let key = self.key(source);
        let before = self.decisions.insert(key.clone(), verdict);
        if verdict == Verdict::Runner && before != Some(Verdict::Runner) {
            self.pins += 1;
        }
        // The next verdict looks for a concurrent pin on disk at once.
        self.checked.remove(&key);
        if verdict == Verdict::Runner {
            self.present.remove(&key);
        }
        let Some(dir) = &self.directory else {
            return;
        };
        let bytes: &[u8] = if verdict == Verdict::Runner {
            b"runner\n"
        } else {
            b"native-verified\n"
        };
        // Atomic markers with exact payloads: an interrupted write cannot
        // be accepted as native verification.
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(dir)?;
            // Separate markers: a simultaneous native verification can
            // never overwrite an ANGLE pin from another process.
            static WRITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let suffix = if verdict == Verdict::Runner {
                "runner"
            } else {
                "native"
            };
            let serial = WRITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let temp = dir.join(format!(".{key}.{}.{serial}", std::process::id()));
            std::fs::write(&temp, bytes)?;
            let result = std::fs::rename(&temp, dir.join(format!("{key}.{suffix}")));
            if result.is_err() {
                let _ = std::fs::remove_file(temp);
            }
            result
        };
        if let Err(error) = write() {
            eprintln!("shader parity decision could not be persisted: {error}");
        }
    }
}

pub fn matches(native: &FallbackPixels, angle: &FallbackPixels) -> bool {
    native.width == angle.width
        && native.height == angle.height
        && native.rgba16f.len() == angle.rgba16f.len()
        && native.rgba16f.len() == native.width as usize * native.height as usize * 4
        && native.rgba16f.iter().zip(&angle.rgba16f).all(|(n, a)| {
            if n == a {
                return true;
            }
            let n = pm_gpu::readback::half_to_f32(*n);
            let a = pm_gpu::readback::half_to_f32(*a);
            n.is_finite() && a.is_finite() && (n - a).abs() * 255.0 <= 1.0
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixels(v: f32) -> FallbackPixels {
        FallbackPixels {
            width: 1,
            height: 1,
            rgba16f: vec![pm_gpu::readback::f32_to_half(v); 4],
        }
    }

    #[test]
    fn an_async_answer_judges_its_original_frame_and_keeps_its_angle_presentation() {
        let mut p = Parity::default();
        p.configure(Some("async".into()), None);
        p.start("frame-at-t0", "stable", pixels(0.5));
        assert!(p.pending("stable"));
        p.answer("frame-at-t0", &pixels(0.501));
        assert!(!p.pending("stable"));
        assert_eq!(p.verdict("stable"), Verdict::NativeVerified);
        assert_eq!(p.take_present("stable").as_deref(), Some("frame-at-t0"));
        assert_eq!(p.take_present("stable"), None);
        p.start("frame-at-t1", "unstable", pixels(0.5));
        p.answer("frame-at-t1", &pixels(0.8));
        assert_eq!(p.verdict("unstable"), Verdict::Runner);
        p.start("failed", "retry", pixels(0.5));
        p.forget("failed");
        assert!(!p.pending("retry"));
        assert_eq!(p.verdict("retry"), Verdict::Unknown);
    }

    #[test]
    fn a_lasting_reference_failure_pins_and_a_transient_one_leaves_it_unknown() {
        let failure = |reason: &str| FallbackFailure {
            reason: reason.into(),
            message: String::new(),
        };
        let dir = std::env::temp_dir().join(format!("pm-parity-fail-{}", std::process::id()));
        let mut p = Parity::default();
        p.configure(Some("failures".into()), Some(dir.clone()));
        for (reason, pinned) in [
            ("compile", true),
            ("watchdog", true),
            ("tooLarge", true),
            ("contextLost", false),
            ("deadline", false),
            ("unavailable", false),
            ("size", false),
        ] {
            let source = format!("program failing with {reason}");
            p.start(reason, &source, pixels(0.5));
            p.failed(reason, &failure(reason));
            assert!(!p.pending(&source), "{reason}");
            let expected = if pinned {
                Verdict::Runner
            } else {
                Verdict::Unknown
            };
            assert_eq!(p.verdict(&source), expected, "{reason}");
            let mut restarted = Parity::default();
            restarted.configure(Some("failures".into()), Some(dir.clone()));
            assert_eq!(restarted.verdict(&source), expected, "{reason} persisted");
        }
        // A failure of a request that was not a probe changes nothing.
        p.failed("not a probe", &failure("compile"));
        assert_eq!(p.verdict("unrelated"), Verdict::Unknown);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_cached_verification_looks_for_concurrent_pins_at_a_bounded_rate() {
        let dir = std::env::temp_dir().join(format!("pm-parity-rate-{}", std::process::id()));
        let mut viewer = Parity::default();
        viewer.configure(Some("rate".into()), Some(dir.clone()));
        viewer.judge("stable", &pixels(0.5), &pixels(0.5));
        assert_eq!(viewer.verdict("stable"), Verdict::NativeVerified);
        let mut job = Parity::default();
        job.configure(Some("rate".into()), Some(dir.clone()));
        job.pin("stable");
        // Within the period the cached verification stands (no disk read
        // per draw); after it, the other process's pin is seen.
        assert_eq!(viewer.verdict("stable"), Verdict::NativeVerified);
        std::thread::sleep(PIN_CHECK);
        assert_eq!(viewer.verdict("stable"), Verdict::Runner);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_channel_is_judged_without_a_pixel_budget() {
        assert!(matches(&pixels(0.5), &pixels(0.503)));
        assert!(!matches(&pixels(0.5), &pixels(0.505)));
        assert!(!matches(&pixels(2.0), &pixels(2.01)));
        assert!(!matches(&pixels(f32::INFINITY), &pixels(1.0)));
        assert!(!matches(
            &pixels(0.5),
            &FallbackPixels {
                width: 2,
                ..pixels(0.5)
            }
        ));
    }

    #[test]
    fn pins_and_verifications_survive_restart_and_versions_invalidate_them() {
        let dir = std::env::temp_dir().join(format!("pm-parity-{}", std::process::id()));
        let mut p = Parity::default();
        p.configure(
            Some("engine1; gpu1; driver1; browser1".into()),
            Some(dir.clone()),
        );
        assert_eq!(p.verdict("unstable"), Verdict::Unknown);
        assert_eq!(
            p.judge("unstable", &pixels(0.5), &pixels(0.7)),
            Verdict::Runner
        );
        assert_eq!(
            p.judge("stable", &pixels(0.5), &pixels(0.5)),
            Verdict::NativeVerified
        );
        let mut restarted = Parity::default();
        restarted.configure(p.environment.clone(), Some(dir.clone()));
        assert_eq!(restarted.verdict("unstable"), Verdict::Runner);
        assert_eq!(restarted.verdict("stable"), Verdict::NativeVerified);
        restarted.judge("stable", &pixels(0.5), &pixels(0.8));
        assert_eq!(
            p.verdict("stable"),
            Verdict::Runner,
            "a cached verification observes a pin from another renderer/process"
        );
        // Different inputs probed concurrently cannot demote a pin on disk.
        p.judge("unstable", &pixels(0.5), &pixels(0.5));
        restarted.configure(p.environment.clone(), Some(dir.clone()));
        assert_eq!(restarted.verdict("unstable"), Verdict::Runner);
        assert_eq!(
            restarted.verdict("stable + changed constant"),
            Verdict::Unknown
        );
        restarted.configure(
            Some("engine1; gpu1; driver1; browser2".into()),
            Some(dir.clone()),
        );
        assert_eq!(restarted.verdict("stable"), Verdict::Unknown);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
