//! Shell 2 models: drag-to-dock targets, tab order, panel close/reopen, saved
//! layouts and the multi-window session. Screen-free.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{layout::*, session::*};

fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
const BOUNDS: Rect = Rect {
    x: 6.,
    y: 58.,
    w: 1428.,
    h: 810.,
};
fn geometry(l: &Layout) -> Geometry {
    l.geometry(BOUNDS, &tokens())
}
fn rect_of(l: &Layout, id: &str) -> Rect {
    let g = geometry(l);
    g.panels
        .iter()
        .find(|p| l.docks[p.dock].groups[p.group].tabs.iter().any(|t| t == id))
        .unwrap()
        .rect
}

#[test]
fn drop_targets_cover_header_group_viewer_splits_and_edges() {
    let l = Layout::preset("design");
    let g = geometry(&l);
    let t = tokens();
    let inspector = rect_of(&l, "inspector");
    // Header → join the tab group; right half of a 1-tab header appends.
    let p = l
        .drop_target(
            &g,
            BOUNDS,
            inspector.x + inspector.w * 0.9,
            inspector.y + 4.,
            &t,
        )
        .unwrap();
    assert_eq!(
        p.target,
        DropTarget::Tab {
            dock: 3,
            group: 0,
            index: 1
        }
    );
    // The group body lights up, per main-delta-ui §2.
    assert_eq!(
        p.rect,
        Rect {
            y: inspector.y + t.metrics.header,
            h: inspector.h - t.metrics.header,
            ..inspector
        }
    );
    // Body upper/lower half → split before/after with a half-height preview.
    let viewer = rect_of(&l, "viewer");
    let mid = viewer.x + viewer.w / 2.;
    let upper = l
        .drop_target(&g, BOUNDS, mid, viewer.y + viewer.h * 0.3, &t)
        .unwrap();
    assert_eq!(upper.target, DropTarget::Split { dock: 1, group: 0 });
    assert!(upper.rect.h < viewer.h / 2. && upper.rect.y == viewer.y);
    let lower = l
        .drop_target(&g, BOUNDS, mid, viewer.y + viewer.h * 0.7, &t)
        .unwrap();
    assert_eq!(lower.target, DropTarget::Split { dock: 1, group: 1 });
    assert_eq!(lower.rect.y + lower.rect.h, viewer.y + viewer.h);
    // Edge zones beat the panel underneath them.
    let left = l
        .drop_target(&g, BOUNDS, BOUNDS.x + 2., BOUNDS.y + 200., &t)
        .unwrap();
    assert_eq!(left.target, DropTarget::Edge(Edge::Left));
    let bottom = l
        .drop_target(&g, BOUNDS, mid, BOUNDS.y + BOUNDS.h - 2., &t)
        .unwrap();
    assert_eq!(bottom.target, DropTarget::Edge(Edge::Bottom));
    assert!(l.drop_target(&g, BOUNDS, -5., 300., &t).is_none());
}

#[test]
fn drag_to_new_edge_dock_creates_it_and_reopens_collapsed_docks() {
    let mut l = Layout::preset("review"); // center + right only
    assert!(!l.docks.iter().any(|d| d.id == "left"));
    l.move_panel_to("notes", DropTarget::Edge(Edge::Left))
        .unwrap();
    let left = l.docks.iter().find(|d| d.id == "left").unwrap();
    assert_eq!(left.groups[0].tabs, ["notes"]);
    assert_eq!(left.size, Some(DEFAULT_LEFT_FOR_TEST));
    // The emptied right dock disappears from geometry but the layout stays valid.
    l.validate().unwrap();
    let g = geometry(&l);
    assert!(g.panels.iter().all(|p| l.docks[p.dock].id != "right"));
    // Collapsed dock reopens with the dropped panel on top.
    let mut d = Layout::preset("design");
    d.docks
        .iter_mut()
        .find(|d| d.id == "right")
        .unwrap()
        .collapsed = true;
    d.move_panel_to("assets", DropTarget::Edge(Edge::Right))
        .unwrap();
    let right = d.docks.iter().find(|d| d.id == "right").unwrap();
    assert!(!right.collapsed);
    assert_eq!(right.groups[0].tabs, ["assets"]);
    assert_eq!(right.groups.len(), 3);
}
const DEFAULT_LEFT_FOR_TEST: f32 = pm_design::workspace::DEFAULT_LEFT;

