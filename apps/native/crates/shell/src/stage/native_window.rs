//! The Welcome in a real window. Once the wave has drawn the window on the
//! stage, a standard titled window takes its place on the same rectangle:
//! close and minimize buttons, zoom disabled, a fixed size. It holds the same
//! lit ground and step (`Stage::window_content`), with the step's AppKit
//! controls in it, and stands just above the stage while the stage's lights
//! go down. Minimizing it then takes the stage down with it; the stage comes
//! back under it when it returns. Once the stage has gone (`stage_left`) it
//! is an ordinary window: normal level, its own Space, behind other apps when
//! the person switches to them. It is shown once its ground has drawn, so the
//! hand-over from the stage's drawing of the window never shows a frame
//! without it.
use super::Stage;
use crate::welcome_view::WelcomeView;
use gpui::{prelude::*, *};
use objc2::{
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowDidDeminiaturizeNotification, NSWindowTitleVisibility,
    NSWindowWillMiniaturizeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
use std::{ptr::NonNull, rc::Rc, time::Duration};

/// Just above the stage (`NSStatusWindowLevel`, 25), while it stands.
const LEVEL: isize = 26;
/// `NSNormalWindowLevel`: once the stage has gone.
const NORMAL_LEVEL: isize = 0;
/// The hand-off: the window eases out and settles a few points
/// (`stage::WINDOW_OUT`).
const OUT_SECONDS: f64 = 0.32;
const OUT_SETTLE: f64 = 8.;

/// What the window's close button does: true closes the window.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

pub struct NativeWelcome {
    stage: Entity<Stage>,
    /// 0: hidden until the ground draws; 1: the ground drew, shown on the
    /// next frame; 2: shown.
    shown: u8,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl NativeWelcome {
    /// Stops following minimize with the stage.
    fn unfollow(&mut self) {
        for observer in self.observers.drain(..) {
            unsafe {
                NSNotificationCenter::defaultCenter()
                    .removeObserver(&*(Retained::as_ptr(&observer) as *const AnyObject));
            }
        }
    }
}
impl Drop for NativeWelcome {
    fn drop(&mut self) {
        self.unfollow();
    }
}

/// The stage has gone: the window stops following it (a deminimize would
/// otherwise order the removed stage back in) and becomes an ordinary window,
/// at the normal level on the current Space. It is not brought forward.
pub fn stage_left(handle: AnyWindowHandle, cx: &mut App) {
    let Some(handle) = handle.downcast::<NativeWelcome>() else {
        return;
    };
    let _ = handle.update(cx, |view, window, _| {
        view.unfollow();
        if let Some(ns) = ns_window(window) {
            ns.setLevel(NORMAL_LEVEL);
            ns.setCollectionBehavior(NSWindowCollectionBehavior::Default);
        }
    });
}

impl Render for NativeWelcome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (content, drawn) = self
            .stage
            .update(cx, |stage, cx| stage.window_content(window, cx));
        match self.shown {
            0 if drawn => {
                // Shown once this frame (the first with its ground) is up.
                self.shown = 1;
                window.request_animation_frame();
            }
            1 => {
                self.shown = 2;
                if let Some(ns) = ns_window(window) {
                    ns.setAlphaValue(1.);
                }
                self.stage.update(cx, |stage, cx| stage.native_shown(cx));
                // Its step moves from pending to current: one more frame.
                window.request_animation_frame();
            }
            _ => {}
        }
        content
    }
}

/// The AppKit window behind a GPUI window.
pub fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(handle)) =
        HasWindowHandle::window_handle(window).map(|h| h.as_raw())
    else {
        return None;
    };
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    view.window()
}

