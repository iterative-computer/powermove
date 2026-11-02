//! Real AppKit regression for a single GPUI window's native tab-bar accessory.
//! Run only in an unlocked, GUI-lock-owned session (`--verify-tab-bar`).
use anyhow::{Result, ensure};
use gpui::{prelude::*, *};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{NSToolbar, NSView, NSWindow, NSWindowToolbarStyle};
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, rc::Rc, time::Duration};

struct Probe;
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

pub fn run() -> Result<()> {
    let passed = Rc::new(Cell::new(false));
    let result = passed.clone();
    let platform = Rc::new(gpui_macos::MacPlatform::new(false));
    Application::with_platform(platform).run(move |cx| {
        let mut native = None;
        cx.open_window(
            WindowOptions::new().tabbing_identifier(Some("pm-p120-probe".into())),
            |window, cx| {
                let RawWindowHandle::AppKit(handle) = window.window_handle().unwrap().as_raw() else {
                    unreachable!()
                };
                let view: Retained<NSView> = unsafe {
                    Retained::retain(handle.ns_view.as_ptr().cast()).unwrap()
                };
                native = view.window();
                cx.new(|_| Probe)
            },
        ).unwrap();
        let native: Retained<NSWindow> = native.unwrap();
        let mtm = objc2::MainThreadMarker::new().unwrap();
        let toolbar = NSToolbar::initWithIdentifier(
            NSToolbar::alloc(mtm), &NSString::from_str("pm-p120-native-tabs"),
        );
        native.setToolbar(Some(&toolbar));
        native.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
        native.setTitlebarAppearsTransparent(true);
        cx.activate(true);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let shown = cx.update(|_| {
                native.toggleTabBar(None);
                native.tabGroup().is_some_and(|group| group.isTabBarVisible())
            });
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let accessory_visible = cx.update(|_| {
                native.titlebarAccessoryViewControllers().iter().any(|controller| {
                    let view = controller.view();
                    !view.isHidden() && view.frame().size.height > 0.
                })
            });
            let hidden = cx.update(|_| {
                native.toggleTabBar(None);
                native.tabGroup().is_some_and(|group| !group.isTabBarVisible())
            });
            // Let AppKit lay out the final state before closing the owned window.
            cx.background_executor().timer(Duration::from_millis(100)).await;
            cx.update(|cx| {
                println!("P120_TAB_BAR shown={shown} accessory_visible={accessory_visible} hidden={hidden}");
                result.set(shown && accessory_visible && hidden);
                cx.quit();
            });
        }).detach();
    });
    ensure!(passed.get(), "single-window native tab-bar regression");
    Ok(())
}
