//! Blender worker supervision (the S8 stand-in for main's Blender supervision).
//!
//! Blender is a foreign renderer: its GPU work cannot be cut into 2 ms command buffers,
//! so contract 13 can only act on the process. The supervisor launches Blender the way
//! Powermove's worker does (`--background --factory-startup --disable-autoexec`), runs
//! `blender/eevee_load.py`, reads its per-frame lines, and applies a playback policy:
//!
//! - `run`: no control.
//! - `bg`: the whole process in the Darwin background state for the entire run.
//! - `bg-on-playback`: Darwin background only while playback runs.
//! - `pause`: `SIGSTOP` while playback runs, `SIGCONT` after.
//! - `yield`: `SIGSTOP` for the yield window after a deadline miss (or at-risk).
//! - `throttle`: while playback runs, alternate `run-ms` running and `stop-ms` stopped.
//!
//! `SIGSTOP` stops CPU-side submission only; command buffers Blender already committed
//! still run to completion on the GPU, which is exactly what the spike measures.

use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Run,
    Bg,
    BgOnPlayback,
    Pause,
    Yield,
    Throttle,
}

impl Policy {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "run" => Self::Run,
            "bg" => Self::Bg,
            "bg-on-playback" => Self::BgOnPlayback,
            "pause" => Self::Pause,
            "yield" => Self::Yield,
            "throttle" => Self::Throttle,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Bg => "bg",
            Self::BgOnPlayback => "bg-on-playback",
            Self::Pause => "pause",
            Self::Yield => "yield",
            Self::Throttle => "throttle",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub executable: String,
    pub script: String,
    pub out_dir: String,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub policy: Policy,
    pub run_ms: f64,
    pub stop_ms: f64,
}

/// What the supervisor wants Blender to be doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desired {
    Running,
    Stopped,
}

/// Pure policy decision, kept separate from process control so it can be tested.
///
/// `playing`: playback is running; `yielding`: inside a yield window after a miss;
/// `since_playback_ns`: time since playback started (throttle phase).
pub fn desired(policy: Policy, playing: bool, yielding: bool, since_playback_ns: u64, cfg: &Config) -> Desired {
    match policy {
        Policy::Run | Policy::Bg | Policy::BgOnPlayback => Desired::Running,
        Policy::Pause if playing => Desired::Stopped,
        Policy::Yield if yielding => Desired::Stopped,
        Policy::Throttle if playing => {
            let run = (cfg.run_ms * 1e6) as u64;
            let cycle = run + (cfg.stop_ms * 1e6) as u64;
            if cycle == 0 || since_playback_ns % cycle < run {
                Desired::Running
            } else {
                Desired::Stopped
            }
        }
        _ => Desired::Running,
    }
}

#[derive(Serialize, Default, Clone)]
pub struct Stats {
    pub policy: &'static str,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    /// Frames finished before the measured window (shader compilation and warm-up).
    pub warm_frames: u64,
    /// Frames finished while the supervisor ran its loop.
    pub frames: u64,
    pub render_ms: crate::stats::Dist,
    pub write_ms: crate::stats::Dist,
    pub stops: u64,
    pub stopped_ms: f64,
    pub bg_transitions: u64,
    pub errors: Vec<String>,
}

pub struct Blender {
    child: Child,
    pid: i32,
    cfg: Config,
    stopped: bool,
    stopped_since: u64,
    bg: bool,
    frames: Arc<Mutex<Vec<(u64, f64, f64)>>>,
    pub stats: Stats,
}

fn set_darwin_bg(pid: i32, on: bool) -> Result<(), String> {
    const PRIO_DARWIN_PROCESS: i32 = 4;
    const PRIO_DARWIN_BG: i32 = 0x1000;
    // SAFETY: plain call on a child process we own.
    let rc = unsafe { libc::setpriority(PRIO_DARWIN_PROCESS, pid as u32, if on { PRIO_DARWIN_BG } else { 0 }) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!(
            "setpriority(PRIO_DARWIN_BG, {pid}) errno {}",
            std::io::Error::last_os_error()
        ))
    }
}