/// Opens the Welcome's real window on `bounds` (screen points) over the
/// stage window `stage_window`, holding `card` (made by `card` in the new
/// window, so its controls live there). It opens out of sight; `show`
/// orders it in (it stays clear until its ground draws).
pub fn open(
    stage: Entity<Stage>,
    stage_window: Option<Retained<NSWindow>>,
    bounds: Bounds<Pixels>,
    card: impl FnOnce(&mut Window, &mut App) -> Entity<WelcomeView> + 'static,
    on_close: OnClose,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<NativeWelcome>> {
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Welcome to Powermove".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            focus: false,
            show: false,
            kind: WindowKind::Normal,
            is_movable: true,
            // Not resizable: the zoom button is disabled, as the system
            // does for a window of fixed size.
            is_resizable: false,
            is_minimizable: true,
            ..Default::default()
        },
        move |window, cx| {
            let mut observers = vec![];
            if let Some(ns) = ns_window(window) {
                ns.setLevel(LEVEL);
                ns.setTitleVisibility(NSWindowTitleVisibility::Hidden);
                ns.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::FullScreenAuxiliary,
                );
                ns.setAlphaValue(0.);
                if let Some(stage_ns) = stage_window {
                    observers = follow_minimize(&ns, stage_ns);
                }
            }
            let card = card(window, cx);
            let handle = window.window_handle();
            stage.update(cx, |stage, cx| {
                stage.host_natively(handle, card.clone(), cx)
            });
            card.update(cx, |card, cx| card.hold_focus(window, cx));
            window.on_window_should_close(cx, move |window, cx| on_close(window, cx));
            cx.new(|cx| {
                // The stage holds the step and its change: this window
                // draws again whenever it changes.
                cx.observe(&stage, |_, _, cx| cx.notify()).detach();
                NativeWelcome {
                    stage,
                    shown: 0,
                    observers,
                }
            })
        },
    )
}

/// Orders the window in, key: clear until its ground draws.
pub fn show(handle: AnyWindowHandle, cx: &mut App) {
    let _ = handle.update(cx, |_, window, _| {
        if let Some(ns) = ns_window(window) {
            ns.makeKeyAndOrderFront(None);
        }
    });
}

/// Whether the window lies wholly off every display (a capture's).
pub fn off_every_display(handle: AnyWindowHandle, cx: &mut App) -> bool {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;
    handle
        .update(cx, |_, window, _| {
            let Some(ns) = ns_window(window) else {
                return false;
            };
            let frame = ns.frame();
            let Some(mtm) = MainThreadMarker::new() else {
                return false;
            };
            NSScreen::screens(mtm).iter().all(|screen| {
                let s = screen.frame();
                frame.origin.x + frame.size.width <= s.origin.x
                    || frame.origin.x >= s.origin.x + s.size.width
                    || frame.origin.y + frame.size.height <= s.origin.y
                    || frame.origin.y >= s.origin.y + s.size.height
            })
        })
        .unwrap_or(false)
}

/// The stage leaves with the window when it is minimized and comes back
/// under it when it returns.
fn follow_minimize(
    window: &NSWindow,
    stage: Retained<NSWindow>,
) -> Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>> {
    let center = NSNotificationCenter::defaultCenter();
    let hide = {
        let stage = stage.clone();
        block2::RcBlock::new(move |_: NonNull<NSNotification>| stage.orderOut(None))
    };
    let show = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
        // Its level keeps it under the window.
        stage.orderFront(None);
    });
    unsafe {
        vec![
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWindowWillMiniaturizeNotification),
                Some(window),
                None,
                &hide,
            ),
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWindowDidDeminiaturizeNotification),
                Some(window),
                None,
                &show,
            ),
        ]
    }
}

/// The hand-off: the window eases out, settling a few points, then closes.
pub fn ease_out(handle: AnyWindowHandle, cx: &mut App) {
    let _ = handle.update(cx, |_, window, _| {
        let Some(ns) = ns_window(window) else {
            return;
        };
        let mut frame = ns.frame();
        frame.origin.y -= OUT_SETTLE;
        NSAnimationContext::runAnimationGroup(&block2::RcBlock::new(
            move |context: NonNull<NSAnimationContext>| {
                let context = unsafe { context.as_ref() };
                context.setDuration(OUT_SECONDS);
                let animator = ns.animator();
                animator.setAlphaValue(0.);
                animator.setFrame_display(frame, true);
            },
        ));
    });
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_secs_f64(OUT_SECONDS + 0.04))
            .await;
        cx.update(|cx| {
            if handle.update(cx, |_, window, _| window.remove_window()).is_err() {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                });
            }
        });
    })
    .detach();
}
