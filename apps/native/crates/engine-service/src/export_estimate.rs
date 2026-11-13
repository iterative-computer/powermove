//! Explicit, idle-worker Studio calibration. No document edits or UI policy.
use crate::{Shared, Stamp, Viewer, render, studio};
use pm_protocol::common::{Size, Ticks};
use serde::Serialize;
use std::sync::Arc;

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
    pub render_scale:f32,
    pub render_width:u32,
    pub render_height:u32,
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
    time: Ticks,
    size: Size,
    frames: u64,
    render_scale:f32,
    reply: async_channel::Sender<Result<ExportEstimate, String>>,
    #[cfg(feature = "kernel")]
    calibration: Option<Calibration>,
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
        self.estimate_studio_export_scaled(stamp,time,size,frames,1.)
    }
    pub fn estimate_studio_export_scaled(
        &self,
        stamp: Stamp,
        time: Ticks,
        size: Size,
        frames: u64,
        render_scale:f32,
    ) -> anyhow::Result<async_channel::Receiver<Result<ExportEstimate, String>>> {
        validate(size, frames).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(render_scale.is_finite() && render_scale>0. && render_scale<=1.,"Invalid Studio render scale");
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
        if let Some(old) = v.export_estimate.take() {
            let _ = old.reply.try_send(Err("export estimate superseded".into()));
        }
        v.export_estimate = Some(Pending {
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
    let scale = (320. / size.width as f64)
        .min(180. / size.height as f64)
        .min(1.);
    Size::new(
        (size.width as f64 * scale).round().max(1.) as u32,
        (size.height as f64 * scale).round().max(1.) as u32,
    )
}
#[derive(Clone, Copy, Debug)]
struct Sample {
    samples: u32,
    trace: f64,
    oidn: f64,
    guides: f64,
    prepare_post: f64,
}
fn project(
    preset: &str,
    min: u32,
    max: u32,
    cold: Sample,
    warm: Sample,
    area: f64,
    frames: u64,
) -> Result<PresetEstimate, String> {
    for v in [
        cold.trace,
        warm.trace,
        warm.oidn,
        warm.guides,
        warm.prepare_post,
        area,
    ] {
        if !v.is_finite() || v < 0. {
            return Err("invalid measured export timing".into());
        }
    }
    if warm.samples == 0 || cold.samples == 0 || min == 0 || max < min || area <= 0. {
        return Err("invalid export estimate sample policy".into());
    }
    let rate = warm.trace / warm.samples as f64;
    let photon = if matches!(preset, "production" | "final") {
        (cold.trace - rate * cold.samples as f64).max(0.)
    } else {
        0.
    };
    let trace = Milliseconds {
        min: rate * min as f64 * area,
        max: rate * max as f64 * area,
    };
    let oidn = warm.oidn * area;
    let guides = warm.guides * area;
    let prepare_post = warm.prepare_post; // Scene preparation does not scale with image area.
    let fixed = photon + oidn + guides + prepare_post;
    let frame = Milliseconds {
        min: trace.min + fixed,
        max: trace.max + fixed,
    };
    let total = Milliseconds {
        min: frame.min * frames as f64,
        max: frame.max * frames as f64,
    };
    if frames == 0 || !total.max.is_finite() {
        return Err("export estimate exceeds a finite duration".into());
    }
    let calibration = CalibrationTiming {
        cold_samples: cold.samples,
        cold_trace_ms: cold.trace,
        warm_samples: warm.samples,
        warm_trace_ms: warm.trace,
        oidn_ms: warm.oidn,
        guides_and_readback_ms: warm.guides,
    };
    Ok(PresetEstimate {
        preset: preset.into(),
        min_samples: min,
        max_samples: max,
        trace_ms: trace,
        photon_setup_ms: photon,
        oidn_ms: oidn,
        guides_and_readback_ms: guides,
        prepare_and_post_ms: prepare_post,
        frame_ms: frame,
        total_ms: total,
        calibration,
    })
}

#[cfg(feature = "kernel")]
pub(crate) struct Calibration {
    source: studio::DocumentSource,
    capture: pm_render::Renderer,
    service: studio::StudioService,
    request: studio::SceneRequest,
    target: Size,
    render_target:Size,
    render_scale:f32,
    frames: u64,
    stage: usize,
    pair: Vec<Sample>,
    presets: Vec<PresetEstimate>,
    warnings: Vec<String>,
}
#[cfg(feature = "kernel")]
impl Calibration {
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        scene: &render::Scene,
        time: Ticks,
        size: Size,
        frames: u64,
        render_scale:f32,
        definitions: pm_protocol::definitions::DefinitionSetSnapshot,
        fonts: Option<&pm_render::text::host::RenderFonts>,
    ) -> Result<Self, String> {
        validate(size, frames)?;
        let dimensions=pm_studio3d_engine::compositor::scaled_export_request(&studio::SceneRequest {
            composition:scene.comp.clone(),revision:scene.doc.revision(),ticks:time.get(),width:size.width,height:size.height,
        },render_scale)?;
        let render_target=Size::new(dimensions.width,dimensions.height);
        let small = calibration_size(render_target);
        Ok(Self {
            source: studio::DocumentSource::new(scene.doc.clone(), scene.media.as_ref().clone())
                .with_native_fonts(fonts)
                .with_capture_definitions(definitions),
            capture: pm_render::Renderer::new(device.clone(), queue.clone()),
            service: studio::StudioService::with_device(device, queue, small.width, small.height)?,
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
            pair: Vec::new(),
            presets: Vec::new(),
            warnings: Vec::new(),
        })
    }
    /// Exactly one bounded render; the main loop services input before the next.
    fn advance(&mut self) -> Result<Option<ExportEstimate>, String> {
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
        let name = ["draft", "preview", "production", "final"][self.stage / 2];
        let q = pm_scene3d::quality::preset(name).map_err(|e| e.to_string())?;
        let spp = [8, 16][self.stage % 2];
        if self.stage % 2 == 0 {
            self.service.reset_caustic_map();
        }
        let tier = Tier::Final {
            samples: spp,
            salt: 0,
            denoise: Denoise::Oidn,
        };
        if self.source.needs_materials(&self.request)? {
            self.source
                .prepare_materials(&mut self.capture, &self.request, tier)?;
        }
        if !self.source.splat_layers(&self.request)?.is_empty() {
            return Err("Gaussian splat export estimates are not available".into());
        }
        let policy = Policy(
            &self.source,
            FinalQuality {
                sampling: pm_studio3d_engine::sampling::SamplingPolicy {
                    perceptual_noise:q["perceptualNoise"].as_bool().unwrap_or(false),
                    edge_multiplier:q["edgeSamplesMultiplier"].as_u64().unwrap_or(4) as u32,
                    diffuse_bounces:q["diffuseBounces"].as_u64().unwrap_or(0) as u32,
                    glossy_bounces:q["glossyBounces"].as_u64().unwrap_or(0) as u32,
                    transmission_bounces:q["transmissionBounces"].as_u64().unwrap_or(0) as u32,
                },
                min_samples: spp,
                max_samples: spp,
                noise_threshold: 0.,
                max_bounces: q["maxBounces"].as_u64().unwrap() as u32,
                caustics: q["caustics"] == true,
                denoise: true,
                fast_denoise: name == "draft",
                half_resolution_guides: name != "final",
            },
        );
        let frame = self.service.render(&policy, self.request.clone(), tier)?;
        if frame.layers.is_empty() {
            return Err("composition has no Studio contribution to estimate".into());
        }
        for f in &frame.failures {
            let warning = format!("{}: {}", f.layer, f.message);
            if !self.warnings.contains(&warning) {
                self.warnings.push(warning);
            }
        }
        let t = frame.timings;
        self.pair.push(Sample {
            samples: frame.samples,
            trace: t.pipeline.trace_ms,
            oidn: t.pipeline.denoise_ms,
            guides: t.pipeline.guides_ms + t.pipeline.guide_readback_ms,
            prepare_post: t.prepare_ms + (t.pipeline.post_ms-t.pipeline.denoise_ms-t.pipeline.guides_ms-t.pipeline.guide_readback_ms).max(0.),
        });
        self.stage += 1;
        if self.pair.len() == 2 {
            let area = self.render_target.width as f64 * self.render_target.height as f64
                / (self.request.width as f64 * self.request.height as f64);
            self.presets.push(project(
                name,
                q["minSamples"].as_u64().unwrap() as u32,
                q["maxSamples"].as_u64().unwrap() as u32,
                self.pair[0],
                self.pair[1],
                area,
                self.frames,
            )?);
            self.pair.clear();
        }
        Ok((self.stage == 8).then(|| ExportEstimate {
            composition: self.request.composition.clone(),
            revision: self.request.revision,
            ticks: self.request.ticks,
            width: self.target.width,
            height: self.target.height,
            render_scale:self.render_scale,
            render_width:self.render_target.width,
            render_height:self.render_target.height,
            frames: self.frames,
            calibration_width: self.request.width,
            calibration_height: self.request.height,
            extrapolated: self.render_target != Size::new(self.request.width, self.request.height),
            presets: self.presets.clone(),
            warnings: self.warnings.clone(),
            excludes: vec![
                "encode and file I/O".into(),
                "2D composition effects and material-source capture".into(),
                "animation variation beyond the sampled frame".into(),
                "additional motion-blur shutter samples".into(),
            ],
        }))
    }
}
pub(crate) fn cancel(view: &mut crate::View, reason: &str) {
    if let Some(p) = view.export_estimate.take() {
        let _ = p.reply.try_send(Err(reason.into()));
    }
    view.status.export_estimate = ExportEstimateState::Idle;
    view.export_estimate_stamp = None;
}
pub(crate) fn run_one(engine: &mut render::Engine, shared: &Arc<Shared>) {
    let work = {
        let mut state = shared.state.lock().unwrap();
        state.views.iter_mut().find_map(|(id, v)| {
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
            v.stamp == p.stamp && v.export_estimate.is_none() && v.desired_rate.num == 0
        }) && !state.stop
            && state.halted.is_none()
    };
    if !current(&shared.state.lock().unwrap()) {
        let _ = p.reply.try_send(Err(
            "export estimate scene changed before calibration".into()
        ));
        return;
    }
    #[cfg(feature = "kernel")]
    let step = (|| {
        if p.calibration.is_none() {
            engine.set_resources(resources);
            p.calibration = Some(
                engine
                    .start_export_calibration(&scene, p.time, p.size, p.frames,p.render_scale)
                    .map_err(|e| e.to_string())?,
            );
        }
        p.calibration.as_mut().unwrap().advance()
    })();
    #[cfg(not(feature = "kernel"))]
    let step: Result<Option<ExportEstimate>, String> = {
        let _ = (engine, scene, resources);
        Err("Studio estimates need the native document adapter".into())
    };
    let mut state = shared.state.lock().unwrap();
    if !current(&state) {
        let _ = p
            .reply
            .try_send(Err("export estimate scene changed before completion".into()));
        return;
    }
    let v = state.views.get_mut(&id).unwrap();
    match step {
        Ok(Some(estimate)) => {
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
                    total: 8,
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
    fn estimate_separates_fixed_photons_from_pixel_and_sample_work() {
        let cold = Sample {
            samples: 8,
            trace: 18.,
            oidn: 7.,
            guides: 2.,
            prepare_post: 3.,
        };
        let warm = Sample {
            samples: 16,
            trace: 16.,
            ..cold
        };
        let p = project("production", 32, 256, cold, warm, 4., 10).unwrap();
        assert_eq!(p.photon_setup_ms, 10.);
        assert_eq!((p.trace_ms.min, p.trace_ms.max), (128., 1024.));
        assert_eq!(p.oidn_ms, 28.);
        assert_eq!((p.frame_ms.min, p.frame_ms.max), (177., 1073.));
        assert_eq!(p.total_ms.max, 10730.);
    }
    #[test]
    fn calibration_is_bounded_and_bad_measurements_are_refused() {
        assert_eq!(calibration_size(Size::new(1920, 1080)), Size::new(320, 180));
        assert_eq!(calibration_size(Size::new(64, 64)), Size::new(64, 64));
        assert!(validate(Size::new(0, 1080), 1).is_err());
        assert!(validate(Size::new(1920, 1080), 0).is_err());
        let s = Sample {
            samples: 0,
            trace: 1.,
            oidn: 1.,
            guides: 1.,
            prepare_post: 1.,
        };
        assert!(project("final", 512, 1024, s, s, 1., 1).is_err());
        let s = Sample {
            samples: 8,
            trace: f64::NAN,
            ..s
        };
        assert!(project("final", 512, 1024, s, s, 1., 1).is_err());
    }
    #[cfg(feature = "kernel")]
    #[test]
    fn viewer_requests_coalesce_and_changed_frames_cancel_without_calibrating() {
        use crate::{EngineService, Output, ViewerConfig, Quality};
        let service = EngineService::start(pm_gpu::Gpu::new("estimate queue").unwrap()).unwrap();
        let (entered, signal) = std::sync::mpsc::channel();
        let (release, barrier) = std::sync::mpsc::channel();
        *service.shared.before_submit.lock().unwrap() = Some(Box::new(move || {
            entered.send(()).unwrap();
            barrier.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
        }));
        let doc = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        let snapshot = pm_kernel::DocumentStore::new_untitled(doc).snapshot();
        let comp = snapshot.open_comp().unwrap();
        let viewer = service.viewer(ViewerConfig {
            snapshot, composition: comp, media: Default::default(),
            output: Output {width:64,height:64,display:1,headroom:1.,crop:None},
            layer:None,rate:pm_time::Rate::FPS_30,end:None,audio:false,
        }).unwrap();
        signal.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
        let stamp = viewer.stamp().unwrap();
        let old = viewer.estimate_studio_export(stamp,Ticks::ZERO,Size::new(64,64),1).unwrap();
        let next = viewer.estimate_studio_export(stamp,Ticks::ZERO,Size::new(64,64),2).unwrap();
        assert!(old.try_recv().unwrap().unwrap_err().contains("superseded"));
        assert!(matches!(viewer.status().export_estimate,ExportEstimateState::Queued));
        viewer.quality(Quality::Half).unwrap();
        assert!(matches!(viewer.status().export_estimate,ExportEstimateState::Idle));
        release.send(()).unwrap();
        let until = std::time::Instant::now()+std::time::Duration::from_secs(20);
        loop {
            match next.try_recv() {
                Ok(result) => {assert!(result.unwrap_err().contains("scene changed"));break;}
                Err(async_channel::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now()<until);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(e) => panic!("estimate channel closed: {e}"),
            }
        }
        assert!(matches!(viewer.status().export_estimate,ExportEstimateState::Idle));
        viewer.retire();
    }
}
