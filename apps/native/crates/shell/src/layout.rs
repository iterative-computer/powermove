//! Workspace geometry; bottom spans all columns except the full-height right dock.
use pm_design::{Tokens, workspace::*};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub tabs: Vec<String>,
    pub active: usize,
    pub size: Option<f32>,
}
impl Group {
    pub fn active(&self) -> &str {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }
}
/// Equal-width tab segments, with the active tab retained when trailing tabs fold.
/// The caller reserves space for header tools before asking for this track.
#[derive(Clone, Debug, PartialEq)]
pub struct TabTrack {
    pub visible: Vec<usize>,
    pub overflow: Vec<usize>,
    pub width: f32,
}
impl Group {
    pub fn tab_track(&self, available: f32, tokens: &Tokens) -> TabTrack {
        let minimum = tokens.metrics.hit * 3.;
        let n = self.tabs.len();
        let available = available.max(0.);
        let fits = (available / minimum).floor() as usize;
        let capacity = if fits >= n {
            n
        } else {
            ((available - tokens.metrics.hit).max(0.) / minimum).floor() as usize
        }
        .max(1)
        .min(n);
        let mut visible: Vec<_> = (0..capacity).collect();
        if self.active >= capacity {
            visible[capacity - 1] = self.active;
        }
        let overflow: Vec<_> = (0..n).filter(|i| !visible.contains(i)).collect();
        let reserved = if overflow.is_empty() {
            0.
        } else {
            tokens.metrics.hit
        };
        TabTrack {
            width: ((available - reserved).max(0.) / capacity as f32),
            visible,
            overflow,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dock {
    pub id: String,
    pub size: Option<f32>,
    pub groups: Vec<Group>,
    #[serde(default)]
    pub collapsed: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    pub workspace: String,
    pub docks: Vec<Dock>,
}
#[derive(Clone, Debug)]
pub struct Placement {
    pub dock: usize,
    pub group: usize,
    pub rect: Rect,
}
#[derive(Clone, Debug)]
pub struct Divider {
    pub dock: usize,
    pub group: Option<usize>,
    pub horizontal: bool,
    pub rect: Rect,
}
#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub panels: Vec<Placement>,
    pub dividers: Vec<Divider>,
}
pub const PRESETS: [(&str, &str); 7] = [
    ("design", "Default"),
    ("gradient", "Gradient"),
    ("animate", "Animate"),
    ("shaderlab", "Shader Lab"),
    ("edit", "Edit"),
    ("review", "Review"),
    ("focus", "Focus"),
];
fn group(ids: &[&str], size: Option<f32>) -> Group {
    Group {
        tabs: ids.iter().map(|s| s.to_string()).collect(),
        active: 0,
        size,
    }
}
fn dock(id: &str, size: Option<f32>, groups: Vec<Group>) -> Dock {
    Dock {
        id: id.into(),
        size,
        groups,
        collapsed: false,
    }
}
impl Layout {
    pub fn preset(id: &str) -> Self {
        let docks = match id {
            "gradient" => vec![
                dock(
                    "left",
                    Some(300.),
                    vec![
                        group(&["gradient-controls"], None),
                        group(&["assets"], Some(190.)),
                    ],
                ),
                dock("center", None, vec![group(&["viewer"], None)]),
                dock("bottom", Some(300.), vec![group(&["timeline"], None)]),
                dock("right", Some(340.), vec![group(&["inspector"], None)]),
            ],
            "animate" => vec![
                dock(
                    "left",
                    Some(230.),
                    vec![group(&["takes"], None), group(&["assets"], Some(170.))],
                ),
                dock(
                    "center",
                    None,
                    vec![group(&["viewer"], Some(300.)), group(&["timeline"], None)],
                ),
                dock("right", Some(320.), vec![group(&["inspector"], None)]),
            ],
            "shaderlab" => vec![
                dock("left", Some(460.), vec![group(&["shader"], None)]),
                dock("center", None, vec![group(&["viewer"], None)]),
                dock("bottom", Some(200.), vec![group(&["timeline"], None)]),
                dock(
                    "right",
                    Some(290.),
                    vec![group(&["inspector"], None), group(&["perf"], Some(190.))],
                ),
            ],
            "edit" => vec![
                dock(
                    "left",
                    Some(250.),
                    vec![group(&["assets"], None), group(&["fxbrowser"], Some(240.))],
                ),
                dock("center", None, vec![group(&["viewer"], None)]),
                dock("bottom", Some(360.), vec![group(&["timeline"], None)]),
                dock("right", Some(280.), vec![group(&["inspector"], None)]),
            ],
            "review" => vec![
                dock(
                    "center",
                    None,
                    vec![group(&["viewer"], None), group(&["timeline"], Some(180.))],
                ),
                dock("right", Some(300.), vec![group(&["notes"], None)]),
            ],
            "focus" => vec![dock("center", None, vec![group(&["viewer"], None)])],
            _ => vec![
                dock(
                    "left",
                    Some(DEFAULT_LEFT),
                    vec![group(&["assets", "layer-effects"], None)],
                ),
                dock("center", None, vec![group(&["viewer"], None)]),
                dock(
                    "bottom",
                    Some(DEFAULT_TIMELINE),
                    vec![group(&["timeline"], None)],
                ),
                dock(
                    "right",
                    Some(DEFAULT_RIGHT),
                    vec![
                        group(&["inspector"], None),
                        group(&["fxbrowser"], Some(DEFAULT_EFFECTS)),
                    ],
                ),
            ],
        };
        Self {
            version: 1,
            workspace: if PRESETS.iter().any(|p| p.0 == id) {
                id
            } else {
                "design"
            }
            .into(),
            docks,
        }
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.version == 1, "Unsupported shell layout version");
        let mut ids = BTreeSet::new();
        let mut docks = BTreeSet::new();
        for d in &self.docks {
            anyhow::ensure!(docks.insert(&d.id), "Duplicate dock");
            anyhow::ensure!(
                d.size.is_none_or(|n| n.is_finite() && n >= 0.),
                "Invalid dock size"
            );
            for g in &d.groups {
                anyhow::ensure!(
                    !g.tabs.is_empty() && g.active < g.tabs.len(),
                    "Invalid tab group"
                );
                anyhow::ensure!(
                    g.size.is_none_or(|n| n.is_finite() && n > 0.),
                    "Invalid panel size"
                );
                for id in &g.tabs {
                    anyhow::ensure!(
                        !id.is_empty() && ids.insert(id),
                        "Duplicate or empty panel id"
                    );
                }
            }
        }
        anyhow::ensure!(
            ids.contains(&"viewer".to_string()),
            "Viewer must remain in workspace"
        );
        Ok(())
    }
    /// Display a panel at workspace size without changing the saved docking layout.
    pub fn geometry_with_maximized(
        &self,
        bounds: Rect,
        t: &Tokens,
        panel: Option<&str>,
    ) -> Geometry {
        if let Some(panel) = panel {
            for (dock, d) in self.docks.iter().enumerate().filter(|(_, d)| !d.collapsed) {
                if let Some(group) = d
                    .groups
                    .iter()
                    .position(|g| g.tabs.iter().any(|id| id == panel))
                {
                    return Geometry {
                        panels: vec![Placement {
                            dock,
                            group,
                            rect: bounds,
                        }],
                        dividers: vec![],
                    };
                }
            }
        }
        self.geometry(bounds, t)
    }

    pub fn geometry(&self, bounds: Rect, t: &Tokens) -> Geometry {
        let mut out = Geometry::default();
        let gap = t.metrics.gap;
        let visible = |id: &str| {
            self.docks
                .iter()
                .enumerate()
                .find(|(_, d)| d.id == id && !d.collapsed && !d.groups.is_empty())
        };
        let left = visible("left");
        let right = visible("right");
        let bottom = visible("bottom");
        let available = (bounds.w - t.metrics.min_center - gap * 2.).max(0.);
        let requested = left.map_or(0., |(_, d)| d.size.unwrap_or(DEFAULT_LEFT))
            + right.map_or(0., |(_, d)| d.size.unwrap_or(DEFAULT_RIGHT));
        let factor = if requested > available {
            available / requested
        } else {
            1.
        };
        let lw = left.map_or(0., |(_, d)| d.size.unwrap_or(DEFAULT_LEFT) * factor);
        let rw = right.map_or(0., |(_, d)| d.size.unwrap_or(DEFAULT_RIGHT) * factor);
        let lg = if left.is_some() { gap } else { 0. };
        let rg = if right.is_some() { gap } else { 0. };
        let bh = bottom.map_or(0., |(_, d)| {
            d.size
                .unwrap_or(DEFAULT_TIMELINE)
                .max(t.metrics.header + 2. * t.metrics.ruler + 3. * t.metrics.track_row)
                .min((bounds.h - t.metrics.min_panel - gap).max(0.))
        });
        let bg = if bottom.is_some() { gap } else { 0. };
        let upper = bounds.h - bh - bg;
        for (di, d) in self
            .docks
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.collapsed && !d.groups.is_empty())
        {
            let rect = match d.id.as_str() {
                "left" => Rect {
                    x: bounds.x,
                    y: bounds.y,
                    w: lw,
                    h: upper,
                },
                "right" => Rect {
                    x: bounds.x + bounds.w - rw,
                    y: bounds.y,
                    w: rw,
                    h: bounds.h,
                },
                "bottom" => Rect {
                    x: bounds.x,
                    y: bounds.y + upper + bg,
                    w: bounds.w - rw - rg,
                    h: bh,
                },
                _ => Rect {
                    x: bounds.x + lw + lg,
                    y: bounds.y,
                    w: (bounds.w - lw - rw - lg - rg).max(0.),
                    h: upper,
                },
            };
            let n = d.groups.len();
            let space = (rect.h - gap * (n - 1) as f32).max(0.);
            let fixed: f32 = d.groups.iter().filter_map(|g| g.size).sum();
            let flex = d.groups.iter().filter(|g| g.size.is_none()).count();
            // On small windows, shrink sized rows too; never overlap or lose a group.
            let reserved = t.metrics.min_panel * flex as f32;
            let shrink = if fixed > 0. {
                ((space - reserved).max(0.) / fixed).min(1.)
            } else {
                1.
            };
            let fh = if flex > 0 {
                (space - fixed * shrink).max(0.) / flex as f32
            } else {
                0.
            };
            let mut y = rect.y;
            for (gi, g) in d.groups.iter().enumerate() {
                let h = if flex == 0 {
                    space / n as f32
                } else {
                    g.size.map_or(fh, |h| h * shrink)
                };
                out.panels.push(Placement {
                    dock: di,
                    group: gi,
                    rect: Rect {
                        x: rect.x,
                        y,
                        w: rect.w,
                        h,
                    },
                });
                y += h;
                if gi + 1 < n {
                    out.dividers.push(Divider {
                        dock: di,
                        group: Some(gi),
                        horizontal: true,
                        rect: Rect {
                            x: rect.x,
                            y,
                            w: rect.w,
                            h: gap,
                        },
                    });
                    y += gap;
                }
            }
            let divider = match d.id.as_str() {
                "left" => Some((
                    false,
                    Rect {
                        x: rect.x + lw,
                        y: bounds.y,
                        w: gap,
                        h: upper,
                    },
                )),
                "right" => Some((
                    false,
                    Rect {
                        x: rect.x - gap,
                        y: bounds.y,
                        w: gap,
                        h: bounds.h,
                    },
                )),
                "bottom" => Some((
                    true,
                    Rect {
                        x: rect.x,
                        y: rect.y - gap,
                        w: rect.w,
                        h: gap,
                    },
                )),
                _ => None,
            };
            if let Some((horizontal, rect)) = divider {
                out.dividers.push(Divider {
                    dock: di,
                    group: None,
                    horizontal,
                    rect,
                });
            }
        }
        out
    }
    pub fn resize(&mut self, divider: &Divider, delta: f32, t: &Tokens) {
        let d = &mut self.docks[divider.dock];
        if let Some(i) = divider.group {
            // Keep a flexible partner to fill the dock; resize either side of it.
            let target = if d.groups[i].size.is_some() { i } else { i + 1 };
            let sign = if target == i { 1. } else { -1. };
            let old = d.groups[target].size.unwrap_or(t.metrics.min_panel);
            d.groups[target].size = Some((old + delta * sign).max(t.metrics.min_panel));
            if d.groups.iter().all(|g| g.size.is_some()) {
                let other = if target == i { i + 1 } else { i };
                d.groups[other].size = None;
            }
        } else {
            let sign = if d.id == "right" || d.id == "bottom" {
                -1.
            } else {
                1.
            };
            let minimum = if d.id == "bottom" {
                t.metrics.header + 2. * t.metrics.ruler + 3. * t.metrics.track_row
            } else {
                t.metrics.min_panel
            };
            d.size = Some((d.size.unwrap_or(minimum) + delta * sign).max(minimum));
        }
    }
    /// Where `(x, y)` would drop a dragged panel, in the coordinates of `geometry`.
    /// `bounds` is the docking area passed to [`Layout::geometry`]. Edge zones win
    /// over panels so a dock can be created even when its neighbour fills the edge.
    pub fn drop_target(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        x: f32,
        y: f32,
        t: &Tokens,
    ) -> Option<DropPreview> {
        if !bounds.contains(x, y) {
            return None;
        }
        let zone = t.metrics.drag_threshold.max(4.) * 4.;
        let gap = t.metrics.gap;
        let edge = if x < bounds.x + zone {
            Some(Edge::Left)
        } else if x > bounds.x + bounds.w - zone {
            Some(Edge::Right)
        } else if y > bounds.y + bounds.h - zone {
            Some(Edge::Bottom)
        } else {
            None
        };
        if let Some(edge) = edge {
            // Only the drop zone lights up: the outer quarter of the docking area.
            let rect = match edge {
                Edge::Left => Rect {
                    w: bounds.w * EDGE_ZONE,
                    ..bounds
                },
                Edge::Right => Rect {
                    x: bounds.x + bounds.w * (1. - EDGE_ZONE),
                    w: bounds.w * EDGE_ZONE,
                    ..bounds
                },
                Edge::Bottom => Rect {
                    y: bounds.y + bounds.h * (1. - EDGE_ZONE),
                    h: bounds.h * EDGE_ZONE,
                    ..bounds
                },
            };
            return Some(DropPreview {
                target: DropTarget::Edge(edge),
                rect,
            });
        }
        let p = geometry.panels.iter().find(|p| p.rect.contains(x, y))?;
        let r = p.rect;
        let g = &self.docks[p.dock].groups[p.group];
        let header_end = (r.y + t.metrics.header).min(r.y + r.h);
        let relative = (y - header_end) / (r.y + r.h - header_end).max(1.);
        let groupable = !g.tabs.iter().any(|id| id == "viewer");
        if groupable && (y < header_end || (0.3..=0.7).contains(&relative)) {
            let n = g.tabs.len();
            let index = if y < header_end {
                ((((x - r.x) / r.w.max(1.)) * n as f32).round() as usize).min(n)
            } else {
                n
            };
            return Some(DropPreview {
                target: DropTarget::Tab {
                    dock: p.dock,
                    group: p.group,
                    index,
                },
                rect: Rect {
                    y: header_end,
                    h: (r.y + r.h - header_end).max(0.),
                    ..r
                },
            });
        }
        let lower = relative >= 0.5;
        let half = ((r.h - gap) / 2.).max(0.);
        Some(DropPreview {
            target: DropTarget::Split {
                dock: p.dock,
                group: p.group + usize::from(lower),
            },
            rect: Rect {
                y: if lower { r.y + r.h - half } else { r.y },
                h: half,
                ..r
            },
        })
    }
    /// Resolve a real drag. The source has no target when dropping the whole
    /// stack back on itself. Viewer cannot join another stack in either direction.
    pub fn drop_target_for(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        at: (f32, f32),
        t: &Tokens,
        id: &str,
        stack: bool,
    ) -> Option<DropPreview> {
        let (di, gi, _) = self.find(id)?;
        let mut preview = self.drop_target(geometry, bounds, at.0, at.1, t)?;
        if let Some(p) = geometry.panels.iter().find(|p| p.rect.contains(at.0, at.1))
            && (p.dock, p.group) == (di, gi)
            && (stack || self.docks[di].groups[gi].tabs.len() == 1)
        {
            return None;
        }
        if (id == "viewer"
            || (stack
                && self.docks[di].groups[gi]
                    .tabs
                    .iter()
                    .any(|id| id == "viewer")))
            && matches!(preview.target, DropTarget::Tab { .. })
        {
            // An ungroupable source still offers above/below zones.
            let p = geometry
                .panels
                .iter()
                .find(|p| p.rect.contains(at.0, at.1))?;
            let lower = at.1 >= p.rect.y + p.rect.h / 2.;
            let half = ((p.rect.h - t.metrics.gap) / 2.).max(0.);
            preview = DropPreview {
                target: DropTarget::Split {
                    dock: p.dock,
                    group: p.group + usize::from(lower),
                },
                rect: Rect {
                    y: if lower {
                        p.rect.y + p.rect.h - half
                    } else {
                        p.rect.y
                    },
                    h: half,
                    ..p.rect
                },
            };
        }
        Some(preview)
    }
    /// Validated, atomic move. Destination indices address the original layout.
    pub fn move_panel_to(&mut self, id: &str, target: DropTarget) -> anyhow::Result<()> {
        self.move_to(id, target, false)
    }
    /// Header drags retain tab order, active tab and the stack's size.
    pub fn move_stack_to(&mut self, id: &str, target: DropTarget) -> anyhow::Result<()> {
        self.move_to(id, target, true)
    }
    fn move_to(&mut self, id: &str, target: DropTarget, stack: bool) -> anyhow::Result<()> {
        let (di, gi, ti) = self
            .find(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown panel"))?;
        match target {
            DropTarget::Tab { dock, group, index } => {
                let dest = self
                    .docks
                    .get(dock)
                    .and_then(|d| d.groups.get(group))
                    .ok_or_else(|| anyhow::anyhow!("Unknown group"))?;
                if (dock, group) == (di, gi) {
                    return if stack {
                        Ok(())
                    } else {
                        self.reorder_tab(dock, group, ti, index)
                    };
                }
                anyhow::ensure!(
                    id != "viewer"
                        && !(stack
                            && self.docks[di].groups[gi]
                                .tabs
                                .iter()
                                .any(|id| id == "viewer"))
                        && !dest.tabs.iter().any(|id| id == "viewer"),
                    "Viewer cannot join a tab stack"
                );
            }
            DropTarget::Split { dock, group } => {
                anyhow::ensure!(
                    self.docks
                        .get(dock)
                        .is_some_and(|d| group <= d.groups.len()),
                    "Unknown group"
                );
                if (stack || self.docks[di].groups[gi].tabs.len() == 1)
                    && dock == di
                    && (group == gi || group == gi + 1)
                {
                    return Ok(());
                }
            }
            DropTarget::Edge(_) => {}
        }
        let mut next = self.clone();
        let source = &mut next.docks[di].groups[gi];
        let moved = if stack {
            Group {
                tabs: std::mem::take(&mut source.tabs),
                active: source.active,
                size: source.size,
            }
        } else {
            let item = source.tabs.remove(ti);
            if ti < source.active {
                source.active -= 1;
            }
            self::group(&[&item], None)
        };
        match target {
            DropTarget::Tab { dock, group, index } => {
                let g = &mut next.docks[dock].groups[group];
                let index = index.min(g.tabs.len());
                g.active = index + moved.active;
                g.tabs.splice(index..index, moved.tabs);
                next.docks[dock].collapsed = false;
            }
            DropTarget::Split { dock, group } => {
                next.docks[dock].groups.insert(group, moved);
                next.docks[dock].collapsed = false;
            }
            DropTarget::Edge(edge) => {
                if let Some(d) = next.docks.iter_mut().find(|d| d.id == edge.dock_id()) {
                    if d.collapsed {
                        d.groups.insert(0, moved);
                    } else {
                        d.groups.push(moved);
                    }
                    d.collapsed = false;
                } else {
                    let size = match edge {
                        Edge::Left => DEFAULT_LEFT,
                        Edge::Right => DEFAULT_RIGHT,
                        Edge::Bottom => DEFAULT_TIMELINE,
                    };
                    next.docks
                        .push(dock(edge.dock_id(), Some(size), vec![moved]));
                }
            }
        }
        next.commit(self)
    }
    /// Keyboard/AX counterpart to the pointer zones, resolved by stable panel id.
    pub fn place_relative(
        &mut self,
        id: &str,
        target: &str,
        placement: &str,
    ) -> anyhow::Result<()> {
        let (dock, group, _) = self
            .find(target)
            .ok_or_else(|| anyhow::anyhow!("Unknown target panel"))?;
        let target = match placement {
            "group" => DropTarget::Tab {
                dock,
                group,
                index: usize::MAX,
            },
            "above" => DropTarget::Split { dock, group },
            "below" => DropTarget::Split {
                dock,
                group: group + 1,
            },
            _ => anyhow::bail!("Unknown placement"),
        };
        self.move_panel_to(id, target)
    }
    /// Reorders a tab within its group; the moved tab becomes active.
    pub fn reorder_tab(
        &mut self,
        dock: usize,
        group: usize,
        from: usize,
        to: usize,
    ) -> anyhow::Result<()> {
        let g = self
            .docks
            .get_mut(dock)
            .and_then(|d| d.groups.get_mut(group))
            .ok_or_else(|| anyhow::anyhow!("Unknown group"))?;
        anyhow::ensure!(from < g.tabs.len(), "Unknown tab");
        let item = g.tabs.remove(from);
        // `to` is an insertion slot in the original list; account for the removal.
        let to = if to > from { to - 1 } else { to }.min(g.tabs.len());
        g.tabs.insert(to, item);
        g.active = to;
        Ok(())
    }
    /// Closes a panel. The viewer cannot be closed.
    pub fn close_panel(&mut self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(id != "viewer", "The viewer cannot be closed");
        let (di, gi, ti) = self
            .find(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown panel"))?;
        let mut next = self.clone();
        next.docks[di].groups[gi].tabs.remove(ti);
        next.commit(self)
    }
    /// Shows a panel: activates its tab and reopens its dock, or adds it back to
    /// the dock it occupies in this workspace's preset (else the right dock).
    pub fn show_panel(&mut self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(PANELS.contains(&id), "Unknown panel");
        if let Some((di, gi, ti)) = self.find(id) {
            self.docks[di].groups[gi].active = ti;
            self.docks[di].collapsed = false;
            return Ok(());
        }
        let preset = Layout::preset(&self.workspace);
        let home = preset
            .docks
            .iter()
            .find(|d| d.groups.iter().any(|g| g.tabs.iter().any(|t| t == id)))
            .map_or("right", |d| d.id.as_str());
        let mut next = self.clone();
        let g = self::group(&[id], None);
        match next.docks.iter_mut().find(|d| d.id == home) {
            Some(d) if !d.groups.is_empty() => {
                d.collapsed = false;
                let first = &mut d.groups[0];
                first.tabs.push(id.into());
                first.active = first.tabs.len() - 1;
            }
            Some(d) => {
                d.collapsed = false;
                d.groups.push(g);
            }
            None => {
                let size = preset
                    .docks
                    .iter()
                    .find(|d| d.id == home)
                    .and_then(|d| d.size);
                next.docks
                    .push(dock(home, size.or(Some(DEFAULT_RIGHT)), vec![g]));
            }
        }
        next.commit(self)
    }
    /// Panel ids currently in the layout, in dock/group/tab order.
    pub fn open_panels(&self) -> Vec<&str> {
        self.docks
            .iter()
            .flat_map(|d| d.groups.iter())
            .flat_map(|g| g.tabs.iter().map(String::as_str))
            .collect()
    }
    fn find(&self, id: &str) -> Option<(usize, usize, usize)> {
        self.docks.iter().enumerate().find_map(|(di, d)| {
            d.groups
                .iter()
                .enumerate()
                .find_map(|(gi, g)| g.tabs.iter().position(|p| p == id).map(|ti| (di, gi, ti)))
        })
    }
    /// Normalizes empty groups/active tabs, validates, then replaces `into`.
    fn commit(mut self, into: &mut Layout) -> anyhow::Result<()> {
        for d in &mut self.docks {
            d.groups.retain(|g| !g.tabs.is_empty());
            for g in &mut d.groups {
                g.active = g.active.min(g.tabs.len() - 1);
            }
            if d.groups.iter().all(|g| g.size.is_some())
                && let Some(g) = d.groups.first_mut()
            {
                g.size = None;
            }
        }
        self.validate()?;
        *into = self;
        Ok(())
    }
    pub fn move_panel(
        &mut self,
        id: &str,
        to_dock: usize,
        to_group: usize,
        as_tab: bool,
    ) -> anyhow::Result<()> {
        let target = if as_tab {
            DropTarget::Tab {
                dock: to_dock,
                group: to_group,
                index: usize::MAX,
            }
        } else {
            DropTarget::Split {
                dock: to_dock,
                group: to_group,
            }
        };
        self.move_panel_to(id, target)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutStore {
    pub version: u32,
    #[serde(default)]
    pub active: std::collections::BTreeMap<String, String>,
    pub projects: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Layout>>,
    /// User-named workspaces, shared by every project (Window ▸ Workspace).
    #[serde(default)]
    pub saved: std::collections::BTreeMap<String, Layout>,
}
/// Longest accepted saved-layout name, in characters.
pub const LAYOUT_NAME_MAX: usize = 64;
/// Trims and validates a saved-layout name.
pub fn layout_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "Enter a name for the workspace.");
    anyhow::ensure!(
        name.chars().count() <= LAYOUT_NAME_MAX,
        "Workspace names can be up to {LAYOUT_NAME_MAX} characters."
    );
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "Workspace names can’t contain control characters."
    );
    Ok(name.to_string())
}
impl LayoutStore {
    /// Saves `layout` under a user name, replacing an existing one with that name.
    pub fn save_named(&mut self, name: &str, layout: &Layout) -> anyhow::Result<String> {
        let name = layout_name(name)?;
        layout.validate()?;
        self.version = 1;
        self.saved.insert(name.clone(), layout.clone());
        Ok(name)
    }
    /// The saved layout, re-labelled for `workspace` so it replaces that workspace's
    /// arrangement (presets keep their identity; a saved layout is an arrangement).
    pub fn named(&self, name: &str, workspace: &str) -> Option<Layout> {
        self.saved.get(name).map(|l| Layout {
            workspace: workspace.into(),
            ..l.clone()
        })
    }
    pub fn delete_named(&mut self, name: &str) -> bool {
        self.saved.remove(name).is_some()
    }
    pub fn put(&mut self, project: &str, layout: Layout) {
        self.version = 1;
        self.active.insert(project.into(), layout.workspace.clone());
        self.projects
            .entry(project.into())
            .or_default()
            .insert(layout.workspace.clone(), layout);
    }
    pub fn get(&self, project: &str, workspace: &str) -> Layout {
        self.projects
            .get(project)
            .and_then(|p| p.get(workspace))
            .cloned()
            .unwrap_or_else(|| Layout::preset(workspace))
    }
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        anyhow::ensure!(s.version == 1, "Unsupported layout store");
        for p in s.projects.values() {
            for l in p.values() {
                l.validate()?;
            }
        }
        for (name, l) in &s.saved {
            layout_name(name)?;
            l.validate()?;
        }
        Ok(s)
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = path.with_extension("json.pending");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            f.sync_all()?;
        }
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
/// Share of the docking area an edge-dock preview covers (the outer 25 %).
pub const EDGE_ZONE: f32 = 0.25;
/// Empty state for a panel with nothing to show (language §9/§14): a
/// `heading`, one `reading` line and an optional action (label, command).
pub fn empty_state(
    id: &str,
) -> (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
) {
    match id {
        "assets" => (
            "No Media",
            "Import files or drag them here.",
            Some(("Import…", "import")),
        ),
        "inspector" => (
            "No Selection",
            "Select a layer to see its properties.",
            None,
        ),
        "layer-effects" => ("No Selection", "Select a layer to see its effects.", None),
        "fxbrowser" => (
            "No Effects",
            "Installed effects and presets appear here.",
            None,
        ),
        "viewer" | "timeline" => (
            "No Composition",
            "Open a project to start editing.",
            Some(("Open…", "open")),
        ),
        "gradient-controls" => ("No Gradient", "Select a gradient layer to edit it.", None),
        "takes" => ("No Takes", "Takes you record appear here.", None),
        "shader" => (
            "No Shader",
            "Select a shader layer to edit its source.",
            None,
        ),
        "perf" => (
            "No Measurements",
            "Play the composition to measure it.",
            None,
        ),
        "notes" => (
            "No Notes",
            "Notes you add to this project appear here.",
            None,
        ),
        _ => ("Nothing Here", "This panel has nothing to show yet.", None),
    }
}
pub fn panel_title(id: &str) -> &str {
    match id {
        "assets" => "Project",
        "layer-effects" => "Effect Controls",
        "viewer" => "Viewer",
        "timeline" => "Timeline",
        "inspector" => "Properties",
        "fxbrowser" => "Effects",
        "gradient-controls" => "Gradient",
        "takes" => "Takes",
        "shader" => "Shader",
        "perf" => "Performance",
        "notes" => "Notes",
        _ => id,
    }
}
/// Every built-in panel, in Window ▸ Panels order.
pub const PANELS: [&str; 11] = [
    "assets",
    "layer-effects",
    "viewer",
    "timeline",
    "inspector",
    "fxbrowser",
    "gradient-controls",
    "takes",
    "shader",
    "perf",
    "notes",
];
/// Window edges that can receive a docked panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Bottom,
}
impl Edge {
    pub fn dock_id(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Bottom => "bottom",
        }
    }
}
/// Where a dragged panel tab lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// Join a tab group at `index` (an index past the end appends).
    Tab {
        dock: usize,
        group: usize,
        index: usize,
    },
    /// A new group inserted into `dock` before `group` (`group == len` appends).
    Split { dock: usize, group: usize },
    /// Dock at a window edge; creates or reopens that dock.
    Edge(Edge),
}
/// A drop target plus the rectangle the shell highlights while dragging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropPreview {
    pub target: DropTarget,
    pub rect: Rect,
}
