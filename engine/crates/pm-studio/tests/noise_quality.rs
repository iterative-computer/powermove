//! Numeric image acceptance against freshly rendered high-sample references.
#![cfg(feature = "oidn")]
#[path = "support/quality_scenes.rs"]
mod scenes;
use pm_studio::{
    environment::AdaptiveSampling,
    gpu::Renderer,
    oidn::{Denoiser, Quality},
};
use std::time::Instant;

fn metrics(actual: &[u8], reference: &[u8], w: usize, h: usize) -> (f64, f64, f64, usize) {
    let luma =
        |p: &[u8]| (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) / 255.;
    let r: Vec<_> = reference.chunks_exact(4).map(luma).collect();
    let error: Vec<_> = actual
        .chunks_exact(4)
        .zip(&r)
        .map(|(a, r)| luma(a) - r)
        .collect();
    let mut smooth = vec![false; w * h];
    let mut squared = 0.;
    let mut noise = 0.;
    let mut n = 0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if reference[i * 4 + 3] < 250 {
                continue;
            }
            squared += error[i] * error[i];
            let neighbors = [i - 1, i + 1, i - w, i + w];
            if neighbors
                .iter()
                .all(|j| reference[j * 4 + 3] >= 250 && (r[i] - r[*j]).abs() < 0.025)
            {
                smooth[i] = true;
                let mean = neighbors.iter().map(|j| error[*j]).sum::<f64>() / 4.;
                noise += (error[i] - mean).powi(2);
                n += 1;
            }
        }
    }
    let mut biases = vec![];
    for y in (0..h - 7).step_by(8) {
        for x in (0..w - 7).step_by(8) {
            let ids: Vec<_> = (0..8)
                .flat_map(|dy| (0..8).map(move |dx| (y + dy) * w + x + dx))
                .filter(|i| smooth[*i])
                .collect();
            if ids.len() >= 24 {
                biases.push((ids.iter().map(|i| error[*i]).sum::<f64>() / ids.len() as f64).abs());
            }
        }
    }
    biases.sort_by(f64::total_cmp);
    let valid = reference
        .chunks_exact(4)
        .filter(|p| p[3] >= 250)
        .count()
        .max(1);
    (
        (squared / valid as f64).sqrt(),
        (noise / n.max(1) as f64).sqrt(),
        biases
            .get((biases.len() * 95 / 100).min(biases.len().saturating_sub(1)))
            .copied()
            .unwrap_or(1.),
        n,
    )
}
fn save(name: &str, w: u32, h: u32, pixels: &[u8]) {
    let Some(root) = std::env::var_os("PM_STUDIO_NOISE_OUT") else {
        return;
    };
    std::fs::create_dir_all(&root).unwrap();
    let file =
        std::fs::File::create(std::path::Path::new(&root).join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(file, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}
fn check_noise_at(names: &[&str], w:u32, h:u32) {
    let mut renderer = Renderer::new(w, h).unwrap();
    let denoiser = Denoiser::new().unwrap();
    let mut failures = vec![];
    for &name in names {
        let scene = if name == "chrome-set" {
            // The owner's product stress set includes glass, lacquer and the
            // shadow catcher; a chrome sphere alone misses those caustics.
            pm_studio::scene::preset("chrome", "starlight", 0.).unwrap()
        } else {
            scenes::product(name)
        };
        scene.validate().unwrap();
        // A camera seed alone does not change the fixed precomputed photon
        // map. Rotate the physically identical emitters so each emitter uses
        // a disjoint set of photon IDs in the reference. Otherwise shared
        // baked caustic grain can incorrectly score as zero error.
        let mut reference_scene = scene.clone();
        assert!(reference_scene.lights.len() > 1);
        reference_scene.lights.rotate_left(1);
        renderer.set_scene(&reference_scene, &reference_scene.prepare());
        let backdrop = (name == "chrome-set").then(|| scene.backdrop.to_array());
        renderer.set_void(backdrop);
        renderer.set_precomputed_caustics(true);
        renderer.set_adaptive_sampling(AdaptiveSampling {
            enabled: false,
            ..Default::default()
        });
        renderer.set_time_seed(7);
        let reference = reference(&mut renderer, &reference_scene, name, w, h);
        save(&format!("{name}-reference4096"), w, h, &reference);
        renderer.set_scene(&scene, &scene.prepare());
        renderer.set_void(backdrop);
        for (quality, max_samples, min_samples, threshold) in
            [("production", 256, 32, 0.03), ("final", 1024, 512, 0.015)]
        {
            denoiser.set_quality(if quality == "final" {
                Quality::High
            } else {
                Quality::Balanced
            });
            denoiser.set_half_resolution_guides(quality != "final");
            renderer.set_time_seed(0);
            renderer.set_adaptive_sampling_with_floor(
                AdaptiveSampling {
                    enabled: true,
                    min_samples,
                    relative_error: threshold,
                    absolute_error: 0.001,
                },
                true,
            );
            let start = Instant::now();
            renderer.trace(max_samples, 8, true, 0);
            let actual = renderer
                .finish_with_layers(&|color, shade, a, n| {
                    denoiser.run_layers_interruptible(
                        &mut [color, shade],
                        a,
                        n,
                        4,
                        w as usize,
                        h as usize,
                        None,
                    )
                })
                .0;
            let counts = renderer.sample_counts();
            let (rms, noise, bias, pixels) = metrics(&actual, &reference, w as usize, h as usize);
            println!(
                "NOISE_GATE {}",
                serde_json::json!({"scene":name,"policy":quality,"width":w,"height":h,"reference_spp":4096,"sample_min":counts.iter().copied().fold(f32::INFINITY,f32::min),"sample_max":counts.iter().copied().fold(0.,f32::max),"luma_rms":rms,"smooth_noise_rms":noise,"block_bias_p95":bias,"smooth_pixels":pixels,"frame_ms":start.elapsed().as_secs_f64()*1000.})
            );
            save(&format!("{name}-{quality}"), w, h, &actual);
            // Normalized sRGB code-value limits catch speckle and low-frequency
            // OIDN blotches in both policies. Neither has a wall-clock deadline.
            if !(rms < 0.02 && noise < 0.006 && bias < 0.012 && pixels > 100)
            {
                failures.push(format!(
                    "{name}/{quality}: RMS {rms:.5}, noise {noise:.5}, block bias {bias:.5}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Production/Final noise acceptance: {}",
        failures.join("; ")
    );
}
#[test]
fn teapot_glass_and_chrome_production_and_final_have_bounded_noise_and_blotches() {
    check_noise_at(&["teapot", "glass", "chrome"],192,128);
}
#[test]
fn chrome_glass_studio_caustics_match_independent_photon_references() {
    check_noise_at(&["chrome-set"],192,128);
}
#[test]
fn matte_enamel_uses_authored_perceptual_roughness_without_a_mirror_coat() {
    let scene = scenes::product("teapot");
    let prepared = scene.prepare();
    assert_eq!(scene.materials[0].roughness, 0.7);
    assert_eq!(prepared.materials[0].spec[0], 0.7);
    assert_eq!(prepared.materials[0].spec[1], 0.);
}

// Expensive, explicit acceptance gate. Keep the exact small-fixture thresholds;
// resolution increases coverage, rather than relaxing what counts as noise.
#[test]
#[ignore = "1080p/4096-spp reference acceptance; run explicitly under the build lock"]
fn full_hd_product_noise_acceptance() {
    let selected=std::env::var("PM_STUDIO_NOISE_SCENES").unwrap_or_else(|_|"teapot,glass,chrome,chrome-set".into());
    let names:Vec<_>=selected.split(',').collect();
    assert!(!names.is_empty() && names.iter().all(|n|["teapot","glass","chrome","chrome-set"].contains(n)),"unknown full-HD fixture");
    check_noise_at(&names,1920,1080);
}
fn fingerprint(bytes:&[u8])->String {
    format!("{:016x}",bytes.iter().fold(0xcbf29ce484222325u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100000001b3)))
}
// References may be held fixed across implementation changes: their physical
// scene, sampling policy and bytes are checked, and provenance stays visible.
// A camera seed plus rotated emitters gives independent paths AND photon IDs.
fn reference(r:&mut Renderer, scene:&pm_studio::scene::Scene,name:&str,w:u32,h:u32)->Vec<u8> {
    let key=serde_json::json!({"scene":fingerprint(&serde_json::to_vec(scene).unwrap()),"width":w,"height":h,"spp":4096,"seed":7,"rotated_lights":true,"filter":"blackman-harris-1.5","denoise":false});
    let cache=std::env::var_os("PM_STUDIO_NOISE_REFERENCES").map(std::path::PathBuf::from);
    let stem=format!("{name}-{w}x{h}-4096");
    if let Some(root)=&cache {
        let metadata=root.join(format!("{stem}.json"));let png=root.join(format!("{stem}.png"));
        if metadata.exists() || png.exists() {
            let metadata:serde_json::Value=serde_json::from_slice(&std::fs::read(metadata).unwrap()).unwrap();
            assert_eq!(metadata["key"],key,"stale physical reference; use a new cache directory");
            let pixels=image::open(png).unwrap().into_rgba8();assert_eq!(pixels.dimensions(),(w,h));
            assert_eq!(metadata["pixels"],fingerprint(pixels.as_raw()),"corrupt reference");
            println!("NOISE_REFERENCE {}",serde_json::json!({"scene":name,"cached":true,"provenance":metadata["provenance"]}));
            return pixels.into_raw();
        }
    }
    let start=Instant::now();
    r.trace_with_progress(4096,8,true,0,&|done,target| {
        if done%256==0 {eprintln!("REFERENCE_PROGRESS {name} {done}/{target} {:.1}s",start.elapsed().as_secs_f64());}true
    });let pixels=r.finish(0).0;
    if let Some(root)=cache {
        std::fs::create_dir_all(&root).unwrap();
        pm_studio::gpu::write_png(&root.join(format!("{stem}.png")),w,h,&pixels).unwrap();
        let provenance=std::env::var("PM_STUDIO_REFERENCE_REVISION").unwrap_or_else(|_|"unrecorded; generated by this test run".into());
        let metadata=serde_json::json!({"key":key,"pixels":fingerprint(&pixels),"provenance":provenance});
        std::fs::write(root.join(format!("{stem}.json")),serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
    }
    println!("NOISE_REFERENCE {}",serde_json::json!({"scene":name,"cached":false}));pixels
}
