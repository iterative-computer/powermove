//! Execute the production canonical audio stretch on Linux without the
//! Apple media stack. Only clock/profiling and scheduling are test doubles;
//! the run, pre-roll, source reads and native DSP are real. This is neither
//! a device test nor AUDIO-2 timing evidence.
use std::cell::Cell;

mod error {
    #[derive(Debug)]
    pub enum AudioError {
        Device(String),
    }
    impl std::fmt::Display for AudioError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Device(message) => f.write_str(message),
            }
        }
    }
    pub type Result<T> = std::result::Result<T, AudioError>;
}
// The run imports only these two device-independent constants.
mod mixer {
    pub const CONTROL: i64 = 16;
}
mod ring {
    pub const CHUNK: usize = 256;
}
mod profile {
    pub enum Kind {
        Map,
        Dsp,
        DspInit,
        DspSeek,
        DspProcess,
        Read,
    }
    pub struct Timer;
    pub fn timer(_: Kind) -> Timer {
        Timer
    }
}
mod realtime {
    use super::Cell;
    thread_local! { static GUARDED: Cell<bool> = const { Cell::new(false) }; }
    pub fn has_active_guard() -> bool {
        GUARDED.get()
    }
    pub fn checkpoint_dsp_step() {}
    pub fn checkpoint_phase(_: &str) {}
    pub fn checkpoint_phase_reserve(_: &str, _: u64) {}
    pub fn guarded<T>(active: bool, f: impl FnOnce() -> T) -> T {
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                GUARDED.set(self.0);
            }
        }
        let _reset = Reset(GUARDED.replace(active));
        f()
    }
}
// Compile the actual source and its exactness regressions. Its public API
// has methods used by the mixer but not by this standalone DSP harness.
#[allow(dead_code)]
#[path = "../../pm-audio/src/stretch.rs"]
mod stretch;

type Reads = Vec<(i64, usize)>;
type PcmBits = Vec<Vec<u32>>;

fn canonical(guarded: bool, speed: f64, direction: f64, trim: f64) -> (Reads, PcmBits) {
    realtime::guarded(guarded, || {
        let mut run = stretch::StretchRun::new(2, 48_000, 0);
        let mut reads = Vec::new();
        let mut map = |c: i64| {
            Ok(stretch::MapPoint {
                x: trim + direction * c as f64 * mixer::CONTROL as f64 * speed,
                on: true,
            })
        };
        let mut read =
            |from: i64, frames: usize, out: &mut [Vec<f32>]| {
                reads.push((from, frames));
                for (c, out) in out.iter_mut().enumerate() {
                    out.clear();
                    out.extend((from..from + frames as i64).map(|sample| {
                        (0.1 * (sample as f64 * (0.07 + c as f64 * 0.13)).sin()) as f32
                    }));
                }
                Ok(true)
            };
        let mut pcm = vec![Vec::new(); 2];
        let mut chunk = vec![Vec::new(); 2];
        // Both cold seek locations from AUDIO-2, then another discontinuity.
        // Each start retains the canonical pre-roll and 8,192 kept frames.
        for start in [96_000, 144_000, 81_919] {
            run.abandon_retry();
            for offset in (0..8192).step_by(256) {
                assert!(
                    run.render(start + offset, 256, &mut map, &mut read, &mut chunk)
                        .unwrap()
                );
                for (pcm, chunk) in pcm.iter_mut().zip(&chunk) {
                    pcm.extend(chunk.iter().map(|v| v.to_bits()));
                }
            }
        }
        (reads, pcm)
    })
}

#[test]
fn guarded_canonical_distinct_voice_seeks_keep_reads_and_every_pcm_bit() {
    for speed in [0.35, 1.5, 1.6] {
        for direction in [-1.0, 1.0] {
            for voice in 0..16 {
                let trim = 240_000.0 + voice as f64 * 0.0613 * 48_000.0;
                let scalar = canonical(false, speed, direction, trim);
                let batched = canonical(true, speed, direction, trim);
                assert_eq!(
                    scalar.0, batched.0,
                    "native reads {speed}/{direction}/{voice}"
                );
                assert_eq!(scalar.1, batched.1, "PCM bits {speed}/{direction}/{voice}");
            }
        }
    }
}
