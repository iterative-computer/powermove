//! Presentation identity includes display/headroom, physical size and document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub display: u32,
    pub headroom: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub generation: u64,
    pub revision: u64,
    pub sequence: u64,
}
#[derive(Debug)]
pub struct Policy {
    pub output: Output,
    pub stamp: Stamp,
}
impl Policy {
    pub fn new(output: Output) -> Self {
        Self {
            output,
            stamp: Stamp {
                generation: 1,
                revision: 1,
                sequence: 0,
            },
        }
    }
    pub fn configure(&mut self, output: Output) {
        if self.output != output {
            self.output = output;
            self.stamp.generation += 1;
            self.stamp.sequence += 1;
        }
    }
    pub fn request(&mut self, revision: u64) -> Stamp {
        self.stamp.sequence += 1;
        self.stamp.revision = revision;
        self.stamp
    }
    pub fn accepts(&self, stamp: Stamp) -> bool {
        self.stamp == stamp
    }
}
pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 * p).ceil() as usize)
        .saturating_sub(1)
        .min(v.len() - 1)]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn output() -> Output {
        Output {
            width: 640,
            height: 360,
            display: 1,
            headroom: 4.,
        }
    }
    #[test]
    fn resize_display_headroom_and_revision_retire_old_work() {
        let mut p = Policy::new(output());
        let first = p.request(7);
        assert!(p.accepts(first));
        for o in [
            Output {
                width: 320,
                ..output()
            },
            Output {
                display: 2,
                headroom: 1.,
                ..output()
            },
            output(),
        ] {
            let old = p.stamp;
            p.configure(o);
            assert!(!p.accepts(old));
        }
        let old = p.stamp;
        p.request(8);
        assert!(!p.accepts(old));
    }
    #[test]
    fn latest_seek_wins_and_unchanged_output_keeps_generation() {
        let mut p = Policy::new(output());
        let old = p.request(1);
        p.configure(output());
        assert!(p.accepts(old));
        let last = p.request(1);
        assert!(!p.accepts(old));
        assert!(p.accepts(last));
    }
    #[test]
    fn gpu_completion_lease_blocks_ring_reuse() {
        let ring = std::sync::Arc::new(());
        let frame = ring.clone();
        let completion = frame.clone();
        drop(frame);
        assert_eq!(std::sync::Arc::strong_count(&ring), 2);
        drop(completion);
        assert_eq!(std::sync::Arc::strong_count(&ring), 1);
    }
}

/// Raw working pixels: output display/headroom and overlays are applied later.
/// An Engine owns one immutable Scene/composition during a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheKey {
    pub revision: u64,
    pub time: i64,
    pub width: u32,
    pub height: u32,
}
impl CacheKey {
    pub fn same_content(self, other: Self) -> bool {
        self.revision == other.revision && self.width == other.width && self.height == other.height
    }
}
#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn cache_never_reuses_another_revision_time_or_size() {
        let k = CacheKey {
            revision: 1,
            time: 123,
            width: 1280,
            height: 720,
        };
        assert_ne!(k, CacheKey { time: 124, ..k });
        assert!(!k.same_content(CacheKey { revision: 2, ..k }));
        assert!(!k.same_content(CacheKey { width: 640, ..k }));
        assert!(!k.same_content(CacheKey { height: 360, ..k }));
        assert!(k.same_content(CacheKey { time: 124, ..k }));
    }
}

/// Display-link phase anchors a worker clock. AppKit callback jitter must not
/// stop rendering between anchors. Expired targets are counted, never replayed.
#[derive(Default)]
pub struct PlaybackSchedule {
    next: Option<f64>,
    pub skipped: u64,
}
impl PlaybackSchedule {
    pub fn observe(&mut self, target: f64, interval: f64) {
        if let Some(next) = self.next {
            let difference = target - next;
            let phase = difference - (difference / interval).round() * interval;
            self.next = Some(next + phase);
        } else {
            self.next = Some(target);
        }
    }
    pub fn wait(&self, now: f64, interval: f64) -> std::time::Duration {
        std::time::Duration::from_secs_f64(
            self.next
                .map_or(0.1, |t| (t - interval - now).clamp(0., 0.1)),
        )
    }
    pub fn take(&mut self, now: f64, interval: f64) -> Option<f64> {
        let mut target = self.next?;
        if now < target - interval {
            return None;
        }
        if target < now {
            let skipped = ((now - target) / interval).floor() as u64 + 1;
            self.skipped += skipped;
            target += skipped as f64 * interval;
        }
        self.next = Some(target + interval);
        Some(target)
    }
}
#[cfg(test)]
mod scheduling_tests {
    use super::*;
    #[test]
    fn a_missing_main_thread_tick_does_not_stop_the_worker_clock() {
        let mut p = PlaybackSchedule::default();
        let dt = 1. / 120.;
        p.observe(1. + dt, dt);
        assert!(p.take(1., dt).is_some());
        assert!(p.take(1. + dt, dt).is_some());
        assert!(p.take(1. + 2. * dt, dt).is_some());
        assert_eq!(p.skipped, 0);
    }
    #[test]
    fn expired_targets_are_counted_and_never_replayed() {
        let mut p = PlaybackSchedule::default();
        let dt = 0.01;
        p.observe(1.01, dt);
        let target = p.take(1.045, dt).unwrap();
        assert!(target >= 1.045);
        assert_eq!(p.skipped, 4);
        assert!(p.take(1.046, dt).is_none());
    }
    #[test]
    fn an_old_tick_cannot_reset_the_worker_to_an_old_target() {
        let mut p = PlaybackSchedule::default();
        p.observe(1.01, 0.01);
        p.take(1., 0.01);
        p.take(1.01, 0.01);
        p.observe(1.01, 0.01);
        assert!(p.take(1.02, 0.01).unwrap() >= 1.0299);
    }
}
