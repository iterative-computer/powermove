#![cfg(target_os = "macos")]
//! The Welcome window under real input events, through GPUI's test window.
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    welcome::{Button, Choice, Frame, Row, Severity},
    welcome_view::WelcomeView,
};
use std::{cell::RefCell, rc::Rc};

fn tokens() -> Tokens {
    Tokens::new(Scheme::Light, Accessibility::default())
}
fn frame() -> Frame {
    Frame {
        surface: "S108".into(),
        title: "Pick Your Timeline.".into(),
        line: "Work the way you already do.".into(),
        cards: true,
        back: Some("Back to Welcome".into()),
        chosen: Some("layers".into()),
        choices: ["layers", "tracks"]
            .map(|id| Choice {
                id: id.into(),
                label: id.into(),
                detail: None,
                lines: vec!["1  Title".into()],
            })
            .into(),
        buttons: vec![
            Button::new("quit", "Quit").leading(),
            Button::new("continue", "Continue"),
        ],
        default: Some("continue".into()),
        ..Frame::default()
    }
}
type Log = Rc<RefCell<Vec<String>>>;
fn open(
    cx: &mut TestAppContext,
    frame: Frame,
) -> (gpui::Entity<WelcomeView>, &mut VisualTestContext, Log) {
    let log: Log = Default::default();
    let sink = log.clone();
    let (view, cx) = cx.add_window_view(move |_, cx| {
        let mut view = WelcomeView::new(
            tokens(),
            frame,
            Rc::new(move |id, _| sink.borrow_mut().push(id.to_owned())),
            cx,
        );
        view.clock = Some(1.);
        view
    });
    cx.update(|window, cx| window.focus(&view.read(cx).focus_handle(), cx));
    cx.run_until_parked();
    (view, cx, log)
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

#[test]
fn return_and_a_click_reach_the_default_action() {
    let mut cx = TestAppContext::single();
    let (_, cx, log) = open(&mut cx, frame());
    cx.simulate_keystrokes("enter");
    click(cx, "welcome-continue");
    assert_eq!(*log.borrow(), ["continue", "continue"]);
}
#[test]
fn a_held_return_acts_once() {
    let mut cx = TestAppContext::single();
    let (_, cx, log) = open(&mut cx, frame());
    let held = gpui::KeyDownEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
        is_held: true,
        prefer_character_input: false,
    };
    cx.simulate_keystrokes("enter");
    cx.simulate_event(held);
    assert_eq!(*log.borrow(), ["continue"]);
}
#[test]
fn the_animation_starts_from_its_top_when_it_comes_into_view() {
    let mut cx = TestAppContext::single();
    let (view, cx, _) = open(&mut cx, frame());
    let time = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.time());
    view.update(cx, |view, _| view.clock = Some(12.));
    assert_eq!(time(cx), 12.);
    let mut animated = frame();
    animated.animate = true;
    view.update(cx, |view, cx| view.set_frame(animated.clone(), cx));
    assert_eq!(time(cx), 0.);
    view.update(cx, |view, cx| {
        view.clock = Some(14.5);
        view.set_frame(animated, cx);
    });
    assert_eq!(time(cx), 2.5, "an animation already in view keeps playing");
}
#[test]
fn the_leading_button_and_back_are_separate_actions() {
    let mut cx = TestAppContext::single();
    let (_, cx, log) = open(&mut cx, frame());
    click(cx, "welcome-quit");
    click(cx, "welcome-back");
    assert_eq!(*log.borrow(), ["quit", "back"]);
}
#[test]
fn arrows_and_clicks_choose_and_a_disabled_default_ignores_return() {
    let mut cx = TestAppContext::single();
    let mut waiting = frame();
    waiting.buttons[1].enabled = false;
    let (_, cx, log) = open(&mut cx, waiting);
    cx.simulate_keystrokes("right");
    click(cx, "choice-tracks");
    cx.simulate_keystrokes("enter");
    click(cx, "welcome-continue");
    assert_eq!(*log.borrow(), ["choose:tracks", "choose:tracks"]);
}
#[test]
fn a_new_frame_replaces_the_old_one_and_details_stay_closed_until_asked() {
    let mut cx = TestAppContext::single();
    let (view, cx, log) = open(&mut cx, frame());
    let next = Frame {
        surface: "N001".into(),
        title: "Copying Stopped".into(),
        line: "Your original app and library weren’t changed.".into(),
        details: Some("synthetic failure".into()),
        rows: vec![Row {
            symbol: "info.circle",
            severity: Severity::Note,
            text: "1 media file was already offline.".into(),
        }],
        buttons: vec![Button::new("try_again", "Try Again")],
        default: Some("try_again".into()),
        ..Frame::default()
    };
    view.update(cx, |view, cx| view.set_frame(next, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("welcome-continue").is_none());
    click(cx, "welcome-details");
    cx.simulate_keystrokes("enter");
    assert_eq!(*log.borrow(), ["try_again"]);
}
#[test]
fn the_animation_never_blocks_the_buttons_and_reduce_motion_skips_it() {
    let mut cx = TestAppContext::single();
    let mut animated = frame();
    animated.animate = true;
    let (view, cx, log) = open(&mut cx, animated);
    click(cx, "welcome-continue");
    assert_eq!(*log.borrow(), ["continue"]);
    assert!(cx.debug_bounds("welcome-mark").is_some());
    view.update(cx, |view, cx| {
        view.tokens.accessibility.reduce_motion = true;
        cx.notify();
    });
    for clock in [1., 5.] {
        view.update(cx, |view, cx| {
            view.clock = Some(clock);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("welcome-mark").is_none(), "at {clock}s");
        assert!(cx.debug_bounds("welcome-title").is_some());
    }
    click(cx, "welcome-continue");
    assert_eq!(*log.borrow(), ["continue", "continue"]);
}
