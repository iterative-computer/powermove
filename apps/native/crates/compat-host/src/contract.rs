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
    pub bounds: Rect,
    pub clip: Rect,
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
/// Page messages arrive on AppKit's main thread. Never wait for V8, an actor
/// response, a GPU completion, or page JS here.
pub trait SyncHandler {
    fn event(&self, event: HostEvent);
    /// Only enqueue to a bounded routing-owner mailbox. Never enter V8 or wait
    /// on that owner here. Refusal must be explicit; no dropped acknowledgements.
    fn remote_dom(&self, payload: Value) -> Result<()>;
}
/// A face the page lays text out with: the engine's own face for what the
/// project's text asks for, under the family name the page's CSS uses.
#[derive(Clone, Debug, PartialEq)]
pub struct PageFace {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// One whole sfnt face.
    pub bytes: std::sync::Arc<[u8]>,
}
impl PageFace {
    /// The face's name in the page's `font` route.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.family, self.weight, u8::from(self.italic))
    }
}
#[derive(Clone, Debug)]
pub enum HostEvent {
    Ready {
        generation: Generation,
    },
    ProcessTerminated,
    Failure(&'static str),
    /// The page settled these faces (`PageFace::key`), loaded or failed, after
    /// `MacDomHost::add_fonts`.
    FontsLoaded(Vec<String>),
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
    fn focus(&mut self, panel: PanelId, node: Option<u64>) -> Result<()>;
}
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug)]
pub(crate) struct Panel {
    #[cfg(target_os = "macos")]
    pub spec: PanelSpec,
    pub slot: Option<PanelSlot>,
}
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
pub(crate) struct Panels {
    pub items: BTreeMap<PanelId, Panel>,
    pub viewport: Rect,
    /// One document can show through several shell rectangles. `None` shows its
    /// slots; `Some` is the exact mask, so a document with no region shows nothing.
    pub regions: Option<Vec<Rect>>,
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
            },
        );
        Ok(())
    }
    pub fn get(&mut self, id: PanelId) -> Result<&mut Panel> {
        self.items.get_mut(&id).ok_or(Error::UnknownPanel)
    }
    /// Window-space rectangles the page shows through, clipped to the viewport.
    pub fn visible(&self) -> Vec<Rect> {
        match &self.regions {
            None => self
                .items
                .values()
                .filter_map(|p| p.slot.as_ref())
                .map(|s| s.visible().intersection(self.viewport))
                .collect(),
            Some(regions) => regions
                .iter()
                .map(|r| r.intersection(self.viewport))
                .filter(|r| r.w > 0. && r.h > 0.)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel(p: &mut Panels, id: u64) {
        p.create(PanelSpec {
            id,
            label: "Fixture".into(),
        })
        .unwrap();
        p.get(id).unwrap().slot = Some(PanelSlot {
            bounds: Rect::new(10., 20., 100., 100.),
            clip: Rect::new(10., 20., 60., 100.),
        });
    }
    #[test]
    fn regions_replace_the_slot_for_a_shared_document() {
        let mut p = Panels {
            viewport: Rect::new(0., 0., 400., 400.),
            ..Panels::default()
        };
        panel(&mut p, 1);
        p.regions = Some(vec![
            Rect::new(0., 0., 50., 50.),
            Rect::new(300., 300., 200., 200.),
        ]);
        assert_eq!(
            p.visible(),
            vec![
                Rect::new(0., 0., 50., 50.),
                Rect::new(300., 300., 100., 100.)
            ]
        );
        // Every panel closed: the whole-window slot must not unmask the window.
        p.regions = Some(vec![]);
        assert!(p.visible().is_empty());
    }
    #[test]
    fn logical_lifecycle_and_duplicates() {
        let mut p = Panels {
            viewport: Rect::new(0., 0., 400., 300.),
            ..Panels::default()
        };
        panel(&mut p, 1);
        assert_eq!(
            p.create(PanelSpec {
                id: 1,
                label: "duplicate".into()
            }),
            Err(Error::DuplicatePanel)
        );
        assert_eq!(p.get(2).unwrap_err(), Error::UnknownPanel);
        assert_eq!(p.visible(), [Rect::new(10., 20., 60., 100.)]);
        p.get(1).unwrap().slot = None;
        assert!(p.visible().is_empty());
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
