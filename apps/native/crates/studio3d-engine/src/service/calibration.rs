//! Worker-only, interruptible calibration using the actual export sampling rule.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Probe {
    request: SceneRequest,
    prepared: PreparedScene,
    projected_size: (u32, u32),
    target: u32,
    bounces: u32,
    done: u32,
    trace_ms: f64,
    photon_ms: f64,
    prepare_ms: f64,
}
#[derive(Clone, Debug, Default)]
pub struct Measurement {
    pub samples: u32,
    pub average_samples: f64,
    pub cap_fraction: f64,
    pub trace_ms: f64,
    pub photon_ms: f64,
    pub prepare_ms: f64,
    pub guides_ms: f64,
    pub denoise_ms: f64,
    pub beauty_pixels: u64,
    pub projected_beauty_pixels: f64,
    pub beauty_layers: f64,
}
impl StudioService {
    pub fn begin_calibration(
        &mut self,
        source: &dyn SceneSource,
        request: SceneRequest,
        samples: u32,
        projected_size: (u32, u32),
    ) -> Result<Probe, String> {
        let start = std::time::Instant::now();
        let prepared = self.prepare_scene(
            source,
            request.clone(),
            Tier::Final {
                samples,
                salt: 0,
                denoise: Denoise::Oidn,
            },
        )?;
        if prepared.outputs.is_empty() {
            return Err("composition has no Studio contribution to estimate".into());
        }
        let q = prepared
            .quality
            .ok_or("export calibration needs an explicit sample policy")?;
        let r = self.renderer.as_mut().unwrap();
        r.live = false;
        r.sampling_policy = q.sampling;
        r.caustics_enabled = q.caustics;
        r.set_precomputed_caustics(q.caustics);
        r.set_time_seed(0);
        r.set_adaptive_sampling_with_floor(
            crate::environment::AdaptiveSampling {
                enabled: q.noise_threshold > 0.,
                min_samples: q.min_samples,
                relative_error: q.noise_threshold,
                absolute_error: 0.001,
            },
            true,
        );
        Ok(Probe {
            request,
            prepared,
            projected_size,
            target: samples.min(q.max_samples),
            bounces: q.max_bounces,
            done: 0,
            trace_ms: 0.,
            photon_ms: 0.,
            prepare_ms: start.elapsed().as_secs_f64() * 1000.,
        })
    }
    /// One four-sample pass, then return to the worker's input loop. No preview
    /// deadline replaces the saved preset's convergence target.
    pub fn advance_calibration(
        &mut self,
        p: &mut Probe,
        cancel: &AtomicU64,
        epoch: u64,
    ) -> Result<Option<Measurement>, String> {
        if cancel.load(Ordering::Relaxed) != epoch {
            return Err("export estimate cancelled by input".into());
        }
        let r = self.renderer.as_mut().unwrap();
        let n = (p.target - p.done).min(4);
        p.trace_ms += r.trace(n, p.bounces, p.done == 0, 64);
        p.done += n;
        p.photon_ms += r.timings.photon_setup_ms;
        let quiet = p.done % 32 == 0 && r.current_max_samples() <= p.done.saturating_sub(4);
        if p.done < p.target && !quiet {
            return Ok(None);
        }
        if cancel.load(Ordering::Relaxed) != epoch {
            return Err("export estimate cancelled by input".into());
        }
        let counts = r.sample_counts();
        let mut m = Measurement {
            samples: p.done,
            average_samples: counts.iter().map(|&n| f64::from(n)).sum::<f64>()
                / counts.len().max(1) as f64,
            cap_fraction: counts.iter().filter(|&&n| n >= p.target as f32).count() as f64
                / counts.len().max(1) as f64,
            trace_ms: (p.trace_ms - p.photon_ms).max(0.),
            photon_ms: p.photon_ms,
            prepare_ms: p.prepare_ms,
            ..Default::default()
        };
        r.set_output_mask(None, false);
        #[cfg(feature = "oidn")]
        if p.prepared.denoise == Denoise::Oidn {
            DENOISER.with(|slot| -> Result<(), String> {
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = Some(crate::oidn::Denoiser::new()?);
                }
                let d = slot.as_ref().unwrap();
                let half = p.prepared.quality.unwrap().half_resolution_guides;
                d.set_half_resolution_guides(half);
                d.set_crop(true);
                d.set_quality(if half {
                    crate::oidn::Quality::Balanced
                } else {
                    crate::oidn::Quality::High
                });
                let catcher = r.has_shadow_catcher();
                let projected = std::cell::Cell::new((0., 0.));
                r.prepare_frame(
                    0,
                    Some(&|color, shade, albedo, normal| {
                        let mut layers = vec![color];
                        if catcher {
                            layers.push(shade)
                        }
                        let areas: Vec<_> = layers
                            .iter()
                            .map(|color| {
                                projected_beauty_area(
                                    color,
                                    p.request.width,
                                    p.request.height,
                                    p.projected_size,
                                    half,
                                )
                            })
                            .filter(|area| *area > 0.)
                            .collect();
                        projected.set((areas.iter().sum(), areas.len() as f64));
                        d.run_layers_interruptible(
                            &mut layers,
                            albedo,
                            normal,
                            4,
                            p.request.width as usize,
                            p.request.height as usize,
                            Some((cancel, epoch)),
                        )
                    }),
                )?;
                m.beauty_pixels = d.timings().beauty_pixels;
                (m.projected_beauty_pixels, m.beauty_layers) = projected.get();
                Ok(())
            })?;
        } else {
            r.prepare_frame(
                if p.prepared.denoise == Denoise::Guided {
                    4
                } else {
                    0
                },
                None,
            )?;
        }
        #[cfg(not(feature = "oidn"))]
        r.prepare_frame(0, None)?;
        m.guides_ms = r.timings.guides_ms + r.timings.guide_readback_ms;
        m.denoise_ms = r.timings.denoise_ms;
        self.resource_start = p.prepared.resource_start;
        self.previous = Some((p.request.clone(), p.prepared.key.clone()));
        self.previous_tier = Some(Tier::Final {
            samples: p.target,
            salt: 0,
            denoise: Denoise::Oidn,
        });
        Ok(Some(m))
    }
}

