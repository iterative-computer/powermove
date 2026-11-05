use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    keyboard::*,
    layout::*,
    menu,
    transient::{Choice, Picker},
};
fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
#[test]
fn default_matches_approved_workspace() {
    let l = Layout::preset("design");
    l.validate().unwrap();
    assert_eq!(l.docks[0].groups[0].tabs, ["assets", "layer-effects"]);
    assert_eq!(l.docks[0].size, Some(300.));
    assert_eq!(l.docks[2].size, Some(344.));
    assert_eq!(l.docks[3].groups[1].size, Some(268.));
    let g = l.geometry(
        Rect {
            x: 6.,
            y: 58.,
            w: 1428.,
            h: 810.,
        },
        &tokens(),
    );
    let left = &g.panels[0].rect;
    let viewer = &g.panels[1].rect;
    let timeline = &g.panels[2].rect;
    let inspector = &g.panels[3].rect;
    let effects = &g.panels[4].rect;
    assert_eq!(timeline.x, left.x);
    assert_eq!(timeline.w, left.w + 6. + viewer.w);
    assert_eq!(timeline.h, 344.);
    assert_eq!(inspector.y, left.y);
    assert_eq!(effects.y + effects.h, timeline.y + timeline.h);
    assert_eq!(effects.h, 268.);
}
#[test]
fn every_preset_and_small_window_is_bounded() {
    for (id, _) in PRESETS {
        let l = Layout::preset(id);
        l.validate().unwrap();
        for (w, h) in [(1428., 810.), (968., 528.), (350., 200.)] {
            let g = l.geometry(Rect { x: 0., y: 0., w, h }, &tokens());
            for p in g.panels {
                assert!(p.rect.w >= 0. && p.rect.h >= 0., "{id}");
                assert!(p.rect.x + p.rect.w <= w + 0.01);
                assert!(p.rect.y + p.rect.h <= h + 0.01, "{id}: {:?}", p.rect);
            }
        }
    }
}
#[test]
fn persistence_round_trip_and_project_isolation() {
    let mut s = LayoutStore::default();
    for (id, _) in PRESETS {
        s.put("project-a", Layout::preset(id));
    }
    let mut b = Layout::preset("design");
    b.docks[0].size = Some(410.);
    s.put("project-b", b.clone());
    let path =
        std::env::temp_dir().join(format!("pm-shell-layout-test-{}.json", std::process::id()));
    s.save(&path).unwrap();
    let loaded = LayoutStore::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(s, loaded);
    assert_eq!(loaded.get("project-b", "design"), b);
    assert_eq!(loaded.get("project-a", "design").docks[0].size, Some(300.));
}
#[test]
fn move_dock_and_tabs_is_atomic_and_preserves_panels() {
    let mut l = Layout::preset("design");
    l.move_panel("layer-effects", 3, 0, true).unwrap();
    assert_eq!(l.docks[3].groups[0].tabs, ["inspector", "layer-effects"]);
    assert_eq!(l.docks[3].groups[0].active(), "layer-effects");
    l.move_panel("layer-effects", 3, 1, false).unwrap();
    assert_eq!(l.docks[3].groups[1].active(), "layer-effects");
    let before = l.clone();
    assert!(l.move_panel("assets", 3, 99, true).is_err());
    assert_eq!(l, before);
    l.validate().unwrap();
}
#[test]
fn keymap_matches_every_w1_cell() {
    let doc = include_str!("../../../../../docs/native/w1-keymap.md");
    let rows: Vec<_> = doc
        .lines()
        .filter(|s| s.starts_with("| K") && s.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .collect();
    let bindings = shipped();
    assert_eq!(bindings.len(), 197);
    assert_eq!(rows.len(), 195);
    for (row, b) in rows.iter().zip(bindings) {
        let c: Vec<_> = row.split('|').map(str::trim).collect();
        assert_eq!(b.id, c[1]);
        assert_eq!(b.chord, c[2].replace("&#96;", "`"));
        assert_eq!(b.command, c[3]);
        assert_eq!(
            b.args,
            serde_json::from_str::<Vec<serde_json::Value>>(c[4]).unwrap()
        );
        assert_eq!(b.priority, c[5].parse::<i32>().unwrap());
        assert_eq!(b.in_fields, c[6] == "yes");
        assert_eq!(b.repeat, c[7] == "yes");
        assert_eq!(b.loose, c[8] == "yes");
    }
}
#[test]
fn menu_matches_every_w1_equivalent_and_installs_every_command() {
    let doc = include_str!("../../../../../docs/native/w1-keymap.md");
    let rows: Vec<_> = doc.lines().filter(|s| s.starts_with("| M0")).collect();
    let equivalents = menu::equivalents();
    assert_eq!(rows.len(), 35);
    let installed: Vec<_> = menu::menus().into_iter().flat_map(|(_, i)| i).collect();
    for (row, e) in rows.iter().zip(equivalents) {
        let c: Vec<_> = row.split('|').map(str::trim).collect();
        assert_eq!(
            [e.id.as_str(), &e.accelerator, &e.command, &e.label],
            [c[1], c[2], c[3], c[4]]
        );
        let command = match e.command.as_str() {
            "hide" => "hide:",
            "hideOthers" => "hideOtherApplications:",
            "quit" => "terminate:",
            c => c,
        };
        assert!(
            installed
                .iter()
                .any(|i| i.command == command && i.chord == e.accelerator),
            "{}",
            e.command
        );
    }
}
fn key(token: u64, chord: &str) -> Key {
    Key {
        token,
        chord: chord.into(),
        field: false,
        repeat: false,
        up: false,
        compat: false,
    }
}
#[test]
fn declines_replay_in_order_and_bypasses_do_not_wait() {
    let mut r = Resolver::new();
    let out = r.submit(key(1, "j"));
    assert!(
        matches!(&out[0],Effect::Dispatch{command,class:Class::MayDecline,..}if command=="prevVisibleEvent")
    );
    assert!(r.submit(key(2, "k")).is_empty());
    let mut up = key(3, "j");
    up.up = true;
    assert!(r.submit(up).is_empty());
    assert!(matches!(&r.submit(key(4,"cmd+q"))[0],Effect::Dispatch{command,..}if command=="quit"));
    assert_eq!(r.pending_token(), Some(1));
    let out = r.resolve(1, false);
    assert_eq!(out, [Effect::Replay(1)]);
    assert!(matches!(
        &r.replayed(1)[0],
        Effect::Dispatch { token: 2, .. }
    ));
    assert_eq!(r.resolve(2, true), [Effect::Consumed(2), Effect::Replay(3)]);
}
#[test]
fn field_priority_repeat_and_extension_decline() {
    let mut r = Resolver::new();
    let mut k = key(1, "j");
    k.field = true;
    assert_eq!(r.submit(k), [Effect::Replay(1)]);
    r.replayed(1);
    let mut k = key(2, "cmd+s");
    k.field = true;
    assert!(matches!(&r.submit(k)[0],Effect::Dispatch{command,..}if command=="save"));
    r.resolve(2, false);
    r.replayed(2);
    let mut k = key(3, "j");
    k.repeat = true;
    assert_eq!(r.submit(k), [Effect::Replay(3)]);
    r.replayed(3);
    let mut extension = shipped()[0].clone();
    extension.command = "extension.test".into();
    extension.priority = -1;
    r.bindings.push(extension);
    assert!(
        matches!(&r.submit(key(4,"cmd+k"))[0],Effect::Dispatch{command,..}if command=="extension.test")
    );
    assert!(
        matches!(&r.resolve(4,false)[0],Effect::Dispatch{command,class:Class::NeverDeclines,..}if command=="palette")
    );
    assert_eq!(r.pending_token(), None);
}
#[test]
fn picker_search_wrap_and_commit() {
    let mut p = Picker {
        open: true,
        choices: ["Position", "Scale", "Rotation"]
            .iter()
            .map(|s| Choice {
                label: s.to_string(),
                command: s.to_string(),
            })
            .collect(),
        ..Default::default()
    };
    p.navigate("up");
    assert_eq!(p.selected, 2);
    p.search("sc".into());
    assert_eq!(p.filtered(), [1]);
    assert_eq!(p.navigate("enter"), Some("Scale".into()));
    assert!(!p.open);
}
#[test]
fn spring_retarget_preserves_value_and_velocity() {
    use pm_shell::transient::Spring;
    let s = Spring {
        from: 0.96,
        velocity: 0.,
        target: 1.,
        mass: 1.,
        stiffness: 400.,
        damping: 28.,
    };
    assert_eq!(s.sample(0.), (0.96, 0.));
    let (value, velocity) = s.sample(0.08);
    let retarget = Spring {
        from: value,
        velocity,
        target: 0.96,
        ..s
    };
    assert_eq!(retarget.sample(0.), (value, velocity));
    let (value, velocity) = s.sample(10.);
    assert!((value - 1.).abs() < 1e-9 && velocity.abs() < 1e-9);
}
#[test]
fn escape_decline_bypasses_without_resolving_held_command() {
    let mut r = Resolver::new();
    r.submit(key(1, "j"));
    let effects = r.submit(key(2, "escape"));
    assert!(matches!(
        effects[0],
        Effect::Dispatch {
            token: 2,
            class: Class::MayDecline,
            ..
        }
    ));
    assert!(matches!(&r.resolve(2,false)[0],Effect::Dispatch{command,..}if command=="deselect"));
    assert_eq!(r.resolve(2, false), [Effect::Replay(2)]);
    assert_eq!(r.pending_token(), Some(1));
}
#[test]
fn shifted_physical_keys_and_escape_extension_priority() {
    use pm_shell::keyboard::physical_fallback;
    assert_eq!(physical_fallback("{".into(), 33, true), "[");
    assert_eq!(physical_fallback("?".into(), 44, true), "/");
    assert_eq!(physical_fallback("!".into(), 18, true), "1");
    assert_eq!(physical_fallback("\u{f70c}".into(), 101, false), "f9");
    let mut r = Resolver::new();
    let mut b = shipped().pop().unwrap();
    b.chord = "escape".into();
    b.command = "extension.escape".into();
    b.priority = -10;
    r.bindings.push(b);
    r.submit(key(1, "j"));
    assert!(
        matches!(&r.submit(key(2,"escape"))[0],Effect::Dispatch{command,..}if command=="extension.escape")
    );
    assert_eq!(r.resolve(2, true), [Effect::Consumed(2)]);
    assert_eq!(r.pending_token(), Some(1));
}

#[test]
fn maximizing_keeps_saved_layout_and_fills_workspace() {
    let layout = Layout::preset("design");
    let original = layout.clone();
    let bounds = Rect {
        x: 6.,
        y: 58.,
        w: 1428.,
        h: 810.,
    };
    let full = layout.geometry_with_maximized(bounds, &tokens(), Some("timeline"));
    assert_eq!(full.panels.len(), 1);
    assert_eq!(full.panels[0].rect, bounds);
    assert_eq!(
        layout.docks[full.panels[0].dock].groups[full.panels[0].group].active(),
        "timeline"
    );
    assert!(full.dividers.is_empty());
    assert_eq!(layout, original);
    assert_eq!(
        layout
            .geometry_with_maximized(bounds, &tokens(), None)
            .panels
            .len(),
        5
    );
    assert_eq!(
        layout
            .geometry_with_maximized(bounds, &tokens(), Some("absent"))
            .panels
            .len(),
        5
    );
}

#[test]
fn current_main_actions_keep_the_exact_registration_metadata() {
    for (chord, command, priority) in [("`", "maximizePanel", 100), ("m", "toggleLayerStrips", 90)]
    {
        let binding = shipped()
            .into_iter()
            .find(|b| b.command == command)
            .unwrap();
        assert_eq!(binding.chord, chord);
        assert_eq!(binding.priority, priority);
        assert!(binding.args.is_empty());
        assert!(!binding.in_fields && !binding.repeat && !binding.loose);
    }
}
