//! Window ▸ Migration Report (N003): a standard window, like the Error Log.
use gpui::{prelude::*, *};
use pm_first_launch::surfaces::migration::{Report, about_native_files};
use pm_shell::{
    kit::{Button, Kit, Recipes},
    motion::Motion,
    tokens,
};
use std::path::PathBuf;

pub struct ReportWindow {
    motion: Motion<u64>,
    report: Report,
    profile: PathBuf,
}
impl ReportWindow {
    fn motion(&mut self) -> &mut Motion<u64> {
        &mut self.motion
    }
    fn action(&self, id: &str) {
        match id {
            "show_library" => crate::platform::reveal(&self.profile),
            "copy_report" => copy_text(&report_text(&self.report)),
            "about_native_files" => {
                let view = about_native_files();
                crate::alert::note(&view.title, &view.line);
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
        self.motion.frame(window);
        let t = tokens();
        let kit = Kit::new(&t, &self.motion, Self::motion, window, cx);
        let mut body = div()
            .id("migration-report")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(t.space.lg));
        if let Some((title, line)) = self.report.empty_copy() {
            body = body.items_center().justify_center().child(kit.empty_state(
                "checkmark.circle.fill",
                title,
                line,
            ));
        }
        for (kind, rows) in self.report.sections() {
            let (section, action) = kind.section();
            // Report sections describe exceptions; a clean report shows the success state.
            let mut card = kit.card().elevated(&t, pm_design::Elevation::Well);
            for (index, row) in rows.into_iter().enumerate() {
                if index > 0 {
                    card = card.child(kit.separator());
                }
                card = card.child(
                    kit.form_row(
                        &row.display_name,
                        Some(action),
                        kit.symbol(
                            "exclamationmark.triangle.fill",
                            t.metrics.symbol_panel,
                            pm_shell::symbol::Weight::Regular,
                            t.colors.warning,
                        )
                        .into_any_element(),
                    ),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.md))
                    .child(div().text_role(&t, pm_design::Text::Heading).child(section))
                    .child(card),
            );
        }
        let footer = self
            .report
            .footer_actions()
            .into_iter()
            .map(|button| {
                let id = button.id;
                kit.button(id, button.label, Button::Neutral, None, None)
                    .debug_selector(move || format!("report-{id}"))
                    .on_click(cx.listener(move |v, _, _, _| v.action(id)))
                    .into_any_element()
            })
            .collect();
        kit.utility_window(
            "Migration Report",
            None,
            vec![],
            body.into_any_element(),
            footer,
        )
    }
}
const UNREADABLE: &str = "The Migration Report couldn’t be read.";
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
        let report = match work.await {
            Ok(report) => report,
            Err(error) => {
                eprintln!("Could not read the Migration Report: {error:#}");
                cx.update(|cx| {
                    crate::error_log::shared(cx)
                        .borrow_mut()
                        .push("Library", UNREADABLE);
                });
                crate::alert::note(UNREADABLE, "Your library wasn’t changed.");
                return;
            }
        };
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
                |_, cx| {
                    cx.new(|_| ReportWindow {
                        motion: Motion::default(),
                        report,
                        profile,
                    })
                },
            )
        });
    })
    .detach();
}

#[::core::prelude::v1::test]
#[ignore = "writes synthetic review captures"]
fn capture_utility_migration() {
    use pm_first_launch::surfaces::migration::{ReportKind, ReportRow};
    crate::utility_capture_tests::capture("migration-report", |_, cx| {
        cx.new(|_| ReportWindow {
            motion: Motion::default(),
            report: Report {
                rows: vec![
                    ReportRow {
                        id: "synthetic".into(),
                        display_name: "Opening Sequence".into(),
                        kind: ReportKind::History,
                    },
                    ReportRow {
                        id: "synthetic-missing".into(),
                        display_name: "Archive Footage".into(),
                        kind: ReportKind::Media,
                    },
                ],
            },
            profile: std::env::temp_dir(),
        })
    });
}
