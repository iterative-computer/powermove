//! Run explicitly on a Metal-enabled OIDN install; CPU-only builders skip.
#![cfg(feature = "oidn")]
use pm_studio::oidn::{Backend, Denoiser, DevicePreference, Quality};
fn metal() -> Option<Denoiser> {
    match Denoiser::new_with_device(DevicePreference::Metal) {
        Ok(d) => Some(d),
        Err(error) => {
            assert!(
                std::env::var_os("PM_STUDIO_REQUIRE_METAL").is_none(),
                "{error}"
            );
            eprintln!("SKIP Metal denoiser: {error}");
            None
        }
    }
}
#[test]
fn metal_cpu_picture_tolerance_rebinding_and_padded_channels() {
    let Some(gpu) = metal() else {
        return;
    };
    let cpu = Denoiser::new_with_device(DevicePreference::Cpu).unwrap();
    assert_eq!(
        Denoiser::new_with_device(DevicePreference::Auto)
            .unwrap()
            .backend(),
        Backend::Metal,
        "automatic selection did not prefer an available Metal device"
    );
    for (w, h, stride, half, quality) in [
        (97, 65, 4, false, Quality::High),
        (513, 257, 5, true, Quality::Balanced),
        (97, 65, 4, true, Quality::Fast),
    ] {
        let mut state = 19u32;
        let input: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                let base = if i % w < w / 2 { 0.3 } else { 1.2 };
                let mut p = vec![
                    base + 0.2 * noise,
                    base * 0.8 + 0.15 * noise,
                    base * 0.6 + 0.1 * noise,
                    0.25,
                ];
                if stride == 5 {
                    p.push(0.875);
                }
                p
            })
            .collect();
        let albedo: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = if i % w < w / 2 { 0.3 } else { 1.2 };
                let mut p = vec![v, v * 0.8, v * 0.6, 0.];
                if stride == 5 {
                    p.push(0.);
                }
                p
            })
            .collect();
        let normal: Vec<f32> = (0..w * h)
            .flat_map(|_| {
                let mut p = vec![0., 0., 1., 0.];
                if stride == 5 {
                    p.push(0.);
                }
                p
            })
            .collect();
        let mut a = input.clone();
        let mut b = input.clone();
        for (d, c) in [(&gpu, &mut a), (&cpu, &mut b)] {
            d.set_quality(quality);
            d.set_half_resolution_guides(half);
            d.run(c, &albedo, &normal, stride, w, h).unwrap();
        }
        assert_eq!(gpu.backend(), Backend::Metal);
        assert_eq!(gpu.timings().backend, Backend::Metal);
        assert_eq!(cpu.backend(), Backend::Cpu);
        let display = |v: f32| v.max(0.) / (1. + v.max(0.));
        let mut differences: Vec<f32> = a
            .chunks_exact(stride)
            .zip(b.chunks_exact(stride))
            .flat_map(|(a, b)| (0..3).map(move |i| (display(a[i]) - display(b[i])).abs()))
            .collect();
        let rms =
            (differences.iter().map(|d| d * d).sum::<f32>() / differences.len() as f32).sqrt();
        differences.sort_by(f32::total_cmp);
        let p99 = differences[differences.len() * 99 / 100];
        println!(
            "OIDN_DEVICE_TOLERANCE {w}x{h} half={half} rms={rms} p99={p99} metal={:?} cpu={:?}",
            gpu.timings(),
            cpu.timings()
        );
        assert!(rms < 0.003 && p99 < 0.01, "CPU/Metal picture mismatch");
        assert!(
            a.chunks_exact(stride)
                .all(|p| p[3] == 0.25 && (stride == 4 || p[4] == 0.875))
        );
    }
}
#[test]
fn cancelled_metal_frame_keeps_original_and_next_frame_renders() {
    use std::sync::atomic::AtomicU64;
    let Some(gpu) = metal() else {
        return;
    };
    let (w, h) = (96, 64);
    let input: Vec<f32> = (0..w * h)
        .flat_map(|i| [0.4 + 0.1 * (i % 3) as f32, 0.3, 0.2, 0.75])
        .collect();
    let albedo = vec![0.5; w * h * 4];
    let normal: Vec<f32> = (0..w * h).flat_map(|_| [0., 0., 1., 0.]).collect();
    let mut out = input.clone();
    let revision = AtomicU64::new(2);
    assert!(
        gpu.run_interruptible(&mut out, &albedo, &normal, 4, w, h, Some((&revision, 1)))
            .is_err()
    );
    assert_eq!(out, input);
    gpu.run(&mut out, &albedo, &normal, 4, w, h).unwrap();
    assert_eq!(gpu.backend(), Backend::Metal);
    let fresh = metal().unwrap();
    let mut expected = input;
    fresh.run(&mut expected, &albedo, &normal, 4, w, h).unwrap();
    assert!(out.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6));
}

#[test]
fn stable_multi_layer_metal_filters_match_fresh_filters_after_input_and_crop_changes() {
    let Some(reused) = metal() else { return };
    let (w, h, stride) = (768, 512, 4);
    let albedo = vec![0.5; w * h * stride];
    let normal: Vec<f32> = (0..w * h).flat_map(|_| [0., 0., 1., 0.]).collect();
    for phase in 0..3 {
        let right = 500 + if phase == 2 { 8 } else { 0 };
        let color: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let x = i % w;
                let y = i / w;
                if (220..right).contains(&x) && (160..350).contains(&y) {
                    [
                        0.5 + (i % 13) as f32 * 0.005 + phase as f32 * 0.03,
                        0.3,
                        0.2,
                        0.25,
                    ]
                } else {
                    [0., 0., 0., 0.25]
                }
            })
            .collect();
        let shade: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = 0.7 + ((i + phase) % 17) as f32 * 0.005;
                [v, v, v, 0.75]
            })
            .collect();
        let mut actual = [color.clone(), shade.clone()];
        let mut expected = [color, shade];
        let fresh = metal().unwrap();
        for (d, images) in [(&reused, &mut actual), (&fresh, &mut expected)] {
            d.set_quality(Quality::Balanced);
            d.set_half_resolution_guides(true);
            let (a, b) = images.split_at_mut(1);
            d.run_layers_interruptible(
                &mut [&mut a[0], &mut b[0]],
                &albedo,
                &normal,
                stride,
                w,
                h,
                None,
            )
            .unwrap();
        }
        for (layer, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            let max = actual
                .iter()
                .zip(expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(
                max < 1e-6,
                "stale layer/filter binding, phase={phase} layer={layer}: {max}"
            );
            assert!(
                actual
                    .chunks_exact(stride)
                    .all(|p| p[3] == if layer == 0 { 0.25 } else { 0.75 })
            );
        }
        assert_eq!(reused.backend(), Backend::Metal);
    }
}
