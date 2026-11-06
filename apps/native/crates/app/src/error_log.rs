//! Window ▸ Error Log (S053). Rows are what the app told the person, newest
//! first, on the recovery lane's table recipe. Nothing here leaves this Mac.
use crate::surfaces::recovery::notices::{LOG_EMPTY_LINE, LOG_EMPTY_TITLE, LOG_TITLE};
use gpui::{prelude::*, *};
use pm_shell::tokens;
use std::{cell::RefCell, rc::Rc, time::SystemTime};

#[derive(Clone, Debug)]
pub struct Entry {
    pub at: SystemTime,
    pub source: String,
    pub message: String,
}
#[derive(Default)]
pub struct Log {
    pub entries: Vec<Entry>,
}
impl Log {
    /// Repeats of the newest row collapse: a stuck failure is one row.
    pub fn push(&mut self, source: &str, message: &str) {
        if self
            .entries
            .last()
            .is_some_and(|e| e.source == source && e.message == message)
        {
            return;
        }
        self.entries.push(Entry {
            at: SystemTime::now(),
            source: source.into(),
            message: message.into(),
        });
    }
}
/// System-relative time, in the column's short form.
fn age(at: SystemTime) -> String {
    let minutes = at.elapsed().map_or(0, |e| e.as_secs() / 60);
    match minutes {
        0 => "Now".into(),
        1..=59 => format!("{minutes} min ago"),
        _ => format!("{} hr ago", minutes / 60),
    }
}
pub type Shared = Rc<RefCell<Log>>;
struct ErrorLogWindow(Shared);
impl Render for ErrorLogWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = tokens();
        let (c, m) = (&t.colors, &t.metrics);
        let log = self.0.borrow();
        let mut body = div()
            .id("error-log")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .p(px(m.inset))
            .bg(rgb(c.panel))
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT);
        if log.entries.is_empty() {
            body = body
                .items_center()
                .justify_center()
                .gap(px(m.gap))
                .child(
                    div()
                        .text_size(px(t.typography.title))
                        .child(LOG_EMPTY_TITLE),
                )
                .child(div().text_color(rgb(c.secondary)).child(LOG_EMPTY_LINE));
        }
        for entry in log.entries.iter().rev() {
            body = body.child(
                div()
                    .flex()
                    .gap(px(m.inset))
                    .p(px(m.gap / 2.))
                    .child(
                        div()
                            .flex_none()
                            .w(px(m.settings_sidebar))
                            .text_color(rgb(c.secondary))
                            .child(format!("{} · {}", age(entry.at), entry.source)),
                    )
                    .child(entry.message.clone()),
            );
        }
        body
    }
}
pub fn open(log: Shared, cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find(|w| w.downcast::<ErrorLogWindow>().is_some())
    {
        let _ = window.update(cx, |_, w, _| w.activate());
        return;
    }
    let t = tokens();
    let _ = cx.open_window(
        WindowOptions::new()
            .titlebar(Some(TitlebarOptions {
                title: Some(LOG_TITLE.into()),
                ..Default::default()
            }))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(t.metrics.toolbar), px(t.metrics.toolbar)),
                size(px(t.metrics.settings_width), px(t.metrics.settings_height)),
            )))),
        |_, cx| cx.new(|_| ErrorLogWindow(log)),
    );
}
#[cfg(test)]
mod tests {
    use super::Log;
    #[test]
    fn repeats_collapse_and_order_is_kept() {
        let mut log = Log::default();
        log.push("Editor", "One");
        log.push("Editor", "One");
        log.push("Editor", "Two");
        log.push("Editor", "One");
        assert_eq!(log.entries.len(), 3);
    }
}
