use super::*;
use gpui::{AppContext, HeadlessAppContext, KeyDownEvent, Keystroke, size};
use std::sync::Arc;

#[::core::prelude::v1::test]
fn dock_tabs_instant_drag_frames() {
    let Some(dir) = std::env::var_os("PM_DOCK_TABS_CAPTURE") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (scheme, theme) in [
        (pm_design::Scheme::Light, "light"),
        (pm_design::Scheme::Dark, "dark"),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(gpui_parley::ParleyTextSystem::new(
                gpui_parley::SystemFonts::Load,
            )),
            Arc::new(()),
            || {
                Some(Box::new(
                    gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                ))
            },
        );
        let window = cx
            .open_window(size(px(1100.), px(700.)), |_, cx| {
                cx.new(|cx| Editor::new(Tokens::new(scheme, Default::default()), None, cx))
            })
            .unwrap();
        cx.run_until_parked();
        let snapshot = |cx: &mut HeadlessAppContext, name: &str| {
            cx.run_until_parked();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(dir.join(format!("{theme}-instant-{name}.png")))
                .unwrap();
        };
        let (from, upper, lower) = window
            .update(&mut cx, |editor, _, _| {
                let tab = editor.tab_rect("inspector").unwrap();
                let left = editor.panel_rect("assets").unwrap();
                (
                    point(px(tab.x + tab.w / 2.), px(tab.y + tab.h / 2.)),
                    point(px(left.x + left.w / 2.), px(left.y + left.h * 0.13)),
                    point(px(left.x + left.w / 2.), px(left.y + left.h * 0.89)),
                )
            })
            .unwrap();
        let down = |cx: &mut HeadlessAppContext, position| {
            cx.simulate_event(
                window.into(),
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    ..Default::default()
                },
            );
        };
        let drag = |cx: &mut HeadlessAppContext, position| {
            cx.simulate_event(
                window.into(),
                MouseMoveEvent {
                    position,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                },
            );
        };
        let up = |cx: &mut HeadlessAppContext, position| {
            cx.simulate_event(
                window.into(),
                MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    ..Default::default()
                },
            );
        };
        let target = |cx: &mut HeadlessAppContext| {
            window
                .update(cx, |editor, _, _| match &editor.gesture {
                    Some(Gesture::Panel(drag)) => drag.preview.map(|preview| preview.target),
                    _ => None,
                })
                .unwrap()
        };
        crate::drag::set_frame_time(Some(std::time::Instant::now()));
        snapshot(&mut cx, "0-rest");
        let before = window.update(&mut cx, |v, _, _| v.layout.clone()).unwrap();
        down(&mut cx, from);
        drag(&mut cx, upper);
        snapshot(&mut cx, "1-pickup");
        let first = target(&mut cx);
        assert!(first.is_some(), "pickup uses its destination");
        drag(&mut cx, lower);
        snapshot(&mut cx, "2-target");
        assert_ne!(
            target(&mut cx),
            first,
            "destination moved in the input frame"
        );
        up(&mut cx, lower);
        snapshot(&mut cx, "3-drop");
        window
            .update(&mut cx, |v, _, _| {
                assert_ne!(v.layout, before, "drop commits on release");
                assert!(!v.dragging_panel() && !v.tab_slides.travelling());
                assert!(v.slides.moving.is_empty());
            })
            .unwrap();

        window
            .update(&mut cx, |v, _, cx| {
                v.layout = before.clone();
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        down(&mut cx, from);
        drag(&mut cx, lower);
        snapshot(&mut cx, "4-before-cancel");
        cx.simulate_event(
            window.into(),
            KeyDownEvent {
                keystroke: Keystroke::parse("escape").unwrap(),
                is_held: false,
                prefer_character_input: false,
            },
        );
        snapshot(&mut cx, "5-cancel");
        window
            .update(&mut cx, |v, _, _| {
                assert_eq!(v.layout, before);
                assert!(!v.dragging_panel());
            })
            .unwrap();
        crate::drag::set_frame_time(None);
    }
}