// Match OIDN's physical crop rule at the *delivery render size*. The 64-pixel
// context halo does not grow with resolution, and sub-300k calibration images
// deliberately do not crop; scaling their beauty pixel count would overcharge
// full-HD scenes with a small isolated product.
fn projected_beauty_area(color: &[f32], w: u32, h: u32, target: (u32, u32), crop: bool) -> f64 {
    let constant = &color[..3];
    if constant.iter().all(|v| v.is_finite()) && color.chunks_exact(4).all(|p| p[..3] == *constant)
    {
        return 0.;
    }
    let (tw, th) = target;
    let full = f64::from(tw) * f64::from(th);
    if !crop || full < 300_000. || color[..3] != [0.; 3] {
        return full;
    }
    let (mut lo, mut hi) = ((w, h), (0, 0));
    for (i, p) in color.chunks_exact(4).enumerate() {
        if p[..3] != [0.; 3] {
            let (x, y) = (i as u32 % w, i as u32 / w);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x + 1), hi.1.max(y + 1));
        }
    }
    if hi == (0, 0) {
        return full;
    }
    let x0 = (u64::from(lo.0) * u64::from(tw) / u64::from(w)).saturating_sub(64);
    let y0 = (u64::from(lo.1) * u64::from(th) / u64::from(h)).saturating_sub(64);
    let x1 = ((u64::from(hi.0) * u64::from(tw)).div_ceil(u64::from(w)) + 64).min(u64::from(tw));
    let y1 = ((u64::from(hi.1) * u64::from(th)).div_ceil(u64::from(h)) + 64).min(u64::from(th));
    let area = ((x1 - x0) * (y1 - y0)) as f64;
    if area > full * 0.8 { full } else { area }
}

