//! Retiming: the source time of video, precomp, and audio layers
//! (`legacy/core/retiming.ts`, plan section 5 rule 5, appendix A item 9).
//!
//! One mapping serves playback, seeking, nested compositions, and export:
//!
//! 1. With time remapping on (`d.timeRemap`) and a `d.sourceTime` channel,
//!    the source time is that channel (`|| 0`).
//! 2. With a constant speed, `local × speed + trim`.
//! 3. With an animated or expression speed, the speed is integrated over
//!    layer-local time from the layer start, so changing a rate never moves
//!    the time already elapsed.
//!
//! # The integration step (rule 5, amendment 16)
//!
//! The legacy code integrates with a midpoint rule at a step of
//! `1 / (max(30, PM.proj.fps) × 2)` seconds: twice the *open* composition's
//! rate, at least 60 Hz, so a nested layer's source time depends on which
//! composition is open (appendix A item 9). Amendment 16 keeps the same rule
//! with the *owning* composition's rate ([`Retiming::Engine`]: two midpoint
//! samples per frame of `comp.fps || PM.proj.fps`, the rate expressions
//! see, at no less than 60 Hz). A root composition then matches today bit
//! for bit; a nested composition keeps its own rate whichever composition
//! is open. [`Retiming::Legacy`] reproduces the TypeScript exactly for
//! parity comparisons.

use std::collections::HashMap;

use pm_model::{JsString, Node};

use crate::js_sem::{JsResult, JsValue};
use crate::node;
use crate::scope::{Scope, js_max};

/// Which integration step variable speed uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Retiming {
    /// Amendment 16: two midpoint steps per frame of the owning
    /// composition, at no less than 60 Hz.
    #[default]
    Engine,
    /// `retiming.ts`: `1 / (max(30, PM.proj.fps) × 2)` seconds.
    Legacy,
}

/// Prefix sums of integrated speed, reused across calls at one document
/// revision (the legacy `integrals` cache, keyed explicitly).
#[derive(Default, Debug)]
pub struct RetimeCache {
    entries: HashMap<Key, Vec<f64>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    root: usize,
    revision: u64,
    comp: JsString,
    layer: JsString,
    from: u64,
    step: (Retiming, u64, u64),
}

impl RetimeCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn sums(&mut self, key: Key) -> &mut Vec<f64> {
        if self.entries.len() >= 1024 && !self.entries.contains_key(&key) {
            self.entries.clear();
        }
        self.entries.entry(key).or_insert_with(|| vec![0.0])
    }
}

impl<'d> Scope<'_, 'd> {
    /// `sourceTime(PM, layer, time)` with the engine's integration step.
    pub fn source_time(
        &mut self,
        layer: &'d Node,
        time: f64,
        cache: Option<&mut RetimeCache>,
    ) -> JsResult<f64> {
        self.source_time_with(layer, time, Retiming::Engine, cache)
    }

    /// `sourceTime` with a chosen integration step.
    pub fn source_time_with(
        &mut self,
        layer: &'d Node,
        time: f64,
        mode: Retiming,
        cache: Option<&mut RetimeCache>,
    ) -> JsResult<f64> {
        let d = self.ev.resolve_content(self.comp, layer, time)?;
        let get = |k: &str| d.get(&JsString::from(k)).cloned().unwrap_or_default();
        let from = node::js_value_opt(node::member(Some(layer), "from"));
        let local = time - from.to_number();
        let ld = node::get_checked(Some(layer), b"d")?;
        let ld = ld.node();
        if get("timeRemap").truthy() {
            let st = node::get_checked(ld, b"sourceTime")?.node();
            if node::is_property(st) {
                let v = self.channel_p(layer, st, time, "c.sourceTime")?.to_number();
                return Ok(crate::scope::or0(v));
            }
        }
        let speed = node::get_checked(ld, b"speed")?.node();
        let trim = {
            let t = get("trim");
            if t.truthy() { t.to_number() } else { 0.0 }
        };
        let animated = node::is_property(speed)
            && (node::array(node::member(speed, "kf")).is_some_and(|k| !k.is_empty())
                || node::truthy(node::member(speed, "expr")));
        if !animated {
            let s = match get("speed") {
                JsValue::Undefined | JsValue::Null => 1.0,
                v => v.to_number(),
            };
            return Ok(local * s + trim);
        }
        let from_n = from.to_number();
        let end = js_max(0.0, local);
        let mut scratch = vec![0.0];
        let key = |step: (Retiming, u64, u64), cx: &crate::comp::Comp<'_>| Key {
            root: cx.root() as *const Node as usize,
            revision: cx.revision(),
            comp: cx.id().clone(),
            layer: node::to_js_string(node::member(Some(layer), "id")),
            from: from_n.to_bits(),
            step,
        };
        // Amendment 16: two midpoint samples per frame, at no less than
        // 60 Hz, of the owning composition (engine) or of the open one
        // (legacy, `PM.proj.fps`). At the root both are the open
        // composition's rate, so the engine matches today bit for bit there.
        let fps = match mode {
            Retiming::Legacy => node::to_number(self.project.member("fps")),
            Retiming::Engine => self.comp.expression_fps().to_number(),
        };
        let step = 1.0 / (js_max(30.0, fps) * 2.0);
        let sums = match cache {
            Some(c) => c.sums(key((mode, step.to_bits(), 0), self.comp)),
            None => &mut scratch,
        };
        let full = (end / step).floor();
        if !full.is_finite() {
            return Ok(f64::NAN);
        }
        let full_i = full as usize;
        while sums.len() - 1 < full_i {
            let i = (sums.len() - 1) as f64;
            let at = from_n + (i + 0.5) * step;
            let v = self.speed_at(layer, speed, at)?;
            let last = *sums.last().unwrap();
            sums.push(last + v * step);
        }
        let base = sums[full_i];
        let remainder = end - full * step;
        let v = self.speed_at(layer, speed, from_n + full * step + remainder / 2.0)?;
        Ok(base + v * remainder + trim)
    }

    /// `Number(evaluatedValue(PM, layer, speed, t, 'c.speed'))`.
    fn speed_at(&mut self, layer: &'d Node, speed: Option<&'d Node>, at: f64) -> JsResult<f64> {
        Ok(self.evaluated(layer, speed, at, "c.speed")?.to_number())
    }
}
