//! Section 16 resampling bars, measured: at least 120 dB stopband and a flat
//! passband to 0.45·fs, for the voices' exact-phase banks (rational ratios),
//! the free-position bank (speed and remaps), and the rubato converter that
//! takes the master to the device rate.

use pm_audio::kernel::{self, Bank};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType};
use std::f64::consts::TAU;

const N: usize = 1 << 15;

fn sine(rate: f64, hz: f64, len: usize, phase: f64) -> Vec<f32> {
    (0..len)
        .map(|i| (TAU * hz * i as f64 / rate + phase).sin() as f32)
        .collect()
}

/// Least-squares fit of `a·sin + b·cos` at `hz`; returns (amplitude, residual
/// RMS) over the signal, both relative to full scale.
fn fit(x: &[f32], rate: f64, hz: f64) -> (f64, f64) {
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = TAU * hz * i as f64 / rate;
        let (s, c) = w.sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    let det = ss * cc - sc * sc;
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    let mut r = 0.0;
    for (i, &v) in x.iter().enumerate() {
        let w = TAU * hz * i as f64 / rate;
        let e = v as f64 - (a * w.sin() + b * w.cos());
        r += e * e;
    }
    ((a * a + b * b).sqrt(), (r / x.len() as f64).sqrt())
}

