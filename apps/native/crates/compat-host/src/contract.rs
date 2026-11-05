use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(any(target_os = "macos", test))]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

pub type PanelId = u64;
pub type Generation = u64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownPanel,
    DuplicatePanel,
    InvalidGeometry,
    StaleGeneration,
    Forbidden,
    InvalidRequest,
    Backpressure,
    Unsupported(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// Points in the window content coordinate system, origin at its top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    pub fn valid(self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|x| x.is_finite())
            && self.w >= 0.
            && self.h >= 0.
    }
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self::new(
            x,
            y,
            (self.x + self.w).min(other.x + other.w).max(x) - x,
            (self.y + self.h).min(other.y + other.h).max(y) - y,
        )
    }
}
/// P-LAYER placement. A shared page is a BELOW slot. GPUI/above slots and child
/// windows retain priority. Rectangular clips are explicit; affine/polygon slots
/// must be flattened by the compositor or rejected, never silently approximated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSlot {
    pub id: u64,
    pub bounds: Rect,
    pub clip: Rect,
    pub reading_order: u32,
}
impl PanelSlot {
    pub fn visible(&self) -> Rect {
        self.bounds.intersection(self.clip)
    }
}
#[derive(Clone, Debug)]
pub struct PanelSpec {
    pub id: PanelId,
    pub label: String,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompositionSnapshot {
    pub panels: Vec<(PanelId, PanelSlot)>,
    /// Window-space rectangles, clipped to the viewport. The shell subtracts
    /// its interactive AND visually occluding regions before opening hit holes.
    pub overlay_holes: Vec<Rect>,
    pub visible_union: Vec<Rect>,
}
impl CompositionSnapshot {
    pub fn page_at(&self, x: f64, y: f64) -> bool {
        self.visible_union.iter().any(|r| r.contains(x, y))
    }
    pub fn accepts_gpui(&self, x: f64, y: f64, occluding: &[Rect]) -> bool {
        occluding.iter().any(|r| r.contains(x, y)) || !self.page_at(x, y)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    Trusted,
    Store {
        extension: String,
        permissions: BTreeSet<String>,
    },
}
impl Trust {
    pub fn permits(&self, permission: &str) -> bool {
        match self {
            Self::Trusted => true,
            Self::Store { permissions, .. } => permissions.contains(permission),
        }
    }
    pub fn is_store(&self) -> bool {
        matches!(self, Self::Store { .. })
    }
}
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub trust: Trust,
    pub generation: Generation,
    pub panel: PanelId,
}
/// Must return a bounded cached/native answer or enqueue work. Never wait for
/// V8, an actor response, a GPU completion, or page JS on AppKit's main thread.
/// Store admission is repeated here, under the extension's identity, by kernel.
pub trait SyncHandler {
    fn call(&self, context: &RequestContext, method: &str, args: Value) -> Result<Value>;
    fn event(&self, event: HostEvent);
    /// Only enqueue to a bounded routing-owner mailbox. Never enter V8 or wait
    /// on that owner here. Refusal must be explicit; no dropped acknowledgements.
    fn remote_dom(&self, _context: &RequestContext, _payload: Value) -> Result<()> {
        Err(Error::Unsupported(
            "remote DOM feedback queue is not attached",
        ))
    }
    /// Called BEFORE reset. Seal acknowledged gestures and revoke the old bridge
    /// generation. Returning Err aborts Stop without touching the page.
    fn prepare_reset(&self, _generation: Generation) -> Result<()> {
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub enum HostEvent {
    Ready {
        generation: Generation,
    },
    Dom {
        context: RequestContext,
        payload: Value,
    },
    Key {
        context: RequestContext,
        payload: Value,
    },
    CompositionChanged,
    PopoutOpened,
    ProcessTerminated,
    Recovered {
        generation: Generation,
        old_pid: i32,
        new_pid: i32,
    },
    Failure(&'static str),
}
/// Narrow boundary for pm-remote-dom. Invoke only on the AppKit main thread.
/// A successful send is admission, not a DOM-paint acknowledgement. The bridge
/// owns node identity, snapshots, replay, event semantics and paint acknowledgements.
pub trait DomPanelHost {
    fn generation(&self) -> Generation;
    fn create(&mut self, panel: PanelSpec) -> Result<()>;
    fn attach(&mut self, panel: PanelId, slot: PanelSlot) -> Result<()>;
    fn detach(&mut self, panel: PanelId) -> Result<()>;
    fn destroy(&mut self, panel: PanelId) -> Result<()>;
    fn apply(&mut self, generation: Generation, panel: PanelId, batch: Value) -> Result<()>;
    fn send_event(&mut self, generation: Generation, panel: PanelId, event: Value) -> Result<()>;
    fn focus(&mut self, panel: PanelId, node: Option<u64>) -> Result<()>;
    fn composition(&self) -> CompositionSnapshot;
}
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug)]
pub(crate) struct Panel {
    #[cfg(target_os = "macos")]
    pub spec: PanelSpec,
    pub slot: Option<PanelSlot>,
    pub overlays: Vec<Rect>,
}
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
pub(crate) struct Panels {
    pub items: BTreeMap<PanelId, Panel>,
    pub viewport: Rect,
}
#[cfg(any(target_os = "macos", test))]
impl Panels {
    pub fn create(&mut self, spec: PanelSpec) -> Result<()> {
        if self.items.contains_key(&spec.id) {
            return Err(Error::DuplicatePanel);
        }
        self.items.insert(
            spec.id,
            Panel {
                #[cfg(target_os = "macos")]
                spec,
                slot: None,
                overlays: vec![],
            },
        );
        Ok(())
    }
    pub fn get(&mut self, id: PanelId) -> Result<&mut Panel> {
        self.items.get_mut(&id).ok_or(Error::UnknownPanel)
    }
    pub fn snapshot(&self) -> CompositionSnapshot {
        let mut panels: Vec<_> = self
            .items
            .iter()
            .filter_map(|(id, p)| p.slot.clone().map(|s| (*id, s)))
            .collect();
        panels.sort_by_key(|(id, s)| (s.reading_order, *id));
        let overlay_holes: Vec<_> = panels
            .iter()
            .flat_map(|(id, _)| self.items[id].overlays.iter())
            .map(|r| r.intersection(self.viewport))
            .filter(|r| r.w > 0. && r.h > 0.)
            .collect();
        let mut visible_union: Vec<_> = panels
            .iter()
            .map(|(_, s)| s.visible().intersection(self.viewport))
            .collect();
        visible_union.extend(overlay_holes.iter().copied());
        CompositionSnapshot {
            panels,
            overlay_holes,
            visible_union,
        }
    }
    pub fn overlays(&mut self, id: PanelId, rects: Vec<Rect>) -> Result<()> {
        if rects.len() > 128 || rects.iter().any(|r| !r.valid()) {
            return Err(Error::InvalidGeometry);
        }
        let p = self.get(id)?;
        if p.slot.is_none() {
            return Err(Error::Forbidden);
        }
        p.overlays = rects;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel(p: &mut Panels, id: u64, order: u32) {
        p.create(PanelSpec {
            id,
            label: "Fixture".into(),
        })
        .unwrap();
        p.get(id).unwrap().slot = Some(PanelSlot {
            id,
            bounds: Rect::new(10., 20., 100., 100.),
            clip: Rect::new(10., 20., 60., 100.),
            reading_order: order,
        });
    }
    #[test]
    fn logical_lifecycle_and_duplicates() {
        let mut p = Panels::default();
        panel(&mut p, 1, 0);
        assert_eq!(
            p.create(PanelSpec {
                id: 1,
                label: "duplicate".into()
            }),
            Err(Error::DuplicatePanel)
        );
        assert_eq!(p.get(2).unwrap_err(), Error::UnknownPanel);
        assert!(p.snapshot().panels.len() == 1);
        p.get(1).unwrap().slot = None;
        assert!(p.snapshot().visible_union.is_empty());
    }
    #[test]
    fn overlay_and_clip_geometry_matches_hit_regions() {
        let mut p = Panels {
            viewport: Rect::new(0., 0., 400., 300.),
            ..Default::default()
        };
        panel(&mut p, 1, 0);
        p.overlays(1, vec![Rect::new(60., 110., 100., 40.)])
            .unwrap();
        let s = p.snapshot();
        assert!(s.page_at(20., 30.));
        assert!(!s.page_at(80., 30.));
        assert!(s.page_at(100., 125.));
        assert!(s.accepts_gpui(100., 125., &[Rect::new(90., 120., 20., 20.)]));
        assert!(!s.accepts_gpui(100., 125., &[]));
    }
    #[test]
    fn overlay_reports_are_bounded_and_finite() {
        let mut p = Panels::default();
        panel(&mut p, 1, 0);
        assert_eq!(
            p.overlays(1, vec![Rect::new(f64::NAN, 0., 1., 1.)]),
            Err(Error::InvalidGeometry)
        );
        assert_eq!(
            p.overlays(1, vec![Rect::default(); 129]),
            Err(Error::InvalidGeometry)
        );
        p.get(1).unwrap().slot = None;
        assert_eq!(p.overlays(1, vec![]), Err(Error::Forbidden));
    }
    #[test]
    fn reading_order_is_independent_of_panel_id() {
        let mut p = Panels::default();
        panel(&mut p, 1, 9);
        panel(&mut p, 2, 1);
        assert_eq!(
            p.snapshot()
                .panels
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [2, 1]
        );
    }
    #[test]
    fn holes_are_clamped_to_viewport() {
        let mut p = Panels {
            viewport: Rect::new(0., 0., 100., 100.),
            ..Default::default()
        };
        panel(&mut p, 1, 0);
        p.overlays(1, vec![Rect::new(90., 90., 50., 50.)]).unwrap();
        assert_eq!(p.snapshot().overlay_holes, [Rect::new(90., 90., 10., 10.)]);
    }
    #[test]
    fn store_permission_default_is_deny() {
        let t = Trust::Store {
            extension: "fixture".into(),
            permissions: BTreeSet::from(["project:read".into()]),
        };
        assert!(t.permits("project:read"));
        assert!(!t.permits("project:write"));
    }
}

/// Product watchdog policy consumes JS execution time, excluding time waiting
/// on a native call. The V8/bridge owner supplies that accounting; wall time is
/// not a substitute. Trusted Stop is always a shell decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchdogAction {
    None,
    Notice,
    PromptStopWait,
    StopStore,
}
#[derive(Default)]
pub struct Watchdog {
    notice: bool,
    prompt: bool,
    last_elapsed: std::time::Duration,
    next_prompt: std::time::Duration,
}
impl Watchdog {
    pub fn observe(&mut self, js_elapsed: std::time::Duration, trust: &Trust) -> WatchdogAction {
        self.last_elapsed = js_elapsed;
        if js_elapsed >= self.next_prompt.max(std::time::Duration::from_secs(8)) && !self.prompt {
            self.prompt = true;
            self.notice = true;
            return if trust.is_store() {
                WatchdogAction::StopStore
            } else {
                WatchdogAction::PromptStopWait
            };
        }
        if js_elapsed >= std::time::Duration::from_secs(2) && !self.notice {
            self.notice = true;
            return WatchdogAction::Notice;
        }
        WatchdogAction::None
    }
    pub fn wait(&mut self) {
        self.prompt = false;
        self.next_prompt = self
            .last_elapsed
            .saturating_add(std::time::Duration::from_secs(8));
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
#[cfg(test)]
mod watchdog_tests {
    use super::*;
    #[test]
    fn trusted_hang_prompts_and_store_stops() {
        use std::time::Duration;
        let mut w = Watchdog::default();
        assert_eq!(
            w.observe(Duration::from_secs(1), &Trust::Trusted),
            WatchdogAction::None
        );
        assert_eq!(
            w.observe(Duration::from_secs(2), &Trust::Trusted),
            WatchdogAction::Notice
        );
        assert_eq!(
            w.observe(Duration::from_secs(3), &Trust::Trusted),
            WatchdogAction::None
        );
        assert_eq!(
            w.observe(Duration::from_secs(8), &Trust::Trusted),
            WatchdogAction::PromptStopWait
        );
        w.wait();
        assert_eq!(
            w.observe(Duration::from_secs(9), &Trust::Trusted),
            WatchdogAction::None
        );
        assert_eq!(
            w.observe(Duration::from_secs(16), &Trust::Trusted),
            WatchdogAction::PromptStopWait
        );
        w.reset();
        let store = Trust::Store {
            extension: "fixture".into(),
            permissions: Default::default(),
        };
        assert_eq!(
            w.observe(Duration::from_secs(8), &store),
            WatchdogAction::StopStore
        );
    }
}
