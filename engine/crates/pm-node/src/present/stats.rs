//! Presentation stats per viewer session (the Phase 3c render transport,
//! contract 2's `RenderStats`): what presenting the session costs, sent to
//! the session's renderer at most twice a second while it presents and
//! once after it goes idle. They replace today's `GL.stats` and GPU timing
//! for the status bar and the performance panel. They also carry the
//! session's audio health (issue A5), sent when it changes even while
//! nothing presents (a voice dropped by the bounded wait, A3, while paused).

use pm_protocol::common::{Count, Revision};
use pm_protocol::snapshot::{AudioDiagnostics, RenderStats};

use super::audio::AudioStatus;

/// Shortest interval between two reports of one session (µs).
pub const REPORT_INTERVAL_US: i64 = 500_000;
/// The average's weight for a new frame (today's `E.ms = E.ms × .85 + ms × .15`).
const AVERAGE_WEIGHT: f64 = 0.15;

/// One session's counters.
#[derive(Debug, Default, Clone)]
pub struct ViewStats {
    frames: u64,
    timed: u64,
    late: u64,
    layer_failures: u64,
    cpu_us: u32,
    gpu_us: u32,
    layers: u32,
    average_us: Option<f64>,
    revision: u64,
    sent_at: Option<i64>,
    sent_frames: u64,
    sent_late: u64,
    audio: Option<AudioDiagnostics>,
    sent_audio: Option<AudioDiagnostics>,
}

fn us(v: i64) -> u32 {
    v.clamp(0, i64::from(u32::MAX)) as u32
}

fn count(v: u64) -> Count {
    Count::new(v).unwrap_or(Count::MAX)
}

impl ViewStats {
    /// A frame went to the viewer.
    pub fn frame(
        &mut self,
        timed: bool,
        cpu_us: i64,
        gpu_us: i64,
        layers: usize,
        failures: usize,
        revision: u64,
    ) {
        self.frames += 1;
        self.timed += u64::from(timed);
        self.layer_failures += failures as u64;
        self.cpu_us = us(cpu_us);
        self.gpu_us = us(gpu_us);
        self.layers = layers.min(u32::MAX as usize) as u32;
        self.revision = revision;
        let total = (cpu_us.max(0) + gpu_us.max(0)) as f64;
        self.average_us = Some(match self.average_us {
            Some(avg) => avg * (1.0 - AVERAGE_WEIGHT) + total * AVERAGE_WEIGHT,
            None => total,
        });
    }

    /// A playback frame was skipped as late.
    pub fn late(&mut self) {
        self.late += 1;
    }

    /// The session's audio status (`None`: no audio playback).
    pub fn audio(&mut self, status: Option<&AudioStatus>) {
        self.audio = status.map(diagnostics);
    }

    /// Whether a report is due at host µs `now`: something changed since
    /// the last one, and the interval passed.
    pub fn due(&self, now: i64) -> bool {
        (self.frames != self.sent_frames
            || self.late != self.sent_late
            || self.audio != self.sent_audio)
            && self.sent_at.is_none_or(|at| now - at >= REPORT_INTERVAL_US)
    }

    /// The report, marking it sent at `now`.
    pub fn report(
        &mut self,
        now: i64,
        programs: usize,
        rasters: usize,
        snapshots: u64,
        snapshot_us_max: i64,
    ) -> RenderStats {
        self.sent_at = Some(now);
        self.sent_frames = self.frames;
        self.sent_late = self.late;
        self.sent_audio = self.audio.clone();
        RenderStats {
            frames: count(self.frames),
            timed_frames: count(self.timed),
            late_skipped: count(self.late),
            cpu_us: self.cpu_us,
            gpu_us: self.gpu_us,
            layers: self.layers,
            frame_us_avg: us(self.average_us.unwrap_or(0.0).round() as i64),
            programs: programs.min(u32::MAX as usize) as u32,
            rasters: rasters.min(u32::MAX as usize) as u32,
            layer_failures: count(self.layer_failures),
            snapshots: count(snapshots),
            snapshot_us_max: us(snapshot_us_max),
            revision: Revision::new(self.revision).unwrap_or(Revision::ZERO),
            audio: self.audio.clone(),
        }
    }
}

