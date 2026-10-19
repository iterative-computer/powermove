//! The binding against the library's documented behavior: pitch preserved
//! at every rate, level kept, alignment by the reported latencies, and the
//! same bytes for the same calls.

use pm_stretch::Stretcher;
use std::sync::atomic::{AtomicUsize, Ordering};

const SR: u32 = 48_000;

fn sine(freq: f64, len: usize, amp: f64) -> Vec<f32> {
    (0..len)
        .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / SR as f64).sin()) as f32)
        .collect()
}

/// Least-squares fit of `a·sin + b·cos` at `freq`: (amplitude, residual
/// rms relative to the amplitude, in dB).
fn fit(x: &[f32], freq: f64) -> (f64, f64) {
    let w = std::f64::consts::TAU * freq / SR as f64;
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    let det = ss * cc - sc * sc;
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    let mut res = 0.0;
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        let e = v as f64 - a * s - b * c;
        res += e * e;
    }
    let amp = (a * a + b * b).sqrt();
    let rms = (res / x.len() as f64).sqrt();
    (amp, 20.0 * (rms / (amp / 2f64.sqrt())).log10())
}

/// The frequency (Hz) whose fit leaves the least residual, searched near
/// `freq`.
fn best_freq(x: &[f32], freq: f64) -> f64 {
    let mut best = (f64::INFINITY, freq);
    let mut f = freq * 0.98;
    while f <= freq * 1.02 {
        let (_, r) = fit(x, f);
        if r < best.0 {
            best = (r, f);
        }
        f += freq * 0.0005;
    }
    best.1
}

/// Stretches `input` at a constant `rate` (input samples per output
/// sample) in interval-sized calls, the way pm-audio drives it.
fn stretch(s: &mut Stretcher, input: &[f32], rate: f64, out_len: usize) -> Vec<f32> {
    let lead = s.output_seek_length(rate);
    s.output_seek(&[&input[..lead]]).unwrap();
    let interval = s.interval_samples();
    let mut fed = lead;
    let mut out = Vec::with_capacity(out_len);
    let mut buf = vec![0f32; interval];
    while out.len() < out_len {
        let produced = out.len() + interval;
        let want = (lead as f64 + produced as f64 * rate).round() as usize;
        let want = want.min(input.len());
        s.process(&[&input[fed..want]], &mut [&mut buf]).unwrap();
        fed = want;
        out.extend_from_slice(&buf);
    }
    out.truncate(out_len);
    out
}

#[test]
fn cooperative_checkpoints_keep_interval_calls_bit_exact() {
    static CHECKPOINTS: AtomicUsize = AtomicUsize::new(0);
    extern "C" fn checkpoint() {
        CHECKPOINTS.fetch_add(1, Ordering::Relaxed);
    }

    let input = sine(440.0, SR as usize * 2, 0.4);
    for rate in [0.5, 0.75, 1.5, 2.0] {
        let mut ordinary = Stretcher::new(1, SR).unwrap();
        let mut cooperative = Stretcher::new(1, SR).unwrap();
        let lead = ordinary.output_seek_length(rate);
        let before_seek = CHECKPOINTS.load(Ordering::Relaxed);
        ordinary.output_seek(&[&input[..lead]]).unwrap();
        assert_eq!(CHECKPOINTS.load(Ordering::Relaxed), before_seek);
        cooperative
            .output_seek_with_checkpoint(&[&input[..lead]], checkpoint)
            .unwrap();
        assert!(CHECKPOINTS.load(Ordering::Relaxed) > before_seek);
        let interval = ordinary.interval_samples();
        let mut fed = lead;
        for call in 1..=8 {
            let want = (lead as f64 + (call * interval) as f64 * rate).round() as usize;
            let mut expected = vec![0.0; interval];
            let mut actual = vec![0.0; interval];
            ordinary
                .process(&[&input[fed..want]], &mut [&mut expected])
                .unwrap();
            let before_process = CHECKPOINTS.load(Ordering::Relaxed);
            cooperative
                .process_with_checkpoint(&[&input[fed..want]], &mut [&mut actual], checkpoint)
                .unwrap();
            assert!(CHECKPOINTS.load(Ordering::Relaxed) > before_process);
            assert_eq!(
                expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                actual.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                "rate {rate}, interval {call}"
            );
            fed = want;
        }

        let mut expected = vec![0.0; 2 * interval];
        let mut actual = vec![0.0; 2 * interval];
        ordinary.flush(&mut [&mut expected], rate).unwrap();
        let before_flush = CHECKPOINTS.load(Ordering::Relaxed);
        cooperative
            .flush_with_checkpoint(&mut [&mut actual], rate, checkpoint)
            .unwrap();
        assert!(CHECKPOINTS.load(Ordering::Relaxed) > before_flush);
        assert_eq!(
            expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            actual.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            "rate {rate}, flush"
        );

        // A prior cooperative call must not leave the thread-local hook set
        // for ordinary calls, including zero-input and sustained silence.
        let zeroes = vec![0.0; interval];
        let silent_channels = [&zeroes[..]];
        for call in 0..=10 {
            let mut expected = vec![0.0; interval];
            let mut actual = vec![0.0; interval];
            let before_plain = CHECKPOINTS.load(Ordering::Relaxed);
            let input: &[&[f32]] = if call == 0 { &[] } else { &silent_channels };
            ordinary.process(input, &mut [&mut expected]).unwrap();
            cooperative.process(input, &mut [&mut actual]).unwrap();
            assert_eq!(CHECKPOINTS.load(Ordering::Relaxed), before_plain);
            assert_eq!(
                expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                actual.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                "rate {rate}, plain zero call {call}"
            );
        }
    }
}

