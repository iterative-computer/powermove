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
            let colour = NSColorPanel::sharedColorPanel(mtm).color();
            let _ = self.ivars().tx.try_send(hex(&colour));
        }
    }
);
thread_local! { static WATCH: RefCell<Option<Retained<Watch>>> = const { RefCell::new(None) }; }
/// Shows the panel; resolves with its colour when the person closes it.
pub fn choose(initial: Option<u32>) -> async_channel::Receiver<Option<String>> {
    let mtm = MainThreadMarker::new().unwrap();
    let (tx, rx) = async_channel::bounded(1);
    let panel = NSColorPanel::sharedColorPanel(mtm);
    if let Some(rgb) = initial {
        panel.setColor(&crate::dialogs::sheet_layout::native::colour(rgb));
    }
    let watch: Retained<Watch> =
        unsafe { msg_send![super(Watch::alloc(mtm).set_ivars(WatchState { tx })), init] };
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
    }
    panel.makeKeyAndOrderFront(None);
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
/// Installed families, for the font menu.
pub fn font_families() -> Vec<String> {
    let mtm = MainThreadMarker::new().unwrap();
    objc2_app_kit::NSFontManager::sharedFontManager(mtm)
        .availableFontFamilies()
        .iter()
        .map(|name| name.to_string())
        .collect()
}
