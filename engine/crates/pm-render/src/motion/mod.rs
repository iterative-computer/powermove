//! Motion blur (plan section 8, "Motion blur"; section 5 rule 8).
//!
//! Today's compositor (`legacy/gl/compositor.ts`, `renderProject`) blurs a
//! layer whose `mblur` switch is on by drawing it `n` times into its own
//! cleared target, at the times `T + ((s + ½)/n − ½) × shutter / fps`, each
//! draw at `opacity / n` with additive blending (`ONE, ONE`), and then
//! drawing that target into the stack with source-over at opacity 1. A
//! group renders its members' compositing pass at every sample time (with
//! motion blur off inside) and accumulates the passes at `1/n`, then draws
//! the sum at the group's opacity. The engine builds the same graph: one
//! [`crate::plan::Source::Accumulate`] target per blurred layer whose
//! draws are the samples, accumulated in the working encoding at the
//! frame's float precision (`rgba16float` as today's `RGBA16F` target, or
//! `rgba32float`), with no adaptive reduction.
//!
//! - **Sample counts** come from the composition's `motionBlurSamples` for
//!   the output kind ([`MotionBlur::Kind`]), defaulting to today's: 16 for
//!   opaque exports, 20 for alpha (and WebM) exports, 6 for preview while
//!   playing, 12 for preview when paused. [`MotionBlur::Samples`] asks for
//!   a count directly (parity captures, tests, the scope tool).
//! - **Shutter.** `composition.shutter || 0.5` frames of the root
//!   composition (today's `PM.proj.shutter || .5`: a shutter of 0 means
//!   half a frame), divided by the rate of the composition the layer is in
//!   (`proj.fps` at each nesting level).
//! - **Video in samples.** Today a video layer drawn at a sample time shows
//!   the frame selected at the center time (the exporter prepares frames
//!   only for the center time; the preview reads the playhead's clip
//!   times), while its transform moves. The engine keeps that: video
//!   sources inside a blurred sample select their frame at the center time
//!   of their composition level.
//! - **Opacity** is evaluated at the center time and divided among the
//!   samples, as today; skip rules (active, opacity ≤ 0.001) apply at the
//!   center time.
//!
//! Design notes and measurements: `docs/engine/motion.md`.

use pm_eval::node;
use pm_eval::{Comp, JsValue};
use pm_model::{Node, NodeKind};
use pm_protocol::frame::MotionBlurKind;

pub(crate) mod plan;

/// Layer types whose motion blur is drawn and compared with today's: every
/// type the renderer draws (shapes and text re-rasterize at each sample's
/// density, 3D object layers draw their scene at each sample time,
/// adjustment layers are never blurred, as today).
pub const DRAWN_TYPES: [&str; 11] = [
    "solid",
    "image",
    "video",
    "precomp",
    "group",
    "shape",
    "text",
    "captions",
    "shader",
    "extension",
    "adjustment",
];

/// How a render applies motion blur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MotionBlur {
    /// Layers with motion blur draw sharp (today's `mblur: false`).
    #[default]
    Off,
    /// The count the composition's `motionBlurSamples` gives the output
    /// kind (contract 2's `MotionBlurKind`), today's when absent.
    Kind(MotionBlurKind),
    /// An explicit sample count (at least 1).
    Samples(u32),
}

/// Today's sample counts: `renderOpaqueFrame` (16), the alpha and WebM
/// export paths (20), and the viewer (`mbSamples: PM.playing ? 6 : 12`).
pub const fn default_samples(kind: MotionBlurKind) -> Option<u32> {
    match kind {
        MotionBlurKind::Export => Some(16),
        MotionBlurKind::AlphaExport => Some(20),
        MotionBlurKind::PreviewPlaying => Some(6),
        MotionBlurKind::PreviewPaused => Some(12),
        MotionBlurKind::Off => None,
    }
}

/// The largest sample count a document or request may ask for.
pub const MAX_SAMPLES: u32 = 256;

/// The key of a kind in `motionBlurSamples`.
pub const fn kind_key(kind: MotionBlurKind) -> &'static str {
    match kind {
        MotionBlurKind::Export => "export",
        MotionBlurKind::AlphaExport => "alphaExport",
        MotionBlurKind::PreviewPlaying => "previewPlaying",
        MotionBlurKind::PreviewPaused => "previewPaused",
        MotionBlurKind::Off => "off",
    }
}

/// The count `motionBlurSamples` (an object keyed by [`kind_key`]) gives a
/// kind: a finite number ≥ 1 is floored and capped at [`MAX_SAMPLES`];
/// anything else (missing, not a number, below 1) keeps today's count.
pub fn samples_for(composition: &Node, kind: MotionBlurKind) -> Option<u32> {
    let default = default_samples(kind)?;
    let field = node::member(Some(composition), "motionBlurSamples")
        .filter(|f| matches!(f.kind(), NodeKind::Object(_)));
    let value = node::member(field, kind_key(kind)).map(|v| node::js_value_opt(Some(v)));
    Some(match value {
        Some(JsValue::Number(n)) if n.is_finite() && n >= 1.0 => {
            (n.floor() as u32).min(MAX_SAMPLES)
        }
        _ => default,
    })
}