/// The signal with the best-fit sinusoid at `hz` removed.
fn remove(x: &[f32], rate: f64, hz: f64) -> Vec<f32> {
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (TAU * hz * i as f64 / rate).sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    let det = ss * cc - sc * sc;
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    x.iter()
        .enumerate()
        .map(|(i, &v)| {
            let (s, c) = (TAU * hz * i as f64 / rate).sin_cos();
            (v as f64 - a * s - b * c) as f32
        })
        .collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

fn stop_text(db: f64) -> String {
    if db == f64::MIN {
        "n/a (nothing folds into the passband)".into()
    } else {
        format!("{db:.1} dB")
    }
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

/// Reads `input` (at `source` Hz) at `project` Hz through the exact-phase
/// bank, as a voice with a linear map does.
fn grid(input: &[f32], source: u32, project: u32, out_len: usize) -> Vec<f32> {
    let bank: std::sync::Arc<Bank> = kernel::grid_bank(source, project).unwrap();
    let g = kernel::gcd(source as u64, project as u64) as i128;
    let phases = bank.phases as i128;
    let step = source as i128 / g;
    // Start where the filter is clear of the signal's start.
    let start = (bank.taps as f64 * project as f64 / source as f64).ceil() as i64 + 1;
    let last = ((start + out_len as i64) as i128 * step).div_euclid(phases) as usize + bank.taps;
    assert!(
        last <= input.len(),
        "{source}->{project}: needs {last} input samples, has {}",
        input.len()
    );
    (0..out_len as i64)
        .map(|n| {
            let u = (start as i128 + n as i128) * step;
            let (i, ph) = (u.div_euclid(phases) as i64, u.rem_euclid(phases) as usize);
            bank.grid(input, 0, i, ph)
        })
        .collect()
}

/// Reads `input` at fractional positions `start + n·step` through the free bank.
fn free(input: &[f32], start: f64, step: f64, out_len: usize) -> Vec<f32> {
    let bank = kernel::free_bank(step.abs());
    let mut k = vec![0f32; bank.taps];
    (0..out_len)
        .map(|n| {
            let p = start + n as f64 * step;
            let fl = p.floor();
            bank.free_kernel(p - fl, &mut k);
            let at = fl as usize - bank.before();
            kernel::dot(&input[at..at + bank.taps], &k)
        })
        .collect()
}

#[test]
fn exact_phase_banks_meet_the_bars() {
    let mut report = Vec::new();
    for (source, project) in [
        (44_100u32, 48_000u32),
        (48_000, 44_100),
        (96_000, 48_000),
        (22_050, 48_000),
        (32_000, 48_000),
    ] {
        let low = source.min(project) as f64;
        let taps = kernel::grid_bank(source, project).unwrap().taps;
        let len_in = (N as f64 * source as f64 / project as f64).ceil() as usize + 2 * taps + 32;
        // Passband: flat to 0.45·fs of the lower rate.
        let mut worst_pass = 0f64;
        let mut worst_noise = f64::MIN;
        for frac in [0.01, 0.1, 0.25, 0.4, 0.45] {
            let hz = frac * low;
            let x = sine(source as f64, hz, len_in, 0.3);
            let y = grid(&x, source, project, N);
            let (amp, resid) = fit(&y, project as f64, hz);
            worst_pass = worst_pass.max(db(amp).abs());
            worst_noise = worst_noise.max(db(resid / amp));
        }
        // Stopband: whatever folds or images into the passband. Downsampling:
        // input tones from 0.55·fs_out (whose aliases land at or below
        // 0.45·fs_out) up to the input Nyquist. Upsampling: the image of a
        // passband tone at fs_in − f (at or above 0.55·fs_in).
        let mut worst_stop = f64::MIN;
        if source > project {
            let (lo, hi) = (0.55 * project as f64, 0.5 * source as f64);
            for frac in [0.0, 0.5, 0.95] {
                if lo >= hi {
                    break;
                }
                let hz = lo + frac * (hi - lo);
                let x = sine(source as f64, hz, len_in, 0.1);
                let y = grid(&x, source, project, N);
                worst_stop = worst_stop.max(db(rms(&y) * 2f64.sqrt()));
            }
        } else {
            for frac in [0.2, 0.4, 0.45] {
                let hz = frac * source as f64;
                let image = source as f64 - hz;
                let x = sine(source as f64, hz, len_in, 0.1);
                let y = grid(&x, source, project, N);
                // Remove the tone first: its leakage onto the image's basis
                // over a finite window would read as about −78 dB.
                let rest = remove(&y, project as f64, hz);
                let (img, _) = fit(&rest, project as f64, image.min(project as f64 - image));
                worst_stop = worst_stop.max(db(img));
            }
        }
        report.push(format!(
            "{source}->{project}: passband max |dev| {worst_pass:.4} dB to 0.45 fs, noise+distortion {worst_noise:.1} dB, stopband {}", stop_text(worst_stop)
        ));
        assert!(
            worst_pass <= 0.01,
            "{source}->{project}: passband deviation {worst_pass} dB"
        );
        assert!(
            worst_stop <= -120.0,
            "{source}->{project}: stopband {worst_stop} dB"
        );
        assert!(
            worst_noise <= -120.0,
            "{source}->{project}: noise {worst_noise} dB"
        );
    }
    for line in report {
        eprintln!("{line}");
    }
}

#[test]
fn free_positions_meet_the_bars() {
    // An irrational read rate (speed 1.0731 at 44.1 → 48 kHz), the cubic
    // phase interpolation's worst case, and decimating speeds.
    for step in [0.918_75 * 1.0731, 0.618_034, 1.5, 2.0] {
        let len_in = (N as f64 * step) as usize + 8192;
        let mut worst_pass = 0f64;
        let mut worst_noise = f64::MIN;
        let out_rate = 48_000.0;
        let in_rate = out_rate * step;
        let low = in_rate.min(out_rate);
        for frac in [0.05, 0.25, 0.45] {
            let hz = frac * low;
            let x = sine(in_rate, hz, len_in, 0.7);
            let y = free(&x, 4096.25, step, N);
            let (amp, resid) = fit(&y, out_rate, hz);
            worst_pass = worst_pass.max(db(amp).abs());
            worst_noise = worst_noise.max(db(resid / amp));
        }
        let mut worst_stop = f64::MIN;
        if step > 1.0 {
            let (lo, hi) = (0.55 * out_rate, 0.5 * in_rate);
            for frac in [0.0, 0.9] {
                let hz = lo + frac * (hi - lo);
                let x = sine(in_rate, hz, len_in, 0.2);
                let y = free(&x, 4096.25, step, N);
                worst_stop = worst_stop.max(db(rms(&y) * 2f64.sqrt()));
            }
        }
        eprintln!(
            "free step {step:.5}: passband max |dev| {worst_pass:.4} dB, noise+distortion {worst_noise:.1} dB, stopband {}",
            stop_text(worst_stop)
        );
        assert!(worst_pass <= 0.01, "step {step}: passband {worst_pass}");
        assert!(worst_noise <= -120.0, "step {step}: noise {worst_noise}");
        assert!(worst_stop <= -120.0, "step {step}: stopband {worst_stop}");
    }
}

/// The master-to-device conversion: rubato's sinc resampler with the same
/// filter, cubic phase interpolation.
#[test]
fn device_conversion_meets_the_bars() {
    for (project, device) in [(48_000u32, 44_100u32), (44_100, 48_000)] {
        let params = SincInterpolationParameters::new(kernel::TAPS, kernel::WINDOW)
            .oversampling_factor(256)
            .interpolation(SincInterpolationType::Cubic);
        let ratio = device as f64 / project as f64;
        let low = project.min(device) as f64;
        let mut worst_pass = 0f64;
        let mut worst_noise = f64::MIN;
        let mut worst_stop = f64::MIN;
        for (frac, stop) in [
            (0.05, false),
            (0.25, false),
            (0.45, false),
            (0.6, true),
            (0.45, true),
        ] {
            let mut rs =
                Async::<f32>::new_sinc(ratio, 1.0, &params, 1024, 1, FixedAsync::Input).unwrap();
            // Downsampling: a tone whose alias lands in the passband.
            // Upsampling: the image of a passband tone.
            let up = project < device;
            if stop && (up != (frac < 0.5)) {
                continue;
            }
            let hz = if stop && !up {
                frac * project as f64
            } else {
                frac * low
            };
            let x = sine(project as f64, hz.min(0.49 * project as f64), N + 8192, 0.4);
            let input = vec![x];
            let len = rs.process_all_needed_output_len(N + 8192);
            let mut out = vec![vec![0f32; len]];
            let a = SequentialSliceOfVecs::new(&input, 1, N + 8192).unwrap();
            let mut b = SequentialSliceOfVecs::new_mut(&mut out, 1, len).unwrap();
            let (_, written) = rs
                .process_all_into_buffer(&a, &mut b, N + 8192, None)
                .unwrap();
            let y = &out[0][4096..written - 4096];
            if stop && up {
                let image = project as f64 - hz;
                let rest = remove(y, device as f64, hz);
                let (img, _) = fit(&rest, device as f64, image.min(device as f64 - image));
                worst_stop = worst_stop.max(db(img));
            } else if stop {
                worst_stop = worst_stop.max(db(rms(y) * 2f64.sqrt()));
            } else {
                let (amp, resid) = fit(y, device as f64, hz);
                worst_pass = worst_pass.max(db(amp).abs());
                worst_noise = worst_noise.max(db(resid / amp));
            }
        }
        eprintln!(
            "rubato {project}->{device}: passband max |dev| {worst_pass:.4} dB, noise+distortion {worst_noise:.1} dB, stopband {}",
            stop_text(worst_stop)
        );
        assert!(
            worst_pass <= 0.01,
            "{project}->{device}: passband {worst_pass}"
        );
        assert!(
            worst_noise <= -120.0,
            "{project}->{device}: noise {worst_noise}"
        );
        assert!(
            worst_stop <= -120.0,
            "{project}->{device}: stopband {worst_stop}"
        );
    }
}
