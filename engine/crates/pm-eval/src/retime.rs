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
//! # The integration step (rule 5)
//!
//! The legacy code integrates with a midpoint rule at a step of
//! `1 / (max(30, PM.proj.fps) × 2)` seconds: twice the *open* composition's
//! rate, at least 60 Hz, so a nested layer's source time depends on which
//! composition is open (appendix A item 9). The engine integrates in ticks
//! at the *owning* composition's exact rate ([`Retiming::Engine`]): one
//! midpoint sample per composition frame of layer-local time, frame
//! boundaries at `frame_start(n)` (section 5), plus a partial step to the
//! requested time. [`Retiming::Legacy`] reproduces the TypeScript exactly for
//! parity comparisons.

use std::collections::HashMap;

use pm_model::{JsString, Node};
use pm_time::{Rate, TICKS_PER_SECOND};

use crate::js_sem::{JsResult, JsValue};
use crate::node;
use crate::scope::{Scope, js_max};

/// Which integration step variable speed uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Retiming {
    /// Section 5 rule 5: one step per frame of the owning composition.
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
    pub fn source_time(&mut self, layer: &'d Node, time: f64, cache: Option<&mut RetimeCache>) -> JsResult<f64> {
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
        match mode {
            Retiming::Legacy => {
                let fps = node::to_number(self.project.member("fps"));
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
            Retiming::Engine => {
                let rate = self
                    .comp
                    .rate()
                    .map(|r| r.rate)
                    .unwrap_or_else(|| Rate::integer(30).expect("30 fps is a rate"));
                let ticks = |s: f64| -> Option<i64> {
                    match self.comp.rate() {
                        Some(r) => r.time(s).ok().map(|t| t.ticks),
                        None => pm_time::ticks_from_seconds(s).ok(),
                    }
                };
                let (Some(t_ticks), Some(f_ticks)) = (ticks(time), ticks(from_n)) else {
                    return Ok(f64::NAN);
                };
                let local_ticks = (t_ticks - f_ticks).max(0);
                let full = rate.frame_of(local_ticks);
                let tps = TICKS_PER_SECOND as f64;
                let sums = match cache {
                    Some(c) => c.sums(key((mode, rate.num() as u64, rate.den() as u64), self.comp)),
                    None => &mut scratch,
                };
                let start_of = |n: i64| rate.frame_start(n).unwrap_or(i64::MAX);
                while (sums.len() as i64) - 1 < full {
                    let i = (sums.len() - 1) as i64;
                    let (a, b) = (start_of(i), start_of(i + 1));
                    let mid = (a as f64 + b as f64) / 2.0 / tps;
                    let v = self.speed_at(layer, speed, from_n + mid)?;
                    let last = *sums.last().unwrap();
                    sums.push(last + v * ((b - a) as f64 / tps));
                }
                let base = sums[full as usize];
                let a = start_of(full);
                let remainder = (local_ticks - a) as f64;
                let mid = (a as f64 + remainder / 2.0) / tps;
                let v = self.speed_at(layer, speed, from_n + mid)?;
                Ok(base + v * (remainder / tps) + trim)
            }
        }
    }

    /// `Number(evaluatedValue(PM, layer, speed, t, 'c.speed'))`.
    fn speed_at(&mut self, layer: &'d Node, speed: Option<&'d Node>, at: f64) -> JsResult<f64> {
        Ok(self.evaluated(layer, speed, at, "c.speed")?.to_number())
    }
}

