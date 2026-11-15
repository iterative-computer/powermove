//! Blender-style panel corner operations, matching the Electron area controls.
use crate::layout::{Geometry, Layout, Rect};

pub const HIT: f32 = 11.;
pub const MIN_HEIGHT: f32 = 88.;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
impl Corner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];
    pub fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }
    pub fn left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }
    pub fn rect(self, r: Rect) -> Rect {
        Rect {
            x: if self.left() { r.x } else { r.x + r.w - HIT },
            y: if self.top() { r.y } else { r.y + r.h - HIT },
            w: HIT,
            h: HIT,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Split { top: bool, height: f32 },
    Join { target: String },
    Dock { target: String, place: Place },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    Above,
    Below,
    Replace,
}
impl Layout {
    pub fn area_action(
        &self,
        geometry: &Geometry,
        source: &str,
        corner: Corner,
        x: f32,
        y: f32,
        gap: f32,
    ) -> Action {
        let Some((di, gi, _)) = self.find(source) else {
            return Action::None;
        };
        let Some(src) = geometry
            .panels
            .iter()
            .find(|p| (p.dock, p.group) == (di, gi))
        else {
            return Action::None;
        };
        if src.rect.contains(x, y) {
            let height = ((if corner.top() {
                y - src.rect.y
            } else {
                src.rect.y + src.rect.h - y
            }) - gap / 2.)
                .round();
            return if height >= MIN_HEIGHT && src.rect.h - height - gap >= MIN_HEIGHT {
                Action::Split {
                    top: corner.top(),
                    height,
                }
            } else {
                Action::None
            };
        }
        let Some(dst) = geometry.panels.iter().find(|p| p.rect.contains(x, y)) else {
            return Action::None;
        };
        let target = self.docks[dst.dock].groups[dst.group].active().to_string();
        if dst.dock == di && dst.group.abs_diff(gi) == 1 {
            return if target == "viewer" {
                Action::None
            } else {
                Action::Join { target }
            };
        }
        let relative = (y - dst.rect.y) / dst.rect.h.max(1.);
        let mut place = if relative < 0.3 {
            Place::Above
        } else if relative > 0.7 {
            Place::Below
        } else {
            Place::Replace
        };
        if target == "viewer" && place == Place::Replace {
            place = if relative < 0.5 {
                Place::Above
            } else {
                Place::Below
            };
        }
        Action::Dock { target, place }
    }
}

impl Layout {
    /// Atomic split: only a closed panel can fill the new area.
    pub fn split_area(
        &mut self,
        source: &str,
        new: &str,
        top: bool,
        height: f32,
        total: f32,
        gap: f32,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            height.is_finite() && height >= MIN_HEIGHT && total - height - gap >= MIN_HEIGHT,
            "Too small to split"
        );
        anyhow::ensure!(self.find(new).is_none(), "Panel is already open");
        let (d, g, _) = self
            .find(source)
            .ok_or_else(|| anyhow::anyhow!("Unknown panel"))?;
        let mut next = self.clone();
        let source = &mut next.docks[d].groups[g];
        if source.size.is_some() {
            source.size = Some(total - height - gap);
        }
        next.docks[d].groups.insert(
            g + usize::from(!top),
            crate::layout::Group {
                tabs: vec![new.into()],
                active: 0,
                collapsed: false,
                size: Some(height),
            },
        );
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn apply_area(
        &mut self,
        source: &str,
        action: &Action,
        geometry: &Geometry,
        gap: f32,
    ) -> anyhow::Result<()> {
        let mut next = self.clone();
        let height = |id: &str| -> f32 {
            self.find(id)
                .and_then(|(d, g, _)| geometry.panels.iter().find(|p| (p.dock, p.group) == (d, g)))
                .map_or(0., |p| p.rect.h)
        };
        match action {
            Action::Join { target } => {
                let (d, g, _) = next
                    .find(source)
                    .ok_or_else(|| anyhow::anyhow!("Unknown panel"))?;
                let (td, tg, _) = next
                    .find(target)
                    .ok_or_else(|| anyhow::anyhow!("Unknown target"))?;
                anyhow::ensure!(
                    d == td && g.abs_diff(tg) == 1 && target != "viewer",
                    "Cannot join areas"
                );
                let total = height(source) + height(target) + gap;
                let fill = next.docks[d].groups[g].size.is_none()
                    || next.docks[td].groups[tg].size.is_none();
                next.close_panel(target)?;
                let (d, g, _) = next.find(source).unwrap();
                next.docks[d].groups[g].size = if fill { None } else { Some(total) };
            }
            Action::Dock { target, place } => {
                let total = height(target);
                let (d, g, _) = next
                    .find(target)
                    .ok_or_else(|| anyhow::anyhow!("Unknown target"))?;
                let size = next.docks[d].groups[g].size;
                if *place != Place::Replace {
                    anyhow::ensure!((total - gap) / 2. >= MIN_HEIGHT, "Too small to split");
                }
                next.move_panel_to(
                    source,
                    crate::layout::DropTarget::Split {
                        dock: d,
                        group: g + usize::from(*place == Place::Below),
                    },
                )?;
                if *place == Place::Replace {
                    next.close_panel(target)?;
                } else if let Some((d, g, _)) = next.find(target) {
                    if size.is_some() {
                        next.docks[d].groups[g].size = Some((total - gap) / 2.);
                    }
                }
                let (d, g, _) = next.find(source).unwrap();
                next.docks[d].groups[g].size = if *place == Place::Replace {
                    size
                } else {
                    Some((total - gap) / 2.)
                };
            }
            _ => return Ok(()),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Dock, Group, Placement};
    fn fixture() -> (Layout, Geometry) {
        let layout = Layout {
            version: 1,
            workspace: "test".into(),
            docks: vec![
                Dock {
                    id: "left".into(),
                    size: Some(250.),
                    collapsed: false,
                    groups: vec![
                        Group {
                            tabs: vec!["assets".into()],
                            active: 0,
                            collapsed: false,
                            size: Some(300.),
                        },
                        Group {
                            tabs: vec!["inspector".into()],
                            active: 0,
                            collapsed: false,
                            size: None,
                        },
                    ],
                },
                Dock {
                    id: "center".into(),
                    size: None,
                    collapsed: false,
                    groups: vec![Group {
                        tabs: vec!["viewer".into()],
                        active: 0,
                        collapsed: false,
                        size: None,
                    }],
                },
            ],
        };
        let geo = Geometry {
            panels: vec![
                Placement {
                    dock: 0,
                    group: 0,
                    rect: Rect {
                        x: 0.,
                        y: 0.,
                        w: 250.,
                        h: 300.,
                    },
                },
                Placement {
                    dock: 0,
                    group: 1,
                    rect: Rect {
                        x: 0.,
                        y: 308.,
                        w: 250.,
                        h: 300.,
                    },
                },
                Placement {
                    dock: 1,
                    group: 0,
                    rect: Rect {
                        x: 258.,
                        y: 0.,
                        w: 700.,
                        h: 608.,
                    },
                },
            ],
            dividers: vec![],
            ..Geometry::default()
        };
        (layout, geo)
    }
    #[test]
    fn corner_hit_areas_leave_content_and_header_tools_alone() {
        let (_, geo) = fixture();
        let r = geo.panels[0].rect;
        for corner in Corner::ALL {
            let hit = corner.rect(r);
            assert!(hit.contains(hit.x + 5., hit.y + 5.));
            assert!(!hit.contains(r.x + 20., r.y + 20.));
            assert!(!hit.contains(r.x + r.w - 22., r.y + 16.));
        }
    }
    #[test]
    fn corner_split_bounds_and_cancel_are_atomic() {
        let (mut l, geo) = fixture();
        let original = l.clone();
        assert_eq!(
            l.area_action(&geo, "assets", Corner::TopLeft, 5., 4., 8.),
            Action::None
        );
        assert_eq!(
            l.area_action(&geo, "assets", Corner::BottomRight, 240., 180., 8.),
            Action::Split {
                top: false,
                height: 116.
            }
        );
        assert!(
            l.split_area("assets", "notes", true, 20., 300., 8.)
                .is_err()
        );
        assert_eq!(l, original);
        l.split_area("assets", "notes", true, 100., 300., 8.)
            .unwrap();
        assert_eq!(l.docks[0].groups[0].active(), "notes");
        assert_eq!(l.docks[0].groups[1].size, Some(192.));
    }
    #[test]
    fn corner_join_and_dock_protect_viewer_and_preserve_fill() {
        let (mut l, geo) = fixture();
        let action = l.area_action(&geo, "assets", Corner::BottomLeft, 10., 350., 8.);
        assert_eq!(
            action,
            Action::Join {
                target: "inspector".into()
            }
        );
        l.apply_area("assets", &action, &geo, 8.).unwrap();
        assert!(l.find("inspector").is_none());
        assert_eq!(l.docks[0].groups[0].size, None);
        let (mut l, geo) = fixture();
        let action = l.area_action(&geo, "assets", Corner::TopRight, 400., 300., 8.);
        assert!(matches!(
            action,
            Action::Dock {
                place: Place::Above,
                ..
            }
        ));
        l.apply_area("assets", &action, &geo, 8.).unwrap();
        assert!(l.find("viewer").is_some());
        assert_eq!(l.find("assets").unwrap().0, l.find("viewer").unwrap().0);
    }
}
