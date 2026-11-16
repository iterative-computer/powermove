//! PM-424: whatever the app shows over a project as a modal, nothing under it
//! can be edited. Every such surface is a sheet on the editor's window, of
//! one of three kinds: a GPUI dialog (Export), an AppKit form sheet
//! (Composition Settings, New Project, the keyframe and caption forms) or an
//! alert (Recovery, every confirmation). With each kind up, the menu bar
//! offers only what acts on the app, never on the project, and ⌘Z typed in
//! the sheet undoes nothing. (Clicks under a sheet are AppKit's to refuse;
//! drops are refused by the GPUI view, `covered_by_sheet`.)
//! Separate process: AppKit owns one application and requires the main thread.
#[cfg(target_os = "macos")]
fn main() {
    use gpui::{
        AnyElement, App, AppContext as _, Bounds, Context, IntoElement, Render, Window,
        WindowBounds, WindowKind, WindowOptions, div, point, px, size,
    };
    use objc2::{MainThreadMarker, MainThreadOnly, msg_send, rc::Retained, sel};
    use objc2_app_kit::{
        NSAlert, NSApplication, NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType,
        NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use pm_shell::{Command, Host, integration::Integration, layout::Rect};
    use std::{cell::RefCell, collections::BTreeSet, rc::Rc, time::Duration};

    struct Probe(Rc<RefCell<Vec<String>>>);
    impl Integration for Probe {
        fn observe(&self, _: &mut Context<pm_shell::view::Editor>) {}
        fn command(&self, command: &Command, _: &mut Window, _: &mut App) -> bool {
            self.0.borrow_mut().push(command.name.clone());
            true
        }
        fn panel(&self, _: &str, _: Rect, _: &mut Window, _: &mut App) -> Option<AnyElement> {
            None
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }
    struct Empty;
    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    /// The commands the menu bar offers now: every row the shell answers for
    /// (its tag is the row's place in `menu::menus`, separators counted).
    fn offered() -> BTreeSet<String> {
        let rows: Vec<String> = pm_shell::menu::menus()
            .into_iter()
            .flat_map(|(_, items)| items)
            .map(|i| i.command)
            .collect();
        let mtm = MainThreadMarker::new().unwrap();
        let bar = NSApplication::sharedApplication(mtm).mainMenu().unwrap();
        let mut on = BTreeSet::new();
        for top in bar.itemArray() {
            let Some(menu) = top.submenu() else { continue };
            for item in menu.itemArray() {
                if item.action() != Some(sel!(act:)) || item.representedObject().is_some() {
                    continue;
                }
                let Some(target) = item.target() else {
                    continue;
                };
                let enabled: bool = unsafe { msg_send![&*target, validateMenuItem: &*item] };
                if enabled && let Some(command) = rows.get(item.tag() as usize) {
                    on.insert(command.clone());
                }
            }
        }
        on
    }
    /// ⌘Z typed where the person's keys go while the sheet is up.
    fn command_z(sheet: &NSWindow) {
        let mtm = MainThreadMarker::new().unwrap();
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    kind,
                    NSPoint::new(0., 0.),
                    NSEventModifierFlags::Command,
                    0.,
                    sheet.windowNumber(),
                    None,
                    &NSString::from_str("z"),
                    &NSString::from_str("z"),
                    false,
                    6,
                )
            .unwrap();
            NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
        }
    }

    let scratch = std::env::temp_dir().join(format!(
        "pm-modal-sheets-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    // No restored user windows and no writes to the person's shell session.
    unsafe {
        std::env::set_var("PM_NATIVE_SHELL_STATE", scratch.join("session.json"));
    }
    let heard = Rc::new(RefCell::new(Vec::<String>::new()));
    let started = Rc::new(std::cell::Cell::new(false));
    let factory = Rc::new(
        move |host: Rc<RefCell<Host>>, window: &mut Window, cx: &mut App| -> Rc<dyn Integration> {
            let probe = Rc::new(Probe(heard.clone()));
            if started.replace(true) {
                return probe;
            }
            let editor: Retained<NSWindow> = host.borrow().window.clone();
            let handle = window.window_handle();
            let heard = heard.clone();
            cx.spawn(async move |cx| {
                let wait = |ms| cx.background_executor().timer(Duration::from_millis(ms));
                wait(800).await;
                // Nothing over the editor: the project's commands are offered
                // (else the checks below would prove nothing).
                let open = offered();
                for command in ["contextUndo", "delete", "save", "import"] {
                    assert!(open.contains(command), "{command} is offered with no sheet up");
                }
                // And ⌘Z typed in the editor reaches the project.
                command_z(&editor);
                wait(400).await;
                assert!(
                    heard.borrow().iter().any(|c| c.contains("ndo")),
                    "⌘Z in the editor reaches the project ({:?})",
                    heard.borrow()
                );
                let mtm = MainThreadMarker::new().unwrap();
                let held = |surface: &str, sheet: &NSWindow| {
                    assert!(
                        editor.attachedSheet().is_some(),
                        "{surface} is a sheet on the editor"
                    );
                    let open = offered();
                    let stray: Vec<_> = open
                        .iter()
                        .filter(|c| !pm_shell::menu::BESIDE_SHEET.contains(&c.as_str()))
                        .collect();
                    assert!(
                        stray.is_empty(),
                        "{surface}: the menu bar still offers {stray:?} for the project under it"
                    );
                    heard.borrow_mut().clear();
                    command_z(sheet);
                };
                let nothing_heard = |surface: &str| {
                    let heard = heard.borrow();
                    assert!(
                        !heard.iter().any(|c| c.contains("ndo") || c == "undo"),
                        "{surface}: ⌘Z in the sheet reached the project ({heard:?})"
                    );
                };
                // Export: a GPUI dialog on the editor.
                let dialog = cx
                    .update(|cx| {
                        let mut options = WindowOptions::new().kind(WindowKind::Dialog);
                        options.parent = Some(handle);
                        options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(420.), px(260.)),
                        )));
                        cx.open_window(options, |_, cx| cx.new(|_| Empty))
                    })
                    .unwrap();
                wait(400).await;
                let sheet = editor.attachedSheet().expect("the Export dialog is a sheet");
                held("Export", &sheet);
                wait(400).await;
                nothing_heard("Export");
                cx.update(|cx| dialog.update(cx, |_, w, _| w.remove_window()).unwrap());
                wait(400).await;
                if let Some(sheet) = editor.attachedSheet() {
                    editor.endSheet(&sheet);
                    wait(300).await;
                }
                // Composition Settings and the other forms: an AppKit sheet.
                let form = unsafe {
                    NSWindow::initWithContentRect_styleMask_backing_defer(
                        NSWindow::alloc(mtm),
                        NSRect::new(NSPoint::new(0., 0.), NSSize::new(420., 260.)),
                        NSWindowStyleMask::Titled,
                        NSBackingStoreType::Buffered,
                        false,
                    )
                };
                unsafe { form.setReleasedWhenClosed(false) };
                let (on, shown) = (editor.clone(), form.clone());
                gpui_macos::child_window::sheet(
                    &editor,
                    &form,
                    move || on.beginSheet_completionHandler(&shown, None),
                    || {},
                );
                wait(400).await;
                held("Composition Settings", &form);
                wait(400).await;
                nothing_heard("Composition Settings");
                editor.endSheet(&form);
                wait(400).await;
                // Recovery and every confirmation: an alert sheet.
                let alert = NSAlert::new(mtm);
                alert.setMessageText(&NSString::from_str("Recover the unsaved changes?"));
                alert.beginSheetModalForWindow_completionHandler(&editor, None);
                wait(400).await;
                let sheet = editor.attachedSheet().expect("the alert is a sheet");
                held("Recovery", &sheet);
                wait(400).await;
                nothing_heard("Recovery");
                editor.endSheet(&sheet);
                wait(300).await;
                println!(
                    "modal_sheets: Export, Composition Settings and Recovery each hold the project's menu commands and ⌘Z"
                );
                std::process::exit(0);
            })
            .detach();
            probe
        },
    );
    pm_shell::run_integrated(metal::Device::system_default().expect("Metal"), factory).unwrap();
}
#[cfg(not(target_os = "macos"))]
fn main() {}
