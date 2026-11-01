use crate::{
    EditEvent, NumField, PointField, RampField, RampValue, SelectField, SelectOption, TextField,
    ToggleField, input::TextInput,
};
use gpui::{Focusable, Modifiers, MouseButton, TestAppContext, point, px};
use std::{cell::RefCell, rc::Rc};
#[gpui::test]
fn numeric_drag_commits_once_and_escape_rolls_back(cx: &mut TestAppContext) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    let (field, cx) = cx.add_window_view(|w, cx| {
        NumField::new(42., w, cx).on_edit(move |e, _| log.borrow_mut().push(e.clone()))
    });
    cx.simulate_mouse_down(
        point(px(20.), px(12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(24.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(24.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 45.);
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|e| matches!(e, EditEvent::Commit(_)))
            .count(),
        1
    );
    cx.simulate_mouse_down(
        point(px(20.), px(12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(40.), px(12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 45.);
    assert!(matches!(
        events.borrow().last(),
        Some(EditEvent::Cancel(45.))
    ));
}
#[gpui::test]
fn numeric_typed_arithmetic_unclamped_and_unchanged_negative(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    let (field, cx) = cx.add_window_view(|w, cx| {
        NumField::new(-2., w, cx)
            .range(0., 10.)
            .on_edit(move |e, _| log.borrow_mut().push(e.clone()))
    });
    cx.update(|w, cx| field.read(cx).focus_handle(cx).focus(w, cx));
    cx.simulate_keystrokes("f2 enter");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), -2.);
    assert!(events.borrow().is_empty());
    cx.simulate_click(point(px(10.), px(12.)), Modifiers::default());
    cx.simulate_input("20 * 3");
    cx.simulate_keystrokes("enter");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 60.);
    cx.simulate_click(point(px(10.), px(12.)), Modifiers::default());
    cx.simulate_input("-=3");
    cx.simulate_keystrokes("enter");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 57.);
}
#[gpui::test]
fn toggle_mixed_keyboard_and_disabled(cx: &mut TestAppContext) {
    let (field, cx) = cx.add_window_view(|w, cx| ToggleField::new(false, w, cx).mixed(true));
    cx.update(|w, cx| field.read(cx).focus_handle(cx).focus(w, cx));
    cx.simulate_keystrokes("space");
    assert!(field.read_with(cx, |f, _| *f.value()));
    cx.simulate_keystrokes("left");
    assert!(!field.read_with(cx, |f, _| *f.value()));
    field.update(cx, |f, cx| {
        f.state.disabled = true;
        cx.notify();
    });
    cx.simulate_keystrokes("right");
    assert!(!field.read_with(cx, |f, _| *f.value()));
}
#[gpui::test]
fn text_live_escape_and_commit(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (field, cx) = cx.add_window_view(|w, cx| TextField::new("Original", w, cx));
    cx.update(|w, cx| field.read(cx).focus_handle(cx).focus(w, cx));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Draft");
    assert_eq!(field.read_with(cx, |f, _| f.value().clone()), "Draft");
    cx.simulate_keystrokes("escape");
    assert_eq!(field.read_with(cx, |f, _| f.value().clone()), "Original");
    cx.update(|w, cx| field.read(cx).focus_handle(cx).focus(w, cx));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Final");
    cx.simulate_keystrokes("enter");
    assert_eq!(field.read_with(cx, |f, _| f.value().clone()), "Final");
}
#[gpui::test]
fn select_preserves_typed_value_and_unknown_readout(cx: &mut TestAppContext) {
    let (field, cx) = cx.add_window_view(|w, cx| {
        SelectField::new(
            999,
            vec![
                SelectOption::new(24, "24 fps"),
                SelectOption::new(30, "30 fps"),
            ],
            w,
            cx,
        )
    });
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 999);
    cx.update(|w, cx| field.read(cx).focus_handle(cx).focus(w, cx));
    cx.simulate_keystrokes("space down enter");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 30);
}
#[gpui::test]
fn point_keyboard_independent_normalized_steps(cx: &mut TestAppContext) {
    let (field, cx) = cx.add_window_view(|w, cx| {
        PointField::new([0.1, 0.2], w, cx)
            .range([0.; 2], [1.; 2])
            .steps([Some(0.1), Some(0.2)])
    });
    cx.update(|w, cx| field.read(cx).focus_handle(cx).focus(w, cx));
    cx.simulate_keystrokes("right down");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), [0.2, 0.4]);
    cx.simulate_keystrokes("home");
    assert_eq!(field.read_with(cx, |f, _| *f.value()), [0.; 2]);
}
#[gpui::test]
fn transaction_refusal_leaves_number_unchanged(cx: &mut TestAppContext) {
    let (field, cx) = cx.add_window_view(|w, cx| NumField::new(12., w, cx).on_begin(|_| false));
    cx.simulate_mouse_down(
        point(px(20.), px(12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(50.), px(12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(50.), px(12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    assert_eq!(field.read_with(cx, |f, _| *f.value()), 12.);
}
#[gpui::test]
fn ramp_renders_and_cancel_restores_stops(cx: &mut TestAppContext) {
    let (field, cx) = cx.add_window_view(|w, cx| RampField::new(RampValue::default(), w, cx));
    let before = field.read_with(cx, |f, _| f.value().clone());
    cx.simulate_mouse_down(
        point(px(2.), px(30.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(100.), px(30.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(field.read_with(cx, |f, _| f.value().clone()), before);
}
#[gpui::test]
fn input_ime_utf16_surrogate_range(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (input, cx) = cx.add_window_view(|w, cx| TextInput::new("a😀b", "IME", w, cx));
    cx.update(|w, cx| {
        input.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(Some(1..3), "かな", Some(1..2), w, cx);
            assert_eq!(input.value(), "aかなb");
            assert_eq!(input.selected_text_range(false, w, cx).unwrap().range, 2..3);
            input.replace_text_in_range(None, "仮名", w, cx);
            assert_eq!(input.value(), "a仮名b");
        })
    });
}

#[gpui::test]
fn color_preview_cancel_and_outside_commit(cx: &mut TestAppContext) {
    use crate::{ColorField, color::Color};
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    let (field, cx) = cx.add_window_view(|w, cx| {
        ColorField::new(Color::new(1., 0.5, 0., 1.), w, cx)
            .on_edit(move |e, _| log.borrow_mut().push(e.clone()))
    });
    cx.update(|w, cx| field.update(cx, |f, cx| f.open_picker(w, cx)));
    let pad = cx
        .debug_bounds("color-pad")
        .expect("color pad rendered above panel");
    cx.simulate_mouse_down(pad.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        pad.origin + gpui::point(px(20.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(pad.center(), MouseButton::Left, Modifiers::default());
    assert_ne!(
        field.read_with(cx, |f, _| *f.value()),
        Color::new(1., 0.5, 0., 1.)
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(
        field.read_with(cx, |f, _| *f.value()),
        Color::new(1., 0.5, 0., 1.)
    );
    assert!(matches!(events.borrow().last(), Some(EditEvent::Cancel(_))));
    cx.update(|w, cx| field.update(cx, |f, cx| f.open_picker(w, cx)));
    let pad = cx.debug_bounds("color-pad").unwrap();
    cx.simulate_click(pad.center(), Modifiers::default());
    cx.simulate_click(point(px(4.), px(500.)), Modifiers::default());
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|e| matches!(e, EditEvent::Commit(_)))
            .count(),
        1
    );
}
#[gpui::test]
fn fill_nested_picker_closes_without_reentrant_update(cx: &mut TestAppContext) {
    use crate::{FillField, FillValue};
    let original = FillValue::default();
    let (field, cx) = cx.add_window_view(|w, cx| FillField::new(original.clone(), w, cx));
    cx.update(|w, cx| field.update(cx, |f, cx| f.open_picker(w, cx)));
    let pad = cx.debug_bounds("color-pad").unwrap();
    cx.simulate_click(pad.center(), Modifiers::default());
    cx.simulate_keystrokes("escape");
    assert_eq!(
        field.read_with(cx, |f, _| f.value().clone()),
        original.normalized(crate::color::Color::default())
    );
    cx.update(|w, cx| field.update(cx, |f, cx| f.open_picker(w, cx)));
    let pad = cx.debug_bounds("color-pad").unwrap();
    cx.simulate_click(pad.center(), Modifiers::default());
    cx.simulate_click(point(px(4.), px(700.)), Modifiers::default());
    assert_ne!(field.read_with(cx, |f, _| f.value().clone()), original);
}
