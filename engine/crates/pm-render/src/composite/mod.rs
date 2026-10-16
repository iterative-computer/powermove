//! Compositing (plan sections 8, 9, 10; Phase 3a): layer masks, track
//! mattes, the nine blend modes, adjustment layers, styled groups as
//! compositing boundaries, linear blending, 16- and 32-bit precision,
//! region of interest with effect padding, and isolation.
//!
//! Today's compositor (`legacy/gl/compositor.ts` `renderProject`) draws a
//! layer straight into the accumulator when it has no masks, effects, blend
//! mode, motion blur, transition, or matte (the fast path). Otherwise the
//! layer goes through its own target, in this order:
//!
//! 1. the layer's content (with its opacity; a group's members with the
//!    group's opacity left for the composite),
//! 2. masks (`applyMasks`: analytic coverage, multiplied in),
//! 3. effects (`runEffects`),
//! 4. the track matte (`applyTrackMatte`: the matte source rendered alone,
//!    alpha or Rec.709 luma, optionally inverted, multiplied in),
//! 5. the composite: premultiplied source-over for normal, else
//!    `FRAG_COMPOSITE` (unpremultiply both, blend, mix by source alpha,
//!    re-premultiply by the union alpha).
//!
//! An adjustment layer runs its effects on the accumulator, masks the
//! result, and composites it over the accumulator with its opacity and
//! blend mode through `FRAG_COMPOSITE` (`compositeAdjustment`).
//!
//! The plan carries these steps as [`Node`]s (a [`crate::plan::Content`]
//! variant); [`exec`] runs them, splitting the target's render pass at
//! each node. Every intermediate keeps the legacy storage roundings (each
//! pass writes the working format), so Standard output matches today's.
//!
//! Design notes: `docs/engine/composite.md`.

pub mod build;
pub mod color;
pub mod effects;
pub(crate) mod exec;
pub(crate) mod gpu;
pub mod region;

use pm_protocol::color::BlendMode;

use crate::plan::{CompPlan, Draw};

pub use color::{ColorSettings, project_color};
pub use effects::{EffectHost, EffectPasses, Padding, PreparedEffects};

/// Today's `MASK_MAX`: the parametric route evaluates at most 8 masks.
pub const MASK_MAX: usize = pm_vector::mask::MAX_PARAMETRIC_MASKS;

/// Today's matte recursion limit (`opt.matteDepth >= 8`).
pub const MATTE_MAX_DEPTH: u32 = 8;

/// A compositing step that needs the layer in its own target, or the
/// accumulator itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Layer(LayerNode),
    Adjustment(AdjustmentNode),
}

impl Node {
    /// The layer this node belongs to.
    pub fn layer(&self) -> &str {
        match self {
            Node::Layer(l) => &l.layer,
            Node::Adjustment(a) => &a.layer,
        }
    }
}

/// A layer through its own target (today's slow path).
#[derive(Debug, Clone, PartialEq)]
pub struct LayerNode {
    pub layer: String,
    /// The layer's content, drawn into a cleared target with premultiplied
    /// source-over (`drawContent` with the layer's opacity; for a group,
    /// its compositing boundary at opacity 1).
    pub content: Vec<Draw>,
    pub masks: Option<Masks>,
    pub effects: Option<PreparedEffects>,
    pub matte: Option<Matte>,
    pub blend: BlendMode,
    /// Opacity applied at the composite (`compositeAlpha`: the group's
    /// opacity for a group, 1 otherwise).
    pub alpha: f64,
}

/// An adjustment layer: its effects over the accumulator beneath it.
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustmentNode {
    pub layer: String,
    pub effects: PreparedEffects,
    pub masks: Option<Masks>,
    pub blend: BlendMode,
    /// The layer's opacity.
    pub alpha: f64,
}

/// A layer's masks on today's parametric route, ready for the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct Masks {
    /// Output pixels to layer pixels (`u_inv`), a column-major 3×3 matrix:
    /// the inverse world matrix, or for a 3D layer the inverse of its
    /// projected plane.
    pub inverse: [f64; 9],
    pub masks: pm_vector::ParametricMasks,
}

/// A track matte.
#[derive(Debug, Clone, PartialEq)]
pub struct Matte {
    /// Rec.709 luma of the premultiplied, E-encoded matte (else its alpha).
    pub luma: bool,
    pub inverted: bool,
    /// The matte pass: the source layer rendered alone into a transparent
    /// target in the layer's pixel space (`renderProject` with
    /// `mattePass`).
    pub source: Box<CompPlan>,
}

/// Whether a plan draws anything through a [`Node`].
pub fn has_nodes(plan: &CompPlan) -> bool {
    plan.draws
        .iter()
        .any(|d| matches!(d.content, crate::plan::Content::Composite(_)))
}
