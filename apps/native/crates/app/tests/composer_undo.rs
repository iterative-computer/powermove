//! PM-749: type a prompt in the Agent composer, send it (the host empties the
//! field), then ⌘Z with the composer still focused. AppKit's typing undo used
//! to hold the sent prompt's ranges and abort the app (NSRangeException in
//! `-[NSUndoTyping undoRedo:]`). Now the field's undo holds nothing from the
//! sent prompt, and ⌘Z goes on to the project's undo. The real composer text
//! view, in a window that is never shown; this binary keeps the main thread.
use gpui_macos::native_text::{NativeTextSlot, TextKind};
use objc2::{MainThreadMarker, MainThreadOnly, Message, msg_send, rc::Retained};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSResponder, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSUndoManager};

fn type_text(view: &NSView, text: &str) {
    for word in text.split_inclusive(' ') {
        // SAFETY: NSTextView's typing entry point, on the main thread.
        unsafe {
            let _: () = msg_send![view, insertText: &*NSString::from_str(word)];
        }
    }
}

/// One turn of the run loop: AppKit closes the typing's undo group at the
/// end of the event, as the app's loop does.
fn settle() {
    // SAFETY: Foundation's run loop and date, on the main thread.
    unsafe {
        let run_loop: Retained<objc2::runtime::AnyObject> =
            msg_send![objc2::class!(NSRunLoop), currentRunLoop];
        let date: Retained<objc2::runtime::AnyObject> =
            msg_send![objc2::class!(NSDate), dateWithTimeIntervalSinceNow: 0.05f64];
        let _: () = msg_send![&*run_loop, runUntilDate: &*date];
    }
}

fn main() {
    let mtm = MainThreadMarker::new().expect("the test binary keeps the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon, never in front of anyone.
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(480., 160.));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let gpui = NSView::initWithFrame(NSView::alloc(mtm), frame);
    window.setContentView(Some(&gpui));
    let slot = NativeTextSlot::new(&gpui, TextKind::Composer, "Composer", "");
    let field: Retained<NSView> = slot.native_view().retain();
    // The slot's wrapper (around its scroll view) stands over the GPUI view
    // in the window's frame: give it room.
    let scroll = field.enclosingScrollView().expect("the composer scrolls");
    let wrapper = unsafe { scroll.superview() }.expect("the slot's wrapper");
    scroll.setFrame(NSRect::new(NSPoint::new(0., 0.), frame.size));
    wrapper.setFrame(frame);
    slot.begin_editing();
    assert!(
        window.makeFirstResponder(Some(&field)),
        "the composer takes the keyboard"
    );

    // The person types a prompt.
    let prompt = "Make the title bigger and blur the background";
    type_text(&field, prompt);
    settle();
    assert_eq!(slot.value(), prompt);
    let undo: Retained<NSUndoManager> = unsafe { msg_send![&*field, undoManager] };
    assert!(undo.canUndo(), "typing is undoable before the send");

    // Send: the host empties the field once the model has the prompt, and
    // the turn ends with the composer still focused.
    slot.replace_value("");
    assert_eq!(slot.value(), "");

    // ⌘Z: whatever the field still holds runs, and nothing aborts.
    let undone = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
        if undo.canUndo() {
            undo.undo();
        }
    }));
    assert!(undone.is_ok(), "⌘Z after the send raised {undone:?}");
    assert!(
        !undo.canUndo(),
        "the field's undo holds nothing from the sent prompt"
    );
    assert_eq!(slot.value(), "", "the sent prompt never comes back");
    // The Edit menu's Undo then reaches the project's undo (the agent's
    // change set), never the emptied field.
    let responder: &NSResponder = &field;
    assert!(pm_shell::composer_passes(responder, "contextUndo"));
    assert!(pm_shell::composer_passes(responder, "contextRedo"));

    // The next draft's typing is the field's own again.
    type_text(&field, "Now the subtitle");
    settle();
    assert!(undo.canUndo());
    assert!(!pm_shell::composer_passes(responder, "contextUndo"));
    undo.undo();
    assert_eq!(slot.value(), "", "the new draft's typing undoes");
    window.close();
    println!("composer_undo: ok");
}
