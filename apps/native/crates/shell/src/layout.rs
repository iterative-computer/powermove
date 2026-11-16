//! Workspace geometry; bottom spans all columns except the full-height right dock.
use pm_design::{Tokens, workspace::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
/// `area` less `holes`, as horizontal bands of rectangles.
pub fn outside(area: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut edges = vec![area.y, area.y + area.h];
    for h in holes {
        edges.extend([h.y, h.y + h.h]);
    }
    edges.retain(|y| *y >= area.y && *y <= area.y + area.h);
    edges.sort_by(f32::total_cmp);
    edges.dedup();
    let mut out = vec![];
    for band in edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        if bottom <= top {
            continue;
        }
        let mut cuts: Vec<(f32, f32)> = holes
            .iter()
            .filter(|h| h.y <= top && h.y + h.h >= bottom)
            .map(|h| (h.x.max(area.x), (h.x + h.w).min(area.x + area.w)))
            .filter(|(a, b)| b > a)
            .collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = area.x;
        for (a, b) in cuts.into_iter().chain([(area.x + area.w, area.x + area.w)]) {
            if a > x {
                out.push(Rect {
                    x,
                    y: top,
                    w: a - x,
                    h: bottom - top,
                });
            }
            x = x.max(b);
        }
    }
    out
}
impl Rect {
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self {
            x,
            y,
            w: ((self.x + self.w).min(other.x + other.w) - x).max(0.),
            h: ((self.y + self.h).min(other.y + other.h) - y).max(0.),
        }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub tabs: Vec<String>,
    pub active: usize,
    pub size: Option<f32>,
    /// Folded to its header (a double-click on the header, as in the
    /// original app); remembered with the workspace.
    #[serde(default)]
    pub collapsed: bool,
}
impl Group {
    pub fn active(&self) -> &str {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }
}
/// Content-width tab titles. Tabs that do not fit fold into the trailing More
/// Panels menu, whose hit square is always reserved; the active tab always
/// stays shown. A title truncates only when the active tab alone is wider than
/// the room left beside More Panels (`active_limit`).
/// The caller reserves space for header tools before asking for this track.
#[derive(Clone, Debug, PartialEq)]
pub struct TabTrack {
    pub visible: Vec<usize>,
    pub overflow: Vec<usize>,
    /// Width the active tab must fit in, when it cannot take its full title.
    pub active_limit: Option<f32>,
}
impl Group {
    /// `widths[i]` is the room tab `i` takes, padding and spacing included.
    pub fn tab_track(&self, available: f32, widths: &[f32], tokens: &Tokens) -> TabTrack {
        let n = self.tabs.len();
        let width = |i: usize| widths.get(i).copied().unwrap_or(tokens.metrics.hit * 3.);
        let total = |set: &[usize]| set.iter().map(|&i| width(i)).sum::<f32>();
        let all: Vec<_> = (0..n).collect();
        if total(&all) <= available {
            return TabTrack {
                visible: all,
                overflow: vec![],
                active_limit: None,
            };
        }
        // The More Panels chevron takes one hit square.
        let room = (available - tokens.metrics.hit).max(0.);
        let active = self.active.min(n.saturating_sub(1));
        let mut visible = vec![active];
        for i in (0..n).filter(|&i| i != active) {
            let mut next = visible.clone();
            next.push(i);
            if total(&next) <= room {
                visible = next;
            }
        }
        visible.sort_unstable();
        let overflow = (0..n).filter(|i| !visible.contains(i)).collect();
        TabTrack {
            visible,
            overflow,
            active_limit: (width(active) > room).then_some(room),
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
    /// Fluid height bases keyed by the first panel in each group.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flex_bases: BTreeMap<String, f32>,
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
    pub side_docks: std::collections::BTreeMap<usize, Rect>,
    pub panels: Vec<Placement>,
    pub dividers: Vec<Divider>,
    /// The tabs (or the lone title) each header shows, as drawn: per
    /// `(dock, group)`, `(tab index, left, right)` in window points. The
    /// shell fills it as it draws the headers; a drop over a header lands
    /// at the tab the pointer is on. Empty, the header is shared evenly.
    pub tabs: std::collections::HashMap<(usize, usize), Vec<(usize, f32, f32)>>,
}
impl Geometry {
    /// Bubble-only wheel routing: inner scroll views have first refusal. The
    /// dock's divider grab extends past its viewport into the edge gutter.
    pub fn wheel_dock(&self, x: f32, y: f32, reach: f32) -> Option<usize> {
        self.side_docks
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(di, _)| *di)
            .or_else(|| {
                self.dividers.iter().find_map(|d| {
                    if d.horizontal || d.group.is_some() {
                        return None;
                    }
                    let viewport = self.side_docks.get(&d.dock)?;
                    let hit = Rect {
                        x: d.rect.x - reach,
                        w: d.rect.w + 2. * reach,
                        y: viewport.y,
                        h: viewport.h,
                    };
                    hit.contains(x, y).then_some(d.dock)
                })
            })
    }
}

fn dock_for(id: &str, g: Group) -> Dock {
    dock(id, Some(DEFAULT_RIGHT), vec![g])
}
fn group(ids: &[&str], size: Option<f32>) -> Group {
    Group {
        tabs: ids.iter().map(|s| s.to_string()).collect(),
        active: 0,
        size,
        collapsed: false,
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
    /// A built-in workspace's layout (the design one for any other id).
    pub fn preset(id: &str) -> Self {
        crate::workspace::layout(
            &crate::workspace::preset(id)
                .or_else(|| crate::workspace::preset(crate::workspace::FALLBACK))
                .unwrap_or_default(),
        )
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.flex_bases.values().all(|n| n.is_finite() && *n >= 0.),
            "Invalid fluid panel basis"
        );
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
            ids.iter().any(|id| *id == VIEWER),
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
                        ..Geometry::default()
                    };
                }
            }
        }
        self.geometry(bounds, t)
    }

    /// Bounded docking geometry used to plan moves and previews. Content-aware
    /// side scrolling is measured separately by `geometry_with_panel_heights`.
    pub fn geometry(&self, bounds: Rect, t: &Tokens) -> Geometry {
        self.geometry_impl(bounds, t, None)
    }

    /// Natural panel heights are transient measurements, never saved layout sizes.
    pub fn geometry_with_panel_heights(
        &self,
        bounds: Rect,
        t: &Tokens,
        natural: &BTreeMap<String, f32>,
    ) -> Geometry {
        self.geometry_impl(bounds, t, Some(natural))
    }

    fn geometry_impl(
        &self,
        bounds: Rect,
        t: &Tokens,
        natural: Option<&BTreeMap<String, f32>>,
    ) -> Geometry {
        let mut out = Geometry::default();
        let gap = t.metrics.dock_gap;
        let shown = |d: &&Dock| !d.collapsed && !d.groups.is_empty();
        let right = self
            .docks
            .iter()
            .enumerate()
            .find(|(_, d)| d.id == "right" && shown(d));
        let bottom = self
            .docks
            .iter()
            .enumerate()
            .find(|(_, d)| d.id == "bottom" && shown(d));
        // A dock dropped at the window's top edge: across the timeline's
        // row, above the columns (the bottom dock's mirror).
        let top = self
            .docks
            .iter()
            .enumerate()
            .find(|(_, d)| d.id == "top" && shown(d));
        // v1's arrangement: every other dock is a column, in authored order;
        // the fluid one (center, else the viewer's) takes the width left over.
        let columns: Vec<usize> = (0..self.docks.len())
            .filter(|&i| {
                let d = &self.docks[i];
                shown(&d) && !is_band(&d.id) && d.id != "right"
            })
            .collect();
        let fluid = self.fluid(&columns);
        let width = |d: &Dock| {
            d.size.unwrap_or(if d.id == "right" {
                DEFAULT_RIGHT
            } else {
                DEFAULT_LEFT
            })
        };
        let fixed: Vec<usize> = columns
            .iter()
            .copied()
            .filter(|&i| Some(i) != fluid)
            .collect();
        let gaps = (columns.len().saturating_sub(1) + usize::from(right.is_some())) as f32 * gap;
        let available = (bounds.w - t.metrics.min_center - gaps).max(0.);
        let requested = fixed.iter().map(|&i| width(&self.docks[i])).sum::<f32>()
            + right.map_or(0., |(_, d)| width(d));
        let factor = if requested > available {
            available / requested
        } else {
            1.
        };
        let rw = right.map_or(0., |(_, d)| width(d) * factor);
        let rg = if right.is_some() { gap } else { 0. };
        let bh = bottom.map_or(0., |(_, d)| {
            d.size
                .unwrap_or(DEFAULT_TIMELINE)
                .max(dock_floor(d, t))
                .min((bounds.h - upper_least(bounds, t) - gap).max(0.))
        });
        let bg = if bottom.is_some() { gap } else { 0. };
        let th = top.map_or(0., |(_, d)| {
            d.size
                .unwrap_or(DEFAULT_TOP)
                .max(dock_floor(d, t))
                .min((bounds.h - bh - bg - upper_least(bounds, t) - gap).max(0.))
        });
        let tg = if top.is_some() { gap } else { 0. };
        let upper = bounds.h - bh - bg;
        let columns_y = bounds.y + th + tg;
        let columns_h = (upper - th - tg).max(0.);
        let row = (bounds.w - rw - rg).max(0.);
        let fixed_w: f32 = fixed.iter().map(|&i| width(&self.docks[i]) * factor).sum();
        let gaps_w = columns.len().saturating_sub(1) as f32 * gap;
        let fluid_w = (row - fixed_w - gaps_w).max(0.);
        let mut rects = std::collections::HashMap::new();
        let mut x = bounds.x;
        for &i in &columns {
            let w = if Some(i) == fluid {
                fluid_w
            } else {
                width(&self.docks[i]) * factor
            };
            rects.insert(
                i,
                Rect {
                    x,
                    y: columns_y,
                    w,
                    h: columns_h,
                },
            );
            x += w + gap;
        }
        if let Some((i, _)) = right {
            rects.insert(
                i,
                Rect {
                    x: bounds.x + bounds.w - rw,
                    y: bounds.y,
                    w: rw,
                    h: bounds.h,
                },
            );
        }
        if let Some((i, _)) = bottom {
            rects.insert(
                i,
                Rect {
                    x: bounds.x,
                    y: bounds.y + upper + bg,
                    w: row,
                    h: bh,
                },
            );
        }
        if let Some((i, _)) = top {
            rects.insert(
                i,
                Rect {
                    x: bounds.x,
                    y: bounds.y,
                    w: row,
                    h: th,
                },
            );
        }
        for (di, d) in self
            .docks
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.collapsed && !d.groups.is_empty())
        {
            let rect = rects[&di];
            let n = d.groups.len();
            let space = (rect.h - gap * (n - 1) as f32).max(0.);
            let heights = if natural.is_some() && matches!(d.id.as_str(), "left" | "right") {
                out.side_docks.insert(di, rect);
                side_group_heights(
                    &d.groups,
                    space,
                    bounds.h + t.metrics.toolbar + t.metrics.status + t.metrics.dock_gap,
                    t,
                    natural.unwrap(),
                    &self.flex_bases,
                )
            } else {
                group_heights(&d.groups, space, t)
            };
            let mut y = rect.y;
            for (gi, _) in d.groups.iter().enumerate() {
                let h = heights[gi];
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
            // Columns before the fluid one resize from their trailing edge,
            // later ones (and the right dock) from their leading edge.
            let divider = match d.id.as_str() {
                "bottom" => Some((
                    true,
                    Rect {
                        x: rect.x,
                        y: rect.y - gap,
                        w: rect.w,
                        h: gap,
                    },
                )),
                "top" => Some((
                    true,
                    Rect {
                        x: rect.x,
                        y: rect.y + rect.h,
                        w: rect.w,
                        h: gap,
                    },
                )),
                _ if Some(di) == fluid => None,
                _ if self.trailing(di) => Some((
                    false,
                    Rect {
                        x: rect.x - gap,
                        y: rect.y,
                        w: gap,
                        h: rect.h,
                    },
                )),
                _ => Some((
                    false,
                    Rect {
                        x: rect.x + rect.w,
                        y: rect.y,
                        w: gap,
                        h: rect.h,
                    },
                )),
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
    /// The column that takes the width the others leave: `center`, else the
    /// viewer's or the timeline's dock, else the first column (v1's rule).
    fn fluid(&self, columns: &[usize]) -> Option<usize> {
        let has = |i: &usize, id: &str| {
            self.docks[*i]
                .groups
                .iter()
                .any(|g| g.tabs.iter().any(|t| t == id))
        };
        columns
            .iter()
            .find(|&&i| self.docks[i].id == "center")
            .or_else(|| {
                columns
                    .iter()
                    .find(|i| has(i, VIEWER) || has(i, "timeline"))
            })
            .or(columns.first())
            .copied()
    }
    /// A dock that sits after the fluid column (or is the right dock): it
    /// grows when its leading edge moves left.
    fn trailing(&self, dock: usize) -> bool {
        if self.docks[dock].id == "right" {
            return true;
        }
        let columns: Vec<usize> = (0..self.docks.len())
            .filter(|&i| {
                let d = &self.docks[i];
                !d.collapsed && !d.groups.is_empty() && d.id != "right" && !is_band(&d.id)
            })
            .collect();
        self.fluid(&columns).is_some_and(|f| dock > f)
    }
    /// Moves `divider` by `delta` points from this layout as laid out in
    /// `bounds`: the panels on either side take their present sizes (none
    /// jumps), the one growing takes only what its neighbour can give (it
    /// keeps `min_panel`; the fluid column keeps `min_center`; the timeline
    /// its least height), so nothing overlaps or scales the other docks, and
    /// dragging back to where it began gives the layout it began with.
    pub fn resize(&mut self, divider: &Divider, delta: f32, bounds: Rect, t: &Tokens) {
        self.resize_impl(divider, delta, bounds, t, None);
    }

    pub fn resize_with_panel_heights(
        &mut self,
        divider: &Divider,
        delta: f32,
        bounds: Rect,
        t: &Tokens,
        natural: &BTreeMap<String, f32>,
    ) {
        self.resize_impl(divider, delta, bounds, t, Some(natural));
    }

    fn resize_impl(
        &mut self,
        divider: &Divider,
        delta: f32,
        bounds: Rect,
        t: &Tokens,
        natural: Option<&BTreeMap<String, f32>>,
    ) {
        let geometry = self.geometry_impl(bounds, t, natural);
        let min = t.metrics.min_panel;
        let height = |dock: usize, group: usize| {
            geometry
                .panels
                .iter()
                .find(|p| (p.dock, p.group) == (dock, group))
                .map(|p| p.rect.h)
        };
        if let Some(i) = divider.group {
            let (Some(a), Some(b)) = (height(divider.dock, i), height(divider.dock, i + 1)) else {
                return;
            };
            let total = a + b;
            let side = natural.is_some()
                && matches!(self.docks[divider.dock].id.as_str(), "left" | "right");
            let groups = &self.docks[divider.dock].groups;
            let least = |g: &Group| {
                let floor = group_least(g, t);
                if side && !g.collapsed && g.size.is_none() {
                    floor.max(200.)
                } else {
                    floor
                }
            };
            let (least_a, least_b) = (least(&groups[i]), least(&groups[i + 1]));
            let upper = if total >= least_a + least_b {
                (a + delta).clamp(least_a, total - least_b)
            } else {
                a
            };
            let d = &mut self.docks[divider.dock];
            if side {
                // Freeze every open fluid group's displayed share before
                // changing the pair, so spare space is not redistributed.
                for (index, group) in d.groups.iter().enumerate() {
                    if !group.collapsed
                        && group.size.is_none()
                        && let Some(displayed) = height(divider.dock, index)
                    {
                        self.flex_bases.insert(group.tabs[0].clone(), displayed);
                    }
                }
                // A side stack can exceed its viewport. Record the requested
                // adjacent heights; never turn a fixed neighbour into a fluid
                // filler, or it would shrink as soon as this splitter moves.
                for (index, height) in [(i, upper), (i + 1, total - upper)] {
                    let group = &mut d.groups[index];
                    if group.size.is_none() {
                        self.flex_bases.insert(group.tabs[0].clone(), height);
                    } else {
                        group.size = Some(height);
                    }
                }
                return;
            }
            // Every group at the height it shows; the first that was flexible
            // stays so (it takes exactly that height, and a window resize).
            let flexible = d
                .groups
                .iter()
                .position(|g| g.size.is_none())
                .unwrap_or(d.groups.len() - 1);
            for (gi, g) in d.groups.iter_mut().enumerate() {
                g.size = height(divider.dock, gi);
            }
            d.groups[i].size = Some(upper);
            d.groups[i + 1].size = Some(total - upper);
            d.groups[flexible].size = None;
            return;
        }
        let rect = |dock: usize| {
            let mut placed = geometry.panels.iter().filter(|p| p.dock == dock);
            let first = placed.next()?.rect;
            let last = placed.last().map_or(first, |p| p.rect);
            Some(Rect {
                h: last.y + last.h - first.y,
                ..first
            })
        };
        let Some(current) = rect(divider.dock) else {
            return;
        };
        if is_band(&self.docks[divider.dock].id) {
            // The other band and the columns' least height are what it may
            // not take.
            let gap = t.metrics.dock_gap;
            let other: f32 = geometry
                .panels
                .iter()
                .filter(|p| p.dock != divider.dock && is_band(&self.docks[p.dock].id))
                .map(|p| p.rect.h + gap)
                .sum();
            // The original app: at least `band_least` (and its panels'
            // floors), leaving `band_room` for the columns.
            let least = dock_floor(&self.docks[divider.dock], t).max(t.metrics.band_least);
            let most = (bounds.h - other - upper_least(bounds, t) - gap).max(least);
            // The bottom band grows as its top edge moves up; the top band
            // as its bottom edge moves down.
            let grow = if self.docks[divider.dock].id == "top" {
                delta
            } else {
                -delta
            };
            self.docks[divider.dock].size = Some((current.h + grow).clamp(least, most));
            return;
        }
        // Columns give and take with the fluid column only; every fixed
        // column keeps the width it shows (so none rescales).
        let columns: Vec<usize> = (0..self.docks.len())
            .filter(|&i| {
                let d = &self.docks[i];
                !d.collapsed && !d.groups.is_empty() && !is_band(&d.id)
            })
            .collect();
        let fluid = self.fluid(
            &columns
                .iter()
                .copied()
                .filter(|&i| self.docks[i].id != "right")
                .collect::<Vec<_>>(),
        );
        for &i in &columns {
            if Some(i) != fluid
                && let Some(r) = rect(i)
            {
                self.docks[i].size = Some(r.w);
            }
        }
        let room = fluid
            .and_then(rect)
            .map_or(0., |f| (f.w - t.metrics.min_center).max(0.));
        let sign = if self.trailing(divider.dock) { -1. } else { 1. };
        // The original app's side columns: `column_least` to `column_most`
        // wide, and never more than the fluid column can give.
        let _ = min;
        let least = t.metrics.column_least.min(current.w);
        let most = (current.w + room)
            .min(t.metrics.column_most.max(current.w))
            .max(least);
        let width = (current.w + delta * sign).clamp(least, most);
        self.docks[divider.dock].size = Some(width);
    }
    /// Where `(x, y)` would drop a dragged panel, in the coordinates of `geometry`.
    /// `bounds` is the docking area passed to [`Layout::geometry`].
    ///
    /// The original app's zones (After Effects' docking, on columns that
    /// stack panels top to bottom):
    /// - over a panel's header: join its tabs where the pointer is, before
    ///   the first tab whose middle is right of it;
    /// - over its body: the top 30% docks above it, the bottom 30% below,
    ///   the middle joins its tabs (last); a panel that takes no tabs (the
    ///   viewer) splits at its middle into above and below;
    /// - off every panel: the nearest dock within `dock_reach` takes it
    ///   between the panels nearest the pointer, and where no left, right or
    ///   bottom dock stands a strip along that edge makes one.
    ///
    /// Side-by-side columns and the top band are not pointer zones (the
    /// original app has neither); the panel menu still offers them. `rect`
    /// is the zone; [`Layout::drop_target_for`] gives the panel's exact place.
    pub fn drop_target(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        x: f32,
        y: f32,
        t: &Tokens,
    ) -> Option<DropPreview> {
        // A folded panel is not a zone (its header is all there is of it):
        // the dock around it places the panel above or below it.
        if let Some(p) = geometry
            .panels
            .iter()
            .find(|p| p.rect.contains(x, y) && !self.docks[p.dock].groups[p.group].collapsed)
        {
            return Some(self.panel_zone(geometry, p, x, y, t, true));
        }
        self.off_panel_target(geometry, bounds, x, y, t)
    }
    /// Off every panel: the nearest dock of `geometry` within `dock_reach`
    /// (between its panels), or a strip where a side or bottom dock is missing.
    fn off_panel_target(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        x: f32,
        y: f32,
        t: &Tokens,
    ) -> Option<DropPreview> {
        let m = &t.metrics;
        let gap = m.dock_gap;
        let shown = |id: &str| {
            self.docks
                .iter()
                .position(|d| d.id == id && !d.collapsed && !d.groups.is_empty())
        };
        let dock_rect = |di: usize| {
            let mut placed = geometry.panels.iter().filter(|p| p.dock == di);
            let first = placed.next()?.rect;
            Some(placed.fold(first, |r, p| {
                let (x0, y0) = (r.x.min(p.rect.x), r.y.min(p.rect.y));
                let x1 = (r.x + r.w).max(p.rect.x + p.rect.w);
                let y1 = (r.y + r.h).max(p.rect.y + p.rect.h);
                Rect {
                    x: x0,
                    y: y0,
                    w: x1 - x0,
                    h: y1 - y0,
                }
            }))
        };
        // Strips where a dock is missing come first: on a tie they win.
        let mut docks: Vec<(Option<Edge>, Option<usize>, Rect)> = vec![];
        if shown("left").is_none() {
            docks.push((
                Some(Edge::Left),
                None,
                Rect {
                    w: m.edge_strip.min(bounds.w),
                    ..bounds
                },
            ));
        }
        let right = shown("right").and_then(dock_rect);
        if right.is_none() {
            let w = m.edge_strip.min(bounds.w);
            docks.push((
                Some(Edge::Right),
                None,
                Rect {
                    x: bounds.x + bounds.w - w,
                    w,
                    ..bounds
                },
            ));
        }
        if shown("bottom").is_none() {
            // The bottom dock runs under every column but the right one.
            let end = right.map_or(bounds.x + bounds.w, |r| r.x - gap);
            let h = m.edge_strip_bottom.min(bounds.h);
            docks.push((
                Some(Edge::Bottom),
                None,
                Rect {
                    x: bounds.x,
                    y: bounds.y + bounds.h - h,
                    w: (end - bounds.x).max(0.),
                    h,
                },
            ));
        }
        for (di, d) in self.docks.iter().enumerate() {
            if d.collapsed || d.groups.is_empty() {
                continue;
            }
            if let Some(r) = dock_rect(di) {
                docks.push((None, Some(di), r));
            }
        }
        let reach = m.dock_reach;
        let mut best: Option<(f32, &(Option<Edge>, Option<usize>, Rect))> = None;
        for dock in &docks {
            let r = dock.2;
            if x < r.x - reach || x > r.x + r.w + reach || y < r.y - reach || y > r.y + r.h + reach
            {
                continue;
            }
            let dx = (r.x - x).max(x - (r.x + r.w)).max(0.);
            let dy = (r.y - y).max(y - (r.y + r.h)).max(0.);
            let distance = dx * dx + dy * dy;
            if best.is_none_or(|(b, _)| distance < b) {
                best = Some((distance, dock));
            }
        }
        let (_, &(edge, dock, r)) = best?;
        if let Some(edge) = edge {
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
                Edge::Bottom | Edge::Top => Rect {
                    y: bounds.y + bounds.h * (1. - EDGE_ZONE),
                    h: bounds.h * EDGE_ZONE,
                    ..r
                },
            };
            return Some(DropPreview {
                target: DropTarget::Edge(edge),
                rect,
                near: Near::Edge(edge),
            });
        }
        // Between the panels of a dock: before the first whose middle is
        // below the pointer, else at its end.
        let dock = dock?;
        let placed: Vec<&Placement> = geometry.panels.iter().filter(|p| p.dock == dock).collect();
        let index = placed
            .iter()
            .find(|p| y < p.rect.y + p.rect.h / 2.)
            .map_or(self.docks[dock].groups.len(), |p| p.group);
        let line = placed
            .iter()
            .find(|p| p.group == index)
            .map(|p| p.rect.y - gap)
            .or_else(|| placed.last().map(|p| p.rect.y + p.rect.h))
            .unwrap_or(r.y);
        let near = if index < self.docks[dock].groups.len() {
            Near::Above(dock, index)
        } else if index > 0 {
            Near::Below(dock, index - 1)
        } else {
            Near::Dock(dock)
        };
        Some(DropPreview {
            target: DropTarget::Split { dock, group: index },
            rect: Rect {
                y: line,
                h: gap,
                ..r
            },
            near,
        })
    }
    /// Where a floating panel moved by its own grip (the Agent popover)
    /// would dock, as the original app's floating drag: only into the left
    /// or right column (or the strip where one is missing), and only near a
    /// column's edge (`floating_edge`) or between its panels
    /// (`floating_seam`); anywhere else it keeps floating (`None`).
    pub fn floating_drop_target(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        x: f32,
        y: f32,
        t: &Tokens,
    ) -> Option<DropPreview> {
        // The original app ignores panel zones for a floating drag: only the
        // side columns' inserts between panels, and their strips.
        let side = |p: &Placement| matches!(self.docks[p.dock].id.as_str(), "left" | "right");
        let only_sides = Geometry {
            panels: geometry
                .panels
                .iter()
                .filter(|p| side(p))
                .cloned()
                .collect(),
            ..Geometry::default()
        };
        let preview = self.off_panel_target(&only_sides, bounds, x, y, t)?;
        let near = |r: Rect| {
            let edge = (x - r.x)
                .abs()
                .min((x - r.x - r.w).abs())
                .min((y - r.y).abs())
                .min((y - r.y - r.h).abs());
            edge <= FLOATING_EDGE
        };
        let ok = match preview.target {
            DropTarget::Edge(Edge::Left | Edge::Right) => true,
            DropTarget::Split { dock, .. }
                if matches!(self.docks[dock].id.as_str(), "left" | "right") =>
            {
                let panels: Vec<Rect> = geometry
                    .panels
                    .iter()
                    .filter(|p| p.dock == dock)
                    .map(|p| p.rect)
                    .collect();
                let column = panels.iter().copied().reduce(|a, b| Rect {
                    x: a.x.min(b.x),
                    y: a.y.min(b.y),
                    w: (a.x + a.w).max(b.x + b.w) - a.x.min(b.x),
                    h: (a.y + a.h).max(b.y + b.h) - a.y.min(b.y),
                });
                column.is_some_and(near)
                    || panels
                        .iter()
                        .any(|r| (y - r.y).abs().min((y - r.y - r.h).abs()) <= FLOATING_SEAM)
            }
            _ => false,
        };
        ok.then_some(preview)
    }
    /// The zones over panel `p` (see [`Layout::drop_target`]). `groupable`
    /// false when the dragged panel cannot join a stack (the viewer).
    fn panel_zone(
        &self,
        geometry: &Geometry,
        p: &Placement,
        x: f32,
        y: f32,
        t: &Tokens,
        groupable: bool,
    ) -> DropPreview {
        let r = p.rect;
        let g = &self.docks[p.dock].groups[p.group];
        let group = groupable && g.tabs.iter().all(|id| stacks(id));
        let header_end = (r.y + t.metrics.header).min(r.y + r.h);
        let tab = |index: usize| DropPreview {
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
            near: Near::Tab(p.dock, p.group),
        };
        if group && y < header_end {
            // Before the first tab whose middle is right of the pointer.
            let spans = geometry
                .tabs
                .get(&(p.dock, p.group))
                .cloned()
                .unwrap_or_else(|| {
                    let n = g.tabs.len().max(1);
                    let w = r.w / n as f32;
                    (0..n)
                        .map(|i| (i, r.x + w * i as f32, r.x + w * (i + 1) as f32))
                        .collect()
                });
            let index = spans
                .iter()
                .find(|(_, left, right)| x < (left + right) / 2.)
                .map_or(g.tabs.len(), |(i, ..)| *i);
            return tab(index);
        }
        let relative = if y < header_end {
            0.
        } else {
            (y - header_end) / (r.y + r.h - header_end).max(1.)
        };
        let split = |below: bool| {
            let half = ((r.h - t.metrics.dock_gap) / 2.).max(0.);
            DropPreview {
                target: DropTarget::Split {
                    dock: p.dock,
                    group: p.group + usize::from(below),
                },
                rect: Rect {
                    y: if below { r.y + r.h - half } else { r.y },
                    h: half,
                    ..r
                },
                near: if below {
                    Near::Below(p.dock, p.group)
                } else {
                    Near::Above(p.dock, p.group)
                },
            }
        };
        if !group {
            return split(relative >= 0.5);
        }
        if relative < BODY_EDGE {
            split(false)
        } else if relative > 1. - BODY_EDGE {
            split(true)
        } else {
            tab(g.tabs.len())
        }
    }
    /// Resolve a real drag: the target under the pointer and, as its preview,
    /// exactly where the panel would be once dropped there (the move applied
    /// to a copy and laid out). No target when the drop would change nothing
    /// or can't be made, so a drop always lands where the preview showed.
    pub fn drop_target_for(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        at: (f32, f32),
        t: &Tokens,
        id: &str,
        stack: bool,
    ) -> Option<DropPreview> {
        match self.drop_outcome(geometry, bounds, at, t, id, stack) {
            DropOutcome::Drop(preview) => Some(preview),
            _ => None,
        }
    }
    /// [`Layout::drop_target_for`], telling apart a drop that would leave the
    /// panel where it is (the original app's “already here”) from a point
    /// that is no drop zone at all.
    pub fn drop_outcome(
        &self,
        geometry: &Geometry,
        bounds: Rect,
        at: (f32, f32),
        t: &Tokens,
        id: &str,
        stack: bool,
    ) -> DropOutcome {
        let Some((di, gi, _)) = self.find(id) else {
            return DropOutcome::Nowhere;
        };
        let Some(mut preview) = self.drop_target(geometry, bounds, at.0, at.1, t) else {
            return DropOutcome::Nowhere;
        };
        let whole = stack || self.docks[di].groups[gi].tabs.len() == 1;
        // Over the very stack being moved: it is already there.
        if whole
            && geometry
                .panels
                .iter()
                .any(|p| (p.dock, p.group) == (di, gi) && p.rect.contains(at.0, at.1))
        {
            return DropOutcome::Here;
        }
        let source_stacks =
            stacks(id) && (!stack || self.docks[di].groups[gi].tabs.iter().all(|id| stacks(id)));
        if !source_stacks && let DropTarget::Tab { dock, group, .. } = preview.target {
            // A panel that takes no tabs splits the panel instead, at the
            // middle of its body.
            let Some(p) = geometry
                .panels
                .iter()
                .find(|p| (p.dock, p.group) == (dock, group))
            else {
                return DropOutcome::Nowhere;
            };
            preview = self.panel_zone(geometry, p, at.0, at.1, t, false);
        }
        let mut next = self.clone();
        let moved = next.apply_drop(id, &preview, stack);
        if moved.is_err() {
            return DropOutcome::Nowhere;
        }
        if next == *self {
            return DropOutcome::Here;
        }
        let Some((dock, group, _)) = next.find(id) else {
            return DropOutcome::Nowhere;
        };
        match next
            .geometry(bounds, t)
            .panels
            .into_iter()
            .find(|p| (p.dock, p.group) == (dock, group))
        {
            Some(p) => {
                preview.rect = p.rect;
                DropOutcome::Drop(preview)
            }
            None => DropOutcome::Nowhere,
        }
    }
    /// The tab that shows in group `g` of dock `d` once `moving` (one tab,
    /// unless `stack`) has left it: what a drop against that group is named
    /// after and lands beside.
    pub fn shown_after(&self, d: usize, g: usize, moving: &str, stack: bool) -> Option<String> {
        let group = self.docks.get(d)?.groups.get(g)?;
        if stack || group.active() != moving || group.tabs.len() < 2 {
            return Some(group.active().to_owned());
        }
        let rest: Vec<&String> = group.tabs.iter().filter(|t| *t != moving).collect();
        rest.get(group.active.min(rest.len() - 1))
            .map(|t| (*t).clone())
    }
    /// The panel a drop's preview names as the one it lands against (above
    /// or below it).
    pub fn neighbour(&self, near: Near, moving: &str, stack: bool) -> Option<String> {
        match near {
            Near::Above(d, g) | Near::Below(d, g) => self.shown_after(d, g, moving, stack),
            _ => None,
        }
    }
    /// A release over `preview`: the move, then the neighbour's share
    /// (`drop_panel`). The preview is worked out with this same call, so the
    /// panel lands exactly where the chip and the preview said.
    pub fn apply_drop(
        &mut self,
        id: &str,
        preview: &DropPreview,
        stack: bool,
    ) -> anyhow::Result<()> {
        let neighbour = self.neighbour(preview.near, id, stack);
        self.drop_panel(id, preview.target, stack, neighbour.as_deref())
    }
    /// Validated, atomic move. Destination indices address the original layout.
    pub fn move_panel_to(&mut self, id: &str, target: DropTarget) -> anyhow::Result<()> {
        self.move_to(id, target, false)
    }
    /// A drag's drop: the move, then a panel put above or below another
    /// (`neighbour`, the panel the preview showed it against) takes half of
    /// that panel's height, as the preview showed. A sized neighbour would
    /// otherwise keep all of it and leave the panel its least, two rows; a
    /// flexible one already shares the room with it.
    pub fn drop_panel(
        &mut self,
        id: &str,
        target: DropTarget,
        stack: bool,
        neighbour: Option<&str>,
    ) -> anyhow::Result<()> {
        self.move_to(id, target, stack)?;
        let (DropTarget::Split { .. }, Some(neighbour)) = (target, neighbour) else {
            return Ok(());
        };
        let (Some((nd, ng, _)), Some((md, mg, _))) = (self.find(neighbour), self.find(id)) else {
            return Ok(());
        };
        if nd != md || ng == mg {
            return Ok(());
        }
        let groups = &mut self.docks[nd].groups;
        if groups[ng].collapsed {
            return Ok(());
        }
        if let Some(size) = groups[ng].size {
            let half = size / 2.;
            groups[ng].size = Some(half);
            groups[mg].size = Some(half);
        }
        Ok(())
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
                    stacks(id)
                        && (!stack || self.docks[di].groups[gi].tabs.iter().all(|id| stacks(id)))
                        && dest.tabs.iter().all(|id| stacks(id)),
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
            DropTarget::Beside { dock, after } => {
                let d = self
                    .docks
                    .get(dock)
                    .ok_or_else(|| anyhow::anyhow!("Unknown dock"))?;
                anyhow::ensure!(
                    !is_band(&d.id) && !(d.id == "right" && after),
                    "No column there"
                );
                // A lone panel beside its own column changes nothing.
                if whole_source(self, di, gi, stack) && dock == di && d.groups.len() == 1 {
                    return Ok(());
                }
            }
            DropTarget::Edge(_) => {}
        }
        let mut next = self.clone();
        // Capture both bases before changing either first tab. A moved tab
        // must not carry its source group's basis into the destination.
        let destination_basis = match target {
            DropTarget::Tab { dock, group, .. } => next
                .flex_bases
                .remove(&next.docks[dock].groups[group].tabs[0]),
            _ => None,
        };
        let source = &mut next.docks[di].groups[gi];
        let source_basis = next.flex_bases.remove(&source.tabs[0]);
        // A stack keeps its height within its dock; in another dock it
        // shares the room (its old height means nothing there).
        let same_dock = match target {
            DropTarget::Tab { dock, .. } | DropTarget::Split { dock, .. } => dock == di,
            DropTarget::Beside { .. } | DropTarget::Edge(_) => false,
        };
        let moved = if stack {
            Group {
                tabs: std::mem::take(&mut source.tabs),
                active: source.active,
                size: source.size.filter(|_| same_dock),
                collapsed: false,
            }
        } else {
            let item = source.tabs.remove(ti);
            if ti < source.active {
                source.active -= 1;
            }
            self::group(&[&item], None)
        };
        if let Some(basis) = source_basis {
            if let Some(first) = source.tabs.first() {
                next.flex_bases.insert(first.clone(), basis);
            } else if same_dock && !matches!(target, DropTarget::Tab { .. }) {
                next.flex_bases.insert(moved.tabs[0].clone(), basis);
            }
        }
        match target {
            DropTarget::Tab { dock, group, index } => {
                let g = &mut next.docks[dock].groups[group];
                let index = index.min(g.tabs.len());
                g.active = index + moved.active;
                g.tabs.splice(index..index, moved.tabs);
                if let Some(basis) = destination_basis {
                    next.flex_bases.insert(g.tabs[0].clone(), basis);
                }
                next.docks[dock].collapsed = false;
            }
            DropTarget::Split { dock, group } => {
                next.docks[dock].groups.insert(group, moved);
                next.docks[dock].collapsed = false;
            }
            DropTarget::Beside { dock, after } => {
                // Half the width of the column it is put beside, which gives
                // it up; beside the fluid column, a left dock's width.
                let width = match next.docks[dock].size {
                    Some(w) => {
                        next.docks[dock].size = Some(w / 2.);
                        w / 2.
                    }
                    None => DEFAULT_LEFT,
                };
                let mut n = 1;
                while next.docks.iter().any(|d| d.id == format!("column-{n}")) {
                    n += 1;
                }
                let column = self::dock(&format!("column-{n}"), Some(width), vec![moved]);
                let at = if next.docks[dock].id == "right" {
                    dock
                } else {
                    dock + usize::from(after)
                };
                next.docks.insert(at, column);
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
                        Edge::Top => DEFAULT_TOP,
                    };
                    next.add_dock(dock(edge.dock_id(), Some(size), vec![moved]));
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
            "left" | "right" => DropTarget::Beside {
                dock,
                after: placement == "right",
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
        let first = g.tabs[0].clone();
        let item = g.tabs.remove(from);
        // `to` is an insertion slot in the original list; account for the removal.
        let to = if to > from { to - 1 } else { to }.min(g.tabs.len());
        g.tabs.insert(to, item);
        g.active = to;
        rekey_basis(&mut self.flex_bases, &first, g);
        Ok(())
    }
    /// Closes a panel. The viewer cannot be closed.
    pub fn close_panel(&mut self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(closable(id), "The viewer cannot be closed");
        let (di, gi, ti) = self
            .find(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown panel"))?;
        let mut next = self.clone();
        let g = &mut next.docks[di].groups[gi];
        let first = g.tabs[0].clone();
        g.tabs.remove(ti);
        rekey_basis(&mut next.flex_bases, &first, g);
        // The showing tab keeps showing when a tab before it closes.
        if ti < g.active {
            g.active -= 1;
        }
        next.commit(self)
    }
    /// Closes every other panel in `id`'s tab stack (the original app's
    /// Close Other Panels in Group). The viewer stays.
    pub fn close_others(&mut self, id: &str) -> anyhow::Result<()> {
        let (di, gi, _) = self
            .find(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown panel"))?;
        let mut next = self.clone();
        let g = &mut next.docks[di].groups[gi];
        let first = g.tabs[0].clone();
        g.tabs.retain(|t| t == id || !closable(t));
        rekey_basis(&mut next.flex_bases, &first, g);
        g.active = g.tabs.iter().position(|t| t == id).unwrap_or(0);
        next.commit(self)
    }
    /// Folds `id`'s stack to its header, or opens it again (a double-click
    /// on the header, as in the original app). The viewer never folds.
    /// Whether it changed.
    pub fn toggle_collapsed(&mut self, id: &str) -> bool {
        let Some((di, gi, _)) = self.find(id) else {
            return false;
        };
        let g = &mut self.docks[di].groups[gi];
        if g.tabs.iter().any(|t| t == VIEWER) {
            return false;
        }
        g.collapsed = !g.collapsed;
        true
    }
    /// Before a splitter between two panels is dragged: a folded one beside
    /// it opens again (the original app), so the drag has room to work in.
    pub fn expand_beside(&mut self, divider: &Divider) -> bool {
        let Some(i) = divider.group else {
            return false;
        };
        let mut changed = false;
        if let Some(d) = self.docks.get_mut(divider.dock) {
            for g in d.groups.iter_mut().skip(i).take(2) {
                changed |= std::mem::take(&mut g.collapsed);
            }
        }
        changed
    }
    /// A dock's size as its workspace shipped it (the original app's
    /// double-click on a dock's splitter); false for a divider between panels.
    /// A double-click on a divider between two panels: each goes back to
    /// its height in the workspace as shipped (a panel the preset has in
    /// this dock), else the two share their room equally. The rest of the
    /// dock keeps what it shows.
    pub fn reset_split(&mut self, divider: &Divider, bounds: Rect, t: &Tokens) -> bool {
        self.reset_split_impl(divider, bounds, t, None)
    }
    pub fn reset_split_with_panel_heights(
        &mut self,
        divider: &Divider,
        bounds: Rect,
        t: &Tokens,
        natural: &BTreeMap<String, f32>,
    ) -> bool {
        self.reset_split_impl(divider, bounds, t, Some(natural))
    }
    fn reset_split_impl(
        &mut self,
        divider: &Divider,
        bounds: Rect,
        t: &Tokens,
        natural: Option<&BTreeMap<String, f32>>,
    ) -> bool {
        let Some(i) = divider.group else {
            return false;
        };
        let geometry = self.geometry_impl(bounds, t, natural);
        let height = |group: usize| {
            geometry
                .panels
                .iter()
                .find(|p| (p.dock, p.group) == (divider.dock, group))
                .map(|p| p.rect.h)
        };
        let (Some(a), Some(b)) = (height(i), height(i + 1)) else {
            return false;
        };
        let total = a + b;
        let preset = Layout::preset(&self.workspace);
        let id = self.docks[divider.dock].id.clone();
        let shipped = |g: &Group| {
            preset
                .docks
                .iter()
                .find(|d| d.id == id)
                .and_then(|d| {
                    d.groups
                        .iter()
                        .find(|p| p.tabs.iter().any(|tab| g.tabs.contains(tab)))
                })
                .and_then(|p| p.size)
        };
        let groups = &self.docks[divider.dock].groups;
        let (least_a, least_b) = (group_least(&groups[i], t), group_least(&groups[i + 1], t));
        let upper = match (shipped(&groups[i]), shipped(&groups[i + 1])) {
            (Some(s), _) => s,
            (None, Some(s)) => total - s,
            (None, None) => total / 2.,
        };
        let upper = if total >= least_a + least_b {
            upper.clamp(least_a, total - least_b)
        } else {
            a
        };
        if (upper - a).abs() < 0.5 {
            return false;
        }
        // Use the same adjacent-height update as a drag, including the
        // transient basis of fluid side panels and fixed neighbours.
        self.resize_impl(divider, upper - a, bounds, t, natural);
        true
    }
    pub fn reset_dock(&mut self, divider: &Divider) -> bool {
        if divider.group.is_some() {
            return false;
        }
        let Some(id) = self.docks.get(divider.dock).map(|d| d.id.clone()) else {
            return false;
        };
        let preset = Layout::preset(&self.workspace);
        let size = preset
            .docks
            .iter()
            .find(|d| d.id == id)
            .and_then(|d| d.size)
            .unwrap_or(match id.as_str() {
                "right" => DEFAULT_RIGHT,
                "bottom" => DEFAULT_TIMELINE,
                "top" => DEFAULT_TOP,
                _ => DEFAULT_LEFT,
            });
        self.docks[divider.dock].size = Some(size);
        true
    }
    /// Shows a panel: activates its tab and reopens its dock, or adds it back to
    /// the dock it occupies in this workspace's preset (else the right dock).
    pub fn show_panel(&mut self, id: &str, panels: &Panels) -> anyhow::Result<()> {
        anyhow::ensure!(panels.contains(id), "Unknown panel");
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
                next.add_dock(dock(home, size.or(Some(DEFAULT_RIGHT)), vec![g]));
            }
        }
        next.commit(self)
    }
    /// Puts panel `id` back at `place` without checks (a parked panel kept in
    /// the saved workspace). Out-of-range places land at the nearest end.
    pub fn insert_at(&mut self, id: &str, place: DropTarget) {
        if self.find(id).is_some() {
            return;
        }
        let single = || self::group(&[id], None);
        match place {
            DropTarget::Tab { dock, group, index } => {
                match self
                    .docks
                    .get_mut(dock)
                    .and_then(|d| d.groups.get_mut(group))
                {
                    Some(g) => {
                        let at = index.min(g.tabs.len());
                        let first = g.tabs[0].clone();
                        g.tabs.insert(at, id.into());
                        rekey_basis(&mut self.flex_bases, &first, g);
                        if at <= g.active && g.tabs.len() > 1 {
                            g.active += 1;
                        }
                    }
                    None => self.insert_at(id, DropTarget::Split { dock, group }),
                }
            }
            DropTarget::Split { dock, group } => match self.docks.get_mut(dock) {
                Some(d) => {
                    let at = group.min(d.groups.len());
                    d.groups.insert(at, single());
                }
                None => self.add_dock(dock_for("right", single())),
            },
            DropTarget::Edge(edge) => {
                match self.docks.iter_mut().find(|d| d.id == edge.dock_id()) {
                    Some(d) => d.groups.push(single()),
                    None => self.add_dock(dock_for(edge.dock_id(), single())),
                }
            }
            DropTarget::Beside { dock, .. } => {
                self.insert_at(id, DropTarget::Split { dock, group: 0 })
            }
        }
    }
    /// Adds a dock where its id puts it: a left dock is the first column.
    fn add_dock(&mut self, d: Dock) {
        if d.id == "left" {
            self.docks.insert(0, d);
        } else {
            self.docks.push(d);
        }
    }
    /// Panel ids currently in the layout, in dock/group/tab order.
    pub fn open_panels(&self) -> Vec<&str> {
        self.docks
            .iter()
            .flat_map(|d| d.groups.iter())
            .flat_map(|g| g.tabs.iter().map(String::as_str))
            .collect()
    }
    pub fn find(&self, id: &str) -> Option<(usize, usize, usize)> {
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
        }
        // A column made by a drop goes when its last panel leaves.
        self.docks
            .retain(|d| !(d.groups.is_empty() && d.id.starts_with("column-")));
        for d in &mut self.docks {
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
/// The least height a group of `tabs` keeps: the timeline its ruler and
/// three rows, any other panel `panel_least` (the original app's 88 pt).
pub fn least_height(tabs: &[String], t: &Tokens) -> f32 {
    if tabs.iter().any(|id| id == "timeline") {
        t.metrics.header + 2. * t.metrics.ruler + 3. * t.metrics.track_row
    } else {
        t.metrics.panel_least
    }
}
/// A group's least height: its header alone while it is folded.
pub fn group_least(g: &Group, t: &Tokens) -> f32 {
    if g.collapsed {
        t.metrics.header
    } else {
        least_height(&g.tabs, t)
    }
}
/// The least height of the row above the timeline: a share of the docking
/// area, so a short window shrinks the timeline too and the viewer never
/// collapses to a header.
fn upper_least(bounds: Rect, t: &Tokens) -> f32 {
    // The original app leaves `band_room` above the timeline's dock.
    let _ = bounds;
    t.metrics.band_room.max(t.metrics.min_panel)
}
/// The least height of a dock: every group's least height and the gaps.
fn dock_floor(d: &Dock, t: &Tokens) -> f32 {
    d.groups.iter().map(|g| group_least(g, t)).sum::<f32>()
        + t.metrics.dock_gap * d.groups.len().saturating_sub(1) as f32
}
/// Electron's minimum for a fluid side panel, before content preference.
pub const SIDE_FLUID_FLOOR: f32 = 200.;

/// The automatic floor includes useful content space, capped before long lists
/// take over the window. A saved splitter basis or fixed size remains authoritative.
pub fn side_group_floor(g: &Group, window_height: f32, t: &Tokens, natural: Option<f32>) -> f32 {
    if g.collapsed {
        return t.metrics.header;
    }
    if let Some(size) = g.size {
        return size.max(group_least(g, t));
    }
    let initial = match g.active() {
        "inspector" | "properties" => 360.,
        "fxbrowser" | "effects" | "agent" | "threads" | "chat" => 400.,
        "easing" | "assets" | "project" | "layer-effects" => 320.,
        _ => SIDE_FLUID_FLOOR,
    };
    natural
        .filter(|h| h.is_finite() && *h > 0.)
        .unwrap_or(initial)
        .clamp(
            SIDE_FLUID_FLOOR,
            (window_height * 0.6).max(SIDE_FLUID_FLOOR),
        )
}

fn side_group_heights(
    groups: &[Group],
    space: f32,
    window_height: f32,
    t: &Tokens,
    natural: &BTreeMap<String, f32>,
    bases: &BTreeMap<String, f32>,
) -> Vec<f32> {
    let mut heights: Vec<f32> = groups
        .iter()
        .map(|g| {
            if !g.collapsed && g.size.is_none() {
                bases
                    .get(&g.tabs[0])
                    .copied()
                    .unwrap_or_else(|| {
                        side_group_floor(g, window_height, t, natural.get(g.active()).copied())
                    })
                    .max(SIDE_FLUID_FLOOR)
            } else {
                side_group_floor(g, window_height, t, natural.get(g.active()).copied())
            }
        })
        .collect();
    let flex = groups
        .iter()
        .filter(|g| !g.collapsed && g.size.is_none())
        .count();
    let extra = (space - heights.iter().sum::<f32>()).max(0.);
    if flex > 0 {
        for (g, h) in groups.iter().zip(&mut heights) {
            if !g.collapsed && g.size.is_none() {
                *h += extra / flex as f32;
            }
        }
    }
    heights
}

fn group_heights(groups: &[Group], space: f32, t: &Tokens) -> Vec<f32> {
    // A folded group is its header, however the rest share the room.
    if groups.iter().any(|g| g.collapsed) {
        let folded: f32 = groups
            .iter()
            .filter(|g| g.collapsed)
            .map(|_| t.metrics.header)
            .sum();
        let open: Vec<Group> = groups.iter().filter(|g| !g.collapsed).cloned().collect();
        let mut rest = group_heights(&open, (space - folded).max(0.), t).into_iter();
        return groups
            .iter()
            .map(|g| {
                if g.collapsed {
                    t.metrics.header.min(space)
                } else {
                    rest.next().unwrap_or(0.)
                }
            })
            .collect();
    }
    if groups.is_empty() {
        return vec![];
    }
    let least: Vec<f32> = groups.iter().map(|g| least_height(&g.tabs, t)).collect();
    let total_least: f32 = least.iter().sum();
    if total_least >= space {
        // No room for every least height: in proportion to them.
        return least
            .iter()
            .map(|l| space * l / total_least.max(1.))
            .collect();
    }
    let mut h: Vec<f32> = groups
        .iter()
        .zip(&least)
        .map(|(g, l)| g.size.map_or(*l, |s| s.max(*l)))
        .collect();
    let flex: Vec<usize> = (0..groups.len())
        .filter(|&i| groups[i].size.is_none())
        .collect();
    // Sized groups give way (down to their least) until the flexible ones
    // have their least heights.
    let sized_extra: f32 = (0..groups.len())
        .filter(|i| !flex.contains(i))
        .map(|i| h[i] - least[i])
        .sum();
    let over = (h.iter().sum::<f32>() - space).max(0.);
    if over > 0. && sized_extra > 0. {
        let keep = ((sized_extra - over) / sized_extra).max(0.);
        for i in (0..groups.len()).filter(|i| !flex.contains(i)) {
            h[i] = least[i] + (h[i] - least[i]) * keep;
        }
    }
    let left = (space - h.iter().sum::<f32>()).max(0.);
    if flex.is_empty() {
        // Every group sized: they share what is left as they are.
        let sum: f32 = h.iter().sum();
        return h.iter().map(|x| x * space / sum.max(1.)).collect();
    }
    // Flexible groups share the rest evenly over their least heights, the
    // least first: each ends at the same height where it can.
    let mut pool = left + flex.iter().map(|&i| h[i]).sum::<f32>();
    let mut open: Vec<usize> = flex.clone();
    loop {
        let share = pool / open.len().max(1) as f32;
        let (low, high): (Vec<usize>, Vec<usize>) = open.iter().partition(|&&i| least[i] > share);
        if low.is_empty() {
            for &i in &open {
                h[i] = share;
            }
            break;
        }
        for &i in &low {
            h[i] = least[i];
            pool -= least[i];
        }
        if high.is_empty() {
            break;
        }
        open = high;
    }
    h
}
/// The whole of a group moves (a header drag, or its only tab).
fn whole_source(l: &Layout, dock: usize, group: usize, stack: bool) -> bool {
    stack || l.docks[dock].groups[group].tabs.len() == 1
}
/// Longest accepted workspace name, in characters.
pub const LAYOUT_NAME_MAX: usize = 80;
/// Trims and validates a workspace name.
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
        "color" => ("No Selection", "Select a layer to grade it.", None),
        "easing" => (
            "No Keyframes",
            "Select keyframes in the timeline to ease them.",
            None,
        ),
        "scopes" => ("No Picture", "Open a composition to see its scopes.", None),
        "mixer" => ("No Audio", "No audio in this composition.", None),
        "captions" => ("No Captions", "Open a project to see its captions.", None),
        _ => ("Nothing Here", "This panel has nothing to show yet.", None),
    }
}
/// The panels one window offers: the built-ins, then what its extensions
/// contribute (id, title), in Window ▸ Panels order. A contributed panel
/// docks, tabs and persists like a built-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panels {
    contributed: Vec<(String, String)>,
    /// Titles of panels seen before that have not registered yet (an
    /// extension still starting): a tab shows its name, never its id.
    remembered: Vec<(String, String)>,
}
impl Panels {
    pub fn new(contributed: Vec<(String, String)>) -> Self {
        Self {
            contributed,
            remembered: Vec::new(),
        }
    }
    pub fn with_remembered(mut self, remembered: Vec<(String, String)>) -> Self {
        self.remembered = remembered;
        self
    }
    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = PANELS.to_vec();
        for (id, _) in &self.contributed {
            if !ids.contains(&id.as_str()) {
                ids.push(id);
            }
        }
        ids
    }
    pub fn contains(&self, id: &str) -> bool {
        PANELS.contains(&id) || self.contributed.iter().any(|(p, _)| p == id)
    }
    /// A contribution's title wins over a built-in's of the same id.
    pub fn title<'a>(&'a self, id: &'a str) -> &'a str {
        self.contributed
            .iter()
            .chain(&self.remembered)
            .find(|(p, _)| p == id)
            .map(|(_, t)| t.as_str())
            .unwrap_or_else(|| builtin_title(id))
    }
}
fn builtin_title(id: &str) -> &str {
    match id {
        "assets" => "Project",
        "layer-effects" => "Effect Controls",
        "viewer" => "Viewer",
        "timeline" => "Timeline",
        "inspector" => "Properties",
        "fxbrowser" => "Effects",
        "agent" => "Agent",
        "gradient-controls" => "Gradient",
        "takes" => "Takes",
        "shader" => "Shader",
        "perf" => "Performance",
        "notes" => "Notes",
        "color" => "Color",
        "easing" => "Easing",
        "scopes" => "Scopes",
        "mixer" => "Mixer",
        "captions" => "Captions",
        "workspaces" => "Workspaces",
        _ => id,
    }
}
/// The viewer is the one panel always open; it keeps its stack to itself.
pub const VIEWER: &str = "viewer";
/// A panel the person can close, park or hide.
pub fn closable(id: &str) -> bool {
    id != VIEWER
}
/// A panel that can share a tab stack with others.
pub fn stacks(id: &str) -> bool {
    id != VIEWER
}
/// Every built-in panel, in Window ▸ Panels order.
pub const PANELS: [&str; 18] = [
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
    "color",
    "easing",
    "scopes",
    "mixer",
    "captions",
    "workspaces",
    // Docked when pinned from its popover (agent-panel.md §1).
    "agent",
];
/// Window edges that can receive a docked panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Bottom,
    Top,
}
impl Edge {
    pub fn dock_id(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Bottom => "bottom",
            Edge::Top => "top",
        }
    }
}
/// A dock that runs across the timeline's row (the bottom one, or one
/// dropped at the top edge) rather than standing as a column.
fn is_band(id: &str) -> bool {
    id == "bottom" || id == "top"
}
/// A top dock's height when a panel first lands at the top edge.
const DEFAULT_TOP: f32 = 220.;
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
    /// A new column beside `dock` (a column, or before the right dock).
    Beside { dock: usize, after: bool },
    /// Dock at a window edge; creates or reopens that dock.
    Edge(Edge),
}
/// A drop target plus the rectangle the shell highlights while dragging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropPreview {
    pub target: DropTarget,
    pub rect: Rect,
    /// What the drop is named after, in the layout it was asked of: the
    /// original app's “Tab with Timeline”, “Above Viewer”, “Place in Left
    /// dock”.
    pub near: Near,
}
/// The panel (by dock and group) or dock a drop is named after.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Near {
    #[default]
    Nothing,
    Tab(usize, usize),
    Above(usize, usize),
    Below(usize, usize),
    Dock(usize),
    Edge(Edge),
}
/// What a drag over a point would do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropOutcome {
    /// Move there; the preview is where the panel would land.
    Drop(DropPreview),
    /// Leave the panel where it is (the original app's “already here”).
    Here,
    /// No drop zone.
    Nowhere,
}
/// A floating panel docks only this near a side column's edge ...
pub const FLOATING_EDGE: f32 = 32.;
/// ... or this near the seam between two of its panels (the original app's).
pub const FLOATING_SEAM: f32 = 24.;
/// Share of a panel body's height at its top and its bottom that docks
/// above or below it (the original app's 30%); the middle joins its tabs.
pub const BODY_EDGE: f32 = 0.3;

