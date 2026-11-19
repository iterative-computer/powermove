//! AppKit Agent launcher and its list of running threads.
use super::*;
use crate::agent_launcher::{Launcher, Snapshot, Tone};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn animation_now() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
fn timer(
    delay: f64,
    repeats: bool,
    block: &block2::RcBlock<dyn Fn(std::ptr::NonNull<NSTimer>)>,
) -> Retained<NSTimer> {
    let t = unsafe { NSTimer::timerWithTimeInterval_repeats_block(delay, repeats, block) };
    unsafe {
        NSRunLoop::mainRunLoop().addTimer_forMode(&t, NSRunLoopCommonModes);
    }
    t
}
pub(super) struct State {
    model: RefCell<Launcher>,
    tokens: RefCell<Tokens>,
    target: Retained<Target>,
    presence_visible: Cell<bool>,
    hover: RefCell<Option<Retained<NSTimer>>>,
    popover: RefCell<Option<Retained<NSPopover>>>,
    task_refresh: RefCell<Option<Retained<NSTimer>>>,
}
define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind=MainThreadOnly]
    #[name="PMAgentLauncherButton"]
    #[ivars=State]
    pub(super) struct AgentButton;
    impl AgentButton {
        #[unsafe(method(updateTrackingAreas))]
        fn tracking(&self) {
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
            for a in self.trackingAreas().to_vec() { self.removeTrackingArea(&a); }
            let a = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(<NSTrackingArea as objc2::AllocAnyThread>::alloc(),self.bounds(),NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect,Some(self),None) };
            self.addTrackingArea(&a);
        }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _: &NSEvent) {
            if let Some(t) = self.ivars().hover.borrow_mut().take() { t.invalidate(); }
            let weak = objc2::rc::Weak::new(self);
            let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| { if let Some(button)=weak.load() {button.show_tasks();} });
            *self.ivars().hover.borrow_mut() = Some(timer(0.180,false,&block));
        }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: &NSEvent) {
            if let Some(t) = self.ivars().hover.borrow_mut().take() { t.invalidate(); }
            self.schedule_leave();
        }
        #[unsafe(method(openTask:))]
        fn open_task(&self, sender: &NSButton) {
            if let Some(id) = sender.identifier() {
                self.ivars().target.send(Command { name:"agent.launcher.task".into(), args:vec![serde_json::json!(id.to_string())], token:None });
            }
            self.dismiss();
        }
        #[unsafe(method(launch:))]
        fn launch(&self, _: &NSButton) {
            self.dismiss();
            self.ivars().target.send(Command { name:"agent".into(), args:vec![], token:None });
        }
    }
);
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[name="PMAgentLauncherTaskList"]
    #[ivars=objc2::rc::Weak<AgentButton>]
    struct TaskList;
    impl TaskList {
        #[unsafe(method(updateTrackingAreas))]
        fn tracking(&self) {
            unsafe {let _:()=msg_send![super(self),updateTrackingAreas];}
            for a in self.trackingAreas().to_vec() {self.removeTrackingArea(&a);}
            let area=unsafe {NSTrackingArea::initWithRect_options_owner_userInfo(<NSTrackingArea as objc2::AllocAnyThread>::alloc(),self.bounds(),NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect,Some(self),None)};
            self.addTrackingArea(&area);
        }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self,_:&NSEvent) {if let Some(b)=self.ivars().load() {if let Some(t)=b.ivars().hover.borrow_mut().take(){t.invalidate();}}}
        #[unsafe(method(mouseExited:))]
        fn exited(&self,_:&NSEvent) {if let Some(b)=self.ivars().load() {b.schedule_leave();}}
    }
);
impl AgentButton {
    pub(super) fn new(
        mtm: MainThreadMarker,
        target: Retained<Target>,
        tokens: Tokens,
    ) -> Retained<Self> {
        let b: Retained<Self> = unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(State { model:RefCell::new(Launcher::default()), tokens:RefCell::new(tokens), target, presence_visible:Cell::new(false), hover:RefCell::new(None), popover:RefCell::new(None), task_refresh:RefCell::new(None) })), initWithFrame:NSRect::new(NSPoint::ZERO,NSSize::new(32.,28.))]
        };
        unsafe {
            b.setTarget(Some(&b));
            b.setAction(Some(sel!(launch:)));
        }
        b.setBezelStyle(NSBezelStyle::AccessoryBarAction);
        b.setImagePosition(NSCellImagePosition::ImageOnly);
        b.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.agent")));
        b.paint();
        b
    }
    fn dismiss(&self) {
        if let Some(t) = self.ivars().task_refresh.borrow_mut().take() {
            t.invalidate();
        }
        if let Some(p) = self.ivars().popover.borrow_mut().take() {
            p.close();
        }
    }
    pub(super) fn set_presence_visible(&self, visible: bool) {
        if self.ivars().presence_visible.replace(visible) != visible {
            self.paint();
        }
    }
    pub(super) fn set_tokens(&self, tokens: Tokens) {
        if *self.ivars().tokens.borrow() != tokens {
            *self.ivars().tokens.borrow_mut() = tokens;
            self.paint();
        }
    }
    pub(super) fn publish(&self, snapshot: Snapshot, tokens: Tokens) {
        let state = self.ivars();
        let old = state.model.borrow();
        let old_tone = old.toolbar_tone(state.presence_visible.get());
        let tokens_changed = *state.tokens.borrow() != tokens;
        if old.snapshot == snapshot && !tokens_changed {
            return;
        }
        drop(old);
        *state.tokens.borrow_mut() = tokens;
        let mut model = state.model.borrow_mut();
        model.update(snapshot, animation_now());
        let close_menu = model.snapshot.panel_open || model.running_count() == 0;
        let tone_changed = old_tone != model.toolbar_tone(state.presence_visible.get());
        let label = NSString::from_str(&model.tooltip(now()));
        drop(model);
        if close_menu {
            self.dismiss();
        } else {
            self.refresh_tasks();
        }
        if tone_changed || tokens_changed {
            self.paint();
        }
        self.setToolTip(Some(&label));
        self.setAccessibilityLabel(Some(&label));
    }
    fn paint(&self) {
        let state = self.ivars();
        let model = state.model.borrow();
        let tone = model.toolbar_tone(state.presence_visible.get());
        let mark = image(tone, &state.tokens.borrow());
        let image_size_changed = self.image().is_none_or(|old| old.size() != mark.size());
        self.setImage(Some(&mark));
        let title = if tone == Tone::Attention {
            "Needs your answer"
        } else {
            ""
        };
        let title_changed = self.title().to_string() != title;
        if title_changed {
            self.setTitle(&NSString::from_str(title));
        }
        self.setImagePosition(if title.is_empty() {
            NSCellImagePosition::ImageOnly
        } else {
            NSCellImagePosition::ImageLeft
        });
        if title_changed || image_size_changed {
            self.sizeToFit();
        }

        let label = NSString::from_str(&model.tooltip(now()));
        self.setToolTip(Some(&label));
        self.setAccessibilityLabel(Some(&label));
    }
    fn refresh_tasks(&self) -> bool {
        let Some(pop) = self
            .ivars()
            .popover
            .borrow()
            .clone()
            .filter(|pop| pop.isShown())
        else {
            return false;
        };
        let rows = self.ivars().model.borrow().popover_rows(now());
        let Some(controller) = pop.contentViewController() else {
            return false;
        };
        let views = controller.view().subviews().to_vec();
        let buttons: Vec<_> = views
            .iter()
            .filter_map(|view| view.downcast_ref::<NSButton>())
            .collect();
        if buttons.len() != rows.len()
            || buttons.iter().zip(&rows).any(|(button, (id, _, _))| {
                button
                    .identifier()
                    .is_none_or(|value| value.to_string() != *id)
            })
        {
            let view = self.tasks_view(rows);
            pop.setContentSize(view.frame().size);
            controller.setView(&view);
        } else {
            for (button, (_, title, step)) in buttons.into_iter().zip(rows) {
                let text = format!("●  {title}\n    {step}");
                if button.title().to_string() != text {
                    button.setTitle(&NSString::from_str(&text));
                }
            }
        }
        true
    }
    fn schedule_leave(&self) {
        if let Some(t) = self.ivars().hover.borrow_mut().take() {
            t.invalidate();
        }
        let weak = objc2::rc::Weak::new(self);
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
            let Some(button) = weak.load() else {
                return;
            };
            let point = NSEvent::mouseLocation();
            let inside = button
                .ivars()
                .popover
                .borrow()
                .as_ref()
                .and_then(|p| p.contentViewController())
                .and_then(|c| c.view().window())
                .is_some_and(|w| {
                    let r = w.frame();
                    point.x >= r.origin.x
                        && point.y >= r.origin.y
                        && point.x <= r.origin.x + r.size.width
                        && point.y <= r.origin.y + r.size.height
                });
            if !inside {
                button.dismiss();
            }
        });
        *self.ivars().hover.borrow_mut() = Some(timer(0.120, false, &block));
    }
    fn show_tasks(&self) {
        let model = self.ivars().model.borrow();
        if model.snapshot.panel_open
            || model.running_count() == 0
            || self.window().is_none()
            || self
                .ivars()
                .popover
                .borrow()
                .as_ref()
                .is_some_and(|p| p.isShown())
        {
            return;
        }
        let rows = model.popover_rows(now());
        drop(model);
        let view = self.tasks_view(rows);
        let controller = NSViewController::new(self.mtm());
        controller.setView(&view);
        let pop = NSPopover::new(self.mtm());
        pop.setBehavior(NSPopoverBehavior::Transient);
        pop.setContentViewController(Some(&controller));
        pop.setAnimates(!self.ivars().tokens.borrow().accessibility.reduce_motion);
        pop.showRelativeToRect_ofView_preferredEdge(
            self.bounds(),
            self,
            objc2_foundation::NSRectEdge::MinY,
        );
        *self.ivars().popover.borrow_mut() = Some(pop);

        if let Some(t) = self.ivars().task_refresh.borrow_mut().take() {
            t.invalidate();
        }
        let weak = objc2::rc::Weak::new(self);
        let block = block2::RcBlock::new(move |t: std::ptr::NonNull<NSTimer>| {
            if !weak.load().is_some_and(|button| button.refresh_tasks()) {
                unsafe { t.as_ref().invalidate() };
            }
        });
        *self.ivars().task_refresh.borrow_mut() = Some(timer(1., true, &block));
    }
    fn tasks_view(&self, rows: Vec<(String, String, String)>) -> Retained<TaskList> {
        let t = self.ivars().tokens.borrow();
        let view: Retained<TaskList> = unsafe {
            msg_send![super(TaskList::alloc(self.mtm()).set_ivars(objc2::rc::Weak::new(self))), initWithFrame:
            NSRect::new(
                NSPoint::ZERO,
                NSSize::new(260., 30. + rows.len() as f64 * 48.),
            )]
        };
        let head = NSTextField::labelWithString(
            &NSString::from_str(&if rows.len() == 1 {
                "Running".into()
            } else {
                format!("{} running", rows.len())
            }),
            self.mtm(),
        );
        head.setTextColor(Some(&native_color(t.colors.muted)));
        head.setFont(Some(&NSFont::systemFontOfSize(t.typography.minimum as f64)));
        head.setFrame(NSRect::new(
            NSPoint::new(12., view.frame().size.height - 24.),
            NSSize::new(236., 20.),
        ));
        view.addSubview(&head);
        for (i, (id, title, step)) in rows.iter().enumerate() {
            let row = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&format!("●  {title}\n    {step}")),
                    Some(self),
                    Some(sel!(openTask:)),
                    self.mtm(),
                )
            };
            row.setIdentifier(Some(&NSString::from_str(id)));
            row.setBordered(false);
            row.setAlignment(NSTextAlignment::Left);
            row.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
            row.setContentTintColor(Some(&native_color(t.colors.text)));
            row.setFrame(NSRect::new(
                NSPoint::new(8., view.frame().size.height - 30. - (i + 1) as f64 * 48.),
                NSSize::new(244., 48.),
            ));
            view.addSubview(&row);
        }
        view
    }
}
pub(super) fn image(tone: Tone, t: &Tokens) -> Retained<NSImage> {
    let t = t.clone();
    let block = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
        NSGraphicsContext::saveGraphicsState_class();
        if tone == Tone::Attention {
            native_color(t.colors.warning).setFill();
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                NSPoint::new(8.5, 8.5),
                NSSize::new(7., 7.),
            ))
            .fill();
        } else {
            let rect = NSRect::new(NSPoint::new(5., 5.), NSSize::new(14., 14.));
            toolbar_symbol("sparkles", "Agent", &t).drawInRect(rect);
            native_color(t.colors.muted).set();
            NSRectFillUsingOperation(rect, NSCompositingOperation::SourceAtop);
        }
        NSGraphicsContext::restoreGraphicsState_class();
        objc2::runtime::Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(24., 24.), false, &block)
}
#[cfg(test)]
mod captures {
    use super::*;
    #[test]
    fn agent_launcher_dark_and_light_headless() {
        let Ok(dir) = std::env::var("PM_AGENT_LAUNCHER_CAPTURE") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            let t = Tokens::new(scheme, Accessibility::default());
            let task = crate::agent_launcher::Task {
                id: "one".into(),
                title: "Review the edit".into(),
                step: "Checking the timeline".into(),
                started_at: 1000,
                attention: false,
                running: true,
            };
            let mut working = Launcher::default();
            working.update(
                Snapshot {
                    tasks: vec![task.clone()],
                    ..Default::default()
                },
                1000,
            );
            let mut multi = working.clone();
            multi.snapshot.tasks.push(crate::agent_launcher::Task {
                id: "two".into(),
                title: "Prepare captions".into(),
                step: "Working in the background".into(),
                ..task.clone()
            });
            let mut attention = working.clone();
            attention.snapshot.tasks[0].attention = true;
            attention.snapshot.tasks[0].running = false;
            let mut done = working.clone();
            done.update(Snapshot::default(), 2000);
            let states = vec![
                ("Idle", Launcher::default()),
                ("Running", working),
                ("Done · unseen", done),
                ("Needs your answer", attention),
                ("2 tasks running", multi.clone()),
            ];
            let draw = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
                native_color(t.colors.panel).setFill();
                NSRectFill(NSRect::new(NSPoint::ZERO, NSSize::new(600., 260.)));
                let attrs = objc2_foundation::NSDictionary::from_slices(
                    &[unsafe { NSFontAttributeName }, unsafe {
                        NSForegroundColorAttributeName
                    }],
                    &[
                        &*NSFont::systemFontOfSize(12.) as &AnyObject,
                        &*native_color(t.colors.text) as &AnyObject,
                    ],
                );
                for (i, (label, model)) in states.iter().enumerate() {
                    let y = 220. - i as f64 * 42.;
                    image(model.toolbar_tone(false), &t)
                        .drawInRect(NSRect::new(NSPoint::new(20., y), NSSize::new(24., 24.)));
                    unsafe {
                        NSString::from_str(label)
                            .drawAtPoint_withAttributes(NSPoint::new(80., y + 4.), Some(&attrs));
                    }
                }
                unsafe {
                    NSString::from_str("Running")
                        .drawAtPoint_withAttributes(NSPoint::new(300., 220.), Some(&attrs));
                }
                for (i, (_, title, step)) in multi.popover_rows(81000).iter().enumerate() {
                    unsafe {
                        NSString::from_str(&format!("● {title}\n   {step}"))
                            .drawAtPoint_withAttributes(
                                NSPoint::new(300., 170. - i as f64 * 60.),
                                Some(&attrs),
                            );
                    }
                }
                objc2::runtime::Bool::YES
            });
            let sheet = NSImage::imageWithSize_flipped_drawingHandler(
                NSSize::new(600., 260.),
                false,
                &draw,
            );
            let data = sheet.TIFFRepresentation().unwrap();
            let rep = NSBitmapImageRep::imageRepWithData(&data).unwrap();
            let png = unsafe {
                rep.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &objc2_foundation::NSDictionary::new(),
                )
            }
            .unwrap();
            std::fs::write(format!("{dir}/{name}.png"), png.to_vec()).unwrap();
        }
    }
}
