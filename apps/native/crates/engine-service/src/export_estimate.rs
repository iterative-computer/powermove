//! Explicit, idle-worker Studio calibration. No document edits or UI policy.
use crate::{Shared, Stamp, Viewer, render, studio};
use pm_protocol::common::{Size, Ticks};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ExportEstimateState {
    #[default]
    Idle,
    Queued,
    Measuring { completed: usize, total: usize },
    Ready { estimate: ExportEstimate },
    Failed { message: String },
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Milliseconds { pub min: f64, pub max: f64 }
#[derive(Clone, Debug, Serialize)]
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
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEstimate {
    pub composition: String,
    pub revision: u64,
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
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
    reply: async_channel::Sender<Result<ExportEstimate, String>>,
}
impl Viewer {
    /// Measure the composition's shot camera at a bounded resolution when the
    /// worker is idle. Poll `status().export_estimate` or await this receiver
    /// off the UI thread. Repeated requests coalesce; input/editing cancels a
    /// calibration between GPU passes. Selecting an export preset edits nothing.
    pub fn estimate_studio_export(&self, stamp: Stamp, time: Ticks, size: Size, frames: u64)
        -> anyhow::Result<async_channel::Receiver<Result<ExportEstimate, String>>> {
        validate(size, frames).map_err(anyhow::Error::msg)?;
        let (reply, receive) = async_channel::bounded(1);
        let mut state = self.shared.state.lock().unwrap();
        anyhow::ensure!(!state.stop && state.halted.is_none(), "export estimate worker is stopped");
        let v = state.views.get_mut(&self.id).ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        anyhow::ensure!(v.stamp == stamp, "export estimate scene is stale");
        if let Some(old) = v.export_estimate.take() { let _ = old.reply.try_send(Err("export estimate superseded".into())); }
        v.export_estimate = Some(Pending { stamp, time, size, frames, reply });
        v.export_estimate_stamp = Some(stamp);
        v.status.export_estimate = ExportEstimateState::Queued;
        state.notify();
        drop(state);
        let _ = self.shared.wake.try_send(());
        Ok(receive)
    }
}
fn validate(size: Size, frames: u64) -> Result<(), String> {
    if size.width == 0 || size.height == 0 || size.width > 16384 || size.height > 16384 || frames == 0 || frames > 1_000_000_000 {
        return Err("export estimate needs a valid output size and frame count".into());
    }
    Ok(())
}
fn calibration_size(size: Size) -> Size {
    let scale = (320. / size.width as f64).min(180. / size.height as f64).min(1.);
    Size::new((size.width as f64 * scale).round().max(1.) as u32, (size.height as f64 * scale).round().max(1.) as u32)
}
#[derive(Clone, Copy, Debug)]
struct Sample { samples: u32, trace: f64, oidn: f64, guides: f64, prepare_post: f64 }
fn project(preset: &str, min: u32, max: u32, cold: Sample, warm: Sample, area: f64, frames: u64) -> Result<PresetEstimate, String> {
    for v in [cold.trace, warm.trace, warm.oidn, warm.guides, warm.prepare_post, area] {
        if !v.is_finite() || v < 0. { return Err("invalid measured export timing".into()); }
    }
    if warm.samples == 0 || cold.samples == 0 || min == 0 || max < min || area <= 0. { return Err("invalid export estimate sample policy".into()); }
    let rate = warm.trace / warm.samples as f64;
    let photon = if matches!(preset, "production" | "final") {
        (cold.trace - rate * cold.samples as f64).max(0.)
    } else { 0. };
    let trace = Milliseconds { min: rate * min as f64 * area, max: rate * max as f64 * area };
    let oidn = warm.oidn * area;
    let guides = warm.guides * area;
    let prepare_post = warm.prepare_post; // Scene preparation does not scale with image area.
    let fixed = photon + oidn + guides + prepare_post;
    let frame = Milliseconds { min: trace.min + fixed, max: trace.max + fixed };
    let total = Milliseconds { min: frame.min * frames as f64, max: frame.max * frames as f64 };
    Ok(PresetEstimate { preset: preset.into(), min_samples: min, max_samples: max, trace_ms: trace, photon_setup_ms: photon, oidn_ms: oidn, guides_and_readback_ms: guides, prepare_and_post_ms: prepare_post, frame_ms: frame, total_ms: total })
}

#[cfg(feature = "kernel")]
pub(crate) fn calibrate(
    device: wgpu::Device, queue: wgpu::Queue, scene: &render::Scene, time: Ticks, size: Size, frames: u64,
    definitions: pm_protocol::definitions::DefinitionSetSnapshot,
    fonts: Option<&pm_render::text::host::RenderFonts>,
    progress: &dyn Fn(usize) -> bool,
) -> Result<ExportEstimate, String> {
    use pm_studio3d_engine::{compositor::Source, extract::{FinalQuality, SceneSource}, service::{Denoise, StudioService, Tier}};
    validate(size, frames)?;
    let sample_size = calibration_size(size);
    let request = studio::SceneRequest { composition: scene.comp.clone(), revision: scene.doc.revision(), ticks: time.get(), width: sample_size.width, height: sample_size.height };
    let mut capture = pm_render::Renderer::new(device.clone(), queue.clone());
    let source = studio::DocumentSource::new(scene.doc.clone(), scene.media.as_ref().clone())
        .with_native_fonts(fonts).with_capture_definitions(definitions);
    struct Policy<'a>(&'a studio::DocumentSource, FinalQuality);
    impl SceneSource for Policy<'_> {
        fn scene(&self, r: &studio::SceneRequest) -> Result<studio::SceneInput, String> { self.0.scene(r) }
        fn auxiliary_layers(&self, r: &studio::SceneRequest) -> Result<Vec<pm_studio3d_engine::extract::AuxiliaryLayer>, String> { self.0.auxiliary_layers(r) }
        fn layer_failures(&self, r: &studio::SceneRequest) -> Result<Vec<pm_render::LayerFailure>, String> { self.0.layer_failures(r) }
        fn final_quality(&self, _: &studio::SceneRequest) -> Result<Option<FinalQuality>, String> { Ok(Some(self.1)) }
    }
    let mut presets = Vec::new();
    let mut warnings = Vec::new();
    for (i, name) in ["draft", "preview", "production", "final"].into_iter().enumerate() {
        if !progress(i * 2) { return Err("export estimate cancelled".into()); }
        // Each preset pays for its own photon bootstrap. Sharing Production's
        // fixed map would falsely give Final a zero setup cost.
        let mut service = StudioService::with_device(device.clone(), queue.clone(), sample_size.width, sample_size.height)?;
        let q = pm_scene3d::quality::preset(name).map_err(|e|e.to_string())?;
        let mut samples = Vec::new();
        for (j, spp) in [8, 16].into_iter().enumerate() {
            if !progress(i * 2 + j) { return Err("export estimate cancelled because the scene changed or input resumed".into()); }
            let tier = Tier::Final { samples: spp, salt: 0, denoise: Denoise::Oidn };
            if source.needs_materials(&request)? { source.prepare_materials(&mut capture, &request, tier)?; }
            if !source.splat_layers(&request)?.is_empty() {
                return Err("Gaussian splat export estimates are not available".into());
            }
            let policy = Policy(&source, FinalQuality { min_samples: spp, max_samples: spp, noise_threshold: 0., max_bounces: q["maxBounces"].as_u64().unwrap() as u32, caustics: q["caustics"] == true, denoise: true, half_resolution_guides: name != "final" });
            let frame = service.render(&policy, request.clone(), tier)?;
            if frame.layers.is_empty() { return Err("composition has no Studio contribution to estimate".into()); }
            for f in &frame.failures {
                let warning = format!("{}: {}", f.layer, f.message);
                if !warnings.contains(&warning) { warnings.push(warning); }
            }
            let t = frame.timings;
            samples.push(Sample { samples: frame.samples, trace: t.pipeline.trace_ms, oidn: t.pipeline.denoise_ms, guides: t.pipeline.guides_ms + t.pipeline.guide_readback_ms, prepare_post: t.prepare_ms + t.pipeline.post_ms });
        }
        let area = size.width as f64 * size.height as f64 / (sample_size.width as f64 * sample_size.height as f64);
        presets.push(project(name, q["minSamples"].as_u64().unwrap() as u32, q["maxSamples"].as_u64().unwrap() as u32, samples[0], samples[1], area, frames)?);
    }
    if !progress(8) { return Err("export estimate scene changed before completion".into()); }
    Ok(ExportEstimate { composition: scene.comp.clone(), revision: scene.doc.revision(), ticks: time.get(), width: size.width, height: size.height, frames, calibration_width: sample_size.width, calibration_height: sample_size.height, extrapolated: sample_size != size, presets, warnings, excludes: vec!["encode and file I/O".into(), "2D composition effects and material-source capture".into(), "animation variation beyond the sampled frame".into()] })
}

