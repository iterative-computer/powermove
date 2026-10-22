//! Machine-local measurements for viewer representation choices. The disk
//! probe and power checks run at utility QoS, never on the render thread.
//! GPU observations must come from GPU timestamp queries: CPU encoding time
//! and submit-to-completion wall time are different measurements.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use pm_protocol::error::EngineProcess;
use pm_proxy::MachineProfile;

const POWER_CHECK: Duration = Duration::from_secs(30);
const POWER_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const GPU_WINDOW: usize = 64;
const GPU_MIN_SAMPLES: usize = 8;
const GPU_BUCKET_MS: f64 = 0.25;

trait Probe: Send {
    fn initial(&mut self, root: &Path) -> Result<MachineProfile, String>;
    fn power(&mut self) -> Option<bool>;
}

struct SystemProbe;

impl Probe for SystemProbe {
    fn initial(&mut self, root: &Path) -> Result<MachineProfile, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        pm_proxy::probe_machine_profile(root, 0.0).map_err(|e| e.to_string())
    }

    fn power(&mut self) -> Option<bool> {
        let mut child = Command::new("pmset")
            .args(["-g", "batt"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = std::time::Instant::now() + POWER_COMMAND_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut output = String::new();
                    child.stdout.take()?.read_to_string(&mut output).ok()?;
                    return status.success().then(|| output.contains("Battery Power"));
                }
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }
}

#[derive(Default)]
struct State {
    disk_write_mib_s: Option<f64>,
    on_battery: Option<bool>,
    gpu_samples_ms: VecDeque<f64>,
    gpu_p95_ms: Option<f64>,
    gpu_bucket: Option<u64>,
    generation: u64,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A cached, machine-local profile. The first disk/power sample happens on
/// its utility worker. Until true GPU timestamp durations arrive, `snapshot`
/// stays absent so a caller cannot mislabel CPU wall time as GPU work.
pub(super) struct MachineProfileCache {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub(super) struct GpuObserver(Arc<Shared>);

impl GpuObserver {
    pub(super) fn observe(&self, duration: Duration) {
        observe_gpu(&self.0, duration);
    }
}

impl MachineProfileCache {
    /// The desktop passes a state-file path inside its engine cache. Derive
    /// the default generated-media folder without reading or creating files
    /// on the render thread; the worker does the latter.
    pub(super) fn default_root() -> Option<PathBuf> {
        std::env::var_os("POWERMOVE_PROXY_LOCATIONS")
            .and_then(|file| {
                PathBuf::from(file)
                    .parent()
                    .map(|p| p.join("Generated Media"))
            })
            .or_else(|| std::env::var_os("PM_GENERATED_MEDIA_DIR").map(PathBuf::from))
            .filter(|root| root.is_absolute())
    }

    pub(super) fn start(store_root: PathBuf) -> Result<Self, String> {
        Self::start_with_probe(store_root, POWER_CHECK, Box::new(SystemProbe))
    }

