//! Moving and resizing panels as a person does (owner feedback W1, W2): every
//! drop lands exactly where its preview showed, nothing overlaps or leaves
//! the docking area, a resize never jumps or rescales other docks, and
//! dragging a splitter back gives the layout it began with.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::layout::*;

fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
/// Docking areas of a small, the default and a large window.
const SIZES: [Rect; 3] = [
    Rect {
        x: 6.,
        y: 58.,
        w: 1094.,
        h: 610.,
    },
    Rect {
        x: 6.,
        y: 58.,
        w: 1428.,
        h: 810.,
    },
    Rect {
        x: 6.,
        y: 58.,
        w: 1908.,
        h: 1010.,
    },
];

fn rect_of(l: &Layout, bounds: Rect, id: &str) -> Rect {
    let (d, g, _) = l.find(id).unwrap();
    l.geometry(bounds, &tokens())
        .panels
        .into_iter()
        .find(|p| (p.dock, p.group) == (d, g))
        .unwrap()
        .rect
}

/// Every panel inside `bounds`, with room, and no two overlapping.
fn sound(l: &Layout, bounds: Rect, what: &str) {
    l.validate().unwrap();
    let t = tokens();
    let panels = l.geometry(bounds, &t).panels;
    for p in &panels {
        let r = p.rect;
        assert!(r.w > 1. && r.h > 1., "{what}: empty panel {r:?}");
        // A window of the default size or larger has room for every
        // panel's least height (the timeline its ruler and three rows).
        if bounds.h >= 800. {
            let least = least_height(&l.docks[p.dock].groups[p.group].tabs, &t);
            assert!(r.h >= least - 0.5, "{what}: {r:?} under its least {least}");
        }
        assert!(
            r.x >= bounds.x - 0.5
                && r.y >= bounds.y - 0.5
                && r.x + r.w <= bounds.x + bounds.w + 0.5
                && r.y + r.h <= bounds.y + bounds.h + 0.5,
            "{what}: {r:?} leaves {bounds:?}"
        );
    }
    for (i, a) in panels.iter().enumerate() {
        for b in &panels[i + 1..] {
            let (a, b) = (a.rect, b.rect);
            let overlap_w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
            let overlap_h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
            assert!(
                overlap_w <= 0.5 || overlap_h <= 0.5,
                "{what}: {a:?} overlaps {b:?}"
            );
        }
    }
}

fn close(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 0.5
        && (a.y - b.y).abs() < 0.5
        && (a.w - b.w).abs() < 0.5
        && (a.h - b.h).abs() < 0.5
}

/// Points a person drops on in a panel: its header, its middle, and near
/// each edge of its body.
fn spots(r: Rect, t: &Tokens) -> Vec<(&'static str, f32, f32)> {
    let body = r.y + t.metrics.header;
    let bh = r.y + r.h - body;
    vec![
        ("header", r.x + r.w * 0.3, r.y + t.metrics.header / 2.),
        ("middle", r.x + r.w / 2., body + bh / 2.),
        ("left", r.x + r.w * 0.1, body + bh / 2.),
        ("right", r.x + r.w * 0.9, body + bh / 2.),
        ("top", r.x + r.w / 2., body + bh * 0.1),
        ("bottom", r.x + r.w / 2., body + bh * 0.9),
    ]
}