pub(crate) fn run_one(engine: &mut render::Engine, shared: &Arc<Shared>) {
    let work = {
        let mut state = shared.state.lock().unwrap();
        state.views.iter_mut().find_map(|(id, v)| {
            let p = v.export_estimate.take()?;
            Some((*id, p, render::Scene { doc: v.config.snapshot.document.clone(), comp: v.config.composition.clone(), media: v.paths.clone(), content: v.content }, v.resources.clone()))
        })
    };
    let Some((id, p, scene, resources)) = work else { return; };
    let progress = |completed| {
        let mut state = shared.state.lock().unwrap();
        if state.stop || state.halted.is_some() { return false; }
        let Some(v) = state.views.get_mut(&id) else { return false; };
        if v.stamp != p.stamp || v.export_estimate.is_some() || v.desired_rate.num != 0 { return false; }
        v.status.export_estimate = ExportEstimateState::Measuring { completed, total: 8 };
        state.notify();
        true
    };
    engine.set_resources(resources);
    let result = engine.estimate_studio_export(&scene, p.time, p.size, p.frames, &progress).map_err(|e|e.to_string());
    let mut state = shared.state.lock().unwrap();
    let current = state.views.get_mut(&id).filter(|v| v.stamp == p.stamp && v.export_estimate.is_none());
    let result = if let Some(v) = current {
        v.status.export_estimate = match &result { Ok(estimate) => ExportEstimateState::Ready { estimate: estimate.clone() }, Err(message) => ExportEstimateState::Failed { message: message.clone() } };
        result
    } else { Err("export estimate scene changed before completion".into()) };
    state.notify();
    let _ = p.reply.try_send(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimate_separates_fixed_photons_from_pixel_and_sample_work() {
        let cold = Sample { samples: 8, trace: 18., oidn: 7., guides: 2., prepare_post: 3. };
        let warm = Sample { samples: 16, trace: 16., ..cold };
        let p = project("production", 32, 256, cold, warm, 4., 10).unwrap();
        assert_eq!(p.photon_setup_ms, 10.);
        assert_eq!((p.trace_ms.min, p.trace_ms.max), (128., 1024.));
        assert_eq!(p.oidn_ms, 28.);
        assert_eq!((p.frame_ms.min, p.frame_ms.max), (177., 1073.));
        assert_eq!(p.total_ms.max, 10730.);
    }
    #[test]
    fn calibration_is_bounded_and_bad_measurements_are_refused() {
        assert_eq!(calibration_size(Size::new(1920,1080)), Size::new(320,180));
        assert_eq!(calibration_size(Size::new(64,64)), Size::new(64,64));
        assert!(validate(Size::new(0,1080),1).is_err());
        assert!(validate(Size::new(1920,1080),0).is_err());
        let s = Sample { samples: 0, trace: 1., oidn: 1., guides: 1., prepare_post: 1. };
        assert!(project("final",512,1024,s,s,1.,1).is_err());
        let s = Sample { samples: 8, trace: f64::NAN, ..s };
        assert!(project("final",512,1024,s,s,1.,1).is_err());
    }
}