impl Blender {
    /// Launch Blender and wait until it has finished `warm` frames (shader compilation),
    /// or fail after `timeout_s`.
    pub fn launch(cfg: Config, warm: u64, timeout_s: f64) -> Result<Self, String> {
        std::fs::create_dir_all(&cfg.out_dir).map_err(|e| format!("{}: {e}", cfg.out_dir))?;
        let mut child = Command::new(&cfg.executable)
            .args([
                "--background",
                "--factory-startup",
                "--disable-autoexec",
                "--python-exit-code",
                "1",
                "--python",
                &cfg.script,
                "--",
                "--out",
                &cfg.out_dir,
                "--width",
                &cfg.width.to_string(),
                "--height",
                &cfg.height.to_string(),
                "--samples",
                &cfg.samples.to_string(),
            ])
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", cfg.executable))?;
        let pid = child.id() as i32;
        let frames: Arc<Mutex<Vec<(u64, f64, f64)>>> = Arc::default();
        let stdout = child.stdout.take().unwrap();
        {
            let frames = frames.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let Some(rest) = line.strip_prefix("S8BLENDER ") else {
                        continue;
                    };
                    let f: Vec<f64> = rest.split(' ').filter_map(|x| x.parse().ok()).collect();
                    if f.len() == 3 {
                        frames.lock().unwrap().push((crate::clock::now_ns(), f[1], f[2]));
                    }
                }
            });
        }
        let start = std::time::Instant::now();
        loop {
            if frames.lock().unwrap().len() as u64 >= warm {
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("Blender exited during warm-up: {status}"));
            }
            if start.elapsed().as_secs_f64() > timeout_s {
                let _ = child.kill();
                return Err(format!("Blender did not finish {warm} frames in {timeout_s} s"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let warm_frames = frames.lock().unwrap().len() as u64;
        frames.lock().unwrap().clear();
        let mut b = Self {
            child,
            pid,
            stopped: false,
            stopped_since: 0,
            bg: false,
            frames,
            stats: Stats {
                policy: cfg.policy.name(),
                width: cfg.width,
                height: cfg.height,
                samples: cfg.samples,
                warm_frames,
                ..Default::default()
            },
            cfg,
        };
        if b.cfg.policy == Policy::Bg {
            b.set_bg(true);
        }
        Ok(b)
    }

    fn set_bg(&mut self, on: bool) {
        if self.bg == on {
            return;
        }
        match set_darwin_bg(self.pid, on) {
            Ok(()) => {
                self.bg = on;
                self.stats.bg_transitions += 1;
            }
            Err(e) => self.stats.errors.push(e),
        }
    }

    fn signal(&mut self, sig: i32) {
        // SAFETY: plain signal to our own child.
        if unsafe { libc::kill(self.pid, sig) } != 0 {
            self.stats.errors.push(format!(
                "kill({}, {sig}): {}",
                self.pid,
                std::io::Error::last_os_error()
            ));
        }
    }

    /// Apply the policy for the current state; call often (every millisecond or so).
    pub fn apply(&mut self, playing: bool, yielding: bool, since_playback_ns: u64) {
        if self.cfg.policy == Policy::BgOnPlayback {
            self.set_bg(playing);
        }
        let want = desired(self.cfg.policy, playing, yielding, since_playback_ns, &self.cfg);
        let now = crate::clock::now_ns();
        match (want, self.stopped) {
            (Desired::Stopped, false) => {
                self.signal(libc::SIGSTOP);
                self.stopped = true;
                self.stopped_since = now;
                self.stats.stops += 1;
            }
            (Desired::Running, true) => {
                self.signal(libc::SIGCONT);
                self.stopped = false;
                self.stats.stopped_ms += (now - self.stopped_since) as f64 / 1e6;
            }
            _ => {}
        }
    }

    /// Stop Blender and return its statistics.
    pub fn finish(mut self) -> Stats {
        if self.stopped {
            self.signal(libc::SIGCONT);
            self.stats.stopped_ms += (crate::clock::now_ns() - self.stopped_since) as f64 / 1e6;
            self.stopped = false;
        }
        let frames = std::mem::take(&mut *self.frames.lock().unwrap());
        self.stats.frames = frames.len() as u64;
        self.stats.render_ms = crate::stats::dist(&frames.iter().map(|f| f.1).collect::<Vec<_>>()).rounded(1);
        self.stats.write_ms = crate::stats::dist(&frames.iter().map(|f| f.2).collect::<Vec<_>>()).rounded(1);
        self.stats.stopped_ms = crate::stats::r(self.stats.stopped_ms, 1);
        self.signal(libc::SIGTERM);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return self.stats.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stats.clone()
    }
}

impl Drop for Blender {
    fn drop(&mut self) {
        // Never leave a stopped or orphaned Blender behind (finish() already reaped it in
        // the normal path; these calls are then no-ops on a dead pid).
        if let Ok(None) = self.child.try_wait() {
            // SAFETY: plain signals to our own child.
            unsafe {
                libc::kill(self.pid, libc::SIGCONT);
                libc::kill(self.pid, libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(policy: Policy) -> Config {
        Config {
            executable: String::new(),
            script: String::new(),
            out_dir: String::new(),
            width: 0,
            height: 0,
            samples: 0,
            policy,
            run_ms: 10.0,
            stop_ms: 40.0,
        }
    }

    #[test]
    fn policies_decide_as_documented() {
        use Desired::*;
        let c = cfg(Policy::Run);
        assert_eq!(desired(Policy::Run, true, true, 0, &c), Running);
        assert_eq!(desired(Policy::Bg, true, true, 0, &c), Running);
        assert_eq!(desired(Policy::BgOnPlayback, true, false, 0, &c), Running);
        assert_eq!(desired(Policy::Pause, true, false, 0, &c), Stopped);
        assert_eq!(desired(Policy::Pause, false, true, 0, &c), Running);
        assert_eq!(desired(Policy::Yield, true, false, 0, &c), Running);
        assert_eq!(desired(Policy::Yield, true, true, 0, &c), Stopped);
        // Throttle: 10 ms on, 40 ms off, only while playing.
        assert_eq!(desired(Policy::Throttle, true, false, 5_000_000, &c), Running);
        assert_eq!(desired(Policy::Throttle, true, false, 15_000_000, &c), Stopped);
        assert_eq!(desired(Policy::Throttle, true, false, 55_000_000, &c), Running);
        assert_eq!(desired(Policy::Throttle, false, false, 15_000_000, &c), Running);
        for p in ["run", "bg", "bg-on-playback", "pause", "yield", "throttle"] {
            assert_eq!(Policy::parse(p).unwrap().name(), p);
        }
    }

    /// SIGSTOP and SIGCONT through the supervisor's signal path on a real child.
    #[test]
    fn stop_and_continue_a_child() {
        let child = Command::new("/bin/sleep").arg("5").spawn().unwrap();
        let pid = child.id() as i32;
        let mut b = Blender {
            child,
            pid,
            cfg: cfg(Policy::Pause),
            stopped: false,
            stopped_since: 0,
            bg: false,
            frames: Arc::default(),
            stats: Stats::default(),
        };
        b.apply(true, false, 0);
        assert!(b.stopped);
        let state = Command::new("/bin/ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&state.stdout).trim().starts_with('T'),
            "not stopped"
        );
        b.apply(false, false, 0);
        assert!(!b.stopped);
        assert_eq!(b.stats.stops, 1);
        b.set_bg(true);
        assert!(b.stats.errors.is_empty(), "{:?}", b.stats.errors);
        let s = b.finish();
        assert!(s.stopped_ms >= 0.0);
    }
}
