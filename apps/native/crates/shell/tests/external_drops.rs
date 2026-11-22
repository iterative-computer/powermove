//! Named AppKit pasteboards and completed-drop routing, with no window shown.
#[cfg(target_os = "macos")]
fn main() {
    for (name, case) in gpui_macos::drop_source_tests::CASES {
        case();
        println!("ok {name}");
    }
    resolved_drop_routes_to_release_hitbox_without_synthesizing_mouse_up();
    println!("ok resolved_drop_routes_to_release_hitbox_without_synthesizing_mouse_up");
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn resolved_drop_routes_to_release_hitbox_without_synthesizing_mouse_up() {
    use gpui::{
        AppContext, Context, ExternalPaths, FileDropEvent, InteractiveElement, IntoElement,
        Modifiers, MouseButton, ParentElement, PlatformInput, Render, Styled, TestAppContext,
        Window, div, point, px,
    };
    use std::{
        cell::{Cell, RefCell},
        path::PathBuf,
        rc::Rc,
    };

    #[derive(Default)]
    struct Recorded {
        drops: Vec<(&'static str, bool, Vec<PathBuf>)>,
        mouse_ups: usize,
        pending: Vec<usize>,
    }

    struct Targets(Rc<RefCell<Recorded>>);
    impl Render for Targets {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let left = Rc::clone(&self.0);
            let right = Rc::clone(&self.0);
            let observed = Rc::clone(&self.0);
            div()
                .flex()
                .size_full()
                .on_mouse_up(MouseButton::Left, move |_, _, _| {
                    observed.borrow_mut().mouse_ups += 1
                })
                .child(div().w(px(100.)).h(px(100.)).on_drop(
                    move |paths: &ExternalPaths, window, _| {
                        left.borrow_mut().drops.push((
                            "left",
                            window.modifiers().alt,
                            paths.paths().to_vec(),
                        ));
                    },
                ))
                .child(div().w(px(100.)).h(px(100.)).on_drop(
                    move |paths: &ExternalPaths, window, _| {
                        right.borrow_mut().drops.push((
                            "right",
                            window.modifiers().alt,
                            paths.paths().to_vec(),
                        ));
                    },
                ))
        }
    }

    let observed = Rc::new(RefCell::new(Recorded::default()));
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|_, _| Targets(Rc::clone(&observed)));
    let path = PathBuf::from("/received/image.png");
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        let project = Rc::new(Cell::new(1));
        window.set_external_drop_target({
            let project = project.clone();
            move |_| Some(project.get())
        });
        window.on_external_drop_progress({
            let observed = observed.clone();
            move |pending, _| observed.borrow_mut().pending.push(pending)
        });
        for id in 1..=4 {
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Receiving { id }), cx);
        }
        let newer_drag =
            ExternalPaths([PathBuf::from("/incoming/newer.png")].into_iter().collect());
        let pointer = point(px(20.), px(20.));
        window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Entered {
                position: pointer,
                paths: newer_drag,
            }),
            cx,
        );
        let result = window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Received {
                id: 1,
                position: point(px(120.), px(30.)),
                modifiers: Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
                paths: ExternalPaths([path.clone()].into_iter().collect()),
            }),
            cx,
        );
        assert!(!result.propagate);
        assert_eq!(window.mouse_position(), pointer);
        assert!(!window.modifiers().alt);
        assert!(cx.has_active_drag());
        let result = window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Received {
                id: 2,
                position: point(px(240.), px(30.)),
                modifiers: Modifiers::default(),
                paths: ExternalPaths([path.clone()].into_iter().collect()),
            }),
            cx,
        );
        assert!(result.propagate);
        project.set(2);
        let result = window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Received {
                id: 3,
                position: point(px(120.), px(30.)),
                modifiers: Modifiers::default(),
                paths: ExternalPaths([path.clone()].into_iter().collect()),
            }),
            cx,
        );
        assert!(result.propagate);
        window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::ReceiveFailed { id: 4 }),
            cx,
        );
    })
    .unwrap();
    let observed = observed.borrow();
    assert_eq!(observed.drops.as_slice(), &[("right", true, vec![path])]);
    assert_eq!(observed.mouse_ups, 0);
    assert_eq!(observed.pending, [1, 2, 3, 4, 3, 2, 1, 0]);
}