/// A build's resolved motion blur: samples per blurred layer and the
/// shutter in frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blur {
    pub samples: u32,
    /// Shutter length in frames of the layer's composition.
    pub shutter_frames: f64,
}

/// `composition.shutter || .5` (today's `PM.proj.shutter || .5`): a
/// missing, zero, or NaN shutter is half a frame.
pub fn shutter_frames(composition: &Node) -> f64 {
    let v = node::to_number(node::member(Some(composition), "shutter"));
    if v == 0.0 || v.is_nan() { 0.5 } else { v }
}

/// The blur a request asks of `composition` (the root of the render).
pub fn resolve(composition: &Comp<'_>, request: MotionBlur) -> Option<Blur> {
    let samples = match request {
        MotionBlur::Off => return None,
        MotionBlur::Kind(kind) => samples_for(composition.node(), kind)?,
        MotionBlur::Samples(n) => n.clamp(1, MAX_SAMPLES),
    };
    Some(Blur {
        samples,
        shutter_frames: shutter_frames(composition.node()),
    })
}

/// Sample `s` of `n`'s offset from the center time, in seconds:
/// `((s + .5) / n − .5) × (shutter / fps)` in today's expression order.
pub fn sample_offset(s: u32, n: u32, shutter_frames: f64, fps: f64) -> f64 {
    let shutter = shutter_frames / fps;
    ((f64::from(s) + 0.5) / f64::from(n) - 0.5) * shutter
}

/// The rate dividing the shutter at one composition level: today's
/// `proj.fps` of the composition being rendered. A composition without a
/// usable `fps` falls back to the rate expressions see (`comp.fps ||
/// PM.proj.fps`); today such a composition would sample at NaN times.
pub fn level_fps(comp: &Comp<'_>) -> f64 {
    let fps = node::to_number(comp.member("fps"));
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        let f = comp.expression_fps().to_number();
        if f.is_finite() && f > 0.0 { f } else { 30.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::scene::Shutter;

    fn parse(s: &str) -> Node {
        Node::from_value(pm_model::json::parse(s).unwrap(), 0)
    }

    #[test]
    fn offsets_match_the_compositor_and_the_shutter_contract() {
        // compositor.ts: dt = ((s + .5) / n − .5) × shutter / fps.
        for n in [1u32, 2, 6, 10, 12, 16, 20, 64] {
            for fps in [23.976, 24.0, 29.97, 30.0, 60.0] {
                for shutter in [0.25, 0.5, 0.75, 1.0, 2.0] {
                    let contract = Shutter::from_frames(shutter).sample_offsets(n);
                    for s in 0..n {
                        let ours = sample_offset(s, n, shutter, fps);
                        let today = ((s as f64 + 0.5) / n as f64 - 0.5) * (shutter / fps);
                        assert_eq!(ours.to_bits(), today.to_bits(), "n={n} s={s}");
                        assert!((ours - contract[s as usize] / fps).abs() < 1e-15);
                    }
                }
            }
        }
        // Symmetric around the center time; one sample sits on it.
        assert_eq!(sample_offset(0, 1, 0.5, 30.0), 0.0);
        let sum: f64 = (0..16).map(|s| sample_offset(s, 16, 0.5, 30.0)).sum();
        assert!(sum.abs() < 1e-15);
    }

    #[test]
    fn sample_counts_default_to_todays() {
        let c = parse(r#"{"id":"C"}"#);
        assert_eq!(samples_for(&c, MotionBlurKind::Export), Some(16));
        assert_eq!(samples_for(&c, MotionBlurKind::AlphaExport), Some(20));
        assert_eq!(samples_for(&c, MotionBlurKind::PreviewPlaying), Some(6));
        assert_eq!(samples_for(&c, MotionBlurKind::PreviewPaused), Some(12));
        assert_eq!(samples_for(&c, MotionBlurKind::Off), None);
        let c = parse(
            r#"{"motionBlurSamples":{"export":32,"alphaExport":0,"previewPlaying":"8","previewPaused":7.9}}"#,
        );
        assert_eq!(samples_for(&c, MotionBlurKind::Export), Some(32));
        assert_eq!(
            samples_for(&c, MotionBlurKind::AlphaExport),
            Some(20),
            "below 1"
        );
        assert_eq!(
            samples_for(&c, MotionBlurKind::PreviewPlaying),
            Some(6),
            "not a number"
        );
        assert_eq!(samples_for(&c, MotionBlurKind::PreviewPaused), Some(7));
        let c = parse(r#"{"motionBlurSamples":{"export":100000}}"#);
        assert_eq!(samples_for(&c, MotionBlurKind::Export), Some(MAX_SAMPLES));
        let c = parse(r#"{"motionBlurSamples":[1,2]}"#);
        assert_eq!(samples_for(&c, MotionBlurKind::Export), Some(16));
    }

    #[test]
    fn shutter_zero_means_half_a_frame() {
        assert_eq!(shutter_frames(&parse(r#"{}"#)), 0.5);
        assert_eq!(shutter_frames(&parse(r#"{"shutter":0}"#)), 0.5);
        assert_eq!(shutter_frames(&parse(r#"{"shutter":0.75}"#)), 0.75);
        assert_eq!(shutter_frames(&parse(r#"{"shutter":"1.5"}"#)), 1.5);
    }
}