/// Within about one sample per analysis interval of unity (|rate − 1| below
/// 1/1440 at 48 kHz) the library does not re-analyse the previous block and
/// its output follows the speed (measured: 1.001× moves 997 Hz to 997.5 Hz);
/// pm-audio resamples there instead (`STRETCH_UNITY_BAND`).
#[test]
fn pitch_and_level_are_preserved_at_every_rate() {
    for &(rate, freq) in &[
        (0.5f64, 440.0f64),
        (0.75, 1000.0),
        (1.0, 440.0),
        (1.5, 2500.0),
        (2.0, 440.0),
        (1.003, 997.0),
        (0.997, 997.0),
    ] {
        let input = sine(
            freq,
            (SR as f64 * 6.0 * rate.max(1.0)) as usize + 20_000,
            0.5,
        );
        let mut s = Stretcher::new(1, SR).unwrap();
        let out = stretch(&mut s, &input, rate, SR as usize * 3);
        // Skip the start (the reflected pre-roll), measure 1 s.
        let window = &out[SR as usize..2 * SR as usize];
        let f = best_freq(window, freq);
        let (amp, residual) = fit(window, f);
        println!(
            "rate {rate}: {freq} Hz → {f:.2} Hz, level {:.3} dB, residual {residual:.1} dB",
            20.0 * (amp / 0.5).log10()
        );
        assert!((f / freq - 1.0).abs() < 0.001, "pitch at rate {rate}: {f}");
        assert!(
            (amp / 0.5 - 1.0).abs() < 0.03,
            "level at rate {rate}: {amp}"
        );
        assert!(residual < -40.0, "residual at rate {rate}: {residual} dB");
    }
}

#[test]
fn the_same_calls_give_the_same_bytes() {
    let input = sine(330.0, SR as usize * 3, 0.4);
    let mut a = Stretcher::new(1, SR).unwrap();
    let mut b = Stretcher::new(1, SR).unwrap();
    // Slower than 0.5× uses the random engine.
    let x = stretch(&mut a, &input, 0.4, SR as usize * 2);
    let y = stretch(&mut b, &input, 0.4, SR as usize * 2);
    assert_eq!(x, y);
    // A reset reseeds: the same run again.
    a.reset().unwrap();
    let z = stretch(&mut a, &input, 0.4, SR as usize * 2);
    assert_eq!(x, z);
}

#[test]
fn output_seek_aligns_the_first_output_with_the_first_input() {
    // A click at input sample 24_000, stretched at 0.5×: it lands at
    // output 48_000.
    let mut input = vec![0f32; SR as usize * 4];
    for (i, v) in input.iter_mut().enumerate() {
        // A short Hann-windowed burst, so the stretch has something tonal.
        let d = i as f64 - 24_000.0;
        if d.abs() < 96.0 {
            let w = 0.5 + 0.5 * (std::f64::consts::PI * d / 96.0).cos();
            *v = (w * (std::f64::consts::TAU * 2000.0 * d / SR as f64).sin()) as f32;
        }
    }
    let mut s = Stretcher::new(1, SR).unwrap();
    let out = stretch(&mut s, &input, 0.5, SR as usize * 2);
    // Energy centroid of the stretched burst.
    let (mut num, mut den) = (0.0, 0.0);
    for (i, &v) in out.iter().enumerate() {
        let e = (v as f64) * (v as f64);
        num += e * i as f64;
        den += e;
    }
    let centroid = num / den;
    println!(
        "burst centroid {centroid:.1}, expected 48000 (error {:.2} ms)",
        (centroid - 48_000.0) / 48.0
    );
    assert!((centroid - 48_000.0).abs() < 48.0 * 3.0, "{centroid}");
}
