//! Powermove for Mac. See docs/gpui/architecture.md.

mod workspace;

use pm_glass as glass;

use gpui::{App, AppContext as _, Bounds, Menu, MenuItem, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, px, size};

actions!(powermove, [Quit, About, NewProject, OpenProject, Save, Undo, Redo]);

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![
            Menu { disabled: false, name: "Powermove".into(), items: vec![MenuItem::action("About Powermove", About), MenuItem::separator(), MenuItem::action("Quit Powermove", Quit)] },
            Menu {
                disabled: false,
                name: "File".into(),
                items: vec![MenuItem::action("New Project…", NewProject), MenuItem::action("Open…", OpenProject), MenuItem::separator(), MenuItem::action("Save", Save)],
            },
            Menu { disabled: false, name: "Edit".into(), items: vec![MenuItem::action("Undo", Undo), MenuItem::action("Redo", Redo)] },
        ]);

        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Launch Film".into()), appears_transparent: true, traffic_light_position: None }),
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            let actions = glass::install(window);
            cx.new(|cx| workspace::Workspace::new(actions, window, cx))
        })
        .expect("a window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}