#[test]
fn tab_drop_inserts_at_index_and_same_group_reorders() {
    let mut l = Layout::preset("design");
    l.move_panel_to(
        "fxbrowser",
        DropTarget::Tab {
            dock: 0,
            group: 0,
            index: 1,
        },
    )
    .unwrap();
    assert_eq!(
        l.docks[0].groups[0].tabs,
        ["assets", "fxbrowser", "layer-effects"]
    );
    assert_eq!(l.docks[0].groups[0].active(), "fxbrowser");
    // The right dock lost its second group.
    assert_eq!(l.docks[3].groups.len(), 1);
    // Same group: insertion slot semantics on both sides of the source.
    l.move_panel_to(
        "assets",
        DropTarget::Tab {
            dock: 0,
            group: 0,
            index: 3,
        },
    )
    .unwrap();
    assert_eq!(
        l.docks[0].groups[0].tabs,
        ["fxbrowser", "layer-effects", "assets"]
    );
    l.reorder_tab(0, 0, 2, 0).unwrap();
    assert_eq!(
        l.docks[0].groups[0].tabs,
        ["assets", "fxbrowser", "layer-effects"]
    );
    assert_eq!(l.docks[0].groups[0].active, 0);
}

#[test]
fn split_next_to_itself_is_a_no_op_and_bad_targets_are_atomic() {
    let mut l = Layout::preset("design");
    let before = l.clone();
    l.move_panel_to("viewer", DropTarget::Split { dock: 1, group: 0 })
        .unwrap();
    l.move_panel_to("viewer", DropTarget::Split { dock: 1, group: 1 })
        .unwrap();
    assert_eq!(l, before);
    assert!(
        l.move_panel_to(
            "assets",
            DropTarget::Tab {
                dock: 9,
                group: 0,
                index: 0
            }
        )
        .is_err()
    );
    assert!(
        l.move_panel_to("nope", DropTarget::Edge(Edge::Left))
            .is_err()
    );
    assert_eq!(l, before);
    // Splitting a tab out of a group creates a sibling group.
    l.move_panel_to("layer-effects", DropTarget::Split { dock: 0, group: 1 })
        .unwrap();
    assert_eq!(l.docks[0].groups.len(), 2);
    assert_eq!(l.docks[0].groups[1].tabs, ["layer-effects"]);
    l.validate().unwrap();
}

#[test]
fn close_and_show_panels_round_trip_to_their_home_dock() {
    let panels = Panels::default();
    let mut l = Layout::preset("design");
    assert!(l.close_panel("viewer").is_err());
    l.close_panel("timeline").unwrap();
    assert!(!l.open_panels().contains(&"timeline"));
    l.validate().unwrap();
    l.show_panel("timeline", &panels).unwrap();
    let bottom = l.docks.iter().find(|d| d.id == "bottom").unwrap();
    assert_eq!(bottom.groups[0].tabs, ["timeline"]);
    // A panel that is not in this workspace's preset goes to the right dock.
    l.show_panel("notes", &panels).unwrap();
    let right = l.docks.iter().find(|d| d.id == "right").unwrap();
    assert_eq!(right.groups[0].active(), "notes");
    // Showing an open panel only activates it.
    let before = l.open_panels().len();
    l.show_panel("assets", &panels).unwrap();
    assert_eq!(l.open_panels().len(), before);
    assert!(l.show_panel("unknown", &panels).is_err());
    for id in PANELS {
        assert_ne!(panels.title(id), id, "{id} needs a title");
    }
}

