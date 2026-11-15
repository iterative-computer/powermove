//! Panels move, dock and resize as in the original (Electron) app, which the
//! owner asked for exactly (2026-11-14): its drop zones, edge strips, gap
//! inserts, “already here”, fold-to-header, splitter limits and resets.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::layout::*;

fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
const BOUNDS: Rect = Rect {
    x: 6.,
    y: 58.,
    w: 1428.,
    h: 810.,
};
fn placed(l: &Layout, g: &Geometry, id: &str) -> (usize, usize, Rect) {
    let (d, gi, _) = l.find(id).unwrap();
    let r = g
        .panels
        .iter()
        .find(|p| (p.dock, p.group) == (d, gi))
        .unwrap()
        .rect;
    (d, gi, r)
}
fn body_at(r: Rect, t: &Tokens, fraction: f32) -> f32 {
    let top = r.y + t.metrics.header;
    top + (r.y + r.h - top) * fraction
}

#[test]
fn a_body_docks_above_in_its_top_30_below_in_its_bottom_30_and_tabs_between() {
    let (l, t) = (Layout::preset("design"), tokens());
    let g = l.geometry(BOUNDS, &t);
    let (d, gi, r) = placed(&l, &g, "inspector");
    let at = |f: f32| {
        l.drop_target(&g, BOUNDS, r.x + r.w / 2., body_at(r, &t, f), &t)
            .unwrap()
            .target
    };
    assert_eq!(at(0.25), DropTarget::Split { dock: d, group: gi });
    assert_eq!(at(0.75), DropTarget::Split { dock: d, group: gi + 1 });
    assert_eq!(
        at(0.5),
        DropTarget::Tab {
            dock: d,
            group: gi,
            index: 1
        }
    );
}

#[test]
fn no_side_by_side_column_from_a_panel_edge() {
    let (l, t) = (Layout::preset("design"), tokens());
    let g = l.geometry(BOUNDS, &t);
    for id in ["viewer", "inspector", "timeline"] {
        let (.., r) = placed(&l, &g, id);
        for x in [r.x + r.w * 0.05, r.x + r.w * 0.95] {
            let p = l.drop_target(&g, BOUNDS, x, body_at(r, &t, 0.5), &t).unwrap();
            assert!(
                !matches!(p.target, DropTarget::Beside { .. }),
                "{id}: {:?}",
                p.target
            );
        }
    }
}

#[test]
fn the_viewer_splits_at_its_middle() {
    let (l, t) = (Layout::preset("design"), tokens());
    let g = l.geometry(BOUNDS, &t);
    let (d, gi, r) = placed(&l, &g, "viewer");
    let at = |f: f32| {
        l.drop_target(&g, BOUNDS, r.x + r.w / 2., body_at(r, &t, f), &t)
            .unwrap()
            .target
    };
    assert_eq!(at(0.45), DropTarget::Split { dock: d, group: gi });
    assert_eq!(at(0.55), DropTarget::Split { dock: d, group: gi + 1 });
}

#[test]
fn a_header_tab_lands_before_the_tab_whose_middle_is_right_of_the_pointer() {
    let (l, t) = (Layout::preset("design"), tokens());
    let mut g = l.geometry(BOUNDS, &t);
    let (d, gi, r) = placed(&l, &g, "assets");
    let n = l.docks[d].groups[gi].tabs.len();
    assert_eq!(n, 2, "Project and Effect Controls share the left stack");
    // Tabs of different widths, as drawn.
    let x0 = r.x;
    g.tabs.insert((d, gi), vec![(0, x0, x0 + 60.), (1, x0 + 62., x0 + 150.)]);
    let y = r.y + t.metrics.header / 2.;
    let index = |x: f32| match l.drop_target(&g, BOUNDS, x, y, &t).unwrap().target {
        DropTarget::Tab { index, .. } => index,
        other => panic!("{other:?}"),
    };
    assert_eq!(index(x0 + 20.), 0);
    assert_eq!(index(x0 + 100.), 1, "left of the second tab's middle (106)");
    assert_eq!(index(x0 + 120.), 2, "past every middle: last");
}

#[test]
fn edge_strips_only_where_a_dock_is_missing_and_no_top_band() {
    let t = tokens();
    let l = Layout::preset("design");
    let g = l.geometry(BOUNDS, &t);
    let mid = BOUNDS.y + BOUNDS.h / 2.;
    // A left dock stands: just outside it, the panel goes into it.
    let p = l.drop_target(&g, BOUNDS, BOUNDS.x - 10., mid, &t).unwrap();
    let left = l.docks.iter().position(|d| d.id == "left").unwrap();
    assert!(matches!(p.target, DropTarget::Split { dock, .. } if dock == left), "{p:?}");
    // Without a left dock the strip makes one.
    let mut bare = l.clone();
    bare.close_panel("assets").unwrap();
    bare.close_panel("layer-effects").unwrap();
    let g2 = bare.geometry(BOUNDS, &t);
    let p = bare.drop_target(&g2, BOUNDS, BOUNDS.x - 10., mid, &t).unwrap();
    assert_eq!(p.target, DropTarget::Edge(Edge::Left));
    // Never a top band from the pointer (the menu still offers one).
    let (.., viewer) = placed(&l, &g, "viewer");
    for y in [BOUNDS.y - 10., BOUNDS.y + 1.] {
        let p = l.drop_target(&g, BOUNDS, viewer.x + viewer.w / 2., y, &t);
        assert!(
            p.is_none_or(|p| p.target != DropTarget::Edge(Edge::Top)),
            "{p:?}"
        );
    }
    // Far outside every dock: no drop zone.
    assert!(l.drop_target(&g, BOUNDS, BOUNDS.x - 200., mid, &t).is_none());
}