#[test]
fn every_drop_lands_where_its_preview_showed() {
    let t = tokens();
    for bounds in SIZES {
        let start = Layout::preset("design");
        let geometry = start.geometry(bounds, &t);
        let ids: Vec<String> = start.open_panels().iter().map(|s| s.to_string()).collect();
        for source in &ids {
            for stack in [false, true] {
                for target in &geometry.panels {
                    for (zone, x, y) in spots(target.rect, &t) {
                        let what = format!(
                            "{source} (stack {stack}) to {zone} of {} at {}x{}",
                            start.docks[target.dock].groups[target.group].active(),
                            bounds.w,
                            bounds.h
                        );
                        let Some(preview) =
                            start.drop_target_for(&geometry, bounds, (x, y), &t, source, stack)
                        else {
                            continue;
                        };
                        let mut next = start.clone();
                        if stack {
                            next.move_stack_to(source, preview.target).unwrap();
                        } else {
                            next.move_panel_to(source, preview.target).unwrap();
                        }
                        assert_ne!(next, start, "{what}: a preview for no change");
                        sound(&next, bounds, &what);
                        let landed = rect_of(&next, bounds, source);
                        assert!(
                            close(landed, preview.rect),
                            "{what}: previewed {:?}, landed {landed:?}",
                            preview.rect
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn zones_follow_the_nearest_edge_of_a_panel() {
    let t = tokens();
    let bounds = SIZES[1];
    let l = Layout::preset("design");
    let g = l.geometry(bounds, &t);
    let viewer = rect_of(&l, bounds, "viewer");
    let at = |zone: &str| {
        let (_, x, y) = spots(viewer, &t)
            .into_iter()
            .find(|s| s.0 == zone)
            .unwrap();
        l.drop_target_for(&g, bounds, (x, y), &t, "inspector", false)
            .unwrap()
            .target
    };
    let (vd, vg, _) = l.find("viewer").unwrap();
    assert_eq!(
        at("left"),
        DropTarget::Beside {
            dock: vd,
            after: false
        }
    );
    assert_eq!(
        at("right"),
        DropTarget::Beside {
            dock: vd,
            after: true
        }
    );
    assert_eq!(
        at("top"),
        DropTarget::Split {
            dock: vd,
            group: vg
        }
    );
    assert_eq!(
        at("bottom"),
        DropTarget::Split {
            dock: vd,
            group: vg + 1
        }
    );
    // The viewer joins no stack: its middle splits it.
    assert!(matches!(at("middle"), DropTarget::Split { .. }));
    // A stackable panel's middle and header join its stack.
    let effects = rect_of(&l, bounds, "fxbrowser");
    let (ed, eg, _) = l.find("fxbrowser").unwrap();
    for (zone, x, y) in spots(effects, &t) {
        if zone == "middle" || zone == "header" {
            assert!(
                matches!(
                    l.drop_target_for(&g, bounds, (x, y), &t, "assets", false)
                        .unwrap()
                        .target,
                    DropTarget::Tab { dock, group, .. } if (dock, group) == (ed, eg)
                ),
                "{zone}"
            );
        }
    }
}

#[test]
fn a_panel_moved_down_its_own_dock_stays_in_it() {
    // The owner's case: Properties dragged low over Effects, near the
    // window's bottom, went under the timeline at full width.
    let t = tokens();
    for bounds in SIZES {
        let l = Layout::preset("design");
        let g = l.geometry(bounds, &t);
        let effects = rect_of(&l, bounds, "fxbrowser");
        let (ed, _, _) = l.find("fxbrowser").unwrap();
        for y in [
            effects.y + effects.h - 2.,
            bounds.y + bounds.h - 1.,
            bounds.y + bounds.h + 10.,
        ] {
            let p = l
                .drop_target_for(
                    &g,
                    bounds,
                    (effects.x + effects.w / 2., y),
                    &t,
                    "inspector",
                    true,
                )
                .unwrap();
            assert_eq!(p.target, DropTarget::Split { dock: ed, group: 2 }, "{y}");
            let mut next = l.clone();
            next.move_stack_to("inspector", p.target).unwrap();
            let (d, _, _) = next.find("inspector").unwrap();
            assert_eq!(next.docks[d].id, "right");
            assert!(rect_of(&next, bounds, "inspector").y > rect_of(&next, bounds, "fxbrowser").y);
        }
    }
}

#[test]
fn window_edges_dock_and_overshooting_still_counts() {
    let t = tokens();
    let bounds = SIZES[1];
    let l = Layout::preset("design");
    let g = l.geometry(bounds, &t);
    let timeline = rect_of(&l, bounds, "timeline");
    let edge = |x: f32, y: f32| {
        l.drop_target_for(&g, bounds, (x, y), &t, "fxbrowser", false)
            .map(|p| p.target)
    };
    let mid = bounds.y + bounds.h / 3.;
    assert_eq!(edge(bounds.x + 2., mid), Some(DropTarget::Edge(Edge::Left)));
    assert_eq!(edge(bounds.x - 6., mid), Some(DropTarget::Edge(Edge::Left)));
    assert_eq!(
        edge(bounds.x + bounds.w - 2., mid),
        Some(DropTarget::Edge(Edge::Right))
    );
    // Under the timeline, its bottom edge, and the status bar below it.
    let under = timeline.x + timeline.w / 2.;
    assert_eq!(
        edge(under, bounds.y + bounds.h - 2.),
        Some(DropTarget::Edge(Edge::Bottom))
    );
    assert_eq!(
        edge(under, bounds.y + bounds.h + 12.),
        Some(DropTarget::Edge(Edge::Bottom))
    );
    // Far away is no target.
    assert_eq!(edge(under, bounds.y + bounds.h + 200.), None);
}

#[test]
fn a_panel_beside_another_makes_a_column_that_goes_when_emptied() {
    let bounds = SIZES[1];
    let mut l = Layout::preset("design");
    let (vd, _, _) = l.find("viewer").unwrap();
    l.move_panel_to(
        "inspector",
        DropTarget::Beside {
            dock: vd,
            after: true,
        },
    )
    .unwrap();
    sound(&l, bounds, "beside");
    let viewer = rect_of(&l, bounds, "viewer");
    let inspector = rect_of(&l, bounds, "inspector");
    assert!(inspector.x > viewer.x + viewer.w);
    assert!((inspector.y - viewer.y).abs() < 0.5);
    // A workspace record keeps the column.
    let record = pm_shell::workspace::write(&l, &pm_shell::workspace::preset("design").unwrap());
    assert_eq!(pm_shell::workspace::layout(&record).docks, l.docks);
    // Moved back, the column goes.
    let docks = l.docks.len();
    l.move_panel_to(
        "inspector",
        DropTarget::Tab {
            dock: l.find("fxbrowser").unwrap().0,
            group: l.find("fxbrowser").unwrap().1,
            index: 0,
        },
    )
    .unwrap();
    assert_eq!(l.docks.len(), docks - 1);
    sound(&l, bounds, "column gone");
}

#[test]
fn resizing_never_jumps_breaks_or_rescales_and_dragging_back_restores() {
    let t = tokens();
    for bounds in SIZES {
        let start = Layout::preset("design");
        let geometry = start.geometry(bounds, &t);
        for divider in &geometry.dividers {
            let what = format!(
                "divider {:?}/{:?} of {} at {}x{}",
                divider.group, divider.horizontal, start.docks[divider.dock].id, bounds.w, bounds.h
            );
            let before: Vec<Rect> = geometry.panels.iter().map(|p| p.rect).collect();
            // A one-point move moves only the two sides, by one point.
            let mut one = start.clone();
            one.resize(divider, 1., bounds, &t);
            let after: Vec<Rect> = one
                .geometry(bounds, &t)
                .panels
                .iter()
                .map(|p| p.rect)
                .collect();
            assert_eq!(before.len(), after.len());
            for (a, b) in before.iter().zip(&after) {
                let moved = (a.x - b.x).abs() + (a.y - b.y).abs() + (a.w - b.w).abs() + (a.h - b.h).abs();
                assert!(moved <= 2.01, "{what}: {a:?} jumped to {b:?}");
            }
            for delta in [-5000., -120., 120., 5000.] {
                let mut l = start.clone();
                l.resize(divider, delta, bounds, &t);
                sound(&l, bounds, &format!("{what} by {delta}"));
                for p in l.geometry(bounds, &t).panels {
                    assert!(
                        p.rect.h >= t.metrics.min_panel - 0.5 || bounds.h < 700.,
                        "{what} by {delta}: {:?}",
                        p.rect
                    );
                }
                // Dragging back (from the same press) is where it began.
                let mut back = start.clone();
                back.resize(divider, 0., bounds, &t);
                let restored: Vec<Rect> = back
                    .geometry(bounds, &t)
                    .panels
                    .iter()
                    .map(|p| p.rect)
                    .collect();
                for (a, b) in before.iter().zip(&restored) {
                    assert!(close(*a, *b), "{what}: {a:?} came back as {b:?}");
                }
            }
        }
    }
}

#[test]
fn a_short_window_shrinks_the_timeline_too() {
    let t = tokens();
    for h in [460., 520., 610.] {
        let bounds = Rect { h, ..SIZES[0] };
        let l = Layout::preset("design");
        sound(&l, bounds, "short");
        let viewer = rect_of(&l, bounds, "viewer");
        assert!(viewer.h >= h * 0.35 - 0.5, "{h}: viewer {viewer:?}");
        let timeline = rect_of(&l, bounds, "timeline");
        assert!(timeline.h >= least_height(&["timeline".to_owned()], &t) - 0.5 || h < 520.);
    }
}
