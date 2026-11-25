//! The first run on the stage (`pm_shell::stage`): the film, then the
//! first-run steps on its card, then the hand-off into the app. Also the
//! film replayed from Help › Welcome Film and `--replay-onboarding`.
use gpui::{AppContext, Entity, WindowHandle};
use pm_shell::{
    stage::{CueSheet, Stage},
    welcome::{Button, Frame},
    welcome_view::{OnAction, WelcomeView},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

/// What the film's engine needs from the app: its fonts and its folder of
/// translated shaders. Set once at startup.
#[derive(Clone)]
pub struct FilmInputs {
    pub fonts: pm_render::text::host::RenderFonts,
    pub shader_cache: Option<PathBuf>,
}
thread_local! {
    static INPUTS: RefCell<Option<FilmInputs>> = const { RefCell::new(None) };
}
pub fn set_inputs(inputs: FilmInputs) {
    INPUTS.with(|i| *i.borrow_mut() = Some(inputs));
}
fn inputs() -> Option<FilmInputs> {
    INPUTS.with(|i| i.borrow().clone())
}

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
    /// The last step: the wave carries the card away and the room's lights
    /// come up on the app's first window, opened behind the stage.
    pub fn hand_off(&self, cx: &mut gpui::App) {
        let _ = self.window.update(cx, |stage, _, cx| {
            stage.hand_off(None, |window, _| window.remove_window(), cx);
        });
    }
}

/// Opens the stage over the display with `frame` on its card; `film`
/// plays the film first (the first launch's Welcome).
pub fn open(
    frame: Frame,
    on_action: OnAction,
    film: bool,
    on_sound: pm_shell::stage::OnSound,
    cx: &mut gpui::App,
) -> anyhow::Result<StageWindow> {
    // The stage is a screening room: its own dark scheme whatever the
    // system's, with the person's accessibility settings.
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, pm_shell::tokens().accessibility);
    let inputs = inputs().ok_or_else(|| anyhow::anyhow!("the stage has no engine inputs"))?;
    let card = Rc::new(RefCell::new(None::<Entity<WelcomeView>>));
    let made = card.clone();
    let action = on_action.clone();
    let window = pm_shell::stage::open_stage(
        move |window, cx| {
            let room = window.bounds().size;
            let pictures = crate::demo_editor::SharedPictures(Rc::new(RefCell::new(
                crate::stage_pictures::EnginePictures::start(
                    inputs.fonts,
                    inputs.shader_cache,
                    (f64::from(room.width), f64::from(room.height)),
                    crate::stage_pictures::film(),
                ),
            )));
            let demo: Option<Box<dyn pm_shell::stage::Demo>> = film.then(|| {
                Box::new(crate::demo_editor::FilmDemo::start(
                    crate::stage_pictures::film(),
                    tokens.clone(),
                    pictures.clone(),
                )) as Box<dyn pm_shell::stage::Demo>
            });
            let mut stage = Stage::new(
                tokens.clone(),
                CueSheet::default(),
                Box::new(pictures),
                demo,
                cx,
            );
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

/// The film again (Help › Welcome Film, `--replay-onboarding`): the film,
/// then a card whose one action closes the stage. Nothing is set up again.
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
        line: pm_shell::stage::beats::CLOSING.into(),
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
        Err(error) => eprintln!("The Welcome film could not open: {error:#}"),
    }
}
