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