    fn start_with_probe(
        store_root: PathBuf,
        interval: Duration,
        probe: Box<dyn Probe>,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let thread_shared = shared.clone();
        let worker = pm_engine::qos::spawn_engine_thread(
            EngineProcess::Job,
            "pm-viewer-machine-profile",
            move || run(thread_shared, store_root, interval, probe),
        )
        .map_err(|e| format!("could not start machine-profile worker: {e}"))?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub(super) fn observer(&self) -> GpuObserver {
        GpuObserver(self.shared.clone())
    }

    /// Returns `None` until the disk/power probe and eight GPU timestamps
    /// have completed. The profile is never persisted in the project.
    pub(super) fn snapshot(&self) -> Option<MachineProfile> {
        let state = self.shared.lock();
        let gpu = state.gpu_p95_ms?;
        Some(MachineProfile {
            gpu_frame_p95_ms: gpu,
            gpu_headroom_ms: (16.667 - gpu).max(0.0),
            disk_write_mib_s: state.disk_write_mib_s?,
            on_battery: state.on_battery?,
        })
    }

    /// Changes on profile readiness, power changes and materially changed
    /// GPU p95. Inspect it at playback start or clip boundaries; a change
    /// must not replace a representation in the middle of a playing clip.
    pub(super) fn generation(&self) -> u64 {
        self.shared.lock().generation
    }
}

fn observe_gpu(shared: &Shared, duration: Duration) {
    let millis = duration.as_secs_f64() * 1000.0;
    if millis <= 0.0 || millis > 2_000.0 || !millis.is_finite() {
        return;
    }
    let mut state = shared.lock();
    state.gpu_samples_ms.push_back(millis);
    if state.gpu_samples_ms.len() > GPU_WINDOW {
        state.gpu_samples_ms.pop_front();
    }
    if state.gpu_samples_ms.len() < GPU_MIN_SAMPLES {
        return;
    }
    let mut sorted: Vec<_> = state.gpu_samples_ms.iter().copied().collect();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted[(sorted.len() * 95).div_ceil(100) - 1];
    let bucket = (p95 / GPU_BUCKET_MS).round() as u64;
    state.gpu_p95_ms = Some(p95);
    if state.gpu_bucket != Some(bucket) {
        state.gpu_bucket = Some(bucket);
        state.generation = state.generation.saturating_add(1);
    }
}

impl Drop for MachineProfileCache {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.stop = true;
        }
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(shared: Arc<Shared>, root: PathBuf, interval: Duration, mut probe: Box<dyn Probe>) {
    loop {
        if shared.lock().stop {
            return;
        }
        let needs_initial = shared.lock().disk_write_mib_s.is_none();
        if needs_initial {
            if let Ok(profile) = probe.initial(&root) {
                let mut state = shared.lock();
                if !state.stop {
                    state.disk_write_mib_s = Some(profile.disk_write_mib_s);
                    state.on_battery = Some(profile.on_battery);
                    state.generation = state.generation.saturating_add(1);
                    shared.wake.notify_all();
                }
            }
        } else if let Some(on_battery) = probe.power() {
            let mut state = shared.lock();
            if !state.stop && state.on_battery != Some(on_battery) {
                state.on_battery = Some(on_battery);
                state.generation = state.generation.saturating_add(1);
                shared.wake.notify_all();
            }
        }
        let state = shared.lock();
        if state.stop {
            return;
        }
        let (state, _) = shared
            .wake
            .wait_timeout(state, interval)
            .unwrap_or_else(|e| e.into_inner());
        if state.stop {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use tempfile::tempdir;

    use super::*;

    struct FakeProbe {
        battery: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }

    impl Probe for FakeProbe {
        fn initial(&mut self, _root: &Path) -> Result<MachineProfile, String> {
            Ok(MachineProfile {
                gpu_frame_p95_ms: 0.0,
                gpu_headroom_ms: 16.667,
                disk_write_mib_s: 450.0,
                on_battery: self.battery.load(Ordering::Relaxed),
            })
        }

        fn power(&mut self) -> Option<bool> {
            Some(self.battery.load(Ordering::Relaxed))
        }
    }

    impl Drop for FakeProbe {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Relaxed);
        }
    }

    fn until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(condition(), "profile worker did not update");
    }

    #[test]
    fn profile_requires_real_gpu_samples_and_rechecks_power() {
        let root = tempdir().unwrap();
        let battery = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let profile = MachineProfileCache::start_with_probe(
            root.path().to_path_buf(),
            Duration::from_millis(10),
            Box::new(FakeProbe {
                battery: battery.clone(),
                dropped: dropped.clone(),
            }),
        )
        .unwrap();
        until(|| profile.generation() > 0);
        assert!(profile.snapshot().is_none());
        let observer = profile.observer();
        observer.observe(Duration::ZERO);
        for _ in 0..7 {
            observer.observe(Duration::from_millis(3));
        }
        assert!(profile.snapshot().is_none());
        observer.observe(Duration::from_millis(4));
        let first = profile.snapshot().unwrap();
        assert_eq!(first.gpu_frame_p95_ms, 4.0);
        assert_eq!(first.disk_write_mib_s, 450.0);
        assert!(!first.on_battery);
        let generation = profile.generation();
        battery.store(true, Ordering::Relaxed);
        until(|| profile.generation() > generation);
        assert!(profile.snapshot().unwrap().on_battery);
        drop(profile);
        assert!(dropped.load(Ordering::Relaxed));
    }

    #[test]
    fn p95_uses_bounded_window_and_rejects_invalid_samples() {
        let root = tempdir().unwrap();
        let profile = MachineProfileCache::start_with_probe(
            root.path().to_path_buf(),
            Duration::from_secs(1),
            Box::new(FakeProbe {
                battery: Arc::new(AtomicBool::new(false)),
                dropped: Arc::new(AtomicBool::new(false)),
            }),
        )
        .unwrap();
        until(|| profile.generation() > 0);
        for _ in 0..GPU_WINDOW {
            profile.observer().observe(Duration::from_millis(2));
        }
        let old = profile.generation();
        assert_eq!(profile.snapshot().unwrap().gpu_frame_p95_ms, 2.0);
        for _ in 0..GPU_WINDOW {
            profile.observer().observe(Duration::from_millis(7));
        }
        assert_eq!(profile.snapshot().unwrap().gpu_frame_p95_ms, 7.0);
        assert!(profile.generation() > old);
        assert_eq!(profile.shared.lock().gpu_samples_ms.len(), GPU_WINDOW);
    }
}