#[test]
fn a_drop_on_itself_is_already_here_and_off_every_zone_is_nowhere() {
    let (l, t) = (Layout::preset("design"), tokens());
    let g = l.geometry(BOUNDS, &t);
    let (.., r) = placed(&l, &g, "inspector");
    let at = (r.x + r.w / 2., body_at(r, &t, 0.2));
    assert_eq!(
        l.drop_outcome(&g, BOUNDS, at, &t, "inspector", false),
        DropOutcome::Here
    );
    assert_eq!(
        l.drop_outcome(&g, BOUNDS, (BOUNDS.x - 300., 300.), &t, "inspector", false),
        DropOutcome::Nowhere
    );
    // A real move previews where it lands and names what it lands by.
    let (.., v) = placed(&l, &g, "viewer");
    match l.drop_outcome(&g, BOUNDS, (v.x + v.w / 2., body_at(v, &t, 0.9)), &t, "inspector", false) {
        DropOutcome::Drop(p) => assert!(matches!(p.near, Near::Below(..)), "{p:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_header_double_click_folds_a_panel_to_its_header_and_it_is_remembered() {
    let (mut l, t) = (Layout::preset("design"), tokens());
    let before = l.geometry(BOUNDS, &t);
    let (_, _, fixed_effects) = placed(&l, &before, "fxbrowser");
    assert!(l.toggle_collapsed("inspector"));
    let g = l.geometry(BOUNDS, &t);
    let (.., r) = placed(&l, &g, "inspector");
    assert!((r.h - t.metrics.header).abs() < 0.5, "{r:?}");
    // Fixed-height panels retain their height in a scrolling side dock.
    let (.., fx) = placed(&l, &g, "fxbrowser");
    assert_eq!(fx.h, fixed_effects.h);
    // Saved with the workspace, as the original app's `collapsed`.
    let record = pm_shell::workspace::write(&l, &serde_json::json!({"id": "design"}));
    assert_eq!(pm_shell::workspace::layout(&record).docks, l.docks);
    // The viewer never folds.
    assert!(!l.toggle_collapsed("viewer"));
    // A splitter drag beside a folded panel opens it first.
    let divider = g
        .dividers
        .iter()
        .find(|d| d.group.is_some() && l.docks[d.dock].id == "right")
        .unwrap()
        .clone();
    assert!(l.expand_beside(&divider));
    assert!(!l.docks[divider.dock].groups.iter().any(|g| g.collapsed));
}

#[test]
fn side_columns_resize_between_200_and_760_and_reset_on_double_click() {
    let (mut l, t) = (Layout::preset("design"), tokens());
    let preset = Layout::preset("design");
    let g = l.geometry(BOUNDS, &t);
    let right = g
        .dividers
        .iter()
        .find(|d| d.group.is_none() && l.docks[d.dock].id == "right")
        .unwrap()
        .clone();
    let wide = BOUNDS_WIDE;
    l.resize(&right, -2000., wide, &t);
    assert!(l.docks[right.dock].size.unwrap() <= t.metrics.column_most + 0.5);
    let mut narrow = preset.clone();
    narrow.resize(&right, 2000., BOUNDS, &t);
    assert!(narrow.docks[right.dock].size.unwrap() >= t.metrics.column_least - 0.5);
    // Double-click (and Return): the size its workspace shipped with.
    assert!(l.reset_dock(&right));
    assert_eq!(l.docks[right.dock].size, preset.docks[right.dock].size);
}
const BOUNDS_WIDE: Rect = Rect {
    x: 6.,
    y: 58.,
    w: 2400.,
    h: 1200.,
};

/// The Agent popover moved by its grip docks only near a side column's edge
/// or between its panels; the middle of the work area keeps it floating.
#[test]
fn a_floating_panel_docks_only_at_side_column_edges_and_seams() {
    let (l, t) = (Layout::preset("design"), tokens());
    let g = l.geometry(BOUNDS, &t);
    let (.., viewer) = placed(&l, &g, "viewer");
    assert!(l
        .floating_drop_target(&g, BOUNDS, viewer.x + viewer.w / 2., viewer.y + viewer.h / 2., &t)
        .is_none());
    let (.., fx) = placed(&l, &g, "fxbrowser");
    // On the seam above Effects (between Properties and Effects).
    let p = l
        .floating_drop_target(&g, BOUNDS, fx.x + fx.w / 2., fx.y - t.metrics.dock_gap / 2., &t)
        .unwrap();
    assert!(matches!(p.target, DropTarget::Split { .. }), "{p:?}");
    // Deep inside the right column's panel, away from its edges: floating.
    assert!(fx.h / 2. > FLOATING_SEAM && fx.w / 2. > FLOATING_EDGE);
    assert!(l
        .floating_drop_target(&g, BOUNDS, fx.x + fx.w / 2., fx.y + fx.h / 2., &t)
        .is_none());
}
