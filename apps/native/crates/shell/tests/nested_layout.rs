#![cfg(target_os = "macos")]
//! A frame's layout grows with its elements, not with how deeply they nest.
//! taffy 0.12 and 0.13 measured every level of nested flex columns again
//! (twice the work per level): a Settings page of a few hundred rows took
//! over half a second to lay out. vendor/taffy/POWERMOVE.md has the fix.
use gpui::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, div, px,
};
use std::time::{Duration, Instant};

const ROWS: usize = 50;
const DEPTH: usize = 14;

struct Nested;
impl Render for Nested {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(800.))
            .children((0..ROWS).map(|i| {
                let mut row = div()
                    .flex()
                    .flex_col()
                    .child(format!("Row {i}"))
                    .into_any_element();
                for _ in 0..DEPTH {
                    row = div().flex().flex_col().child(row).into_any_element();
                }
                row
            }))
    }
}

#[gpui::test]
fn nested_flex_columns_lay_out_in_one_pass_per_level(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Nested);
    let any: AnyWindowHandle = window.into();
    let draw = |cx: &mut TestAppContext| {
        cx.update_window(any, |_, window, cx| {
            let started = Instant::now();
            window.draw(cx).clear(cx);
            started.elapsed()
        })
        .unwrap()
    };
    draw(cx);
    // Fourteen levels deep, 0.13's cache did 2^14 times the work: tens of
    // seconds here. One pass per level is milliseconds.
    let took = draw(cx);
    assert!(
        took < Duration::from_secs(2),
        "a frame of nested columns took {took:?}"
    );
}
