//! Portable work probe, not AUDIO-2: sixteen independent native histories,
//! sequentially on one thread, without the playback pool, decoder or device.
//! Run `cargo run --locked -p pm-stretch --example cold_work -- 30`.
use pm_stretch::Stretcher;
use std::time::{Duration, Instant};

extern "C" fn checkpoint() {}

fn cohort(batched: bool) -> (Duration, Vec<u32>) {
    let mut bits = Vec::new();
    let mut work = Duration::ZERO;
    for voice in 0..16 {
        let at = Instant::now();
        let mut stretch = Stretcher::new(2, 48_000).unwrap();
        work += at.elapsed();
        let rate = 1.6;
        let lead = stretch.output_seek_length(rate);
        // Two-second seek: keep the canonical grid boundary and 11,520
        // pre-roll frames. Distinct trims prevent any shared PCM head.
        let start: usize = 96_000;
        let from = start / 16_384 * 16_384 - 11_520;
        let frames = start - from + 512;
        let interval = stretch.interval_samples();
        let input_len =
            lead + (frames.div_ceil(interval) as f64 * interval as f64 * rate).ceil() as usize;
        let input: Vec<Vec<f32>> = (0..2)
            .map(|channel| {
                (0..input_len)
                    .map(|i| {
                        let phase = (i as f64 + voice as f64 * 2942.4)
                            * std::f64::consts::TAU
                            * (631.0 + channel as f64 * 366.0)
                            / 48_000.0;
                        (phase.sin() * 0.01) as f32
                    })
                    .collect()
            })
            .collect();
        let seek: Vec<_> = input.iter().map(|c| &c[..lead]).collect();
        let at = Instant::now();
        if batched {
            stretch
                .output_seek_with_checkpoint(&seek, checkpoint)
                .unwrap();
        } else {
            stretch.output_seek(&seek).unwrap();
        }
        work += at.elapsed();
        let mut fed = lead;
        let mut output = vec![vec![0.0; interval]; 2];
        for call in 1..=frames.div_ceil(interval) {
            let to = lead + (call as f64 * interval as f64 * rate).round() as usize;
            let views: Vec<_> = input.iter().map(|c| &c[fed..to]).collect();
            let mut dest: Vec<_> = output.iter_mut().map(Vec::as_mut_slice).collect();
            let at = Instant::now();
            if batched {
                stretch
                    .process_with_checkpoint(&views, &mut dest, checkpoint)
                    .unwrap();
            } else {
                stretch.process(&views, &mut dest).unwrap();
            }
            work += at.elapsed();
            bits.extend(output.iter().flatten().map(|v| v.to_bits()));
            fed = to;
        }
    }
    (work, bits)
}

fn describe(mut samples: Vec<Duration>) -> String {
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1_000.0;
    format!(
        "p50={:.3} p95={:.3} max={:.3} ms",
        ms(samples[samples.len() / 2]),
        ms(samples[(samples.len() * 95).div_ceil(100) - 1]),
        ms(*samples.last().unwrap())
    )
}

fn main() {
    let pairs: usize = std::env::args()
        .nth(1)
        .map_or(30, |s| s.parse().expect("positive pair count"));
    assert!(pairs > 0);
    let (mut scalar, mut batched) = (Vec::new(), Vec::new());
    for pair in 0..pairs {
        let (a, b) = if pair % 2 == 0 {
            (cohort(false), cohort(true))
        } else {
            let b = cohort(true);
            (cohort(false), b)
        };
        assert_eq!(a.1, b.1, "pair {pair}: scalar/batched PCM differs");
        scalar.push(a.0);
        batched.push(b.0);
    }
    println!(
        "{pairs} counterbalanced pairs; 16 independent stereo histories per cohort; all PCM bits equal"
    );
    println!("scalar work: {}", describe(scalar));
    println!("batched work: {}", describe(batched));
    println!(
        "Times sum native construction/seek/process spans; input synthesis and PCM collection are excluded. This is not a playback deadline or a Mac timing qualification."
    );
}
