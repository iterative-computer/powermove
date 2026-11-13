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
    metrics_with_guide(actual, reference, reference, w, h)
}
// Transport returns premultiplied display RGB. Check every pixel on both
// opaque display backgrounds as well as alpha: the opaque-surface noise mask
// alone cannot detect a changed void or broken silhouette coverage.
fn display_coverage(actual: &[u8], reference: &[u8]) -> [f64; 3] {
    assert_eq!(actual.len(), reference.len());
    let mut error = [0.; 3];
    for (a, r) in actual.chunks_exact(4).zip(reference.chunks_exact(4)) {
        let luma = |p: &[u8]| (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) / 255.;
        let black = luma(a) - luma(r);
        let alpha = (a[3] as f64 - r[3] as f64) / 255.;
        error[0] += black * black;
        error[1] += (black - alpha).powi(2);
        error[2] += alpha * alpha;
    }
    error.map(|e| (e / (actual.len() / 4) as f64).sqrt())
}

#[test]
fn whole_frame_checks_include_transparent_background_and_edge_coverage() {
    let (w, h) = (32, 32);
    let mut reference = vec![0u8; w * h * 4];
    for y in 8..24 { for x in 8..24 {
        reference[(y*w+x)*4..(y*w+x)*4+4].copy_from_slice(&[128,128,128,255]);
    }}
    assert_eq!(display_coverage(&reference, &reference), [0.; 3]);
    for color in [0,255] {
        let mut actual = reference.clone();
        for p in actual.chunks_exact_mut(4) { if p[3] == 0 { p.copy_from_slice(&[color,color,color,255]); } }
        assert_eq!(metrics(&actual, &reference, w, h).0, 0.);
        assert!(display_coverage(&actual, &reference).iter().any(|e| *e > 0.02));
    }
}
fn metrics_with_guide(actual: &[u8], reference: &[u8], guide: &[u8], w: usize, h: usize) -> (f64,f64,f64,usize) {
    let luma =
        |p: &[u8]| (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) / 255.;
    let r: Vec<_> = reference.chunks_exact(4).map(luma).collect();
    let error: Vec<_> = actual
        .chunks_exact(4)
        .zip(&r)
        .map(|(a, r)| luma(a) - r)
        .collect();
    let g:Vec<_>=guide.chunks_exact(4).map(luma).collect();
    let mut smooth = vec![false; w * h];
    let mut squared = 0.;
    let mut noise = 0.;
    let mut n = 0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if guide[i * 4 + 3] < 250 {
                continue;
            }
            squared += error[i] * error[i];
            let neighbors = [i - 1, i + 1, i - w, i + w];
            if neighbors
                .iter()
                .all(|j| guide[j * 4 + 3] >= 250 && (g[i] - g[*j]).abs() < 0.025)
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
    let valid = guide
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
fn flicker_difference(actual:&[u8],second:&[u8],guide:&[u8],w:usize,h:usize)->(f64,usize) {
    let luma=|p:&[u8]|(0.2126*p[0] as f64+0.7152*p[1] as f64+0.0722*p[2] as f64)/255.;
    let mut sum=0.;let mut count=0;
    for y in 1..h-1 {for x in 1..w-1 {
        let i=y*w+x;let g=&guide[4*i..4*i+4];
        if g[3]<250 {continue;}
        if [i-1,i+1,i-w,i+w].iter().any(|j|guide[4*j+3]<250 || (luma(g)-luma(&guide[4*j..4*j+4])).abs()>=0.025){continue;}
        let delta=luma(&actual[4*i..4*i+4])-luma(&second[4*i..4*i+4]);sum+=delta*delta;count+=1;
    }}
    ((sum/count.max(1) as f64).sqrt(),count)
}
#[test]
fn flicker_detects_smooth_boil_not_only_high_frequency_speckle() {
    let a=vec![128u8;32*32*4];let mut guide=a.clone();let mut b=a.clone();
    for p in guide.chunks_exact_mut(4){p[3]=255;}
    for p in b.chunks_exact_mut(4){p[..3].fill(132);}
    assert!(flicker_difference(&a,&a,&guide,32,32).0<1e-12);
    assert!(flicker_difference(&a,&b,&guide,32,32).0>0.006);
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
fn save_display_proofs(name: &str, w: u32, h: u32, pixels: &[u8]) {
    if std::env::var_os("PM_STUDIO_NOISE_OUT").is_none() { return; }
    for (label, background) in [("over-black",0u16),("over-white",255u16)] {
        let mut opaque = pixels.to_vec();
        for p in opaque.chunks_exact_mut(4) {
            let empty = 255 - u16::from(p[3]);
            for c in &mut p[..3] { *c = (u16::from(*c) + (background*empty+127)/255).min(255) as u8; }
            p[3] = 255;
        }
        save(&format!("{name}-{label}"),w,h,&opaque);
    }
}
fn check_noise_at(names: &[&str], w:u32, h:u32) {
    let mut renderer = Renderer::new(w, h).unwrap();
    let denoiser = Denoiser::new().unwrap();
    let mut failures = vec![];
    let policies=std::env::var("PM_STUDIO_NOISE_POLICIES").unwrap_or_else(|_|"production,final".into());
    let policies:Vec<_>=policies.split(',').collect();
    assert!(policies.iter().all(|p|["production","final"].contains(p)));
    let baseline=std::env::var_os("PM_STUDIO_NOISE_BASELINE").is_some();
    for &name in names {
        let scene = if name == "bottle" {
            let input=std::env::var("PM_STUDIO_BOTTLE_SCENE").expect("set PM_STUDIO_BOTTLE_SCENE to studio-fixtures product-shot/scene.json");
            serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap()
        } else if name == "chrome-set" {
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
        renderer.sampling_policy=Default::default();
        renderer.set_scene(&reference_scene, &reference_scene.prepare());
        let backdrop = matches!(name,"chrome-set"|"bottle").then(|| scene.backdrop.to_array());
        renderer.set_void(backdrop);
        renderer.set_precomputed_caustics(true);
        renderer.set_adaptive_sampling(AdaptiveSampling {
            enabled: false,
            ..Default::default()
        });
        renderer.set_time_seed(7);
        let reference = reference(&mut renderer, &reference_scene, name, w, h);
        save(&format!("{name}-reference4096"), w, h, &reference);
        save_display_proofs(&format!("{name}-reference4096"),w,h,&reference);
        renderer.set_scene(&scene, &scene.prepare());
        renderer.set_void(backdrop);
        for (quality, max_samples, min_samples, threshold) in
            [("production", if baseline {256}else{128}, if baseline {32}else{8}, if baseline{0.03}else{0.12}), ("final", 1024, 512, 0.015)]
        {
            if !policies.contains(&quality){continue;}
            let denoise_quality=if quality=="final" {"high".to_owned()}else{std::env::var("PM_STUDIO_PRODUCTION_DENOISE").unwrap_or_else(|_|"balanced".into())};
            denoiser.set_quality(match denoise_quality.as_str(){"high"=>Quality::High,"balanced"=>Quality::Balanced,"fast"=>Quality::Fast,_=>panic!("unknown denoising quality")});
            denoiser.set_half_resolution_guides(quality != "final");
            renderer.sampling_policy=if quality=="production" && !baseline {pm_studio::sampling::SamplingPolicy::PRODUCTION}else{Default::default()};
            if quality=="production" && !baseline {
                for (name,value) in [("PM_STUDIO_PRODUCTION_GLOSSY",&mut renderer.sampling_policy.glossy_bounces),("PM_STUDIO_PRODUCTION_DIFFUSE",&mut renderer.sampling_policy.diffuse_bounces),("PM_STUDIO_PRODUCTION_TRANSMISSION",&mut renderer.sampling_policy.transmission_bounces)] {
                    if let Ok(override_value)=std::env::var(name){*value=override_value.parse().unwrap();}
                }
                renderer.sampling_policy.validate().unwrap();
            }
            let min_samples=if quality=="production" {std::env::var("PM_STUDIO_PRODUCTION_MIN").ok().map(|v|v.parse::<u32>().unwrap()).unwrap_or(min_samples)}else{min_samples};
            let threshold=if quality=="production" {std::env::var("PM_STUDIO_PRODUCTION_NOISE").ok().map(|v|v.parse::<f32>().unwrap()).unwrap_or(threshold)}else{threshold};
            assert!(min_samples>0 && threshold.is_finite() && (0. ..=1.).contains(&threshold));
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
            // Trial sampling parameters are explicit; acceptance limits and
            // independent Monte Carlo references stay fixed.
            let max_samples=if quality=="production" {std::env::var("PM_STUDIO_PRODUCTION_SPP").ok().map(|v|v.parse::<u32>().unwrap()).unwrap_or(max_samples)}else{max_samples};
            assert!(max_samples>=min_samples && max_samples<=4096);
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
            let ids=renderer.first_hit_ids();
            let frame_ms=start.elapsed().as_secs_f64()*1000.;
            let pipeline=renderer.timings;
            renderer.set_time_seed(19);
            renderer.trace(max_samples,8,true,0);
            let second=renderer.finish_with_layers(&|color,shade,a,n|{
                denoiser.run_layers_interruptible(&mut [color,shade],a,n,4,w as usize,h as usize,None)
            }).0;
            renderer.set_time_seed(0);
            let flicker=flicker_difference(&actual,&second,&reference,w as usize,h as usize);
            let mut roi=reference.clone();
            for (p,id) in roi.chunks_exact_mut(4).zip(ids.iter().copied()) {
                if id==0 || (!matches!(name,"chrome-set"|"bottle") && id>=scene.objects.len() as u32){p[3]=0;}
            }
            let flicker_roi=flicker_difference(&actual,&second,&roi,w as usize,h as usize);
            println!("FLICKER_GATE {}",serde_json::json!({"scene":name,"policy":quality,"width":w,"height":h,"seeds":[0,19],"whole":flicker,"object":flicker_roi,"smooth_difference_rms_limit":0.006}));
            if std::env::var_os("PM_STUDIO_NOISE_OUT").is_some() {
                // Diagnose the source of boiling without changing the gate or
                // its independent smooth-region mask. ID zero is the void / catcher.
                let mut regions=vec![];
                for id in ids.iter().copied().collect::<std::collections::BTreeSet<_>>() {
                    let mut guide=reference.clone();let mut spp=0f64;let mut count=0usize;
                    for ((pixel,&hit),&samples) in guide.chunks_exact_mut(4).zip(&ids).zip(&counts) {
                        if hit!=id {pixel[3]=0;} else {spp+=samples as f64;count+=1;}
                    }
                    let (rms,smooth)=flicker_difference(&actual,&second,&guide,w as usize,h as usize);
                    regions.push(serde_json::json!({"id":id,"difference_rms":rms,"smooth_pixels":smooth,"mean_spp":spp/count.max(1) as f64}));
                }
                println!("FLICKER_REGIONS {}",serde_json::json!({"scene":name,"policy":quality,"regions":regions}));
                save(&format!("{name}-{quality}-seed19"),w,h,&second);
            }
            if ![flicker,flicker_roi].iter().all(|&(r,n)|r<0.006&&n>100){
                failures.push(format!("{name}/{quality} flicker whole={flicker:?} object={flicker_roi:?}"));
            }
            let object_metrics=if w>=1920 {
                let mut masked=reference.clone();
                for (pixel,id) in masked.chunks_exact_mut(4).zip(ids.iter().copied()) {
                    if id==0 || (!matches!(name,"chrome-set"|"bottle") && id>=scene.objects.len() as u32){pixel[3]=0;}
                }
                Some(metrics(&actual,&masked,w as usize,h as usize))
            }else{None};
            let (rms, noise, bias, pixels) = metrics(&actual, &reference, w as usize, h as usize);
            let coverage=display_coverage(&actual,&reference);
            println!("DISPLAY_COVERAGE_GATE {}",serde_json::json!({"scene":name,"policy":quality,"over_black_rms":coverage[0],"over_white_rms":coverage[1],"alpha_rms":coverage[2],"limit":0.02,"transparent_reference_pixels":reference.chunks_exact(4).filter(|p|p[3]==0).count()}));
            if coverage.iter().any(|v|!v.is_finite() || *v>=0.02) {
                failures.push(format!("{name}/{quality} whole-display RGB/alpha mismatch {coverage:?}"));
            }
            println!(
                "NOISE_GATE {}",
                serde_json::json!({"scene":name,"policy":quality,"width":w,"height":h,"reference_spp":4096,"sample_ceiling":max_samples,"denoise_quality":denoise_quality,"sampling":renderer.sampling_policy,"requested_floor":min_samples,"relative_error":threshold,"mean_spp":counts.iter().map(|&n|n as f64).sum::<f64>()/counts.len() as f64,"pipeline":pipeline,"object_metrics":object_metrics.map(|(rms,noise,bias,n)|serde_json::json!({"luma_rms":rms,"smooth_noise_rms":noise,"block_bias_p95":bias,"smooth_pixels":n})),"sample_min":counts.iter().copied().fold(f32::INFINITY,f32::min),"sample_max":counts.iter().copied().fold(0.,f32::max),"luma_rms":rms,"smooth_noise_rms":noise,"block_bias_p95":bias,"smooth_pixels":pixels,"frame_ms":frame_ms})
            );
            save(&format!("{name}-{quality}"), w, h, &actual);
            save_display_proofs(&format!("{name}-{quality}"),w,h,&actual);
            // Normalized sRGB code-value limits catch speckle and low-frequency
            // OIDN blotches in both policies. Neither has a wall-clock deadline.
            if !(rms < 0.02 && noise < 0.006 && bias < 0.012 && pixels > 100) || object_metrics.is_some_and(|(r,n,b,p)|!(r<0.02 && n<0.006 && b<0.012 && p>100))
            {
                failures.push(format!(
                    "{name}/{quality}: RMS {rms:.5}, noise {noise:.5}, block bias {bias:.5}; object {object_metrics:?}; ceiling {max_samples}"
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

#[test]
fn teapot_flat_lid_has_flat_shading_normals_and_a_smooth_body_profile() {
    let scene=scenes::product("teapot");
    let body=&scene.objects[0].mesh;
    let lid:Vec<_>=body.positions.iter().zip(&body.normals).filter(|(p,_)|
        (p.y-0.7).abs()<1e-6 && p.x*p.x+p.z*p.z>0.16*0.16).collect();
    assert!(lid.len()>=96);
    assert!(lid.iter().all(|(_,n)|n.y>0.9999),"flat lid inherited the shoulder's sloping normals");
    let rows=body.positions.len()/97;
    assert!(rows>32,"a handful of straight profile segments leaves a polygonal silhouette");
}

// Expensive, explicit acceptance gate. Keep the exact small-fixture thresholds;
// resolution increases coverage, rather than relaxing what counts as noise.
#[test]
#[ignore = "1080p/4096-spp reference acceptance; run explicitly under the build lock"]
fn full_hd_product_noise_acceptance() {
    let selected=std::env::var("PM_STUDIO_NOISE_SCENES").unwrap_or_else(|_|"teapot,glass,chrome,chrome-set".into());
    let names:Vec<_>=selected.split(',').collect();
    assert!(!names.is_empty() && names.iter().all(|n|["teapot","glass","chrome","chrome-set","bottle"].contains(n)),"unknown full-HD fixture");
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
