//! Pinned main's docking behavior through the production shell layout API.
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
fn panel(l: &Layout, id: &str) -> Rect {
    l.geometry(BOUNDS, &tokens())
        .panels
        .into_iter()
        .find(|p| l.docks[p.dock].groups[p.group].tabs.iter().any(|s| s == id))
        .unwrap()
        .rect
}
#[test]
fn body_bands_and_middle_group_follow_main() {
    let l = Layout::preset("design");
    let t = tokens();
    let g = l.geometry(BOUNDS, &t);
    let r = panel(&l, "inspector");
    let body = r.h - t.metrics.header;
    for (fraction, expected) in [
        (0.1, DropTarget::Split { dock: 3, group: 0 }),
        (
            0.5,
            DropTarget::Tab {
                dock: 3,
                group: 0,
                index: 1,
            },
        ),
        (0.9, DropTarget::Split { dock: 3, group: 1 }),
    ] {
        let p = l
            .drop_target_for(
                &g,
                BOUNDS,
                (r.x + r.w / 2., r.y + t.metrics.header + body * fraction),
                &t,
                "assets",
                false,
            )
            .unwrap();
        assert_eq!(p.target, expected);
        assert!(p.rect.x >= r.x && p.rect.y >= r.y && p.rect.y + p.rect.h <= r.y + r.h);
    }
}
#[test]
fn viewer_neither_accepts_nor_joins_tabs() {
    let mut l = Layout::preset("design");
    let t = tokens();
    let g = l.geometry(BOUNDS, &t);
    for (source, target) in [("assets", "viewer"), ("viewer", "inspector")] {
        let r = panel(&l, target);
        for y in [r.y + 2., r.y + r.h / 2., r.y + r.h * 0.9] {
            assert!(matches!(
                l.drop_target_for(&g, BOUNDS, (r.x + r.w / 2., y), &t, source, false)
                    .unwrap()
                    .target,
                DropTarget::Split { .. }
            ));
        }
    }
    let before = l.clone();
    assert!(l.place_relative("assets", "viewer", "group").is_err());
    assert!(l.place_relative("viewer", "assets", "group").is_err());
    assert_eq!(l, before);
}
#[test]
fn header_moves_stack_with_order_active_and_size() {
    let mut l = Layout::preset("design");
    l.docks[0].groups[0].active = 1;
    let before = l
        .open_panels()
        .into_iter()
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    l.move_stack_to("layer-effects", DropTarget::Split { dock: 3, group: 1 })
        .unwrap();
    assert!(l.docks[0].groups.is_empty());
    assert_eq!(l.docks[3].groups[1].tabs, ["assets", "layer-effects"]);
    assert_eq!(l.docks[3].groups[1].active(), "layer-effects");
    assert_eq!(
        l.open_panels()
            .into_iter()
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>(),
        before
    );
    l.validate().unwrap();
}
#[test]
fn stack_group_inserts_in_order_and_selects_its_active_member() {
    let mut l = Layout::preset("design");
    l.docks[0].groups[0].active = 1;
    l.move_stack_to(
        "assets",
        DropTarget::Tab {
            dock: 3,
            group: 0,
            index: 0,
        },
    )
    .unwrap();
    assert_eq!(
        l.docks[3].groups[0].tabs,
        ["assets", "layer-effects", "inspector"]
    );
    assert_eq!(l.docks[3].groups[0].active(), "layer-effects");
}
#[test]
fn single_tab_move_keeps_remaining_selection_by_identity() {
    let mut l = Layout::preset("design");
    l.place_relative("fxbrowser", "assets", "group").unwrap();
    l.place_relative("assets", "inspector", "above").unwrap();
    assert_eq!(l.docks[0].groups[0].active(), "fxbrowser");
    assert_eq!(l.docks[0].groups[0].tabs, ["layer-effects", "fxbrowser"]);
}
#[test]
fn source_stack_has_no_preview_and_self_move_does_nothing() {
    let mut l = Layout::preset("design");
    let t = tokens();
    let g = l.geometry(BOUNDS, &t);
    let r = panel(&l, "assets");
    assert!(
        l.drop_target_for(
            &g,
            BOUNDS,
            (r.x + r.w / 2., r.y + r.h / 2.),
            &t,
            "assets",
            true
        )
        .is_none()
    );
    let before = l.clone();
    for target in [
        DropTarget::Tab {
            dock: 0,
            group: 0,
            index: 2,
        },
        DropTarget::Split { dock: 0, group: 0 },
        DropTarget::Split { dock: 0, group: 1 },
    ] {
        l.move_stack_to("assets", target).unwrap();
        assert_eq!(l, before);
    }
}
#[test]
fn invalid_stack_targets_are_atomic() {
    let mut l = Layout::preset("design");
    let before = l.clone();
    for target in [
        DropTarget::Tab {
            dock: 999,
            group: 0,
            index: 0,
        },
        DropTarget::Split {
            dock: 0,
            group: 999,
        },
        DropTarget::Tab {
            dock: 1,
            group: 0,
            index: 0,
        },
    ] {
        assert!(l.move_stack_to("assets", target).is_err());
        assert_eq!(l, before);
    }
}
#[test]
fn same_dock_stack_move_keeps_original_destination_indices() {
    let mut l = Layout::preset("design");
    l.place_relative("assets", "inspector", "above").unwrap();
    l.move_stack_to("assets", DropTarget::Split { dock: 3, group: 3 })
        .unwrap();
    assert_eq!(
        l.docks[3]
            .groups
            .iter()
            .map(Group::active)
            .collect::<Vec<_>>(),
        ["inspector", "fxbrowser", "assets"]
    );
}
#[test]
fn maximize_tracks_group_across_tab_selection_without_persisting_geometry() {
    let mut l = Layout::preset("design");
    let original = l.clone();
    l.docks[0].groups[0].active = 1;
    let g = l.geometry_with_maximized(BOUNDS, &tokens(), Some("assets"));
    assert_eq!(g.panels.len(), 1);
    assert_eq!(g.panels[0].rect, BOUNDS);
    assert_eq!(
        l.docks[g.panels[0].dock].groups[g.panels[0].group].active(),
        "layer-effects"
    );
    let mut restored: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
    restored.docks[0].groups[0].active = 0;
    assert_eq!(restored, original);
}
#[test]
fn overflow_keeps_active_and_never_loses_a_tab() {
    let group = Group {
        tabs: (0..12).map(|i| format!("tab-{i}")).collect(),
        active: 10,
        size: None,
    };
    let t = tokens();
    let widths: Vec<f32> = (0..12).map(|i| 40. + 6. * i as f32).collect();
    for width in [24., 144., 300., 1200.] {
        let track = group.tab_track(width, &widths, &t);
        assert!(track.visible.contains(&10));
        assert!(!track.overflow.contains(&10));
        let mut all = track.visible.clone();
        all.extend(&track.overflow);
        all.sort_unstable();
        assert_eq!(all, (0..12).collect::<Vec<_>>());
        // Shown tabs plus the reserved More Panels square fit the header; only
        // an active tab wider than that room truncates, to exactly that room.
        let room = (width - t.metrics.hit).max(0.);
        let shown: f32 = track.visible.iter().map(|&i| widths[i]).sum();
        match track.active_limit {
            None => assert!(shown <= room),
            Some(limit) => {
                assert_eq!(track.visible, [10]);
                assert!(widths[10] > room);
                assert_eq!(limit, room);
            }
        }
    }
    // Everything fits: no limit and nothing folded.
    let wide = group.tab_track(10_000., &widths, &t);
    assert!(wide.overflow.is_empty() && wide.active_limit.is_none());
}
#[test]
fn main_menu_has_system_show_all_delete_and_one_new_window() {
    let menus = pm_shell::menu::menus();
    let all: Vec<_> = menus.iter().flat_map(|(_, items)| items).collect();
    assert_eq!(all.iter().filter(|i| i.command == "newWindow").count(), 1);
    for command in [
        "unhideAllApplications:",
        "delete",
        "maximizePanel",
        "restorePanel",
        "help",
    ] {
        assert!(all.iter().any(|i| i.command == command), "{command}");
    }
}
#[test]
fn bottom_dock_floor_uses_opus_metrics_and_stays_bounded() {
    let mut l = Layout::preset("design");
    let t = tokens();
    let divider = l
        .geometry(BOUNDS, &t)
        .dividers
        .into_iter()
        .find(|d| d.group.is_none() && l.docks[d.dock].id == "bottom")
        .unwrap();
    l.resize(&divider, 10000., &t);
    let floor = t.metrics.header + 2. * t.metrics.ruler + 3. * t.metrics.track_row;
    assert_eq!(l.docks[divider.dock].size, Some(floor));
    assert_eq!(panel(&l, "timeline").h, floor);
    let small = Rect { h: 80., ..BOUNDS };
    for p in l.geometry(small, &t).panels {
        assert!(p.rect.y + p.rect.h <= small.y + small.h);
    }
}
#[test]
fn historical_viewer_stack_is_preserved_but_cannot_join_another_stack() {
    let mut l = Layout::preset("design");
    l.docks[0].groups[0].tabs.remove(0);
    l.docks[1].groups[0].tabs.push("assets".into());
    l.validate().unwrap();
    let before = l.clone();
    assert!(
        l.move_stack_to(
            "assets",
            DropTarget::Tab {
                dock: 3,
                group: 0,
                index: 0
            }
        )
        .is_err()
    );
    assert_eq!(l, before);
}