/// Affine denoising costs are device/backend/quality specific and cached only
/// on their worker. Guide and beauty costs are fitted separately; beauty work
/// can cover several cropped contributions in one composition.
#[derive(Clone, Copy, Debug, Default)]
pub struct DenoiseModel {
    pub guide_fixed: f64,
    pub guide_pixel: f64,
    pub beauty_fixed: f64,
    pub beauty_pixel: f64,
}
impl DenoiseModel {
    pub fn predict(self, pixels: f64, beauty_pixels: f64, layers: f64) -> f64 {
        self.guide_fixed
            + self.guide_pixel * pixels
            + self.beauty_fixed * layers
            + self.beauty_pixel * beauty_pixels
    }
}
pub struct DenoiseProbe {
    half: bool,
    stage: usize,
    measurements: Vec<(f64, f64, f64)>,
}
#[cfg(feature = "oidn")]
thread_local! {static MODELS:std::cell::RefCell<[Option<DenoiseModel>;2]>=const{std::cell::RefCell::new([None,None])};}
impl DenoiseProbe {
    pub fn new(half: bool) -> Self {
        Self {
            half,
            stage: 0,
            measurements: vec![],
        }
    }
    /// Warm and measure each size on separate idle ticks; progress callbacks
    /// interrupt OIDN if editing resumes. Never extrapolate tiny-image timings
    /// by area or change the actual exported image.
    #[cfg(feature = "oidn")]
    pub fn advance(
        &mut self,
        cancel: &AtomicU64,
        epoch: u64,
    ) -> Result<Option<DenoiseModel>, String> {
        if cancel.load(Ordering::Relaxed) != epoch {
            return Err("export estimate cancelled by input".into());
        }
        // The DENOISER and this cache share one worker/session and therefore
        // one selected device/backend. Never share CPU coefficients with Metal.
        if let Some(model) = MODELS.with(|m| m.borrow()[usize::from(self.half)]) {
            return Ok(Some(model));
        }
        let (w, h) = if self.stage < 2 {
            (480, 270)
        } else {
            (960, 540)
        };
        let mut color = vec![0.; w * h * 4];
        let mut albedo = color.clone();
        let mut normal = color.clone();
        for (i, p) in color.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[0.3 + (i % 97) as f32 / 970., 0.2, 0.1, 1.]);
        }
        for p in albedo.chunks_exact_mut(4) {
            p.copy_from_slice(&[0.3, 0.3, 0.3, 1.]);
        }
        for p in normal.chunks_exact_mut(4) {
            p.copy_from_slice(&[0., 0., 1., 1.]);
        }
        let t = DENOISER.with(|slot| -> Result<crate::oidn::FilterTimings, String> {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(crate::oidn::Denoiser::new()?)
            }
            let d = slot.as_ref().unwrap();
            d.set_half_resolution_guides(self.half);
            d.set_crop(false);
            d.set_quality(if self.half {
                crate::oidn::Quality::Balanced
            } else {
                crate::oidn::Quality::High
            });
            let result =
                d.run_interruptible(&mut color, &albedo, &normal, 4, w, h, Some((cancel, epoch)));
            d.set_crop(true);
            result?;
            Ok(d.timings())
        })?;
        if self.stage % 2 == 1 {
            self.measurements.push((
                (w * h) as f64,
                (t.total_ms - t.beauty_ms).max(0.),
                t.beauty_ms,
            ));
        }
        self.stage += 1;
        if self.stage < 4 {
            return Ok(None);
        }
        let (p0, g0, b0) = self.measurements[0];
        let (p1, g1, b1) = self.measurements[1];
        let affine = |a: f64, b: f64| {
            let slope = ((b - a) / (p1 - p0)).max(0.);
            ((a - slope * p0).max(0.), slope)
        };
        let (gf, gp) = affine(g0, g1);
        let (bf, bp) = affine(b0, b1);
        let model = DenoiseModel {
            guide_fixed: gf,
            guide_pixel: gp,
            beauty_fixed: bf,
            beauty_pixel: bp,
        };
        MODELS.with(|m| m.borrow_mut()[usize::from(self.half)] = Some(model));
        Ok(Some(model))
    }
    #[cfg(not(feature = "oidn"))]
    pub fn advance(&mut self, _: &AtomicU64, _: u64) -> Result<Option<DenoiseModel>, String> {
        Ok(Some(DenoiseModel::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{FinalQuality, MeshInstance, SceneInput, Transform};
    use std::sync::Arc;
    struct Source;
    impl SceneSource for Source {
        fn scene(&self, _: &SceneRequest) -> Result<SceneInput, String> {
            let mut scene = crate::scene::preset("hero", "starlight", 0.).unwrap();
            scene.objects.clear();
            scene.materials = vec![crate::scene::Material::metal(
                crate::math::v3(0.8, 0.2, 0.1),
                0.3,
            )];
            Ok(SceneInput {
                scene,
                instances: vec![MeshInstance {
                    layer: "sphere".into(),
                    mesh: Arc::new(crate::mesh::sphere(crate::math::Vec3::ZERO, 0.8, 16, 32)),
                    material: 0,
                    material_palette: None,
                    attributes: None,
                    primary_visible: true,
                    world: Transform::default(),
                }],
                materials: None,
                maps: None,
                view: Default::default(),
            })
        }
        fn final_quality(&self, _: &SceneRequest) -> Result<Option<FinalQuality>, String> {
            Ok(Some(FinalQuality {
                sampling: Default::default(),
                min_samples: 8,
                max_samples: 64,
                noise_threshold: 0.12,
                max_bounces: 4,
                caustics: false,
                denoise: false,
                fast_denoise: false,
                half_resolution_guides: true,
            }))
        }
    }
    #[test]
    fn idle_chunks_match_export_sampling_and_cancel_before_more_work() {
        let request = SceneRequest {
            composition: "test".into(),
            revision: 1,
            ticks: 0,
            width: 64,
            height: 48,
        };
        let mut full = StudioService::default();
        full.render(
            &Source,
            request.clone(),
            Tier::Final {
                samples: 64,
                salt: 0,
                denoise: Denoise::None,
            },
        )
        .unwrap();
        let expected = full.renderer.as_ref().unwrap().linear_beauty();
        let counts = full.renderer.as_ref().unwrap().sample_counts();
        let mut chunked = StudioService::default();
        let mut p = chunked
            .begin_calibration(&Source, request.clone(), 64, (64, 48))
            .unwrap();
        let cancel = AtomicU64::new(1);
        let mut calls = 0;
        let measurement = loop {
            calls += 1;
            if let Some(m) = chunked.advance_calibration(&mut p, &cancel, 1).unwrap() {
                break m;
            }
            assert!(calls < 20);
        };
        assert!(calls > 1);
        assert_eq!(chunked.renderer.as_ref().unwrap().sample_counts(), counts);
        assert_eq!(chunked.renderer.as_ref().unwrap().linear_beauty(), expected);
        assert!(measurement.average_samples >= 8.);
        let mut p = chunked
            .begin_calibration(&Source, request, 64, (64, 48))
            .unwrap();
        cancel.store(2, Ordering::Relaxed);
        assert!(
            chunked
                .advance_calibration(&mut p, &cancel, 1)
                .unwrap_err()
                .contains("cancelled")
        );
        assert_eq!(p.done, 0);
    }
    #[test]
    fn crop_halo_is_projected_in_full_size_pixels_and_constant_layers_are_skipped() {
        let mut color = vec![0.; 10 * 10 * 4];
        for y in 3..7 {
            for x in 3..7 {
                color[(y * 10 + x) * 4] = 1.;
            }
        }
        assert_eq!(
            projected_beauty_area(&color, 10, 10, (1000, 1000), true),
            528. * 528.
        );
        assert_eq!(
            projected_beauty_area(&color, 10, 10, (1000, 1000), false),
            1_000_000.
        );
        assert_eq!(
            projected_beauty_area(&vec![1.; 400], 10, 10, (1000, 1000), true),
            0.
        );
    }
    #[test]
    fn fixed_denoise_overhead_is_not_multiplied_by_image_area() {
        let model = DenoiseModel {
            guide_fixed: 10.,
            guide_pixel: 0.001,
            beauty_fixed: 5.,
            beauty_pixel: 0.002,
        };
        assert_eq!(model.predict(100., 200., 2.), 20.5);
        assert_eq!(model.predict(400., 800., 2.), 22.);
        #[cfg(feature = "oidn")]
        {
            let mut probe = DenoiseProbe::new(true);
            let cancel = AtomicU64::new(1);
            assert!(probe.advance(&cancel, 0).unwrap_err().contains("cancelled"));
        }
    }
}