#[test]
fn workspace_names_are_trimmed_and_bounded() {
    assert!(layout_name("  ").is_err());
    assert!(layout_name(&"x".repeat(LAYOUT_NAME_MAX + 1)).is_err());
    assert!(layout_name("Bad\nName").is_err());
    assert_eq!(layout_name("  Colour pass ").unwrap(), "Colour pass");
}

fn win(id: u64, tab: Option<u64>) -> WindowState {
    WindowState {
        id,
        frame: [100. * id as f64, 100., 1200., 800.],
        project: format!("p{id}"),
        workspace: "design".into(),
        tab_group: tab,
        fullscreen: false,
    }
}

#[test]
fn session_restores_windows_tabs_and_respects_close_windows_on_quit() {
    let mut s = Session::default();
    s.upsert(win(1, None));
    s.upsert(win(2, Some(1)));
    s.upsert(win(3, None));
    s.upsert(win(4, Some(1)));
    s.key = Some(3);
    assert_eq!(s.next_id(), 5);
    let all = s.restore_plan(true);
    assert_eq!(all.len(), 4);
    // Leaders before their tabs.
    let pos = |id| all.iter().position(|w| w.id == id).unwrap();
    assert!(pos(1) < pos(2) && pos(1) < pos(4));
    // "Close windows when quitting": only the key window, untabbed.
    let one = s.restore_plan(false);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, 3);
    // Closing a tab-group leader promotes the next member.
    s.remove(1);
    assert_eq!(
        s.windows.iter().find(|w| w.id == 2).unwrap().tab_group,
        None
    );
    assert_eq!(
        s.windows.iter().find(|w| w.id == 4).unwrap().tab_group,
        Some(2)
    );
    // Upsert replaces in place.
    let mut moved = win(3, None);
    moved.workspace = "edit".into();
    s.upsert(moved);
    assert_eq!(s.windows.len(), 3);
    assert_eq!(s.windows[1].workspace, "edit");
    let path = std::env::temp_dir().join(format!("pm-shell2-session-{}.json", std::process::id()));
    s.save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap(), s);
    std::fs::write(&path, br#"{"version":2,"windows":[]}"#).unwrap();
    assert!(
        Session::load(&path).is_err(),
        "newer sessions are not overwritten"
    );
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn restored_frames_are_sane_and_on_screen() {
    assert_eq!(
        sane_frame([0., 0., 10., 10.]),
        [0., 0., MIN_FRAME[0], MIN_FRAME[1]]
    );
    assert_eq!(
        sane_frame([f64::NAN, 0., 900., 900.]),
        [0., 0., MIN_FRAME[0], MIN_FRAME[1]]
    );
    let screens = [[0., 0., 1512., 945.], [1512., 0., 2560., 1415.]];
    // Off every screen → first screen, kept whole.
    assert_eq!(
        constrain_to_screens([9000., 9000., 800., 600.], &screens),
        [712., 345., 800., 600.]
    );
    // Mostly on the second screen → pulled fully onto it.
    assert_eq!(
        constrain_to_screens([3900., 100., 400., 300.], &screens),
        [3672., 100., 400., 300.]
    );
    // Larger than its screen → shrunk to fit.
    assert_eq!(
        constrain_to_screens([0., 0., 3000., 3000.], &screens[..1]),
        [0., 0., 1512., 945.]
    );
    // A dangling tab-group reference is dropped instead of failing restore.
    let mut s = Session::default();
    s.upsert(win(7, Some(99)));
    assert_eq!(s.restore_plan(true)[0].tab_group, None);
}

#[test]
fn display_only_menu_items_carry_the_old_apps_registry_shortcuts() {
    use pm_shell::{keyboard::shipped, menu};
    let registry = shipped();
    let mut items: Vec<menu::Item> = menu::menus()
        .into_iter()
        .filter(|(title, _)| menu::DISPLAY_ONLY.contains(title))
        .flat_map(|(_, i)| i)
        // A submenu's parent row (Layer ▸ Add 3D Layer) has no chord of its own.
        .filter(|i| !i.label.is_empty() && !menu::SUBMENUS.contains(&i.command.as_str()))
        .collect();
    items.extend(menu::tools());
    assert!(items.len() > 40);
    for i in &items {
        assert!(
            registry
                .iter()
                .any(|b| b.command == i.command && b.chord == i.chord && b.args == i.args),
            "{} ({}) is not a W1 binding",
            i.label,
            i.chord
        );
    }
    // Each tool in the toolbar has a menu row; the registry's tool chords are all shown.
    for b in registry.iter().filter(|b| b.command.starts_with("tool")) {
        assert!(
            items.iter().any(|i| i.command == b.command),
            "{}",
            b.command
        );
    }
    // New real key equivalents outside display-only menus are also W1 chords.
    let all: Vec<_> = menu::menus().into_iter().flat_map(|(_, i)| i).collect();
    for (command, chord) in [
        ("projects", "cmd+p"),
        ("deselect", "cmd+shift+a"),
        ("palette", "cmd+k"),
        ("agent", "cmd+shift+k"),
    ] {
        assert!(
            registry
                .iter()
                .any(|b| b.command == command && b.chord == chord)
        );
        let item = all.iter().find(|i| i.command == command).unwrap();
        assert_eq!(menu::accelerator(&item.chord), menu::accelerator(chord));
    }
    // Window-tab, workspace and panel menus exist (HIG placement).
    for c in [
        "newTab",
        "mergeAllWindows:",
        "toggleTabBar:",
        "panels",
        "workspaces",
    ] {
        assert!(all.iter().any(|i| i.command == c), "{c}");
    }
}

#[test]
fn accelerators_map_special_keys_and_render_glyphs() {
    use pm_shell::menu::{accelerator, glyphs};
    assert_eq!(
        accelerator("Command+Shift+H"),
        ("h".into(), (1 << 20) | (1 << 17))
    );
    assert_eq!(accelerator("cmd+shift+h"), accelerator("Command+Shift+H"));
    assert_eq!(accelerator("space").0, " ");
    assert_eq!(accelerator("home").0, "\u{F729}");
    assert_eq!(accelerator("f9").0, "\u{F70C}");
    assert_eq!(
        accelerator("cmd+shift++"),
        ("+".into(), (1 << 20) | (1 << 17))
    );
    assert_eq!(accelerator("backspace").0, "\u{8}");
    assert_eq!(glyphs("cmd+shift+k"), "⇧⌘K");
    assert_eq!(glyphs("Control+Shift+Tab"), "⌃⇧⇥");
    assert_eq!(glyphs("shift+f3"), "⇧F3");
    assert_eq!(glyphs("alt+["), "⌥[");
    assert_eq!(glyphs("pageup"), "⇞");
    assert_eq!(glyphs("space"), "Space");
    assert_eq!(glyphs(""), "");
}

#[test]
fn palette_ranks_prefix_word_substring_then_subsequence() {
    use pm_shell::transient::{Choice, Picker, rank};
    let c = |l: &str, cmd: &str| Choice {
        label: l.into(),
        command: cmd.into(),
    };
    let mut p = Picker {
        open: true,
        choices: vec![
            c("Go to Start", "gotoStart"),
            c("Show in Workspace: Timeline", "panel:show:timeline"),
            c("Timeline", "x"),
            c("New Composition…", "newComposition"),
        ],
        ..Default::default()
    };
    p.search("time".into());
    assert_eq!(p.filtered(), [2, 1]);
    p.search("gtst".into());
    assert_eq!(p.filtered(), [0]);
    p.search("newcomp".into());
    assert_eq!(p.filtered(), [3], "command ids match literally");
    p.search(String::new());
    assert_eq!(p.filtered(), [0, 1, 2, 3]);
    assert_eq!(rank("Timeline", "line"), Some(2));
    assert_eq!(rank("Save Layout…", "lay"), Some(1));
    assert_eq!(rank("Zoom", "q"), None);
}
