//! Real main-thread AppKit acceptance probe. Run via followups_check.rs macos.
//! Linux does not count this as a passing test. No worker-thread SKIP fallback.
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("SKIP: native main-thread surface probe requires macOS");
}
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::*;
    use objc2_foundation::{
        NSDate, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSSize, NSString,
    };
    use pm_native_surface_followups::{
        authority, dialogs, kit_native, library, media, media_appkit,
    };
    let mtm = MainThreadMarker::new().expect("this executable must run on the AppKit main thread");
    let _application = NSApplication::sharedApplication(mtm);
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
    let owner = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(
                NSPoint::ZERO,
                NSSize::new(
                    tokens.metrics.min_width.into(),
                    tokens.metrics.min_height.into(),
                ),
            ),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    for (key, keycode) in [("\u{1b}", 53), ("\r", 36)] {
        let surface = dialogs::delete_composition(
            dialogs::DialogId("delete".into()),
            dialogs::WindowId("owner".into()),
            "Synthetic",
            0,
        );
        let prepared = dialogs::appkit::PreparedAlert::new(&surface, &tokens, mtm).unwrap();
        let alert = prepared.alert;
        let (sender, receiver) = std::sync::mpsc::channel();
        let done = block2::RcBlock::new(move |response: NSModalResponse| {
            sender.send(response).unwrap();
        });
        alert.beginSheetModalForWindow_completionHandler(&owner, Some(&done));
        assert_eq!(
            alert.buttons().objectAtIndex(0).title().to_string(),
            "Cancel"
        );
        assert_eq!(
            alert.buttons().objectAtIndex(0).keyEquivalent().to_string(),
            "\u{1b}"
        );
        assert!(alert.window().defaultButtonCell().is_some());
        let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::ZERO,NSEventModifierFlags::empty(),0.,alert.window().windowNumber(),None,&NSString::from_str(key),&NSString::from_str(key),false,keycode).unwrap();
        // Return is handled by the default cell; Escape by its explicit equivalent.
        if !alert.window().performKeyEquivalent(&event) {
            alert.window().keyDown(&event);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let response = loop {
            if let Ok(response) = receiver.try_recv() {
                break response;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Cancel did not receive native key {key:?}"
            );
            NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
        };
        assert_eq!(
            response, NSAlertFirstButtonReturn,
            "a safe default must never dispatch Delete"
        );
    }
    // Native controls/geometry inspected on main, not on cargo's worker threads.
    let target = authority::Target {
        window: "owner".into(),
        object: "extension".into(),
        generation: 1,
    };
    let setup = authority::values::Setup::open(
        target,
        "Synthetic".into(),
        "env".into(),
        vec![(
            authority::values::Declaration {
                key: "key".into(),
                label: "API Key".into(),
                kind: authority::values::Kind::Secret,
                required: true,
                help: None,
            },
            authority::values::SavedValue::Missing,
        )],
        authority::values::Origin::ExtensionSettings,
    )
    .unwrap();
    let sheet = authority::macos::setup_sheet(&setup, &tokens, mtm);
    assert!(sheet.fields["key"].isKindOfClass(NSSecureTextField::class()));
    assert_eq!(sheet.layout.inset, f64::from(tokens.metrics.sheet_inset));
    assert_eq!(sheet.fields["key"].stringValue().to_string(), "");
    let format = library::Format {
        width: 1920,
        height: 1080,
        fps: 30.,
        duration: 10.,
        background: "authored".into(),
    };
    let mut form = library::NewProjectForm::new("owner".into(), None, &format, &[]);
    form.submitting = true;
    let sheet = library::appkit::new_project_sheet(&form, &tokens, mtm);
    assert_eq!(sheet.buttons["create"].title().to_string(), "Creating…");
    assert!(!sheet.buttons["cancel"].isEnabled());
    assert!(!sheet.buttons["create"].isEnabled());
    let sequence = media::sequence::Sequence::new(
        media::Target {
            owner: media::Owner {
                window: "owner".into(),
                project: "p".into(),
                document: 1,
                revision: 1,
            },
            id: "s".into(),
            generation: 1,
        },
        vec![(1, "one.png".into())],
        false,
        30.,
        false,
        false,
    )
    .unwrap();
    let sheet = media_appkit::sequence_sheet(&sequence, &tokens, mtm);
    assert_eq!(
        sheet.layout.value_column,
        f64::from(tokens.metrics.sheet_value_column)
    );
    let library = library::window::LibraryWindow::new(&tokens, mtm);
    assert_eq!(library.window.title().to_string(), "Project Library");
    assert!(kit_native::appkit::menu_width("Auto (Quarter)", &tokens, mtm) > 0.);
    println!(
        "PASS: 2 real default-Cancel key dispatches; setup/New Project/sequence/library native controls; system menu measurement"
    );
}
