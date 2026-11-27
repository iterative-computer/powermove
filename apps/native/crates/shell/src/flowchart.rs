//! AppKit presentation of main-delta-ui.md §3. No document writes here.
use super::*;
#[derive(Clone, Debug)]
pub enum FlowchartRow {
    Heading(String),
    Empty(String),
    Composition {
        id: String,
        name: String,
        current: bool,
    },
}
pub(super) struct Popover {
    popover: Retained<NSPopover>,
    _target: Retained<FlowchartTarget>,
}
impl Popover {
    pub(super) fn owns_event(&self, event: &NSEvent) -> bool {
        self.popover.isShown()
            && self
                .popover
                .contentViewController()
                .and_then(|c| c.view().window())
                .is_some_and(|w| w.windowNumber() == event.windowNumber())
    }
    fn close(&self) {
        self.popover.close();
    }
}
struct FlowchartState {
    ids: Vec<String>,
    document: u64,
    sender: async_channel::Sender<Command>,
    popover: Retained<NSPopover>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMCompositionFlowchartTarget"]
    #[ivars = FlowchartState]
    struct FlowchartTarget;
    unsafe impl NSObjectProtocol for FlowchartTarget {}
    impl FlowchartTarget {
        #[unsafe(method(openComposition:))]
        fn open_composition(&self, button: &NSButton) {
            self.ivars().popover.close();
            if let Some(id) = self.ivars().ids.get(button.tag() as usize) {
                let _ = self.ivars().sender.try_send(Command {
                    name: "composition.open".into(), args: vec![serde_json::json!(id), serde_json::json!(self.ivars().document)], token: None,
                });
            }
        }
    }
);
impl Host {
    /// The resolver may offer Tab in a field; decline so replay can move focus.
    pub fn composition_canvas_focused(&self) -> bool {
        self.canvas_focus.get()
            && !NSApplication::sharedApplication(self.window.mtm()).isFullKeyboardAccessEnabled()
            && self.window.firstResponder().is_none_or(|r| {
                std::ptr::eq(
                    &*r as *const _,
                    &*self.gpui as *const _ as *const NSResponder,
                ) || std::ptr::eq(
                    &*r as *const _,
                    &*self.viewer as *const _ as *const NSResponder,
                )
            })
    }

    pub fn close_flowchart(&mut self) {
        if let Some(chart) = self.flowchart.take() {
            chart.close();
        }
    }
    pub fn flowchart_open(&self) -> bool {
        self.flowchart.as_ref().is_some_and(|c| c.popover.isShown())
    }
    pub fn toggle_flowchart(&mut self, rows: Vec<FlowchartRow>, document: u64) -> bool {
        if self.flowchart_open() {
            self.close_flowchart();
            return true;
        }
        let Some(rect) = self.viewer_rect.get() else {
            return false;
        };
        let mtm = self.window.mtm();
        let t = &self.tokens;
        let width = t.metrics.popover_width as f64;
        let row_height = t.metrics.row as f64;
        let height = row_height * rows.len() as f64;
        let content = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(width, height)),
        );
        content.setWantsLayer(true);
        if let Some(layer) = content.layer() {
            layer.setBackgroundColor(Some(&native_color(t.colors.floating).CGColor()));
        }
        content.setAccessibilityLabel(Some(&NSString::from_str("Composition flowchart")));
        let popover = NSPopover::new(mtm);
        popover.setBehavior(NSPopoverBehavior::Transient);
        let ids = rows
            .iter()
            .filter_map(|r| match r {
                FlowchartRow::Composition { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let target: Retained<FlowchartTarget> = unsafe {
            msg_send![
                super(FlowchartTarget::alloc(mtm).set_ivars(FlowchartState {
                    ids,
                    document,
                    sender: self.sender.clone(),
                    popover: popover.clone(),
                })),
                init
            ]
        };
        let mut tag = 0;
        for (index, row) in rows.into_iter().enumerate() {
            let frame = NSRect::new(
                NSPoint::new(0., height - (index + 1) as f64 * row_height),
                NSSize::new(width, row_height),
            );
            match row {
                FlowchartRow::Heading(text) | FlowchartRow::Empty(text) => {
                    let label = NSTextField::labelWithString(&NSString::from_str(&text), mtm);
                    label.setFrame(frame);
                    label.setFont(Some(&NSFont::systemFontOfSize(t.typography.minimum as f64)));
                    label.setTextColor(Some(&native_color(t.colors.muted)));
                    content.addSubview(&label);
                }
                FlowchartRow::Composition { name, current, .. } => {
                    // Target is retained by Popover and implements this selector.
                    let button = unsafe {
                        NSButton::buttonWithTitle_target_action(
                            &NSString::from_str(&name),
                            Some(&target),
                            Some(sel!(openComposition:)),
                            mtm,
                        )
                    };
                    button.setTag(tag);
                    tag += 1;
                    button.setFrame(frame);
                    button.setBordered(false);
                    button.setAlignment(NSTextAlignment::Left);
                    button.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                    button.setContentTintColor(Some(&native_color(t.colors.text)));
                    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str("rectangle.stack"),
                        None,
                    ) {
                        button.setImage(Some(&image));
                        button.setImagePosition(NSCellImagePosition::ImageLeft);
                    }
                    if current {
                        button.setWantsLayer(true);
                        if let Some(layer) = button.layer() {
                            layer.setBackgroundColor(Some(
                                &native_color(t.colors.selected_fill).CGColor(),
                            ));
                        }
                    }
                    content.addSubview(&button);
                }
            }
        }
        let controller = NSViewController::new(mtm);
        controller.setView(&content);
        popover.setContentViewController(Some(&controller));
        popover.setContentSize(NSSize::new(width, height));
        let y = if self.gpui.isFlipped() {
            rect.y as f64
        } else {
            self.gpui.bounds().size.height - rect.y as f64
        };
        popover.showRelativeToRect_ofView_preferredEdge(
            NSRect::new(NSPoint::new(rect.x as f64, y), NSSize::new(1., 1.)),
            &self.gpui,
            objc2_foundation::NSRectEdge::MaxY,
        );
        self.flowchart = Some(Popover {
            popover,
            _target: target,
        });
        true
    }
}
