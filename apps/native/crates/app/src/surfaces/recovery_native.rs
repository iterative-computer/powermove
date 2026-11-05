//! Native recipes for S053/S054. Host implementations must use the named system
//! widgets; a recipe is not a claim that AppKit rendered on Linux.
use super::recovery::{notices::*, status::*};
use pm_design::{MONO_FONT, Tokens};

#[derive(Clone, Debug, PartialEq)]
pub struct LogWindow {
    pub title: &'static str,
    pub minimum_size: (f32, f32),
    pub autosave_frame: bool,
    pub columns: [&'static str; 3],
    pub alternating_rows: bool,
    pub row_height: f32,
    pub body_size: f32,
    pub details_font: &'static str,
    pub details_label: &'static str,
    pub details_selectable: bool,
    pub buttons: [&'static str; 2],
    pub empty: Option<(&'static str, &'static str)>,
    /// NSTableView gets only these display values. Host formats the instants
    /// using system relative short time, never a lane-authored date pattern.
    pub rows: Vec<LogDisplayRow>,
    /// Only NSSplitView's Details text view gets the diagnostic text.
    pub details: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogDisplayRow {
    pub id: String,
    pub time: std::time::SystemTime,
    pub source: String,
    pub message: String,
}
pub fn error_log(log: &ErrorLog, tokens: &Tokens) -> LogWindow {
    LogWindow {
        title: LOG_TITLE,
        minimum_size: (560., 360.),
        autosave_frame: true,
        columns: LOG_COLUMNS,
        alternating_rows: false,
        row_height: tokens.metrics.row,
        body_size: tokens.typography.body,
        details_font: MONO_FONT,
        details_label: LOG_DETAILS,
        details_selectable: true,
        buttons: LOG_BUTTONS,
        empty: log
            .rows()
            .is_empty()
            .then_some((LOG_EMPTY_TITLE, LOG_EMPTY_LINE)),
        rows: log
            .rows()
            .iter()
            .map(|r| LogDisplayRow {
                id: r.id.clone(),
                time: r.time,
                source: r.source_display.clone(),
                message: r.copy.message.clone(),
            })
            .collect(),
        details: log.details().map(str::to_owned),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SlowdownPopover {
    /// NSPopover arrow points at this stable slowdown indicator id.
    pub anchor: String,
    pub width: f32,
    pub title: &'static str,
    pub title_size: f32,
    pub row_minimum: f32,
    pub name_size: f32,
    pub name_color: u32,
    pub fact_size: f32,
    pub fact_color: u32,
    pub rows: Vec<SlowdownRow>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowdownRow {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub fact: Option<String>,
    pub button: &'static str,
    pub error: Option<String>,
}
pub fn slowdown(model: &Slowdown, anchor: &str, tokens: &Tokens) -> Option<SlowdownPopover> {
    if !model.open || model.issues.is_empty() {
        return None;
    }
    Some(SlowdownPopover {
        anchor: anchor.into(),
        width: tokens.metrics.popover_width * 1.5,
        title: Slowdown::TITLE,
        title_size: tokens.typography.title,
        row_minimum: tokens.metrics.row,
        name_size: tokens.typography.body,
        name_color: tokens.colors.text,
        fact_size: tokens.typography.minimum,
        fact_color: tokens.colors.secondary,
        rows: model
            .issues
            .iter()
            .map(|i| SlowdownRow {
                id: i.id.clone(),
                revision: i.revision,
                name: i.name.clone(),
                fact: i.fact(),
                button: i.button(),
                error: i.error(),
            })
            .collect(),
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct StatusBar {
    pub height: f32,
    pub ground: u32,
    pub size: f32,
    pub color: u32,
    pub ax_label: &'static str,
    pub text: StatusText,
    pub slowdown_symbol: Option<&'static str>,
    pub symbol_size: f32,
    pub symbol_color: u32,
    pub extension_items: Vec<ExtensionItem>,
}
pub fn status(model: &Status, issues: usize, tokens: &Tokens) -> StatusBar {
    StatusBar {
        height: tokens.metrics.status,
        ground: tokens.colors.window,
        size: tokens.typography.minimum,
        color: tokens.colors.muted,
        ax_label: Status::AX_LABEL,
        text: model.text(issues),
        slowdown_symbol: (issues > 0).then_some("gauge.with.dots.needle.33percent"),
        symbol_size: tokens.metrics.symbol_inline,
        symbol_color: tokens.colors.warning,
        extension_items: model.extension_items.clone(),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum NativeUpdate {
    /// One standard titled window, NSTableView over NSSplitView/Details text.
    ErrorLog(LogWindow),
    /// Some = NSPopover anchored to the indicator; None = close it.
    Slowdown(Option<SlowdownPopover>),
    Status(StatusBar),
    Notice {
        source: Source,
        placement: Placement,
        banner: Banner,
        announcement: Priority,
    },
}
/// Nonblocking main-thread delivery. The native adapter must post the specified
/// NSAccessibility announcement once per accepted state change, not per paint.
pub trait NativeSurfaceHost {
    type Error;
    fn enqueue(&mut self, update: NativeUpdate) -> Result<(), Self::Error>;
}
#[derive(Default)]
pub struct HeadlessNativeHost {
    pub updates: Vec<NativeUpdate>,
    pub fail_next: bool,
}
impl NativeSurfaceHost for HeadlessNativeHost {
    type Error = &'static str;
    fn enqueue(&mut self, update: NativeUpdate) -> Result<(), Self::Error> {
        if std::mem::take(&mut self.fail_next) {
            return Err("injected native queue failure");
        }
        self.updates.push(update);
        Ok(())
    }
}
pub fn publish_notice<H: NativeSurfaceHost>(
    notice: &Notice,
    has_viewer: bool,
    host: &mut H,
) -> Result<(), H::Error> {
    host.enqueue(NativeUpdate::Notice {
        source: notice.source.clone(),
        placement: notice.placement(has_viewer),
        banner: Banner::notice(notice),
        announcement: notice.priority(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use pm_design::{Accessibility, Scheme};
    #[test]
    fn native_recipes_follow_tokens_in_every_system_appearance() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            for flags in 0..8 {
                let tokens = Tokens::new(
                    scheme,
                    Accessibility {
                        reduce_transparency: flags & 1 != 0,
                        increase_contrast: flags & 2 != 0,
                        reduce_motion: flags & 4 != 0,
                    },
                );
                let log = error_log(&ErrorLog::default(), &tokens);
                assert_eq!(log.minimum_size, (560., 360.));
                assert!(log.autosave_frame);
                assert!(!log.alternating_rows);
                assert_eq!(log.row_height, tokens.metrics.row);
                assert_eq!(log.details_font, MONO_FONT);
                assert_eq!(
                    log.empty,
                    Some(("No Errors", "Problems Powermove notices appear here."))
                );
                let status = status(
                    &Status {
                        layers: 1,
                        keyframes: 1,
                        playing: false,
                        width: 1920,
                        height: 1080,
                        rate: "30".into(),
                        workspace: "Default".into(),
                        extension_items: vec![],
                    },
                    2,
                    &tokens,
                );
                assert_eq!(status.height, tokens.metrics.status);
                assert_eq!(status.ground, tokens.colors.window);
                assert_eq!(status.ax_label, "Status");
                assert_eq!(status.symbol_color, tokens.colors.warning);
                assert_eq!(
                    status.slowdown_symbol,
                    Some("gauge.with.dots.needle.33percent")
                );
                let model = Slowdown {
                    open: true,
                    issues: vec![Issue {
                        id: "synthetic".into(),
                        revision: 1,
                        name: "Layer".into(),
                        milliseconds: 20.,
                        stage: Stage::Gpu,
                        target: Target::Layer("layer".into()),
                        turn_off_failed: false,
                    }],
                };
                let popover = slowdown(&model, "indicator", &tokens).unwrap();
                assert_eq!(popover.width, tokens.metrics.popover_width * 1.5);
                assert_eq!(popover.anchor, "indicator");
                assert_eq!(popover.title, "What’s Slowing the Editor Down");
                assert_eq!(popover.rows[0].button, "Select Layer");
                assert!(slowdown(&Slowdown::default(), "indicator", &tokens).is_none());
            }
        }
    }
    #[test]
    fn notices_enqueue_typed_source_and_error_priority_without_diagnostics() {
        let n = Notice {
            source: Source {
                surface: "S053",
                code: "offline".into(),
            },
            copy: service_copy(ServiceCode::Offline, None),
            severity: Severity::Error,
            action: Some(Fix::Retry),
        };
        let mut host = HeadlessNativeHost {
            fail_next: true,
            ..Default::default()
        };
        assert!(publish_notice(&n, false, &mut host).is_err());
        assert!(host.updates.is_empty());
        publish_notice(&n, false, &mut host).unwrap();
        assert!(
            matches!(&host.updates[0], NativeUpdate::Notice { source, placement: Placement::InlineNotice, announcement: Priority::High, .. } if source.surface == "S053" && source.code == "offline")
        );
    }
}
