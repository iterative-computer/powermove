//! Presentation stats per viewer session (the Phase 3c render transport,
//! contract 2's `RenderStats`): what presenting the session costs, sent to
//! the session's renderer at most twice a second while it presents and
//! once after it goes idle. They replace today's `GL.stats` and GPU timing
//! for the status bar and the performance panel.

use pm_protocol::common::{Count, Revision};
use pm_protocol::snapshot::RenderStats;

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

    /// Whether a report is due at host µs `now`: something changed since
    /// the last one, and the interval passed.
    pub fn due(&self, now: i64) -> bool {
        (self.frames != self.sent_frames || self.late != self.sent_late)
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
        }
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
}
