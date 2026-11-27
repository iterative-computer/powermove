//! ⌘W in Separate Windows (w83 P1-3): the File menu names it Close Project but
//! dispatches closeTab with closeTab's own key, so a page showing closes
//! first and Home closes the window; Close Project stays its own command.
use pm_shell::{keymap, menu};

fn close_items() -> Vec<menu::Item> {
    menu::menus()
        .into_iter()
        .filter(|(title, _)| *title == "File")
        .flat_map(|(_, items)| items)
        .filter(|item| menu::CLOSES.contains(&item.command.as_str()))
        .collect()
}

#[test]
fn separate_windows_command_w_is_close_tab_with_its_own_key() {
    let default = menu::equivalent_accelerator("closeTab").unwrap();
    for tabs in [true, false] {
        pm_shell::set_tabs_mode(tabs);
        let items = close_items();
        let w = items.iter().find(|i| i.chord == default).expect("⌘W is in the File menu");
        assert_eq!(w.command, "closeTab", "tabs {tabs}");
        assert_eq!(w.label == "Close Project", !tabs);
        assert!(items.iter().all(|i| i.command != "closeProject" || i.chord != default));

        // Rebinding closeTab moves the item's key with it.
        keymap::set_user(&[keymap::Assign {
            command: "closeTab".into(),
            args: vec![],
            keys: vec!["Command-Shift-K".into()],
        }]);
        let items = close_items();
        let w = items.iter().find(|i| i.command == "closeTab").unwrap();
        assert_eq!(w.chord, "Command-Shift-K", "tabs {tabs}");
        keymap::set_user(&[]);
    }
    pm_shell::set_tabs_mode(true);
}