/// What the status bar and the performance panel show of an audio status.
fn diagnostics(status: &AudioStatus) -> AudioDiagnostics {
    AudioDiagnostics {
        error: status.error.clone(),
        problems: status.problems.clone(),
        starved_renders: count(status.starved_renders),
        underruns: count(status.underruns),
        device_overloads: count(status.device_overloads),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_at_most_twice_a_second_and_once_after_idle() {
        let mut s = ViewStats::default();
        assert!(!s.due(0), "nothing to report");
        s.frame(false, 3_000, 5_000, 4, 0, 7);
        assert!(s.due(0));
        let r = s.report(0, 2, 9, 1, 40_000);
        assert_eq!(r.frames.get(), 1);
        assert_eq!(
            (r.cpu_us, r.gpu_us, r.layers, r.revision.get()),
            (3_000, 5_000, 4, 7)
        );
        assert_eq!(r.frame_us_avg, 8_000);
        assert_eq!(
            (r.programs, r.rasters, r.snapshots.get(), r.snapshot_us_max),
            (2, 9, 1, 40_000)
        );
        s.frame(true, 1_000, 1_000, 4, 1, 8);
        assert!(!s.due(400_000), "within the interval");
        assert!(s.due(500_000));
        let r = s.report(500_000, 2, 9, 1, 40_000);
        // 8,000 × 0.85 + 2,000 × 0.15.
        assert_eq!(r.frame_us_avg, 7_100);
        assert_eq!((r.timed_frames.get(), r.layer_failures.get()), (1, 1));
        // Idle: nothing new, nothing sent.
        assert!(!s.due(2_000_000));
        s.late();
        assert!(s.due(2_000_000));
    }

    /// A paused session presents nothing, yet a voice the bounded wait
    /// dropped (A3) or a failed device reaches the status bar: audio
    /// changes make a report due on their own, and unchanged audio does
    /// not.
    #[test]
    fn audio_changes_report_while_nothing_presents() {
        let mut s = ViewStats::default();
        s.audio(None);
        assert!(!s.due(0), "no audio, nothing to report");
        let mut status = AudioStatus {
            starved_renders: 2,
            ..AudioStatus::default()
        };
        s.audio(Some(&status));
        assert!(s.due(0));
        let r = s.report(0, 0, 0, 0, 0);
        let audio = r.audio.expect("audio diagnostics");
        assert_eq!(audio.starved_renders.get(), 2);
        assert!(audio.problems.is_empty() && audio.error.is_none());
        assert_eq!(r.frames.get(), 0, "no frame was presented");
        // The same status again (every render-thread turn): nothing new.
        s.audio(Some(&status));
        assert!(!s.due(1_000_000));
        status
            .problems
            .push("Audio decode exceeded 120 ms; missing voice plays as silence: music".into());
        status.underruns = 3;
        status.device_overloads = 1;
        s.audio(Some(&status));
        assert!(!s.due(400_000), "within the interval");
        assert!(s.due(1_000_000));
        let audio = s.report(1_000_000, 0, 0, 0, 0).audio.unwrap();
        assert_eq!(audio.problems, status.problems);
        assert_eq!(
            (audio.underruns.get(), audio.device_overloads.get()),
            (3, 1)
        );
        // A device that does not open.
        status.error = Some("no audio output: no default output device".into());
        s.audio(Some(&status));
        assert!(s.due(2_000_000));
        let audio = s.report(2_000_000, 0, 0, 0, 0).audio.unwrap();
        assert_eq!(audio.error.as_deref(), status.error.as_deref());
        // The audio closed: the next report says so.
        s.audio(None);
        assert!(s.due(3_000_000));
        assert_eq!(s.report(3_000_000, 0, 0, 0, 0).audio, None);
    }
}
