//! The effect stage of the compositing graph (plan section 8, "effect
//! pass"; section 11).
//!
//! Effects themselves (the kernel's effect table, the GLSL pipeline, the
//! built-in effects) belong to the effects lane. The compositor owns where
//! they run: after a layer's masks and before its matte, and for an
//! adjustment layer on the accumulator beneath it, with the accumulator as
//! the backdrop (`runEffects(L, T, src, W, H, backdrop)`). The two meet at
//! [`EffectHost`]: the renderer asks the host to prepare a layer's effects
//! when the plan is built, and runs the prepared passes when the frame is
//! recorded.
//!
//! Without a host, a layer with renderable effects is unsupported
//! ([`crate::Feature::Effects`] or [`crate::Feature::Adjustment`]) and the
//! composition routes to the legacy engine, as before.

use std::fmt;
use std::sync::Arc;

use pm_eval::{JsResult, Scope};
use pm_gpu::{Target, TargetPool};
use pm_model::{JsString, Node};
use pm_protocol::color::{Precision, WorkingEncoding};

use crate::capability::Feature;
use crate::exec::Region;

/// How far an effect chain reads beyond the pixel it writes, in output
/// pixels (section 8: "region of interest with effect padding;
/// bounds-expanding effects grow up to a limit").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Padding {
    Pixels(u32),
    /// Any pixel of the frame (the limit): the chain renders over the
    /// whole output.
    Full,
}

impl Padding {
    pub const NONE: Padding = Padding::Pixels(0);

    /// The larger reach.
    pub fn max(self, other: Padding) -> Padding {
        std::cmp::max(self, other)
    }

    /// Chained reads add up.
    pub fn then(self, next: Padding) -> Padding {
        match (self, next) {
            (Padding::Pixels(a), Padding::Pixels(b)) => Padding::Pixels(a.saturating_add(b)),
            _ => Padding::Full,
        }
    }
}

/// What the plan builder tells a host about the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectFrame {
    /// The target's full size (output pixels of this level).
    pub width: u32,
    pub height: u32,
    /// Output pixels per composition pixel.
    pub scale: (f64, f64),
    /// Composition time in seconds.
    pub time: f64,
}

/// A layer's effects, prepared at plan time.
#[derive(Clone)]
pub struct PreparedEffects {
    /// How far the chain reads beyond each output pixel, in total.
    pub padding: Padding,
    /// Some effect reads the accumulated stack beneath the layer
    /// (`def.backdrop`).
    pub backdrop: bool,
    pub passes: Arc<dyn EffectPasses>,
}

impl fmt::Debug for PreparedEffects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedEffects")
            .field("padding", &self.padding)
            .field("backdrop", &self.backdrop)
            .field("key", &self.passes.key())
            .finish()
    }
}

impl PartialEq for PreparedEffects {
    fn eq(&self, other: &Self) -> bool {
        self.padding == other.padding
            && self.backdrop == other.backdrop
            && self.passes.key() == other.passes.key()
    }
}

/// The GPU state an effect chain records into.
pub struct EffectContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub pool: &'a mut TargetPool,
    pub precision: Precision,
    pub working: WorkingEncoding,
    /// The full output size of this level, in pixels.
    pub full: (u32, u32),
    /// Targets to return to the pool after the frame is submitted: every
    /// target a chain takes from `pool`, the one it returns included.
    pub taken: &'a mut Vec<Target>,
    /// The renderer's own effect state (`crate::effects`: programs, the
    /// watchdog's timestamps), for the chains it prepares itself.
    pub(crate) native: Option<crate::effects::exec::Native<'a>>,
}

/// A target covering a rectangle of the frame. Targets are in GL row order
/// (row 0 is the bottom of `rect`), as every pm-render target.
pub struct Covered<'a> {
    pub target: &'a Target,
    /// Top-down output pixels.
    pub rect: Region,
}

