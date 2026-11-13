//! Explicit, idle-worker Studio calibration. No document edits or UI policy.
use crate::{Shared, Stamp, Viewer, render, studio};
use pm_protocol::common::{Size, Ticks};
#[cfg(feature = "kernel")]
use pm_studio3d_engine::service::calibration::{DenoiseModel, DenoiseProbe, Measurement, Probe};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ExportEstimateState {
    #[default]
    Idle,
    Queued,
    Measuring {
        completed: usize,
        total: usize,
    },
    Ready {
        estimate: ExportEstimate,
    },
    Failed {
        message: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Milliseconds {
    pub min: f64,
    pub max: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetEstimate {
    pub preset: String,
    pub min_samples: u32,
    pub max_samples: u32,
    pub trace_ms: Milliseconds,
    pub photon_setup_ms: f64,
    pub oidn_ms: f64,
    pub guides_and_readback_ms: f64,
    pub prepare_and_post_ms: f64,
    pub frame_ms: Milliseconds,
    pub total_ms: Milliseconds,
    pub calibration: CalibrationTiming,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationTiming {
    pub projected_beauty_pixels: f64,
    pub beauty_layers: f64,
    pub denoise_guide_fixed_ms: f64,
    pub denoise_guide_ms_per_pixel: f64,
    pub denoise_beauty_fixed_ms: f64,
    pub denoise_beauty_ms_per_pixel: f64,
    pub average_samples: f64,
    pub cap_fraction: f64,
    pub trace_resolution_correction: f64,
    pub cold_samples: u32,
    pub cold_trace_ms: f64,
    pub warm_samples: u32,
    pub warm_trace_ms: f64,
    pub oidn_ms: f64,
    pub guides_and_readback_ms: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEstimate {
    pub composition: String,
    pub revision: u64,
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
    pub render_scale: f32,
    pub render_width: u32,
    pub render_height: u32,
    pub frames: u64,
    pub calibration_width: u32,
    pub calibration_height: u32,
    pub extrapolated: bool,
    pub presets: Vec<PresetEstimate>,
    pub warnings: Vec<String>,
    pub excludes: Vec<String>,
}
pub(crate) struct Pending {
    stamp: Stamp,
    content: u64,
    time: Ticks,
    size: Size,
    frames: u64,
    render_scale: f32,
    cancel: Arc<AtomicU64>,
    epoch: u64,
    reply: async_channel::Sender<Result<ExportEstimate, String>>,
    #[cfg(feature = "kernel")]
    calibration: Option<Calibration>,
}
impl Pending {
    pub(crate) fn runnable(&self) -> bool {
        #[cfg(feature = "kernel")]
        {
            self.calibration.as_ref().is_none_or(Calibration::runnable)
        }
        #[cfg(not(feature = "kernel"))]
        {
            true
        }
    }
}
impl Viewer {
    /// Measure the composition's shot camera at a bounded resolution when the
    /// worker is idle. Poll `status().export_estimate` or await this receiver
    /// off the UI thread. Repeated requests coalesce; input/editing cancels a
    /// calibration between GPU passes. Selecting an export preset edits nothing.
    pub fn estimate_studio_export(
        &self,
        stamp: Stamp,
        time: Ticks,
        size: Size,
        frames: u64,
    ) -> anyhow::Result<async_channel::Receiver<Result<ExportEstimate, String>>> {
        self.estimate_studio_export_scaled(stamp, time, size, frames, 1.)
    }
    pub fn estimate_studio_export_scaled(
        &self,
        stamp: Stamp,
        time: Ticks,
        size: Size,
        frames: u64,
        render_scale: f32,
    ) -> anyhow::Result<async_channel::Receiver<Result<ExportEstimate, String>>> {
        validate(size, frames).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            render_scale.is_finite() && render_scale > 0. && render_scale <= 1.,
            "Invalid Studio render scale"
        );
        let (reply, receive) = async_channel::bounded(1);
        let mut state = self.shared.state.lock().unwrap();
        anyhow::ensure!(
            !state.stop && state.halted.is_none(),
            "export estimate worker is stopped"
        );
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        anyhow::ensure!(v.stamp == stamp, "export estimate scene is stale");
        if let Some((cached_stamp, content, estimate)) = &v.export_estimate_cache {
            if *cached_stamp == stamp
                && *content == v.content
                && estimate.ticks == time.get()
                && estimate.width == size.width
                && estimate.height == size.height
                && estimate.frames == frames
                && estimate.render_scale == render_scale
            {
                let cached = estimate.clone();
                cancel(v, "export estimate superseded by cached result");
                let _ = reply.try_send(Ok(cached.clone()));
                v.status.export_estimate = ExportEstimateState::Ready { estimate: cached };
                v.export_estimate_stamp = Some(stamp);
                state.notify();
                return Ok(receive);
            }
        }
        if let Some(old) = v.export_estimate.take() {
            let _ = old.reply.try_send(Err("export estimate superseded".into()));
        }
        let epoch = v.export_estimate_cancel.fetch_add(1, Ordering::Relaxed) + 1;
        v.export_estimate = Some(Pending {
            content: v.content,
            cancel: v.export_estimate_cancel.clone(),
            epoch,
            stamp,
            time,
            size,
            frames,
            render_scale,
            reply,
            #[cfg(feature = "kernel")]
            calibration: None,
        });
        v.export_estimate_stamp = Some(stamp);
        v.status.export_estimate = ExportEstimateState::Queued;
        state.notify();
        drop(state);
        let _ = self.shared.wake.try_send(());
        Ok(receive)
    }
}
fn validate(size: Size, frames: u64) -> Result<(), String> {
    if size.width == 0
        || size.height == 0
        || size.width > 16384
        || size.height > 16384
        || frames == 0
        || frames > 1_000_000_000
    {
        return Err("export estimate needs a valid output size and frame count".into());
    }
    Ok(())
}
fn calibration_size(size: Size) -> Size {
    let scale = (480. / size.width as f64)
        .min(270. / size.height as f64)
        .min(1.);
    Size::new(
        (size.width as f64 * scale).round().max(1.) as u32,
        (size.height as f64 * scale).round().max(1.) as u32,
    )
}
/// A single point and an honest display band. These factors do not change
/// image targets. Resolution correction remains explicit until a held-out fit
/// has been validated on real exports.
fn band(point: f64) -> Milliseconds {
    Milliseconds {
        min: point * 0.85,
        max: point * 1.2,
    }
}
#[cfg(feature = "kernel")]
fn project(
    preset: &str,
    min: u32,
    max: u32,
    measurements: &[Measurement],
    model: DenoiseModel,
    area: f64,
    pixels: f64,
    frames: u64,
) -> Result<PresetEstimate, String> {
    if measurements.len() != 3
        || frames == 0
        || min == 0
        || max < min
        || !area.is_finite()
        || area <= 0.
    {
        return Err("invalid export estimate calibration".into());
    }
    if !pixels.is_finite()
        || pixels <= 0.
        || [
            model.guide_fixed,
            model.guide_pixel,
            model.beauty_fixed,
            model.beauty_pixel,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.)
        || measurements.iter().any(|m| {
            m.samples == 0
                || [
                    m.average_samples,
                    m.cap_fraction,
                    m.trace_ms,
                    m.photon_ms,
                    m.prepare_ms,
                    m.guides_ms,
                    m.denoise_ms,
                    m.projected_beauty_pixels,
                    m.beauty_layers,
                ]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.)
        })
    {
        return Err("invalid measured export timing".into());
    }
    let weighted = |get: fn(&Measurement) -> f64| {
        (get(&measurements[0]) + 4. * get(&measurements[1]) + get(&measurements[2])) / 6.
    };
    let correction = if preset == "production" { 0.695 } else { 1. }; // Fit product + hero; chrome is held out. See studio3d-port.md.
    let trace = weighted(|m| m.trace_ms) * area * correction;
    // Static maps bootstrap once; moving geometry/light sets rebuild each frame.
    let warm_photons = (measurements[1].photon_ms + measurements[2].photon_ms) * 0.5;
    let photon = if warm_photons > 0. {
        weighted(|m| m.photon_ms)
    } else {
        measurements[0].photon_ms / frames as f64
    };
    let beauty = weighted(|m| m.projected_beauty_pixels);
    let layers = weighted(|m| m.beauty_layers);
    let oidn = if preset == "draft" {
        0.
    } else {
        model.predict(pixels, beauty, layers)
    };
    let guides = weighted(|m| m.guides_ms) * area;
    // Transport, composition and encode measured on the real 1080p product
    // path. Preparation is scene dependent; it is not multiplied by pixels.
    let fixed = weighted(|m| m.prepare_ms) + pixels / 2_073_600. * 90.;
    let point = trace + photon + oidn + guides + fixed;
    if !point.is_finite() || point < 0. {
        return Err("invalid measured export timing".into());
    }
    let frame = band(point);
    let total = band(point * frames as f64);
    Ok(PresetEstimate {
        preset: preset.into(),
        min_samples: min,
        max_samples: max,
        trace_ms: band(trace),
        photon_setup_ms: photon,
        oidn_ms: oidn,
        guides_and_readback_ms: guides,
        prepare_and_post_ms: fixed,
        frame_ms: frame,
        total_ms: total,
        calibration: CalibrationTiming {
            projected_beauty_pixels: beauty,
            beauty_layers: layers,
            denoise_guide_fixed_ms: model.guide_fixed,
            denoise_guide_ms_per_pixel: model.guide_pixel,
            denoise_beauty_fixed_ms: model.beauty_fixed,
            denoise_beauty_ms_per_pixel: model.beauty_pixel,
            average_samples: weighted(|m| m.average_samples),
            cap_fraction: weighted(|m| m.cap_fraction),
            trace_resolution_correction: correction,
            cold_samples: measurements[0].samples,
            cold_trace_ms: measurements[0].trace_ms + measurements[0].photon_ms,
            warm_samples: measurements[1].samples,
            warm_trace_ms: weighted(|m| m.trace_ms),
            oidn_ms: weighted(|m| m.denoise_ms),
            guides_and_readback_ms: weighted(|m| m.guides_ms),
        },
    })
}

#[cfg(feature = "kernel")]
thread_local! {static INITIALIZING:Arc<std::sync::atomic::AtomicBool>=Arc::new(std::sync::atomic::AtomicBool::new(false));}
#[cfg(feature = "kernel")]
struct InitializationFinished {
    busy: Arc<std::sync::atomic::AtomicBool>,
    done: Arc<std::sync::atomic::AtomicBool>,
    wake: std::sync::mpsc::SyncSender<()>,
}
#[cfg(feature = "kernel")]
impl Drop for InitializationFinished {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        self.busy.store(false, Ordering::Release);
        let _ = self.wake.try_send(());
    }
}
#[cfg(feature = "kernel")]
pub(crate) struct Calibration {
    source: studio::DocumentSource,
    capture: Option<pm_render::Renderer>,
    service: Option<studio::StudioService>,
    initializing: Option<
        std::sync::mpsc::Receiver<Result<(studio::StudioService, pm_render::Renderer), String>>,
    >,
    initialization: Option<(wgpu::Device, wgpu::Queue, std::sync::mpsc::SyncSender<()>)>,
    factory_busy: Arc<std::sync::atomic::AtomicBool>,
    initialized: Arc<std::sync::atomic::AtomicBool>,
    request: studio::SceneRequest,
    target: Size,
    render_target: Size,
    render_scale: f32,
    frames: u64,
    stage: usize,
    pair: Vec<Measurement>,
    probe: Option<Probe>,
    denoise_probe: Option<DenoiseProbe>,
    denoise_models: [Option<DenoiseModel>; 2],
    sample_times: [i64; 3],
    presets: Vec<PresetEstimate>,
    warnings: Vec<String>,
}
#[cfg(feature = "kernel")]
impl Calibration {
    fn runnable(&self) -> bool {
        self.service.is_some()
            || self.initialized.load(Ordering::Acquire)
            || (self.initializing.is_none() && !self.factory_busy.load(Ordering::Acquire))
    }
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        scene: &render::Scene,
        time: Ticks,
        size: Size,
        frames: u64,
        render_scale: f32,
        definitions: pm_protocol::definitions::DefinitionSetSnapshot,
        fonts: Option<&pm_render::text::host::RenderFonts>,
        wake: std::sync::mpsc::SyncSender<()>,
    ) -> Result<Self, String> {
        validate(size, frames)?;
        let dimensions = pm_studio3d_engine::compositor::scaled_export_request(
            &studio::SceneRequest {
                composition: scene.comp.clone(),
                revision: scene.doc.revision(),
                ticks: time.get(),
                width: size.width,
                height: size.height,
            },
            render_scale,
        )?;
        let render_target = Size::new(dimensions.width, dimensions.height);
        let small = calibration_size(render_target);
        let value = scene
            .doc
            .to_value()
            .ok_or("empty export estimate document")?;
        let canonical: serde_json::Value =
            serde_json::from_str(&pm_model::json::to_canonical_string(&value))
                .map_err(|e| e.to_string())?;
        let fps = pm_model::ingest::IngestRate::of_composition(&pm_scene3d::to_model(
            &canonical["comps"][&scene.comp],
        ))
        .map_err(|e| e.to_string())?
        .fps;
        let frame_time = |frame: u64| -> Result<i64, String> {
            let offset =
                pm_time::ticks_from_seconds(frame as f64 / fps).map_err(|e| e.to_string())?;
            time.get()
                .checked_add(offset)
                .ok_or_else(|| "export estimate range exceeds supported time".into())
        };
        let sample_times = [
            time.get(),
            frame_time((frames - 1) / 2)?,
            frame_time(frames - 1)?,
        ];
        let initialized = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let factory_busy = INITIALIZING.with(Arc::clone);
        Ok(Self {
            source: studio::DocumentSource::new(scene.doc.clone(), scene.media.as_ref().clone())
                .with_native_fonts(fonts)
                .with_capture_definitions(definitions),
            capture: None,
            service: None, initializing:None, initialization:Some((device,queue,wake)),factory_busy,initialized,
            request: studio::SceneRequest {
                composition: scene.comp.clone(),
                revision: scene.doc.revision(),
                ticks: time.get(),
                width: small.width,
                height: small.height,
            },
            target: size,
            render_target,
            render_scale,
            frames,
            stage: 0,
            pair: Vec::new(),probe:None,denoise_probe:None,denoise_models:[None,None],sample_times,
            presets: Vec::new(),
            warnings: vec!["Production trace correction is fitted on product and hero; other presets and unseen scene classes remain extrapolations".into()],
        })
    }
    /// Exactly one bounded render; the main loop services input before the next.
    fn advance(
        &mut self,
        cancel: &AtomicU64,
        epoch: u64,
    ) -> Result<Option<ExportEstimate>, String> {
        if self.service.is_none() {
            if self.initializing.is_none() {
                if self
                    .factory_busy
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return Ok(None);
                }
                let (device, queue, wake) = self.initialization.take().unwrap();
                let finished = InitializationFinished {
                    busy: self.factory_busy.clone(),
                    done: self.initialized.clone(),
                    wake,
                };
                let (ready, initializing) = std::sync::mpsc::channel();
                self.initializing = Some(initializing);
                let (w, h) = (self.request.width, self.request.height);
                std::thread::Builder::new()
                    .name("studio-estimate-pipelines".into())
                    .spawn(move || {
                        // Also runs on panic: release the one compile slot and wake
                        // the worker so a disconnected result becomes a failure.
                        let _finished = finished;
                        let result =
                            studio::StudioService::with_device(device.clone(), queue.clone(), w, h)
                                .map(|service| (service, pm_render::Renderer::new(device, queue)));
                        let _ = ready.send(result);
                    })
                    .map_err(|e| e.to_string())?;
                return Ok(None);
            }
            match self.initializing.as_ref().unwrap().try_recv() {
                Ok(result) => {
                    let (service, capture) = result?;
                    self.service = Some(service);
                    self.capture = Some(capture);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
                Err(error) => {
                    return Err(format!("Studio estimate pipeline initialization: {error}"));
                }
            }
        }
        use pm_studio3d_engine::{
            compositor::Source,
            extract::{FinalQuality, SceneSource},
            service::{Denoise, Tier},
        };
        struct Policy<'a>(&'a studio::DocumentSource, FinalQuality);
        impl SceneSource for Policy<'_> {
            fn scene(&self, r: &studio::SceneRequest) -> Result<studio::SceneInput, String> {
                self.0.scene(r)
            }
            fn auxiliary_layers(
                &self,
                r: &studio::SceneRequest,
            ) -> Result<Vec<pm_studio3d_engine::extract::AuxiliaryLayer>, String> {
                self.0.auxiliary_layers(r)
            }
            fn layer_failures(
                &self,
                r: &studio::SceneRequest,
            ) -> Result<Vec<pm_render::LayerFailure>, String> {
                self.0.layer_failures(r)
            }
            fn final_quality(
                &self,
                _: &studio::SceneRequest,
            ) -> Result<Option<FinalQuality>, String> {
                Ok(Some(self.1))
            }
        }
        let name = ["draft", "preview", "production", "final"][self.stage / 3];
        let q = pm_scene3d::quality::preset(name).map_err(|e| e.to_string())?;
        let half = name != "final";
        if name != "draft" && self.denoise_models[usize::from(half)].is_none() {
            let probe = self
                .denoise_probe
                .get_or_insert_with(|| DenoiseProbe::new(half));
            if let Some(model) = probe.advance(cancel, epoch)? {
                self.denoise_models[usize::from(half)] = Some(model);
                self.denoise_probe = None;
            }
            return Ok(None);
        }
        let spp = q["maxSamples"].as_u64().unwrap() as u32;
        let tier = Tier::Final {
            samples: spp,
            salt: 0,
            denoise: Denoise::Oidn,
        };
        let policy = Policy(
            &self.source,
            FinalQuality {
                sampling: pm_studio3d_engine::sampling::SamplingPolicy {
                    perceptual_noise: q["perceptualNoise"].as_bool().unwrap_or(false),
                    edge_multiplier: q["edgeSamplesMultiplier"].as_u64().unwrap_or(4) as u32,
                    diffuse_bounces: q["diffuseBounces"].as_u64().unwrap_or(0) as u32,
                    glossy_bounces: q["glossyBounces"].as_u64().unwrap_or(0) as u32,
                    transmission_bounces: q["transmissionBounces"].as_u64().unwrap_or(0) as u32,
                },
                min_samples: q["minSamples"].as_u64().unwrap() as u32,
                max_samples: spp,
                noise_threshold: q["noiseThreshold"].as_f64().unwrap() as f32,
                max_bounces: q["maxBounces"].as_u64().unwrap() as u32,
                caustics: q["caustics"] == true,
                denoise: true,
                fast_denoise: name == "draft",
                half_resolution_guides: half,
            },
        );
        if self.probe.is_none() {
            self.request.ticks = self.sample_times[self.stage % 3];
            for failure in policy.layer_failures(&self.request)? {
                let warning = format!("{}: {}", failure.layer, failure.message);
                if !self.warnings.contains(&warning) {
                    self.warnings.push(warning);
                }
            }
            if self.source.needs_materials(&self.request)? {
                self.source.prepare_materials(
                    self.capture.as_mut().unwrap(),
                    &self.request,
                    tier,
                )?;
            }
            if !self.source.splat_layers(&self.request)?.is_empty() {
                return Err("Gaussian splat export estimates are not available".into());
            }
            if self.stage % 3 == 0 {
                self.service.as_mut().unwrap().reset_caustic_map();
            }
            self.probe = Some(self.service.as_mut().unwrap().begin_calibration(
                &policy,
                self.request.clone(),
                spp,
                (self.render_target.width, self.render_target.height),
            )?);
            return Ok(None);
        }
        let Some(m) = self.service.as_mut().unwrap().advance_calibration(
            self.probe.as_mut().unwrap(),
            cancel,
            epoch,
        )?
        else {
            return Ok(None);
        };
        self.pair.push(m);
        self.probe = None;
        self.stage += 1;
        if self.pair.len() == 3 {
            let pixels = self.render_target.width as f64 * self.render_target.height as f64;
            let area = pixels / (self.request.width as f64 * self.request.height as f64);
            self.presets.push(project(
                name,
                q["minSamples"].as_u64().unwrap() as u32,
                spp,
                &self.pair,
                self.denoise_models[usize::from(half)].unwrap_or_default(),
                area,
                pixels,
                self.frames,
            )?);
            self.pair.clear();
        }
        Ok((self.stage == 12).then(|| ExportEstimate {
            composition: self.request.composition.clone(),
            revision: self.request.revision,
            ticks: self.sample_times[0],
            width: self.target.width,
            height: self.target.height,
            render_scale: self.render_scale,
            render_width: self.render_target.width,
            render_height: self.render_target.height,
            frames: self.frames,
            calibration_width: self.request.width,
            calibration_height: self.request.height,
            extrapolated: self.render_target != Size::new(self.request.width, self.request.height),
            presets: self.presets.clone(),
            warnings: self.warnings.clone(),
            excludes: vec![
                "file I/O variation beyond the measured per-frame allowance".into(),
                "2D composition effects and material-source capture".into(),
                "animation variation between the first/middle/last sampled frames".into(),
                "additional motion-blur shutter samples".into(),
            ],
        }))
    }
}
pub(crate) fn cancel(view: &mut crate::View, reason: &str) {
    view.export_estimate_cancel.fetch_add(1, Ordering::Relaxed);
    if let Some(p) = view.export_estimate.take() {
        let _ = p.reply.try_send(Err(reason.into()));
    }
    view.status.export_estimate = ExportEstimateState::Idle;
    view.export_estimate_stamp = None;
}
fn abandon(shared: &Shared, id: u64, p: &Pending, reason: &str) {
    let mut state = shared.state.lock().unwrap();
    if let Some(v) = state.views.get_mut(&id) {
        if v.export_estimate.is_none()
            && v.export_estimate_cancel.load(Ordering::Relaxed) == p.epoch
        {
            cancel(v, reason);
            state.notify();
        }
    }
    let _ = p.reply.try_send(Err(reason.into()));
}
pub(crate) fn run_one(engine: &mut render::Engine, shared: &Arc<Shared>) {
    let work = {
        let mut state = shared.state.lock().unwrap();
        state.views.iter_mut().find_map(|(id, v)| {
            if !v.export_estimate.as_ref().is_some_and(Pending::runnable) {
                return None;
            }
            let p = v.export_estimate.take()?;
            Some((
                *id,
                p,
                render::Scene {
                    doc: v.config.snapshot.document.clone(),
                    comp: v.config.composition.clone(),
                    media: v.paths.clone(),
                    content: v.content,
                },
                v.resources.clone(),
            ))
        })
    };
    let Some((id, mut p, scene, resources)) = work else {
        return;
    };
    let current = |state: &crate::State| {
        state.views.get(&id).is_some_and(|v| {
            v.stamp == p.stamp
                && v.content == p.content
                && v.export_estimate.is_none()
                && v.desired_rate.num == 0
                && p.cancel.load(Ordering::Relaxed) == p.epoch
        }) && !state.stop
            && state.halted.is_none()
    };
    if !current(&shared.state.lock().unwrap()) {
        abandon(
            shared,
            id,
            &p,
            "export estimate scene changed before calibration",
        );
        return;
    }
    #[cfg(feature = "kernel")]
    let step = (|| {
        if p.calibration.is_none() {
            engine.set_resources(resources);
            p.calibration = Some(
                engine
                    .start_export_calibration(
                        &scene,
                        p.time,
                        p.size,
                        p.frames,
                        p.render_scale,
                        shared.wake.clone(),
                    )
                    .map_err(|e| e.to_string())?,
            );
        }
        p.calibration.as_mut().unwrap().advance(&p.cancel, p.epoch)
    })();
    #[cfg(not(feature = "kernel"))]
    let step: Result<Option<ExportEstimate>, String> = {
        let _ = (engine, scene, resources);
        Err("Studio estimates need the native document adapter".into())
    };
    let mut state = shared.state.lock().unwrap();
    if !current(&state) {
        drop(state);
        abandon(
            shared,
            id,
            &p,
            "export estimate scene changed before completion",
        );
        return;
    }
    let v = state.views.get_mut(&id).unwrap();
    match step {
        Ok(Some(estimate)) => {
            v.export_estimate_cache = Some((p.stamp, p.content, estimate.clone()));
            v.status.export_estimate = ExportEstimateState::Ready {
                estimate: estimate.clone(),
            };
            let _ = p.reply.try_send(Ok(estimate));
        }
        Ok(None) => {
            #[cfg(feature = "kernel")]
            {
                v.status.export_estimate = ExportEstimateState::Measuring {
                    completed: p.calibration.as_ref().unwrap().stage,
                    total: 12,
                };
            }
            v.export_estimate = Some(p);
        }
        Err(message) => {
            v.status.export_estimate = ExportEstimateState::Failed {
                message: message.clone(),
            };
            let _ = p.reply.try_send(Err(message));
        }
    }
    state.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calibration_is_bounded_and_band_preserves_point() {
        assert_eq!(calibration_size(Size::new(1920, 1080)), Size::new(480, 270));
        assert_eq!(calibration_size(Size::new(64, 64)), Size::new(64, 64));
        assert!(validate(Size::new(0, 1080), 1).is_err());
        assert!(validate(Size::new(1920, 1080), 0).is_err());
        let band = band(1000.);
        assert_eq!((band.min, band.max), (850., 1200.));
    }
    #[cfg(feature = "kernel")]
    #[test]
    fn cold_photons_are_amortized_and_denoising_has_fixed_overhead() {
        let mut m = vec![
            Measurement {
                samples: 32,
                average_samples: 20.,
                cap_fraction: 0.1,
                trace_ms: 10.,
                prepare_ms: 3.,
                guides_ms: 2.,
                beauty_pixels: 100,
                projected_beauty_pixels: 400.,
                beauty_layers: 1.,
                ..Default::default()
            };
            3
        ];
        m[0].photon_ms = 30.;
        let model = DenoiseModel {
            guide_fixed: 10.,
            guide_pixel: 0.01,
            beauty_fixed: 5.,
            beauty_pixel: 0.02,
        };
        let p = project("production", 8, 128, &m, model, 4., 400., 30).unwrap();
        assert_eq!(p.photon_setup_ms, 1.);
        assert_eq!(p.oidn_ms, 27.);
        assert_eq!(p.calibration.average_samples, 20.);
        m[1].photon_ms = 30.;
        m[2].photon_ms = 30.;
        assert_eq!(
            project("production", 8, 128, &m, model, 4., 400., 30)
                .unwrap()
                .photon_setup_ms,
            30.
        );
        m[0].trace_ms = f64::NAN;
        assert!(project("production", 8, 128, &m, model, 4., 400., 30).is_err());
        m[0].trace_ms = 10.;
        m[0].samples = 0;
        assert!(project("production", 8, 128, &m, model, 4., 400., 30).is_err());
    }
    #[cfg(feature = "kernel")]
    #[test]
    fn initializer_unwind_releases_the_single_slot_and_wakes_the_worker() {
        use std::sync::atomic::AtomicBool;
        let busy = Arc::new(AtomicBool::new(true));
        let done = Arc::new(AtomicBool::new(false));
        let (wake, receive) = std::sync::mpsc::sync_channel(1);
        let signal = InitializationFinished {
            busy: busy.clone(),
            done: done.clone(),
            wake,
        };
        let result = std::panic::catch_unwind(|| {
            let _signal = signal;
            panic!("initializer failure fixture");
        });
        assert!(result.is_err());
        assert!(!busy.load(Ordering::Acquire));
        assert!(done.load(Ordering::Acquire));
        assert_eq!(receive.try_recv(), Ok(()));
    }
    #[cfg(feature = "kernel")]
    #[test]
    fn viewer_requests_coalesce_and_changed_frames_cancel_without_calibrating() {
        use crate::{EngineService, Output, Quality, ViewerConfig};
        let service = EngineService::start(pm_gpu::Gpu::new("estimate queue").unwrap()).unwrap();
        let (entered, signal) = std::sync::mpsc::channel();
        let (release, barrier) = std::sync::mpsc::channel();
        *service.shared.before_submit.lock().unwrap() = Some(Box::new(move || {
            entered.send(()).unwrap();
            barrier
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap();
        }));
        let doc = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        let snapshot = pm_kernel::DocumentStore::new_untitled(doc).snapshot();
        let comp = snapshot.open_comp().unwrap();
        let viewer = service
            .viewer(ViewerConfig {
                snapshot,
                composition: comp,
                media: Default::default(),
                output: Output {
                    width: 64,
                    height: 64,
                    display: 1,
                    headroom: 1.,
                    crop: None,
                },
                layer: None,
                rate: pm_time::Rate::FPS_30,
                end: None,
                audio: false,
            })
            .unwrap();
        signal
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap();
        let stamp = viewer.stamp().unwrap();
        let old = viewer
            .estimate_studio_export(stamp, Ticks::ZERO, Size::new(64, 64), 1)
            .unwrap();
        let next = viewer
            .estimate_studio_export(stamp, Ticks::ZERO, Size::new(64, 64), 2)
            .unwrap();
        assert!(old.try_recv().unwrap().unwrap_err().contains("superseded"));
        assert!(matches!(
            viewer.status().export_estimate,
            ExportEstimateState::Queued
        ));
        // Simulate an in-flight estimate after the worker took it from the
        // queue. A cache hit must supersede that lease as well as queued work.
        let cached = ExportEstimate {
            composition: viewer.shared.state.lock().unwrap().views[&viewer.id]
                .config
                .composition
                .clone(),
            revision: stamp.revision,
            ticks: 0,
            width: 64,
            height: 64,
            render_scale: 1.,
            render_width: 64,
            render_height: 64,
            frames: 1,
            calibration_width: 64,
            calibration_height: 64,
            extrapolated: false,
            presets: vec![],
            warnings: vec![],
            excludes: vec![],
        };
        let inflight = {
            let mut state = service.shared.state.lock().unwrap();
            let v = state.views.get_mut(&viewer.id).unwrap();
            v.export_estimate_cache = Some((stamp, v.content, cached.clone()));
            v.export_estimate.take().unwrap()
        };
        let hit = viewer
            .estimate_studio_export(stamp, Ticks::ZERO, Size::new(64, 64), 1)
            .unwrap();
        assert_eq!(hit.try_recv().unwrap().unwrap(), cached);
        assert_ne!(inflight.cancel.load(Ordering::Relaxed), inflight.epoch);
        abandon(
            &service.shared,
            viewer.id,
            &inflight,
            "scene changed after cached estimate",
        );
        assert!(
            next.try_recv()
                .unwrap()
                .unwrap_err()
                .contains("cached estimate")
        );
        assert_eq!(
            viewer.status().export_estimate,
            ExportEstimateState::Ready { estimate: cached }
        );
        let next = viewer
            .estimate_studio_export(stamp, Ticks::ZERO, Size::new(64, 64), 2)
            .unwrap();
        viewer.quality(Quality::Half).unwrap();
        assert!(matches!(
            viewer.status().export_estimate,
            ExportEstimateState::Idle
        ));
        release.send(()).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match next.try_recv() {
                Ok(result) => {
                    assert!(result.unwrap_err().contains("scene changed"));
                    break;
                }
                Err(async_channel::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now() < until);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(e) => panic!("estimate channel closed: {e}"),
            }
        }
        assert!(matches!(
            viewer.status().export_estimate,
            ExportEstimateState::Idle
        ));
        viewer.retire();
    }
}
