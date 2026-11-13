//! GPUI side of the motion system (`pm_design::motion`): hover washes and a
//! gliding selection. Owners call [`Motion::frame`] once per render; it steps
//! every value and requests the next frame only while something still moves.
use gpui::{Bounds, Pixels, Window, px};
use pm_design::{
    Tokens,
    motion::{Animated, Clock, POINT_PRECISION, Preset, Spring},
};
use std::{collections::HashMap, hash::Hash, time::Instant};

/// Hover washes keyed by control. The control under the pointer washes in
/// quickly; the one it left for a neighbour vanishes in ~80 ms; a wash left
/// for nothing fades out slowly. Interrupting either keeps its velocity.
pub struct Hovers<K> {
    washes: HashMap<K, Animated<1>>,
    current: Option<K>,
}
impl<K> Default for Hovers<K> {
    fn default() -> Self {
        Self {
            washes: HashMap::new(),
            current: None,
        }
    }
}
impl<K: Copy + Eq + Hash> Hovers<K> {
    /// The pointer entered (`true`) or left `key`.
    pub fn hover(&mut self, key: K, hovered: bool, t: &Tokens) {
        if hovered {
            if let Some(previous) = self.current.replace(key).filter(|p| *p != key)
                && let Some(wash) = self.washes.get_mut(&previous)
            {
                wash.to(0., t.spring(Preset::HoverSwitch));
            }
            self.washes
                .entry(key)
                .or_insert_with(|| Animated::unit(0.))
                .to(1., t.spring(Preset::HoverIn));
        } else {
            if self.current == Some(key) {
                self.current = None;
            }
            if let Some(wash) = self.washes.get_mut(&key)
                && wash.target[0] != 0.
            {
                wash.to(0., t.spring(Preset::HoverOut));
            }
        }
    }
    /// Wash amount, 0–1.
    pub fn amount(&self, key: K) -> f32 {
        self.washes.get(&key).map_or(0., |w| w.get().clamp(0., 1.))
    }
    fn step(&mut self, dt: f32) -> bool {
        let mut moving = false;
        self.washes.retain(|_, w| {
            moving |= w.step(dt);
            w.moving() || w.get() > 0.
        });
        moving
    }
}

/// A selection that slides between items (a segment, a tab): x, y, width,
/// height, springing on `Preset::Layout` from wherever it is.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glide {
    rect: Option<Animated<4>>,
}
impl Glide {
    /// Head for `bounds`; the first target is taken at once.
    pub fn to(&mut self, bounds: Bounds<Pixels>, t: &Tokens) {
        let target = [
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ];
        match &mut self.rect {
            Some(rect) if rect.target != target => {
                rect.animate_to(target, t.spring(Preset::Layout));
            }
            Some(_) => {}
            None => self.rect = Some(Animated::new(target, Spring::SNAP, POINT_PRECISION)),
        }
    }
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.rect.map(|r| {
            let [x, y, w, h] = r.value;
            Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(w), px(h)))
        })
    }
    fn step(&mut self, dt: f32) -> bool {
        self.rect.as_mut().is_some_and(|r| r.step(dt))
    }
}

/// One owner's animated state and frame clock.
pub struct Motion<K> {
    pub hovers: Hovers<K>,
    pub glides: HashMap<K, Glide>,
    /// Surfaces that spring in when they appear and out before they go
    /// (a floating panel): 0 hidden … 1 shown.
    presences: HashMap<K, Animated<1>>,
    clock: Clock,
}
impl<K> Default for Motion<K> {
    fn default() -> Self {
        Self {
            hovers: Hovers::default(),
            glides: HashMap::new(),
            presences: HashMap::new(),
            clock: Clock::default(),
        }
    }
}
impl<K: Copy + Eq + Hash> Motion<K> {
    /// How present `key` is (0–1): a surface seen for the first time starts
    /// at 0 and springs in (`Preset::Enter`); one marked `shown: false`
    /// springs out (`Preset::Exit`) while its owner keeps it on screen.
    pub fn presence(&mut self, key: K, shown: bool, t: &Tokens) -> f32 {
        let target = if shown { 1. } else { 0. };
        let a = self
            .presences
            .entry(key)
            .or_insert_with(|| Animated::unit(0.));
        if a.target[0] != target {
            a.to(
                target,
                t.spring(if shown { Preset::Enter } else { Preset::Exit }),
            );
        }
        a.get().clamp(0., 1.)
    }
    /// Step everything to now; ask for another frame only while moving.
    pub fn frame(&mut self, window: &mut Window) {
        let dt = self.clock.tick(Instant::now());
        let mut moving = self.hovers.step(dt);
        for presence in self.presences.values_mut() {
            moving |= presence.step(dt);
        }
        for glide in self.glides.values_mut() {
            moving |= glide.step(dt);
        }
        if moving {
            window.request_animation_frame();
        } else {
            self.clock.rest();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle(m: &mut Motion<u32>, seconds: f32) {
        let mut t = 0.;
        while t < seconds {
            for p in m.presences.values_mut() {
                p.step(1. / 120.);
            }
            t += 1. / 120.;
        }
    }
    #[test]
    fn a_surface_springs_in_when_it_appears_and_out_while_closing() {
        let t = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let mut m = Motion::<u32>::default();
        // First seen: starts hidden, then springs in.
        assert_eq!(m.presence(7, true, &t), 0.);
        settle(&mut m, 0.05);
        let partway = m.presence(7, true, &t);
        assert!(partway > 0. && partway < 1., "{partway}");
        settle(&mut m, 1.);
        assert!(m.presence(7, true, &t) > 0.99);
        // Marked closing: it springs out (faster than it came in).
        m.presence(7, false, &t);
        settle(&mut m, 0.4);
        assert!(m.presence(7, false, &t) < 0.01);
    }
}
