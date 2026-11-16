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
    // The editor's dock area: the dock gap at the sides and bottom, the
    // toolbar above, the status line below.
    let m = tokens().metrics;
    let g = l.geometry(
        Rect {
            x: m.dock_gap,
            y: m.toolbar,
            w: m.window_width - 2. * m.dock_gap,
            h: m.window_height - m.toolbar - m.status - m.dock_gap,
        },
        &tokens(),
    );
    let left = &g.panels[0].rect;
    let viewer = &g.panels[1].rect;
    let timeline = &g.panels[2].rect;
    let inspector = &g.panels[3].rect;
    let effects = &g.panels[4].rect;
    assert_eq!(timeline.x, left.x);
    assert_eq!(timeline.w, left.w + m.dock_gap + viewer.w);
    assert_eq!(timeline.h, 344.);
    assert_eq!(inspector.y, left.y);
    assert_eq!(effects.y + effects.h, timeline.y + timeline.h);
    assert_eq!(effects.h, 268.);
}
#[test]
fn every_preset_and_small_window_is_bounded() {
    for id in pm_shell::workspace::BUILTIN {
        let l = Layout::preset(id);
        l.validate().unwrap();
        for (w, h) in [(1428., 810.), (968., 528.), (350., 200.)] {
            let g = l.geometry(Rect { x: 0., y: 0., w, h }, &tokens());
            for p in g.panels {
                assert!(p.rect.w >= 0. && p.rect.h >= 0., "{id}");
                assert!(p.rect.x + p.rect.w <= w + 0.01);
                if !g.side_docks.contains_key(&p.dock) {
                    assert!(p.rect.y + p.rect.h <= h + 0.01, "{id}: {:?}", p.rect);
                } else {
                    assert!(p.rect.h >= tokens().metrics.header);
                }
            }
        }
    }
}
#[test]
fn edited_layouts_persist_in_their_workspace_record() {
    use pm_shell::workspace::{layout, preset, write};
    for id in pm_shell::workspace::BUILTIN {
        let record = preset(id).unwrap();
        let mut l = layout(&record);
        if let Some(d) = l.docks.iter_mut().find(|d| d.size.is_some()) {
            d.size = Some(410.);
        }
        let written = write(&l, &record);
        assert_eq!(layout(&written), l, "{id}");
        assert_eq!(written["name"], record["name"]);
        assert_eq!(written["custom"], record["custom"]);
    }
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
    // The 195 W1 cells, then native additions (`main.*`, `native.*` ids).
    assert_eq!(bindings.len(), 296);
    let camera = bindings
        .iter()
        .find(|b| b.id == "native.newCamera")
        .unwrap();
    assert_eq!(camera.command, "newCamera");
    assert_eq!(camera.chord, "cmd+alt+shift+c");
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
    // The shuttle is offered first; stopped, it declines to the shipped key.
    let out = r.submit(key(1, "j"));
    assert!(
        matches!(&out[0],Effect::Dispatch{command,class:Class::MayDecline,..}if command=="shuttleBackward")
    );
    let out = r.resolve(1, false);
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
    // A held K has no repeating binding; a held J reaches only the shuttle.
    let mut k = key(3, "k");
    k.repeat = true;
    assert_eq!(r.submit(k), [Effect::Replay(3)]);
    r.replayed(3);
    let mut k = key(5, "j");
    k.repeat = true;
    assert!(matches!(&r.submit(k)[0],Effect::Dispatch{command,..}if command=="shuttleBackward"));
    assert_eq!(r.resolve(5, false), [Effect::Replay(5)]);
    r.replayed(5);
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
            .map(|s| Choice::new(*s, *s))
            .collect(),
        ..Default::default()
    };
    // Nothing is lit when it opens, and Return then does nothing.
    assert_eq!(p.lit(), None);
    assert_eq!(p.navigate("enter"), None);
    assert!(p.open);
    p.navigate("up");
    assert_eq!(p.lit(), Some(2));
    p.navigate("down");
    assert_eq!(p.lit(), Some(0));
    p.search("sc".into());
    assert_eq!(p.filtered(), [1]);
    assert_eq!(p.navigate("enter"), Some("Scale".into()));
    assert!(!p.open);
}
#[test]
fn picker_light_follows_the_pointer_and_leaves_with_it() {
    let mut p = Picker {
        open: true,
        choices: ["Position", "Scale", "Rotation"]
            .iter()
            .map(|s| Choice::new(*s, *s))
            .collect(),
        ..Default::default()
    };
    assert!(p.point(Some(1)));
    assert_eq!(p.lit(), Some(1));
    assert!(!p.point(Some(1)));
    // It leaves with the pointer: nothing was lit before it came.
    assert!(p.point(None));
    assert_eq!(p.lit(), None);
    // A key takes the light from the pointer and steps from where it was.
    p.point(Some(1));
    p.navigate("down");
    assert_eq!((p.hover, p.keyed), (None, Some(2)));
    // The pointer has it while over a row, and gives it back as it leaves.
    p.point(Some(0));
    assert_eq!(p.lit(), Some(0));
    p.point(None);
    assert_eq!(p.lit(), Some(2));
    // A menu with no search field: typing lights the first row it starts.
    assert!(p.type_ahead("ro"));
    assert_eq!(p.lit(), Some(2));
    assert!(!p.type_ahead("x"));
    // Clearing a query lights nothing again.
    p.search("s".into());
    assert_eq!(p.lit(), Some(0));
    p.search(String::new());
    assert_eq!(p.lit(), None);
}
#[test]
fn picker_keeps_sections_whole_and_orders_them_by_best_match() {
    let row = |label: &str, category: &str| Choice::new(label, label).within(category);
    let mut p = Picker {
        choices: vec![
            row("New Project", "File"),
            row("Export…", "File"),
            row("Copy", "Edit"),
            row("Show Export Queue", "View"),
            row("Paste", "Edit"),
        ],
        ..Default::default()
    };
    // No query: sections in the order they first appear, each one whole.
    assert_eq!(p.filtered(), [0, 1, 2, 4, 3]);
    // A prefix match brings its section first; the word match's follows.
    p.search("exp".into());
    assert_eq!(p.filtered(), [1, 3]);
    p.search("p".into());
    assert_eq!(p.filtered(), [4, 2, 0, 1, 3]);
}
#[test]
fn spring_retarget_preserves_value_and_velocity() {
    use pm_design::motion::Analytic as Spring;
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

#[test]
fn composition_tab_enters_fields_but_declines_back_to_focus_navigation() {
    let binding = shipped()
        .into_iter()
        .find(|b| b.command == "composition.flowchart")
        .unwrap();
    assert_eq!(binding.chord, "tab");
    assert_eq!(binding.priority, 0);
    assert!(binding.in_fields);
    assert!(!binding.repeat && !binding.loose);
    assert!(binding.args.is_empty());
    let mut resolver = Resolver::new();
    let key = Key {
        token: 90,
        chord: "tab".into(),
        field: true,
        repeat: false,
        up: false,
        compat: false,
    };
    assert!(
        matches!(resolver.submit(key).as_slice(), [Effect::Dispatch { command, .. }]
        if command == "composition.flowchart")
    );
    assert_eq!(resolver.resolve(90, false), [Effect::Replay(90)]);
}

/// A long list (the font picker over every installed family) lists a page:
/// keys and Return reach only listed rows, the rest are counted, and the
/// search finds a family past the first page.
#[test]
fn picker_pages_a_long_list_and_the_search_reaches_past_it() {
    let families: Vec<String> = (0..200).map(|i| format!("Family {i:03}")).collect();
    let mut p = Picker {
        open: true,
        page: Some(80),
        choices: families
            .iter()
            .map(|f| Choice::new(f.clone(), format!("fontPick:{f}")).in_font(f.clone()))
            .collect(),
        ..Default::default()
    };
    assert_eq!(p.filtered().len(), 80);
    assert_eq!(p.unlisted(), 120);
    p.navigate("end");
    assert_eq!(
        p.lit(),
        Some(79),
        "the last listed row, not the last family"
    );
    p.navigate("down");
    assert_eq!(p.lit(), Some(0), "wraps within the page");
    p.search("Family 15".into());
    assert_eq!(p.unlisted(), 0);
    assert!(p.filtered().len() <= 80);
    assert_eq!(
        p.navigate("enter").as_deref(),
        Some("fontPick:Family 150"),
        "a family past the first page is one search away"
    );
}
#[test]
fn a_row_that_stays_is_chosen_and_the_menu_stays_open() {
    // The Agent's provider rows: choosing one keeps the menu for the
    // provider's models; a model row closes it.
    let mut p = Picker {
        open: true,
        choices: vec![
            Choice::new("ChatGPT", "agentPick:0").stays(true),
            Choice::new("Claude", "agentPick:1").stays(true),
            Choice::new("GPT-6.1-Sol", "agentPick:2"),
        ],
        ..Default::default()
    };
    p.search("claude".into());
    assert_eq!(p.navigate("down"), None);
    assert_eq!(p.navigate("enter"), Some("agentPick:1".into()));
    assert!(p.open, "a provider keeps the menu open");
    p.search("sol".into());
    p.navigate("down");
    assert_eq!(p.navigate("enter"), Some("agentPick:2".into()));
    assert!(!p.open, "a model closes it");
}

/// Typing straight after ⌘K reaches the palette: the shortcut resolves to
/// the palette at once, and the shell opens it inside that key press
/// (`opens_in_place`, `Host::effects`) so the next key is read with the
/// palette's field already first responder, never by the editor.
#[test]
fn the_palette_opens_inside_its_key_press_so_no_typing_is_lost() {
    let mut r = Resolver::new();
    let out = r.submit(key(1, "cmd+k"));
    assert!(
        matches!(&out[0], Effect::Dispatch { command, class: Class::NeverDeclines, .. } if command == "palette"),
        "{out:?}"
    );
    // Carried out inside this key press (not through the editor's queue):
    // the palette's field is first responder before the next key is read.
    assert!(pm_shell::opens_in_place("palette"));
    assert!(!pm_shell::opens_in_place("save"));
}

#[test]
fn view_menu_exposes_the_pie_choices_without_stealing_maximize() {
    let items = menu::view3d();
    for (group, value) in [
        ("nav", "left"),
        ("nav", "right"),
        ("nav", "bottom"),
        ("nav", "top"),
        ("nav", "front"),
        ("nav", "back"),
        ("look", "camera"),
        ("nav", "selected"),
        ("nav", "persp"),
        ("nav", "roll-left"),
        ("nav", "roll-right"),
    ] {
        assert!(
            items.iter().any(|i| i.command == "studio.view"
                && i.args == vec![serde_json::json!(group), serde_json::json!(value)]),
            "{group}/{value}"
        );
    }
    let bindings = shipped();
    let grave = bindings.iter().find(|b| b.chord == "`").unwrap();
    assert_eq!(grave.command, "maximizePanel");
    for (chord, value) in [
        ("numpad5", "persp"),
        ("5", "persp"),
        ("shift+numpad4", "roll-left"),
        ("shift+numpad6", "roll-right"),
    ] {
        assert!(bindings.iter().any(|b| b.chord == chord
            && b.command == "studio.view"
            && b.args == vec![serde_json::json!("nav"), serde_json::json!(value)]));
    }
}

#[test]
fn overflowing_side_splitter_tracks_the_drag_and_keeps_fixed_neighbours() {
    let t = tokens();
    let bounds = Rect {
        x: 0.,
        y: 0.,
        w: 1000.,
        h: 400.,
    };
    for first in [None, Some(300.)] {
        let mut layout = Layout::preset("design");
        let di = layout
            .docks
            .iter()
            .position(|dock| dock.id == "right")
            .unwrap();
        layout.docks[di].groups = ["inspector", "easing", "fxbrowser"]
            .into_iter()
            .enumerate()
            .map(|(i, id)| Group {
                tabs: vec![id.into()],
                active: 0,
                size: if i == 0 { first } else { Some(300.) },
                collapsed: false,
            })
            .collect();
        let before = layout.geometry_with_panel_heights(bounds, &t, &Default::default());
        let divider = before
            .dividers
            .iter()
            .find(|d| d.dock == di && d.group == Some(0))
            .unwrap();
        let heights = |g: &Geometry| {
            g.panels
                .iter()
                .filter(|p| p.dock == di)
                .map(|p| p.rect)
                .collect::<Vec<_>>()
        };
        let original = heights(&before);
        layout.resize_with_panel_heights(divider, 40., bounds, &t, &Default::default());
        let resized = heights(&layout.geometry_with_panel_heights(bounds, &t, &Default::default()));
        assert_eq!(
            layout.docks[di].groups[0].size.is_none(),
            first.is_none(),
            "resizing preserves fluid sizing"
        );
        let saved =
            pm_shell::workspace::write(&layout, &pm_shell::workspace::preset("design").unwrap());
        let restored = pm_shell::workspace::layout(&saved);
        assert_eq!(
            restored.docks[di].groups.len(),
            layout.docks[di].groups.len()
        );
        for (actual, expected) in restored.docks[di]
            .groups
            .iter()
            .zip(&layout.docks[di].groups)
        {
            let mut expected = expected.clone();
            if let Some(basis) = layout.flex_bases.get(&expected.tabs[0]).copied() {
                assert!(
                    (restored.flex_bases[&actual.tabs[0]] - basis).abs() <= 0.05,
                    "saved basis keeps workspace precision"
                );
            }
            assert_eq!(
                actual, &expected,
                "workspace keeps fixed heights and fluid sizing"
            );
        }
        assert_eq!(pm_shell::workspace::write(&restored, &saved), saved);
        if first.is_none() {
            let larger = layout.geometry_with_panel_heights(
                Rect { h: 1400., ..bounds },
                &t,
                &Default::default(),
            );
            assert!(
                heights(&larger)[0].h > resized[0].h,
                "fluid panel still fills a taller window"
            );
        }
        assert_eq!(resized[0].h, original[0].h + 40.);
        assert_eq!(resized[1].h, original[1].h - 40.);
        assert_eq!(
            resized[2], original[2],
            "unrelated fixed panel stays in place and keeps its height"
        );
    }
}

// Contextual commands (priority 300) get first refusal before the agent's
// global Pause fallback (400). The Cite lane owns whether its source has a
// focused, nonempty prose selection; the shell must honour its handled result.
#[test]
fn agent_prose_selection_cites_without_pausing() {
    let mut resolver = Resolver::new();
    resolver.bindings.push(Binding {
        id: "context.agent.cite".into(),
        chord: "cmd+shift+.".into(),
        command: "agent.citeSelection".into(),
        args: vec![],
        priority: 300,
        in_fields: true,
        repeat: false,
        loose: false,
    });
    let out = resolver.submit(key(500, "cmd+shift+."));
    assert!(
        matches!(out.as_slice(),[Effect::Dispatch{command,..}] if command=="agent.citeSelection")
    );
    // The focused prose source accepts Cite and stops propagation.
    assert_eq!(resolver.resolve(500, true), [Effect::Consumed(500)]);
}
#[test]
fn no_prose_selection_declines_to_global_agent_pause() {
    let mut resolver = Resolver::new();
    resolver.bindings.push(Binding {
        id: "context.agent.cite".into(),
        chord: "cmd+shift+.".into(),
        command: "agent.citeSelection".into(),
        args: vec![],
        priority: 300,
        in_fields: true,
        repeat: false,
        loose: false,
    });
    resolver.submit(key(501, "cmd+shift+."));
    let out = resolver.resolve(501, false);
    assert!(
        matches!(out.as_slice(),[Effect::Dispatch{command,class:Class::MayDecline,..}] if command=="agent.pause")
    );
    assert_eq!(resolver.resolve(501, true), [Effect::Consumed(501)]);
    let out = Resolver::new().submit(key(502, "cmd+shift+."));
    assert!(matches!(out.as_slice(),[Effect::Dispatch{command,..}] if command=="agent.pause"));
}

#[test]
fn properties_easing_effects_scroll_at_normal_height_without_squeezing() {
    let t = tokens();
    let bounds = Rect {
        x: 0.,
        y: 0.,
        w: 1248.,
        h: 705. - t.metrics.toolbar - t.metrics.status - t.metrics.dock_gap,
    };
    let mut layout = Layout::preset("design");
    let di = layout.docks.iter().position(|d| d.id == "right").unwrap();
    layout.docks[di].groups = ["inspector", "easing", "fxbrowser"]
        .into_iter()
        .map(|id| Group {
            tabs: vec![id.into()],
            active: 0,
            size: None,

            collapsed: false,
        })
        .collect();
    let geometry = layout.geometry_with_panel_heights(bounds, &t, &Default::default());
    let viewport = geometry.side_docks[&di];
    let panels: Vec<_> = geometry.panels.iter().filter(|p| p.dock == di).collect();
    assert!(
        panels.last().unwrap().rect.y + panels.last().unwrap().rect.h > viewport.y + viewport.h
    );
    for (p, usable_floor) in panels.iter().zip([360., 320., 400.]) {
        assert!(p.rect.h >= usable_floor);
        assert!(p.rect.h >= SIDE_FLUID_FLOOR);
    }
    let natural = std::collections::BTreeMap::from([
        ("inspector".into(), 2000.),
        ("easing".into(), 360.),
        ("fxbrowser".into(), 5000.),
    ]);
    let measured = layout.geometry_with_panel_heights(bounds, &t, &natural);
    for p in measured.panels.iter().filter(|p| p.dock == di) {
        assert!(p.rect.h >= SIDE_FLUID_FLOOR && p.rect.h <= 705. * 0.6 + 0.01);
    }
    // The splitter starts from the same measured geometry; a saved size wins
    // over automatic natural-content sizing and remains a fluid basis.
    let divider = measured
        .dividers
        .iter()
        .find(|d| d.dock == di && d.group == Some(0))
        .unwrap();
    let before = measured
        .panels
        .iter()
        .find(|p| p.dock == di && p.group == 0)
        .unwrap()
        .rect
        .h;
    layout.resize_with_panel_heights(divider, -40., bounds, &t, &natural);
    let resized = layout.geometry_with_panel_heights(bounds, &t, &natural);
    assert_eq!(
        resized
            .panels
            .iter()
            .find(|p| p.dock == di && p.group == 0)
            .unwrap()
            .rect
            .h,
        before - 40.
    );
    assert!(layout.docks[di].groups[0].size.is_none());
    layout.docks[di].groups[1].size = Some(240.);
    layout
        .flex_bases
        .remove(&layout.docks[di].groups[1].tabs[0]);
    assert_eq!(
        side_group_floor(&layout.docks[di].groups[1], bounds.h, &t, Some(5000.)),
        240.
    );
    let record =
        pm_shell::workspace::write(&layout, &pm_shell::workspace::preset("design").unwrap());
    let restored = pm_shell::workspace::layout(&record);
    assert!(restored.docks[di].groups[0].size.is_none());
    assert!(
        (restored.flex_bases[&restored.docks[di].groups[0].tabs[0]]
            - layout.flex_bases[&layout.docks[di].groups[0].tabs[0]])
            .abs()
            <= 0.05
    );
    assert_eq!(restored.docks[di].groups[1].size, Some(240.));
    assert_eq!(pm_shell::workspace::write(&restored, &record), record);
}

#[test]
fn resetting_a_side_split_updates_fluid_bases_without_moving_its_neighbour() {
    let t = tokens();
    let bounds = Rect {
        x: 0.,
        y: 0.,
        w: 1000.,
        h: 500.,
    };
    let mut layout = Layout::preset("design");
    let di = layout.docks.iter().position(|d| d.id == "right").unwrap();
    layout.docks[di].groups = [("a", 400.), ("b", 300.), ("c", 400.)]
        .into_iter()
        .map(|(id, basis)| Group {
            tabs: vec![id.into()],
            active: 0,
            size: None,
            collapsed: false,
        })
        .collect();
    layout.flex_bases = [("a".into(), 400.), ("b".into(), 300.), ("c".into(), 400.)].into();
    let before = layout.geometry_with_panel_heights(bounds, &t, &Default::default());
    let divider = before
        .dividers
        .iter()
        .find(|d| d.dock == di && d.group == Some(0))
        .unwrap();
    assert!(layout.reset_split_with_panel_heights(divider, bounds, &t, &Default::default()));
    let after = layout.geometry_with_panel_heights(bounds, &t, &Default::default());
    let panels: Vec<_> = after.panels.iter().filter(|p| p.dock == di).collect();
    assert_eq!(panels[0].rect.h, 350.);
    assert_eq!(panels[1].rect.h, 350.);
    assert_eq!(
        panels[2].rect,
        before
            .panels
            .iter()
            .find(|p| p.dock == di && p.group == 2)
            .unwrap()
            .rect
    );
    assert!(layout.docks[di].groups.iter().all(|g| g.size.is_none()));
}

#[test]
fn dock_wheel_hit_test_includes_edge_grabs_and_stacked_gaps_only() {
    let mut geometry = Geometry::default();
    geometry.side_docks.insert(
        0,
        Rect {
            x: 0.,
            y: 40.,
            w: 200.,
            h: 560.,
        },
    );
    geometry.side_docks.insert(
        2,
        Rect {
            x: 800.,
            y: 40.,
            w: 200.,
            h: 560.,
        },
    );
    geometry.dividers = vec![
        Divider {
            dock: 0,
            group: None,
            horizontal: false,
            rect: Rect {
                x: 200.,
                y: 40.,
                w: 10.,
                h: 560.,
            },
        },
        Divider {
            dock: 2,
            group: None,
            horizontal: false,
            rect: Rect {
                x: 790.,
                y: 40.,
                w: 10.,
                h: 560.,
            },
        },
    ];
    for x in [0., 100., 199., 200., 205., 215.] {
        assert_eq!(geometry.wheel_dock(x, 300., 6.), Some(0));
    }
    for x in [784., 790., 799., 800., 900., 999.] {
        assert_eq!(geometry.wheel_dock(x, 300., 6.), Some(2));
    }
    for (x, y) in [
        (216., 300.),
        (783., 300.),
        (500., 300.),
        (205., 39.),
        (205., 600.),
    ] {
        assert_eq!(geometry.wheel_dock(x, y, 6.), None);
    }
    // A horizontal timeline divider and dividers outside side docks never route.
    geometry.dividers.push(Divider {
        dock: 1,
        group: None,
        horizontal: true,
        rect: Rect {
            x: 216.,
            y: 600.,
            w: 568.,
            h: 10.,
        },
    });
    assert_eq!(geometry.wheel_dock(500., 605., 6.), None);
}
