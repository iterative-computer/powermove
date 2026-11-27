//! S035 native colour choice: the system panel, whose eyedropper needs no code,
//! and `NSColorSampler` directly. Values come back as `#RRGGBB`, and only the
//! final colour is written, so one choice is one Undo step.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
    sel,
};
use objc2_app_kit::{NSColor, NSColorPanel, NSColorSampler, NSColorSpace};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString};
use std::cell::RefCell;

pub fn hex(colour: &NSColor) -> Option<String> {
    let colour = colour.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let channel = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        channel(colour.redComponent()),
        channel(colour.greenComponent()),
        channel(colour.blueComponent())
    ))
}
pub struct WatchState {
    tx: async_channel::Sender<Option<String>>,
    /// The panel has no Cancel, so closing it unchanged must write nothing.
    seeded: Option<String>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMColourPanelWatch"]
    #[ivars = WatchState]
    struct Watch;
    unsafe impl NSObjectProtocol for Watch {}
    impl Watch {
        #[unsafe(method(closed:))]
        fn closed(&self, _: &NSNotification) {
            let mtm = MainThreadMarker::new().unwrap();
            let colour = hex(&NSColorPanel::sharedColorPanel(mtm).color());
            let changed = colour.filter(|c| Some(c) != self.ivars().seeded.as_ref());
            let _ = self.ivars().tx.try_send(changed);
        }
        /// The editor that asked closed: the panel goes with it.
        #[unsafe(method(parentClosed:))]
        fn parent_closed(&self, _: &NSNotification) {
            let mtm = MainThreadMarker::new().unwrap();
            NSColorPanel::sharedColorPanel(mtm).close();
        }
    }
);
thread_local! { static WATCH: RefCell<Option<Retained<Watch>>> = const { RefCell::new(None) }; }
/// Shows the panel over the editor window that asked; resolves when the person
/// closes it (or that window closes), with the colour if they changed it.
pub fn choose(
    initial: Option<u32>,
    parent: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>>,
) -> async_channel::Receiver<Option<String>> {
    let mtm = MainThreadMarker::new().unwrap();
    let (tx, rx) = async_channel::bounded(1);
    let panel = NSColorPanel::sharedColorPanel(mtm);
    if let Some(rgb) = initial {
        panel.setColor(&crate::dialogs::sheet_layout::native::colour(rgb));
    }
    let seeded = hex(&panel.color());
    let watch: Retained<Watch> = unsafe {
        msg_send![
            super(Watch::alloc(mtm).set_ivars(WatchState { tx, seeded })),
            init
        ]
    };
    let centre = NSNotificationCenter::defaultCenter();
    WATCH.with(|slot| {
        if let Some(old) = slot.borrow_mut().replace(watch.clone()) {
            unsafe { centre.removeObserver(old.as_ref() as &AnyObject) };
        }
    });
    unsafe {
        centre.addObserver_selector_name_object(
            watch.as_ref() as &AnyObject,
            sel!(closed:),
            Some(&NSString::from_str("NSWindowWillCloseNotification")),
            Some(panel.as_ref() as &AnyObject),
        );
        if let Some(parent) = &parent {
            centre.addObserver_selector_name_object(
                watch.as_ref() as &AnyObject,
                sel!(parentClosed:),
                Some(&NSString::from_str("NSWindowWillCloseNotification")),
                Some(parent.as_ref() as &AnyObject),
            );
        }
    }
    // The system panel floats over every app by default (P-CHILD).
    panel.setFloatingPanel(false);
    gpui_macos::child_window::apply(&panel);
    // The shared panel moves to whichever editor asked last.
    if let Some(parent) = &parent {
        gpui_macos::child_window::attach(&panel, parent);
    }
    gpui_macos::child_window::show(&panel, true);
    rx
}
/// The system loupe. `None` when the person cancels with Escape.
pub fn sample() -> async_channel::Receiver<Option<String>> {
    let (tx, rx) = async_channel::bounded(1);
    let sampler = NSColorSampler::new();
    let done = block2::RcBlock::new(move |colour: *mut NSColor| {
        // SAFETY: the sampler passes a nullable borrowed colour for this call.
        let colour = unsafe { Retained::retain(colour) };
        let _ = tx.try_send(colour.as_deref().and_then(hex));
    });
    unsafe { sampler.showSamplerWithSelectionHandler(&done) };
    rx
}
/// A colour well's click: the two native ways to choose, as a menu at the pointer.
/// `done` runs once, only if a colour was chosen.
pub fn menu(
    window: &mut gpui::Window,
    initial: Option<u32>,
    cx: &mut gpui::App,
    done: impl FnOnce(String, &mut gpui::App) + 'static,
) {
    let options = [
        serde_json::json!({"label":"Colors…"}),
        serde_json::json!({"label":"Sample from Screen"}),
    ];
    let parent = editor_window(window);
    crate::v4_native::menu(window, &options, cx, move |index, cx| {
        let chosen = match index {
            Some(0) => choose(initial, parent),
            Some(1) => sample(),
            _ => return,
        };
        cx.spawn(async move |cx| {
            if let Ok(Some(colour)) = chosen.recv().await {
                cx.update(|cx| done(colour, cx));
            }
        })
        .detach();
    });
}
/// The AppKit window behind a GPUI window.
fn editor_window(window: &gpui::Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let view = unsafe { &*(handle.ns_view.as_ptr() as *const objc2_app_kit::NSView) };
    view.window()
}
/// Installed families, for the font menu.
pub fn font_families() -> Vec<String> {
    let mtm = MainThreadMarker::new().unwrap();
    objc2_app_kit::NSFontManager::sharedFontManager(mtm)
        .availableFontFamilies()
        .iter()
        .map(|name| name.to_string())
        .collect()
}
