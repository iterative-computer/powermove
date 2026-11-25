//! The first run on the stage (`pm_shell::stage`): the intro, then the
//! first-run steps in the Welcome window it draws, then the hand-off into
//! the app. Also the intro replayed from Help and `--replay-onboarding`.
use gpui::{AppContext, Entity, WindowHandle};
use pm_shell::{
    stage::{CueSheet, Stage},
    welcome::{Button, Frame},
    welcome_view::{OnAction, WelcomeView},
};
use std::{cell::RefCell, rc::Rc};

/// The stage and the card on it.
#[derive(Clone)]
pub struct StageWindow {
    pub window: WindowHandle<Stage>,
    card: Rc<RefCell<Entity<WelcomeView>>>,
    on_action: OnAction,
}
impl StageWindow {
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
        let _ = self.window.update(cx, |stage, window, cx| {
            stage.set_card(next.clone(), cx);
            next.update(cx, |c, cx| c.hold_focus(window, cx));
        });
    }
    /// The last step: the window eases out and the room's lights come up
    /// on the app's first window, opened behind the stage a moment before.
    pub fn hand_off(&self, cx: &mut gpui::App) {
        let stage = self.window;
        cx.spawn(async move |cx| {
            // A few frames for the window to draw behind the stage.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            cx.update(|cx| {
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
    Ok(StageWindow {
        window,
        card: Rc::new(RefCell::new(card)),
        on_action,
    })
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
        Rc::new(crate::first_run::play_sound),
        cx,
    ) {
        Ok(stage) => *handle.borrow_mut() = Some(stage),
        Err(error) => eprintln!("The Welcome intro could not open: {error:#}"),
    }
}
