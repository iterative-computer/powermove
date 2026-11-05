//! AppKit-only alert adapter. Queue ownership stays in Authority; callbacks
//! publish typed responses and never run services/JS. Native sheets, app icon,
//! system alert AX semantics and system font/layout are intentionally retained.
use super::*;
use objc2::{
    MainThreadMarker,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSString};
use std::{rc::Rc, sync::mpsc::Sender};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeResponse {
    pub target: Target,
    pub request: RequestId,
    pub action: Action,
}
type BannerHandler = Rc<dyn Fn(&Target, &str)>;

pub struct AppKitHost {
    windows: BTreeMap<String, Retained<NSWindow>>,
    active: BTreeMap<RequestId, (Retained<NSWindow>, Retained<NSAlert>)>,
    responses: Sender<NativeResponse>,
    close_access_popover: Rc<dyn Fn(&str)>,
    /// Shell renders README §3's banner/notice and posts the medium-priority
    /// NSAccessibility announcement. It must not turn this into a toast.
    banner: BannerHandler,
}
impl AppKitHost {
    pub fn new(
        responses: Sender<NativeResponse>,
        close_access_popover: Rc<dyn Fn(&str)>,
        banner: BannerHandler,
    ) -> Self {
        Self {
            windows: BTreeMap::new(),
            active: BTreeMap::new(),
            responses,
            close_access_popover,
            banner,
        }
    }
    pub fn register_window(&mut self, id: String, window: Retained<NSWindow>) {
        self.windows.insert(id, window);
    }
    /// Invalidate target requests in Authority before unregistering the window.
    pub fn unregister_window(&mut self, id: &str) {
        self.windows.remove(id);
    }
}
impl AuthorityHost for AppKitHost {
    fn present(&mut self, view: AlertView<'_>) {
        let mtm = MainThreadMarker::new().expect("AppKit main thread");
        let fallback = view
            .buttons
            .iter()
            .find(|b| b.action == Action::Cancel)
            .or_else(|| view.buttons.iter().find(|b| b.default))
            .map(|b| b.action)
            .unwrap_or(Action::Cancel);
        let target = view.target.clone();
        let request = view.id;
        let Some(window) = self.windows.get(&target.window).cloned() else {
            // These authority actions always own a window. A missing one must
            // not grant or start an app-modal confirmation on another window.
            let _ = self.responses.send(NativeResponse {
                target,
                request,
                action: fallback,
            });
            return;
        };
        if view.close_access_popover_first {
            (self.close_access_popover)(&target.window);
        }
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&view.message));
        alert.setInformativeText(&NSString::from_str(view.informative.expose()));
        alert.setAlertStyle(match view.style {
            AlertStyle::Critical => NSAlertStyle::Critical,
            AlertStyle::Warning => NSAlertStyle::Warning,
            AlertStyle::Informational => NSAlertStyle::Informational,
        });
        alert.setShowsSuppressionButton(false);
        // Adding a first button gives it Return automatically. Clear every
        // implicit key equivalent, then explicitly assign the safe default.
        let buttons: Vec<_> = view
            .buttons
            .iter()
            .map(|b| {
                let native = alert.addButtonWithTitle(&NSString::from_str(b.label));
                native.setKeyEquivalent(&NSString::from_str(""));
                native.setHasDestructiveAction(b.destructive);
                native
            })
            .collect();
        alert.window().setDefaultButtonCell(None);
        for (b, native) in view.buttons.iter().zip(&buttons) {
            if b.default {
                if let Some(cell) = native
                    .cell()
                    .and_then(|c| c.downcast::<NSButtonCell>().ok())
                {
                    alert.window().setDefaultButtonCell(Some(&cell));
                }
                native.setKeyEquivalent(&NSString::from_str("\r"));
            }
            if b.escape {
                // A default Cancel uses its defaultButtonCell for Return, and
                // its key equivalent for Esc (the two paths are independent).
                native.setKeyEquivalent(&NSString::from_str("\u{1b}"));
            }
        }
        if let Some(max_lines) = view.max_lines {
            alert.layout();
            if let Some(content) = alert.window().contentView() {
                limit_summary(&content, view.informative.expose(), max_lines);
            }
        }
        let actions: Vec<_> = view.buttons.into_iter().map(|b| b.action).collect();
        let responses = self.responses.clone();
        let done = block2::RcBlock::new(move |response: NSModalResponse| {
            let action = usize::try_from(response - NSAlertFirstButtonReturn)
                .ok()
                .and_then(|i| actions.get(i))
                .copied()
                .unwrap_or(fallback);
            let _ = responses.send(NativeResponse {
                target: target.clone(),
                request,
                action,
            });
        });
        alert.beginSheetModalForWindow_completionHandler(&window, Some(&done));
        self.active.insert(request, (window, alert));
    }
    fn dismiss(&mut self, _window: &str, request: RequestId) {
        if let Some((window, alert)) = self.active.remove(&request)
            && window.attachedSheet().as_deref() == Some(&*alert.window())
        {
            window.endSheet_returnCode(&alert.window(), NSModalResponseCancel);
        }
    }
    fn copy_concealed(&mut self, value: &PrivateValue) {
        let item = NSPasteboardItem::new();
        item.setString_forType(&NSString::from_str(value.expose()), unsafe {
            NSPasteboardTypeString
        });
        item.setString_forType(
            &NSString::from_str(""),
            &NSString::from_str(CONCEALED_PASTEBOARD_TYPE),
        );
        let writing = ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*item);
        let items = NSArray::from_slice(&[writing]);
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.writeObjects(&items);
    }
    fn changed_banner(&mut self, target: &Target, text: &str) {
        (self.banner)(target, text);
    }
}