#[cfg(test)]
mod contributed_tests {
    use super::*;
    #[test]
    fn outside_leaves_every_hole_unpainted_and_covers_the_rest() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let area = r(0., 0., 100., 50.);
        let holes = [r(10., 10., 20., 20.), r(60., 0., 30., 50.)];
        let rects = outside(area, &holes);
        let covered: f32 = rects.iter().map(|r| r.w * r.h).sum();
        assert_eq!(covered, 100. * 50. - 20. * 20. - 30. * 50.);
        for (x, y) in [(15., 15.), (75., 25.)] {
            assert!(!rects.iter().any(|r| r.contains(x, y)));
        }
        for (x, y) in [(5., 5.), (40., 40.), (95., 49.)] {
            assert!(rects.iter().any(|r| r.contains(x, y)));
        }
        assert_eq!(outside(area, &[]), vec![area]);
    }
    /// PM-427: a panel whose extension is still starting is named by the
    /// title it registered last time, never by its id.
    #[test]
    fn a_panel_not_registered_yet_shows_its_remembered_title() {
        let panels = Panels::new(vec![]).with_remembered(vec![("pexels".into(), "Pexels".into())]);
        assert_eq!(panels.title("pexels"), "Pexels");
        assert!(!panels.contains("pexels"), "still not registered");
        let now = Panels::new(vec![("pexels".into(), "Pexels Browser".into())])
            .with_remembered(vec![("pexels".into(), "Pexels".into())]);
        assert_eq!(
            now.title("pexels"),
            "Pexels Browser",
            "the registered title wins"
        );
    }
    #[test]
    fn contributed_panels_dock_title_and_survive_like_builtins() {
        let panels = Panels::new(vec![("synthetic.panel".into(), "Synthetic".into())]);
        assert_eq!(panels.title("synthetic.panel"), "Synthetic");
        assert!(panels.ids().contains(&"synthetic.panel"));
        let mut layout = Layout::preset("edit");
        layout.show_panel("synthetic.panel", &panels).unwrap();
        assert!(layout.open_panels().contains(&"synthetic.panel"));
        layout.validate().unwrap();
        layout.close_panel("synthetic.panel").unwrap();
        assert!(!layout.open_panels().contains(&"synthetic.panel"));
        // Another window's contributions are not this window's panels.
        let other = Panels::default();
        assert!(layout.show_panel("synthetic.panel", &other).is_err());
        assert_eq!(other.title("synthetic.panel"), "synthetic.panel");
    }
}

