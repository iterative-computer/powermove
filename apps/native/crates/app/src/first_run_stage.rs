//! The first run on the stage (`pm_shell::stage`): the intro, then the
//! first-run steps in the Welcome window (drawn by the wave, then a real
//! window on the same rectangle), then the hand-off into the app. Also the
//! intro replayed from Help and `--replay-onboarding`.
use gpui::{AnyWindowHandle, AppContext, Bounds, Entity, WindowHandle};
use pm_shell::{
    stage::{
        CueSheet, Stage, StageEvent,
        native_window::{self, OnClose},
    },
    welcome::{Button, Frame},
    welcome_view::{OnAction, WelcomeView},
};
use std::{cell::RefCell, rc::Rc};

/// The stage, the card on it, and the real Welcome window once it is live.
#[derive(Clone)]
pub struct StageWindow {
    pub window: WindowHandle<Stage>,
    card: Rc<RefCell<Entity<WelcomeView>>>,
    on_action: OnAction,
    native: Rc<RefCell<Option<AnyWindowHandle>>>,
    /// The window's close button quits (the first run); else it hands off
    /// (the intro replayed).
    quits: bool,
}
impl StageWindow {
    /// The window the person works in: the real Welcome window once live.
    pub fn activate(&self, cx: &mut gpui::App) {
        let native = *self.native.borrow();
        let _ = match native {
            Some(native) => native.update(cx, |_, window, _| window.activate()),
            None => self.window.update(cx, |_, window, _| window.activate()),
        };
    }
    /// The wave has drawn the window: the Welcome moves into a real window
    /// on the same rectangle, with a card of its own.
    fn go_native(&self, stage: Entity<Stage>, cx: &mut gpui::App) {
        let Ok((bounds, stage_ns)) = self.window.update(cx, |s, window, _| {
            let screen = window.bounds();
            let rect = s.welcome_rect(screen.size);
            (
                Bounds::new(screen.origin + rect.origin, rect.size),
                native_window::ns_window(window),
            )
        }) else {
            return;
        };
        let (frame, tokens) = {
            let card = self.card.borrow();
            let card = card.read(cx);
            (card.frame.clone(), card.tokens.clone())
        };
        let on_action = self.on_action.clone();
        let made = self.card.clone();
        let this = self.clone();
        let on_close: OnClose = Rc::new(move |_, cx| {
            if this.quits {
                cx.quit();
                true
            } else {
                this.hand_off(cx);
                false
            }
        });
        let card = move |_: &mut gpui::Window, cx: &mut gpui::App| {
            let card = cx.new(|cx| {
                let mut c = WelcomeView::new(tokens, frame, on_action, cx);
                c.stage = true;
                c
            });
            *made.borrow_mut() = card.clone();
            card
        };
        match native_window::open(stage, stage_ns, bounds, card, on_close, cx) {
            Ok(handle) => {
                native_window::show(handle.into(), cx);
                *self.native.borrow_mut() = Some(handle.into());
            }
            // The stage keeps drawing the window.
            Err(error) => eprintln!("The Welcome window could not open: {error:#}"),
        }
    }
    /// The card shows `frame`; a new step (another surface) arrives with
    /// the wave from the card before.
    pub fn set_frame(&self, frame: Frame, cx: &mut gpui::App) {
        let card = self.card.borrow().clone();
        if card.read(cx).frame.surface == frame.surface {
            card.update(cx, |c, cx| c.set_frame(frame, cx));
            return;
        }
        let tokens = card.read(cx).tokens.clone();
        let on_action = self.on_action.clone();
        let next = cx.new(|cx| {
            let mut c = WelcomeView::new(tokens, frame, on_action, cx);
            c.stage = true;
            c
        });
        *self.card.borrow_mut() = next.clone();
        let native = *self.native.borrow();
        let _ = self.window.update(cx, |stage, window, cx| {
            stage.set_card(next.clone(), cx);
            if native.is_none() {
                next.update(cx, |c, cx| c.hold_focus(window, cx));
            }
        });
        if let Some(native) = native {
            let _ = native.update(cx, |_, window, cx| {
                next.update(cx, |c, cx| c.hold_focus(window, cx));
            });
        }
    }
    /// The last step: the window eases out and the room's lights come up
    /// on the app's first window, opened behind the stage a moment before.
    pub fn hand_off(&self, cx: &mut gpui::App) {
        let stage = self.window;
        let native = self.native.borrow_mut().take();
        cx.spawn(async move |cx| {
            // A few frames for the window to draw behind the stage.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            cx.update(|cx| {
                if let Some(native) = native {
                    native_window::ease_out(native, cx);
                }
                let _ = stage.update(cx, |stage, _, cx| {
                    stage.hand_off(|window, _| window.remove_window(), cx);
                });
            });
        })
        .detach();
    }
}

/// Opens the stage over the display with `frame` in its window; `intro`
/// plays the intro first (the first launch's Welcome).
pub fn open(
    frame: Frame,
    on_action: OnAction,
    intro: bool,
    quits: bool,
    on_sound: pm_shell::stage::OnSound,
    cx: &mut gpui::App,
) -> anyhow::Result<StageWindow> {
    // The stage is a screening room: its own dark scheme whatever the
    // system's, with the person's accessibility settings.
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, pm_shell::tokens().accessibility);
    let card = Rc::new(RefCell::new(None::<Entity<WelcomeView>>));
    let made = card.clone();
    let action = on_action.clone();
    let window = pm_shell::stage::open_stage(
        move |_, cx| {
            let mut stage = Stage::new(tokens.clone(), CueSheet::default(), intro, cx);
            stage.on_sound(on_sound);
            let c = cx.new(|cx| {
                let mut c = WelcomeView::new(tokens, frame, action, cx);
                c.stage = true;
                c
            });
            stage.set_card(c.clone(), cx);
            *made.borrow_mut() = Some(c);
            stage
        },
        cx,
    )?;
    let card = card
        .borrow_mut()
        .take()
        .ok_or_else(|| anyhow::anyhow!("the stage opened without its card"))?;
    let stage = window.root(cx)?;
    let staged = StageWindow {
        window,
        card: Rc::new(RefCell::new(card)),
        on_action,
        native: Rc::default(),
        quits,
    };
    let live = staged.clone();
    cx.subscribe(&stage, move |stage, event: &StageEvent, cx| match event {
        StageEvent::WindowLive => live.go_native(stage, cx),
    })
    .detach();
    Ok(staged)
}

/// The intro again (Help › Replay Intro, `--replay-onboarding`):
/// the intro, then a window whose one action closes the stage. Nothing is
/// set up again.
pub fn replay(cx: &mut gpui::App) {
    let handle = Rc::new(RefCell::new(None::<StageWindow>));
    let close = handle.clone();
    let on_action: OnAction = Rc::new(move |id, cx| {
        if id == "done"
            && let Some(stage) = close.borrow().clone()
        {
            stage.hand_off(cx);
        }
    });
    let frame = Frame {
        surface: "S107-replay".into(),
        // The Welcome's look: the lit mark in the window, its title.
        animate: true,
        title: "Welcome to\nPowermove.".into(),
        centred: true,
        buttons: vec![Button::new("done", "Done")],
        default: Some("done".into()),
        ..Frame::default()
    };
    match open(
        frame,
        on_action,
        true,
        false,
        Rc::new(crate::first_run::play_sound),
        cx,
    ) {
        Ok(stage) => *handle.borrow_mut() = Some(stage),
        Err(error) => eprintln!("The Welcome intro could not open: {error:#}"),
    }
}