/// Use AppKit's actual line layout; no character-count approximation. Full
/// summary already went to AwaitingAnswer.activity_summary for the activity row.
fn limit_summary(view: &NSView, summary: &str, lines: usize) {
    let object: &AnyObject = view;
    if let Some(field) = object.downcast_ref::<NSTextField>()
        && field.stringValue().to_string() == summary
    {
        field.setMaximumNumberOfLines(lines as isize);
        if let Some(cell) = field.cell() {
            cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
    }
    for child in view.subviews() {
        limit_summary(&child, summary, lines);
    }
}

/// S011 native setup sheet. Values are read directly into AppKit fields; neither
/// the safe SheetView nor a v4/DOM node receives their contents. The shell owns
/// the target/delegate and polls its worker; this factory performs no service IO.
pub fn setup_sheet(
    setup: &values::Setup,
    tokens: &pm_design::Tokens,
    mtm: MainThreadMarker,
) -> super::sheet_layout::native::Sheet {
    use super::sheet_layout::native::Sheet;
    use values::Widget;
    let view = setup.view();
    let lines = view
        .rows
        .iter()
        .map(|r| {
            1 + usize::from(r.optional.is_some())
                + usize::from(r.help.is_some())
                + usize::from(r.warning.is_some())
        })
        .sum::<usize>()
        + usize::from(view.explanation.is_some())
        + usize::from(view.error.is_some())
        + 1;
    let mut sheet = Sheet::new(&view.title, lines, tokens, false, mtm);
    if let Some(line) = &view.explanation {
        sheet.line(
            line,
            tokens.typography.reading.into(),
            tokens.colors.secondary,
        );
    }
    for row in &view.rows {
        if row.widget == Widget::AlreadySet {
            let group = NSStackView::new(mtm);
            group.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
            group.setSpacing(tokens.metrics.gap.into());
            let marker = NSTextField::labelWithString(&NSString::from_str("••••"), mtm);
            marker.setAccessibilityLabel(Some(&NSString::from_str(&row.accessibility_label)));
            group.addArrangedSubview(&marker);
            for action in &row.actions {
                let id = format!("{}/{}", row.key, action);
                sheet.button(&id, action, view.can_edit, false, false);
                let button = &sheet.buttons[&id];
                button.setBordered(false);
                group.addArrangedSubview(button);
            }
            sheet.row(&row.label, &group);
        } else {
            sheet.text(
                &row.key,
                &row.label,
                setup.field_value(&row.key).unwrap_or(""),
                row.widget == Widget::NSSecureTextField,
                view.can_edit,
            );
            if let Some(symbol) = row.eye_symbol {
                let id = format!("{}/visibility", row.key);
                sheet.button(&id, "", view.can_edit, false, false);
                let button = &sheet.buttons[&id];
                let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol),
                    None,
                );
                button.setImage(image.as_deref());
                button.setBordered(false);
                button.setToolTip(
                    row.eye_tooltip
                        .as_deref()
                        .map(NSString::from_str)
                        .as_deref(),
                );
                button.setAccessibilityLabel(
                    row.eye_tooltip
                        .as_deref()
                        .map(NSString::from_str)
                        .as_deref(),
                );
                // Place the eye inside the value column, trailing the field.
                let field = &sheet.fields[&row.key];
                let mut rect = field.frame();
                rect.size.width -= f64::from(tokens.metrics.hit);
                field.setFrame(rect);
                button.setFrame(objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(rect.origin.x + rect.size.width, rect.origin.y),
                    objc2_foundation::NSSize::new(
                        tokens.metrics.hit.into(),
                        tokens.metrics.field.into(),
                    ),
                ));
                sheet.content.addSubview(button);
            }
        }
        for line in [row.optional, row.help.as_deref()].into_iter().flatten() {
            sheet.line(line, tokens.typography.minimum.into(), tokens.colors.muted);
        }
        if let Some(line) = row.warning {
            sheet.line(
                line,
                tokens.typography.minimum.into(),
                tokens.colors.warning,
            );
        }
    }
    sheet.line(
        view.note,
        tokens.typography.minimum.into(),
        tokens.colors.muted,
    );
    if let Some(line) = &view.error {
        sheet.line(line, tokens.typography.minimum.into(), tokens.colors.danger);
    }
    sheet.button("cancel", view.cancel_label, view.can_cancel, false, true);
    sheet.button("save", view.save_label, view.can_save, true, false);
    sheet.footer(&["cancel", "save"], None);
    sheet
}