/// A prepared effect chain.
pub trait EffectPasses: Send + Sync {
    /// Identifies the chain and every input that changes its pixels (for
    /// plan equality and render cache keys).
    fn key(&self) -> Vec<u8>;

    /// Records the chain on `input`, writing a new target that covers the
    /// same rectangle. `backdrop` is the accumulated stack beneath the
    /// layer when [`PreparedEffects::backdrop`] (for an adjustment layer,
    /// the same stack as `input`). `Ok(None)` when no effect ran (today's
    /// "a missing or failed effect program leaves the composition
    /// unchanged"); `Err` fails the layer.
    fn record(
        &self,
        cx: &mut EffectContext<'_>,
        input: Covered<'_>,
        backdrop: Option<Covered<'_>>,
    ) -> Result<Option<Target>, String>;
}

/// A layer's active transition, prepared at plan time (section 8, the
/// transition node). It runs after the layer's composite, from the stack
/// beneath the layer to the stack with it (`runTransition`), over the whole
/// frame.
#[derive(Clone)]
pub struct PreparedTransition {
    pub passes: Arc<dyn TransitionPasses>,
}

impl fmt::Debug for PreparedTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTransition")
            .field("key", &self.passes.key())
            .finish()
    }
}

impl PartialEq for PreparedTransition {
    fn eq(&self, other: &Self) -> bool {
        self.passes.key() == other.passes.key()
    }
}

/// A prepared transition.
pub trait TransitionPasses: Send + Sync {
    /// As [`EffectPasses::key`].
    fn key(&self) -> Vec<u8>;

    /// Records the transition from `before` (the stack beneath the layer)
    /// and `with_layer` (the stack with the layer composited), both
    /// covering the frame. `Ok(None)` when the program cannot run (today's
    /// `acc = withLayer`); `Err` fails the layer.
    fn record(
        &self,
        cx: &mut EffectContext<'_>,
        before: Covered<'_>,
        with_layer: Covered<'_>,
    ) -> Result<Option<Target>, String>;
}

/// Prepares a layer's effects.
pub trait EffectHost: Send + Sync {
    /// The renderable effects of `layer` at `frame.time`, or `Ok(None)`
    /// when none renders (every effect off or missing). The inner `Err`
    /// names what this host cannot draw, which routes the composition; the
    /// outer one is a script error evaluating the layer (a layer failure).
    fn prepare<'d>(
        &self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        frame: EffectFrame,
    ) -> JsResult<Result<Option<PreparedEffects>, Feature>>;

    /// Whether a transition type is registered (`PM.transitionDef`), for
    /// `activeTransition` and the group boundary rule.
    fn transition_known(&self, _ty: &JsString) -> bool {
        true
    }

    /// The active transition of `layer` at `frame.time`, or `Ok(None)` when
    /// none is active. A host that draws no transitions routes them.
    fn prepare_transition<'d>(
        &self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        frame: EffectFrame,
    ) -> JsResult<Result<Option<PreparedTransition>, Feature>> {
        let active = scope.active_transition(layer, frame.time, &|ty| self.transition_known(ty))?;
        Ok(match active {
            Some(_) => Err(Feature::Transition),
            None => Ok(None),
        })
    }

    /// The extension layer definitions this host draws fragment layers
    /// with (`crate::shader_layers`); `None` routes them as before.
    fn layer_definitions(&self) -> Option<&crate::effects::Effects> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_adds_along_a_chain_and_takes_the_larger_reach() {
        assert_eq!(
            Padding::Pixels(3).then(Padding::Pixels(4)),
            Padding::Pixels(7)
        );
        assert_eq!(Padding::Pixels(3).then(Padding::Full), Padding::Full);
        assert_eq!(
            Padding::Pixels(3).max(Padding::Pixels(9)),
            Padding::Pixels(9)
        );
        assert_eq!(Padding::Full.max(Padding::Pixels(9)), Padding::Full);
    }
}
