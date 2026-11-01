//! Geometry published once per painted GPUI frame. Coordinates are window points,
//! top-left; clips are local to the slot and transforms map local → window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, p: [f64; 2]) -> bool {
        p[0] >= self.x && p[1] >= self.y && p[0] < self.x + self.w && p[1] < self.y + self.h
    }
}
#[derive(Clone, Debug)]
pub enum Clip {
    Rect(Rect),
    Union(Vec<Rect>),
    Polygon(Vec<[f64; 2]>),
}
impl Clip {
    pub fn contains(&self, p: [f64; 2]) -> bool {
        match self {
            Self::Rect(r) => r.contains(p),
            Self::Union(rs) => rs.iter().any(|r| r.contains(p)),
            Self::Polygon(v) => {
                if v.len() < 3 {
                    return false;
                }
                let mut inside = false;
                for (a, b) in v.iter().zip(v.iter().cycle().skip(1)).take(v.len()) {
                    if (a[1] > p[1]) != (b[1] > p[1])
                        && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Transform(pub [f64; 6]);
impl Transform {
    pub fn translation(x: f64, y: f64) -> Self {
        Self([1., 0., 0., 1., x, y])
    }
    pub fn inverse_point(self, p: [f64; 2]) -> Option<[f64; 2]> {
        let [a, b, c, d, tx, ty] = self.0;
        let det = a * d - b * c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (x, y) = (p[0] - tx, p[1] - ty);
        Some([(d * x - c * y) / det, (-b * x + a * y) / det])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Below,
    Above,
}
#[derive(Clone, Debug)]
pub struct Slot {
    pub id: u64,
    pub band: Band,
    pub bounds: Rect,
    pub clip: Clip,
    pub transform: Transform,
}
impl Slot {
    fn contains(&self, p: [f64; 2]) -> bool {
        self.transform
            .inverse_point(p)
            .is_some_and(|p| self.bounds.contains(p) && self.clip.contains(p))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Child(u64),
    Slot(u64),
    Gpui,
    GpuiNode(&'static str),
    Compat,
    None,
}
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub revision: u64,
    pub slots: Vec<Slot>,
    pub interactive: Vec<Rect>,
    pub occluding: Vec<Rect>,
    pub compat_overlays: Vec<Rect>,
    pub children: Vec<(u64, Rect)>,
    pub accessibility: Vec<Target>,
}
impl Frame {
    pub fn hit(&self, p: [f64; 2]) -> Target {
        if let Some((id, _)) = self.children.iter().rev().find(|(_, r)| r.contains(p)) {
            return Target::Child(*id);
        }
        if let Some(s) = self
            .slots
            .iter()
            .rev()
            .find(|s| s.band == Band::Above && s.contains(p))
        {
            return Target::Slot(s.id);
        }
        if self
            .interactive
            .iter()
            .chain(&self.occluding)
            .any(|r| r.contains(p))
        {
            return Target::Gpui;
        }
        if self.compat_overlays.iter().any(|r| r.contains(p)) {
            return Target::Compat;
        }
        self.slots
            .iter()
            .rev()
            .find(|s| s.band == Band::Below && s.contains(p))
            .map_or(Target::None, |s| Target::Slot(s.id))
    }
}
#[derive(Default)]
pub struct Pointer {
    captured: Option<Target>,
}
impl Pointer {
    pub fn down(&mut self, frame: &Frame, p: [f64; 2]) -> Target {
        let t = frame.hit(p);
        self.captured = Some(t);
        t
    }
    pub fn moved(&self, frame: &Frame, p: [f64; 2]) -> Target {
        self.captured.unwrap_or_else(|| frame.hit(p))
    }
    pub fn up(&mut self) {
        self.captured = None;
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub enter: f64,
    pub bounce: f64,
    pub exit: f64,
    pub start_scale: f64,
}
pub fn menu_motion(reduce: bool) -> Motion {
    if reduce {
        Motion {
            enter: crate::design::REDUCED_DURATION,
            bounce: 0.,
            exit: crate::design::REDUCED_DURATION,
            start_scale: 1.,
        }
    } else {
        Motion {
            enter: crate::design::ENTER_DURATION,
            bounce: crate::design::ENTER_BOUNCE,
            exit: crate::design::EXIT_DURATION,
            start_scale: crate::design::ENTER_SCALE,
        }
    }
}
/// O(visible rows), no allocation or scan of all 5k logical rows.
pub fn visible_rows(total: usize, scroll: f64, height: f64, row: f64) -> std::ops::Range<usize> {
    let start = ((scroll.max(0.) / row).floor() as usize).min(total);
    start
        ..((scroll.max(0.) + height) / row)
            .ceil()
            .max(start as f64)
            .min(total as f64) as usize
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame {
        let r = Rect::new(0., 0., 100., 100.);
        Frame {
            slots: vec![Slot {
                id: 1,
                band: Band::Below,
                bounds: r,
                clip: Clip::Rect(r),
                transform: Transform::translation(0., 0.),
            }],
            ..Default::default()
        }
    }
    #[test]
    fn hit_order_and_overlay_holes() {
        let mut f = frame();
        f.compat_overlays.push(Rect::new(10., 10., 80., 80.));
        assert_eq!(f.hit([15., 15.]), Target::Compat);
        f.occluding.push(Rect::new(20., 20., 60., 60.));
        assert_eq!(f.hit([25., 25.]), Target::Gpui);
        let mut s = f.slots[0].clone();
        s.id = 2;
        s.band = Band::Above;
        f.slots.push(s);
        assert_eq!(f.hit([25., 25.]), Target::Slot(2));
        f.children.push((3, Rect::new(0., 0., 100., 100.)));
        assert_eq!(f.hit([25., 25.]), Target::Child(3));
    }
    #[test]
    fn capture_survives_boundaries_and_republish() {
        let mut p = Pointer::default();
        let f = frame();
        assert_eq!(p.down(&f, [5., 5.]), Target::Slot(1));
        assert_eq!(p.moved(&Frame::default(), [900., 900.]), Target::Slot(1));
        p.up();
        assert_eq!(p.moved(&f, [900., 900.]), Target::None);
    }
    #[test]
    fn transformed_polygon_clip() {
        let mut f = frame();
        f.slots[0].clip = Clip::Polygon(vec![[0., 0.], [100., 0.], [0., 100.]]);
        f.slots[0].transform = Transform([2., 0., 0., 2., 10., 20.]);
        assert_eq!(f.hit([30., 40.]), Target::Slot(1));
        assert_eq!(f.hit([190., 200.]), Target::None);
        f.slots[0].transform = Transform([0.; 6]);
        assert_eq!(f.hit([0., 0.]), Target::None);
    }
    #[test]
    fn z_order_last_slot_wins() {
        let mut f = frame();
        let mut s = f.slots[0].clone();
        s.id = 2;
        f.slots.push(s);
        assert_eq!(f.hit([5., 5.]), Target::Slot(2));
    }
    #[test]
    fn menu_tokens() {
        let m = menu_motion(false);
        assert_eq!(
            (m.enter, m.bounce, m.exit, m.start_scale),
            (0.32, 0.15, 0.12, 0.96)
        );
        let m = menu_motion(true);
        assert_eq!(
            (m.enter, m.exit, m.bounce, m.start_scale),
            (0.15, 0.15, 0., 1.)
        );
    }
    #[test]
    fn virtualized_5k_rows() {
        assert_eq!(visible_rows(5000, 0., 600., 24.), 0..25);
        assert_eq!(visible_rows(5000, 119700., 600., 24.), 4987..5000);
        assert_eq!(visible_rows(5000, 999999., 600., 24.), 5000..5000);
    }
}

#[cfg(test)]
mod union_tests {
    use super::*;
    #[test]
    fn compat_union_keeps_gap_transparent() {
        let clip = Clip::Union(vec![
            Rect::new(20., 20., 100., 100.),
            Rect::new(0., 180., 150., 40.),
        ]);
        assert!(clip.contains([50., 50.]));
        assert!(clip.contains([10., 190.]));
        assert!(!clip.contains([50., 150.]));
    }
}

/// Shared with the fixture's CSS: right:12px; top:304px; width:320px; height:60px.
pub fn compat_overlay(width: f64) -> Rect {
    Rect::new(
        width - 12. - crate::design::COMPAT_OVERLAY_WIDTH,
        304.,
        crate::design::COMPAT_OVERLAY_WIDTH,
        crate::design::COMPAT_OVERLAY_HEIGHT,
    )
}
#[cfg(test)]
mod overlay_tests {
    use super::*;
    #[test]
    fn overlay_clip_includes_leading_label_at_wide_and_narrow_sizes() {
        for width in [1180., 1032.] {
            let r = compat_overlay(width);
            assert_eq!(r.x, width - 332.);
            assert!(r.contains([width - 331., 305.]));
            assert!(r.contains([width - 13., 363.]));
            assert!(!r.contains([width - 333., 305.]));
        }
    }
}
