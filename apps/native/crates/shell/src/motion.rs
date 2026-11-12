//! GPUI side of the motion system (`pm_design::motion`): hover washes and a
//! gliding selection. Owners call [`Motion::frame`] once per render; it steps
//! every value and requests the next frame only while something still moves.
use gpui::{Bounds, Pixels, Window, px};
use pm_design::{
    Tokens,
    motion::{Animated, Clock, POINT_PRECISION, Preset, Spring},
};
use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc, time::Instant};

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
/// height, springing on `Preset::Layout` from wherever it is. `item` is the
/// item it rests on or is heading for.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glide {
    rect: Option<Animated<4>>,
    item: u64,
}
impl Glide {
    /// Head for `item` at `bounds`; the first target is taken at once, and so
    /// is a new place for the same item (a layout change, not a selection).
    pub fn to(&mut self, item: u64, bounds: Bounds<Pixels>, t: &Tokens) {
        let target = [
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ];
        match &mut self.rect {
            Some(rect) if self.item != item => {
                rect.animate_to(target, t.spring(Preset::Layout));
            }
            Some(rect) if rect.target != target => rect.snap(target),
            Some(_) => {}
            None => self.rect = Some(Animated::new(target, Spring::SNAP, POINT_PRECISION)),
        }
        self.item = item;
    }
    /// Whether the selection is on its way to `item` rather than resting on it.
    pub fn travelling(&self, item: u64) -> bool {
        self.rect
            .is_some_and(|rect| self.item != item || rect.moving())
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

/// One owner's animated state and frame clock. Glides and values are shared
/// with the elements that set their targets while painting.
pub struct Motion<K> {
    pub hovers: Hovers<K>,
    pub glides: Rc<RefCell<HashMap<K, Glide>>>,
    /// Single animated numbers (a switch knob, a disclosure, a sheet's height).
    pub values: Rc<RefCell<HashMap<K, Animated<1>>>>,
    clock: Clock,
}
impl<K> Default for Motion<K> {
    fn default() -> Self {
        Self {
            hovers: Hovers::default(),
            glides: Rc::default(),
            values: Rc::default(),
            clock: Clock::default(),
        }
    }
}
impl<K: Copy + Eq + Hash> Motion<K> {
    /// Step everything to now; ask for another frame only while moving.
    pub fn frame(&mut self, window: &mut Window) {
        let dt = self.clock.tick(Instant::now());
        let mut moving = self.hovers.step(dt);
        for glide in self.glides.borrow_mut().values_mut() {
            moving |= glide.step(dt);
        }
        for value in self.values.borrow_mut().values_mut() {
            moving |= value.step(dt);
        }
        if moving {
            window.request_animation_frame();
        } else {
            self.clock.rest();
        }
    }
    /// Hover wash amounts by key, for a [`crate::kit::Kit`] built this frame.
    pub fn washes(&self) -> HashMap<K, f32> {
        self.hovers
            .washes
            .iter()
            .map(|(key, wash)| (*key, wash.get().clamp(0., 1.)))
            .collect()
    }
}
/// The value at `key` heading for `target` on `spring`, as it is now; a new
/// key starts at its target. The owner's [`Motion::frame`] steps it.
pub fn animate(
    values: &RefCell<HashMap<u64, Animated<1>>>,
    key: u64,
    target: f32,
    spring: Spring,
) -> f32 {
    let mut values = values.borrow_mut();
    let value = values.entry(key).or_insert_with(|| Animated::unit(target));
    if value.target[0] != target {
        value.to(target, spring);
    }
    value.get()
}