/// Panes for 1, 2 (side by side) or 4 (two by two) views of `r`, `gap`
/// apart, in reading order.
pub fn view_tiles(r: Rect, count: u8, gap: f32) -> Vec<Rect> {
    let (cols, rows) = match count {
        2 => (2, 1),
        4 => (2, 2),
        _ => return vec![r],
    };
    let w = ((r.w - gap * (cols - 1) as f32) / cols as f32).max(1.);
    let h = ((r.h - gap * (rows - 1) as f32) / rows as f32).max(1.);
    (0..rows)
        .flat_map(|row| {
            (0..cols).map(move |col| Rect {
                x: r.x + col as f32 * (w + gap),
                y: r.y + row as f32 * (h + gap),
                w,
                h,
            })
        })
        .collect()
}
#[cfg(test)]
mod view_tile_tests {
    use super::*;
    #[test]
    fn four_views_tile_two_by_two_with_gaps() {
        let r = Rect {
            x: 10.,
            y: 20.,
            w: 402.,
            h: 202.,
        };
        let t = view_tiles(r, 4, 2.);
        assert_eq!(t.len(), 4);
        assert_eq!((t[0].w, t[0].h), (200., 100.));
        assert_eq!((t[3].x, t[3].y), (212., 122.));
        assert_eq!(view_tiles(r, 1, 2.), vec![r]);
    }
}

// Groups have no stable ID in the flex+size format. Keep the first-tab key
// synchronized when membership changes, deleting it when the group disappears.
fn rekey_basis(bases: &mut BTreeMap<String, f32>, first: &str, group: &Group) {
    if let Some(basis) = bases.remove(first)
        && let Some(next) = group.tabs.first()
    {
        bases.insert(next.clone(), basis);
    }
}
