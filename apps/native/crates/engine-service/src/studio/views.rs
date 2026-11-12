//! Transient view data and commands. Pane placement and controls belong to the UI.
use super::{FreeView, Preview};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Axis {
    Front,
    Left,
    Top,
    Back,
    Right,
    Bottom,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ViewCamera {
    #[default]
    Active,
    Layer {
        id: String,
    },
    Custom {
        view: FreeView,
    },
    Orthographic {
        axis: Axis,
        center: [f64; 3],
        span: f64,
    },
}
impl ViewCamera {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Active => Ok(()),
            Self::Layer { id } if !id.is_empty() && id.len() <= 160 => Ok(()),
            Self::Layer { .. } => Err("View camera needs a layer ID".into()),
            Self::Custom { view } => view.validate(),
            Self::Orthographic { center, span, .. }
                if center.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
                    && span.is_finite()
                    && *span >= 1e-4
                    && *span <= 1e6 =>
            {
                Ok(())
            }
            Self::Orthographic { .. } => {
                Err("Orthographic view needs finite coordinates and a positive span".into())
            }
        }
    }
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_vec(self).unwrap_or_default().hash(&mut hash);
        hash.finish()
    }
    pub(super) fn pose(&self) -> Option<FreeView> {
        match self {
            Self::Custom { view } => Some(*view),
            Self::Orthographic { axis, center, span } => {
                let offset = match axis {
                    Axis::Front => [0., 0., 1.],
                    Axis::Back => [0., 0., -1.],
                    Axis::Left => [-1., 0., 0.],
                    Axis::Right => [1., 0., 0.],
                    Axis::Top => [0., 1., 0.],
                    Axis::Bottom => [0., -1., 0.],
                };
                let distance = (span * 2.).max(10.);
                Some(FreeView {
                    eye: std::array::from_fn(|i| center[i] + offset[i] * distance),
                    target: *center,
                    zoom: 1.,
                })
            }
            _ => None,
        }
    }
}
impl From<Option<FreeView>> for ViewCamera {
    fn from(view: Option<FreeView>) -> Self {
        view.map_or(Self::Active, |view| Self::Custom { view })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pane {
    pub camera: ViewCamera,
    pub tier: Preview,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "RawViews")]
pub struct Views {
    count: u8,
    active: u8,
    panes: [Pane; 4],
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawViews {
    count: u8,
    active: u8,
    panes: [Pane; 4],
}
impl TryFrom<RawViews> for Views {
    type Error = String;
    fn try_from(raw: RawViews) -> Result<Self, String> {
        let views = Self {
            count: raw.count,
            active: raw.active,
            panes: raw.panes,
        };
        views.validate()?;
        Ok(views)
    }
}
impl Default for Views {
    fn default() -> Self {
        Self {
            count: 1,
            active: 0,
            panes: std::array::from_fn(|_| Pane::default()),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub enum ViewCommand {
    SetLayout { count: u8 },
    Activate { pane: u8 },
    SetCamera { pane: u8, camera: ViewCamera },
    SetTier { pane: u8, tier: Preview },
}
impl Views {
    pub fn count(&self) -> usize {
        self.count as usize
    }
    pub fn active(&self) -> usize {
        self.active as usize
    }
    pub fn panes(&self) -> &[Pane] {
        &self.panes[..self.count()]
    }
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.count, 1 | 2 | 4) || self.active >= self.count {
            return Err("Studio views require 1, 2 or 4 panes and a visible active pane".into());
        }
        for pane in &self.panes {
            pane.camera.validate()?;
        }
        Ok(())
    }
    pub fn apply(&mut self, command: ViewCommand) -> Result<bool, String> {
        let mut next = self.clone();
        match command {
            ViewCommand::SetLayout { count } => {
                if !matches!(count, 1 | 2 | 4) {
                    return Err("Studio views require 1, 2 or 4 panes".into());
                }
                next.count = count;
                next.active = next.active.min(count - 1);
            }
            ViewCommand::Activate { pane } => {
                if pane >= next.count {
                    return Err("View pane is not visible".into());
                }
                next.active = pane;
            }
            ViewCommand::SetCamera { pane, camera } => {
                camera.validate()?;
                let target = next
                    .panes
                    .get_mut(pane as usize)
                    .ok_or("View pane is outside 0–3")?;
                target.camera = camera;
            }
            ViewCommand::SetTier { pane, tier } => {
                next.panes
                    .get_mut(pane as usize)
                    .ok_or("View pane is outside 0–3")?
                    .tier = tier;
            }
        }
        next.validate()?;
        let changed = next != *self;
        *self = next;
        Ok(changed)
    }
    /// Bind existing engine viewers to the data; the designer owns their surfaces.
    pub fn apply_to(&self, viewers: &[crate::Viewer]) -> anyhow::Result<()> {
        self.validate().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            viewers.len() == self.count(),
            "View count does not match the Studio layout"
        );
        let shared = &viewers[0].shared;
        let mut ids = std::collections::HashSet::new();
        let mut state = shared.state.lock().unwrap();
        for viewer in viewers {
            anyhow::ensure!(
                std::sync::Arc::ptr_eq(shared, &viewer.shared) && ids.insert(viewer.id),
                "Studio panes need distinct viewers on the same engine service"
            );
            anyhow::ensure!(state.views.contains_key(&viewer.id), "viewer retired");
        }
        for (pane, viewer) in self.panes().iter().zip(viewers) {
            let view = state.views.get_mut(&viewer.id).unwrap();
            if view.studio_view != pane.camera || view.studio_preview != pane.tier {
                view.studio_view = pane.camera.clone();
                view.studio_preview = pane.tier;
                view.stamp.sequence += 1;
                view.dirty = true;
            }
        }
        drop(state);
        shared.wake.try_send(()).ok();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_validate_and_preserve_each_pane_without_authored_edits() {
        let mut views = Views::default();
        assert_eq!(views.count(), 1);
        views.apply(ViewCommand::SetLayout { count: 4 }).unwrap();
        views
            .apply(ViewCommand::SetCamera {
                pane: 3,
                camera: ViewCamera::Orthographic {
                    axis: Axis::Top,
                    center: [1., 2., 3.],
                    span: 8.,
                },
            })
            .unwrap();
        views
            .apply(ViewCommand::SetTier {
                pane: 3,
                tier: Preview::Refine,
            })
            .unwrap();
        views.apply(ViewCommand::Activate { pane: 3 }).unwrap();
        let last = views.panes()[3].clone();
        let before = views.clone();
        assert!(views.apply(ViewCommand::SetLayout { count: 3 }).is_err());
        assert_eq!(views, before);
        assert!(
            views
                .apply(ViewCommand::SetCamera {
                    pane: 0,
                    camera: ViewCamera::Custom {
                        view: FreeView {
                            eye: [0.; 3],
                            target: [0.; 3],
                            zoom: 1.
                        }
                    }
                })
                .is_err()
        );
        assert_eq!(views, before);
        views.apply(ViewCommand::SetLayout { count: 2 }).unwrap();
        assert_eq!(views.active(), 1);
        assert!(views.apply(ViewCommand::Activate { pane: 3 }).is_err());
        views.apply(ViewCommand::SetLayout { count: 4 }).unwrap();
        assert_eq!(views.panes()[3], last);
        let mut invalid = serde_json::to_value(&views).unwrap();
        invalid["count"] = serde_json::json!(3);
        assert!(serde_json::from_value::<Views>(invalid).is_err());
        let json = serde_json::to_value(&views).unwrap();
        assert_eq!(serde_json::from_value::<Views>(json).unwrap(), views);
    }
}
