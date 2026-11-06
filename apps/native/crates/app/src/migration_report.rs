//! Window ▸ Migration Report (N003): a standard window, like the Error Log.
use gpui::{prelude::*, *};
use pm_first_launch::surfaces::migration::{Report, about_native_files};
use pm_shell::tokens;
use std::path::PathBuf;

pub struct ReportWindow {
    report: Report,
    profile: PathBuf,
}
impl ReportWindow {
    fn action(&self, id: &str) {
        match id {
            "show_library" => crate::platform::reveal(&self.profile),
            "copy_report" => copy_text(&report_text(&self.report)),
            "about_native_files" => {
                let view = about_native_files();
                crate::platform::note(&view.title, &view.line);
            }
            _ => {}
        }
    }
}
/// The Copy Report text: names only, nothing private beyond what the window shows.
pub fn report_text(report: &Report) -> String {
    let mut text = String::new();
    for (kind, rows) in report.sections() {
        text.push_str(kind.section().0);
        text.push('\n');
        for row in rows {
            text.push_str("  ");
            text.push_str(&row.display_name);
            text.push('\n');
        }
    }
    text
}
fn copy_text(text: &str) {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}
impl Render for ReportWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens();
        let (c, m) = (&t.colors, &t.metrics);
        let mut body = div()
            .id("migration-report")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(m.inset))
            .p(px(m.sheet_inset));
        if let Some((title, line)) = self.report.empty_copy() {
            body = body
                .items_center()
                .justify_center()
                .child(pm_shell::symbol::symbol(
                    "checkmark.circle",
                    t.typography.display,
                    pm_shell::symbol::Weight::Medium,
                    c.secondary,
                    window,
                ))
                .child(div().text_size(px(t.typography.title)).child(title))
                .child(div().text_color(rgb(c.secondary)).child(line));
        }
        for (kind, rows) in self.report.sections() {
            let (section, action) = kind.section();
            let mut card = div()
                .flex()
                .flex_col()
                .rounded(px(m.group_radius))
                .bg(rgb(c.panel));
            for row in rows {
                card = card.child(
                    div()
                        .flex()
                        .justify_between()
                        .p(px(m.gap))
                        .child(row.display_name.clone())
                        .child(div().text_color(rgb(c.secondary)).child(action)),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(m.gap))
                    .child(div().text_size(px(t.typography.title)).child(section))
                    .child(card),
            );
        }
        let mut footer = div().flex().gap(px(m.gap)).p(px(m.inset));
        for button in self.report.footer_actions() {
            let id = button.id;
            footer = footer.child(
                div()
                    .id(id)
                    .debug_selector(|| format!("report-{id}"))
                    .px(px(m.inset))
                    .h(px(m.row))
                    .flex()
                    .items_center()
                    .rounded(px(m.control_radius))
                    .bg(rgb(c.panel_secondary))
                    .child(button.label)
                    .on_click(cx.listener(move |v, _, _, _| v.action(id))),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(c.window))
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .child(body)
            .child(footer)
    }
}
pub fn open(profile: PathBuf, cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find(|w| w.downcast::<ReportWindow>().is_some())
    {
        let _ = window.update(cx, |_, w, _| w.activate());
        return;
    }
    let work = cx.background_executor().spawn({
        let profile = profile.clone();
        async move { pm_first_launch::report::load(&profile) }
    });
    cx.spawn(async move |cx| {
        let report = work.await.unwrap_or_default();
        let t = cx.update(|_| tokens());
        let _ = cx.update(|cx| {
            cx.open_window(
                WindowOptions::new()
                    .titlebar(Some(TitlebarOptions {
                        title: Some("Migration Report".into()),
                        ..Default::default()
                    }))
                    .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                        point(px(t.metrics.toolbar), px(t.metrics.toolbar)),
                        size(px(t.metrics.settings_width), px(t.metrics.settings_height)),
                    )))),
                |_, cx| cx.new(|_| ReportWindow { report, profile }),
            )
        });
    })
    .detach();
}
