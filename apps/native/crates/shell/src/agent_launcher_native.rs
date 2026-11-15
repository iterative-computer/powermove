//! AppKit launcher. Timers exist only for visible, unfinished motion or hover intent.
use super::*;
use crate::agent_launcher::{Launcher, Snapshot, Tone};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
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
    model: Rc<RefCell<Launcher>>,
    tokens: RefCell<Tokens>,
    target: Retained<Target>,
    animation: RefCell<Option<Retained<NSTimer>>>,
    hover: RefCell<Option<Retained<NSTimer>>>,
    popover: RefCell<Option<Retained<NSPopover>>>,
    last_second: Cell<u64>,
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
            msg_send![super(Self::alloc(mtm).set_ivars(State { model:Rc::new(RefCell::new(Launcher::default())), tokens:RefCell::new(tokens), target, animation:RefCell::new(None), hover:RefCell::new(None), popover:RefCell::new(None), last_second:Cell::new(0) })), initWithFrame:NSRect::new(NSPoint::ZERO,NSSize::new(32.,28.))]
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
        if let Some(p) = self.ivars().popover.borrow_mut().take() {
            p.close();
        }
    }
    pub(super) fn publish(&self, snapshot: Snapshot, tokens: Tokens) {
        let changed = self.ivars().model.borrow().snapshot != snapshot;
        if !changed && *self.ivars().tokens.borrow() == tokens {
            return;
        }
        *self.ivars().tokens.borrow_mut() = tokens;
        self.ivars().model.borrow_mut().update(snapshot, now());
        if self.ivars().model.borrow().snapshot.panel_open
            || self.ivars().model.borrow().running_count() == 0
        {
            self.dismiss();
        }
        self.paint();
        let active = self.ivars().model.borrow().requests_frames(
            now(),
            self.ivars().tokens.borrow().accessibility.reduce_motion,
        );
        if !active {
            if let Some(t) = self.ivars().animation.borrow_mut().take() {
                t.invalidate();
            }
        } else if !self
            .ivars()
            .animation
            .borrow()
            .as_ref()
            .is_some_and(|t| t.isValid())
        {
            let weak = objc2::rc::Weak::new(self);
            let block = block2::RcBlock::new(move |t: std::ptr::NonNull<NSTimer>| {
                let Some(button) = weak.load() else {
                    unsafe {
                        t.as_ref().invalidate();
                    }
                    return;
                };
                button.paint();
                if !button.ivars().model.borrow().requests_frames(
                    now(),
                    button.ivars().tokens.borrow().accessibility.reduce_motion,
                ) {
                    unsafe {
                        t.as_ref().invalidate();
                    }
                }
            });
            *self.ivars().animation.borrow_mut() = Some(timer(1. / 30., true, &block));
        }
    }
    fn paint(&self) {
        let model = self.ivars().model.borrow();
        let t = self.ivars().tokens.borrow();
        let mark = image(&model, now(), &t);
        let image_size_changed = self.image().is_none_or(|old| old.size() != mark.size());
        self.setImage(Some(&mark));
        let title = match model.tone() {
            Tone::Idle => String::new(),
            Tone::Done => "Done".into(),
            Tone::Attention => "Needs your answer".into(),
            Tone::Working => {
                let task = &model.snapshot.tasks[0];
                let label = if task.step.is_empty() {
                    "Working…"
                } else {
                    &task.step
                };
                let short: String = label.chars().take(20).collect();
                format!(
                    "{}  {}",
                    short,
                    crate::agent_launcher::elapsed(now().saturating_sub(task.started_at))
                )
            }
        };
        let title_changed = self.title().to_string() != title;
        self.setImagePosition(if title.is_empty() {
            NSCellImagePosition::ImageOnly
        } else {
            NSCellImagePosition::ImageLeft
        });
        if title_changed || image_size_changed {
            self.setTitle(&NSString::from_str(&title));
            self.sizeToFit();
        }

        let label = NSString::from_str(&model.tooltip(now()));
        self.setToolTip(Some(&label));
        self.setAccessibilityLabel(Some(&label));
        let second = now() / 1000;
        if self.ivars().last_second.replace(second) != second {
            if let Some(controller) = self
                .ivars()
                .popover
                .borrow()
                .as_ref()
                .and_then(|p| p.contentViewController())
            {
                let rows = model.popover_rows(now());
                for child in controller.view().subviews().iter() {
                    if let Some(row) = child.downcast_ref::<NSButton>() {
                        if let Some(id) = row.identifier() {
                            if let Some((_, title, step)) =
                                rows.iter().find(|(key, _, _)| *key == id.to_string())
                            {
                                row.setTitle(&NSString::from_str(&format!(
                                    "●  {title}\n    {step}"
                                )));
                            } else {
                                row.setHidden(true);
                            }
                        }
                    }
                }
            }
        }
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
        let controller = NSViewController::new(self.mtm());
        controller.setView(&view);
        let pop = NSPopover::new(self.mtm());
        pop.setBehavior(NSPopoverBehavior::Transient);
        pop.setContentViewController(Some(&controller));
        pop.setAnimates(!t.accessibility.reduce_motion);
        pop.showRelativeToRect_ofView_preferredEdge(
            self.bounds(),
            self,
            objc2_foundation::NSRectEdge::MinY,
        );
        *self.ivars().popover.borrow_mut() = Some(pop);
    }
}
pub(super) fn image(model: &Launcher, time: u64, t: &Tokens) -> Retained<NSImage> {
    let model = model.clone();
    let t = t.clone();
    let count = model.running_count();
    let width = if count > 1 { 48. } else { 24. };
    let block = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
        let tone = model.tone();
        let reduced = t.accessibility.reduce_motion;
        let color = match tone {
            Tone::Done => t.colors.success,
            Tone::Attention => t.colors.warning,
            Tone::Working => t.colors.accent,
            _ => t.colors.muted,
        };
        let ink = native_color(color);
        ink.setStroke();
        ink.setFill();
        NSGraphicsContext::saveGraphicsState_class();
        if !reduced && let Some(at) = model.receiving_at {
            let x = time.saturating_sub(at) as f64 / 420.;
            if x < 1. {
                let bump = if x < 0.35 {
                    crate::agent_launcher::ease_out(x / 0.35)
                } else {
                    1. - crate::agent_launcher::ease_out((x - 0.35) / 0.65)
                };
                let transform = objc2_foundation::NSAffineTransform::new();
                transform.translateXBy_yBy(width / 2., 12.);
                transform.scaleXBy_yBy(1. + 0.04 * bump, 1. + 0.04 * bump);
                transform.translateXBy_yBy(-width / 2., -12.);
                transform.concat();
            }
        }
        let centre = NSPoint::new(12., 12.);
        if tone == Tone::Working {
            let ring = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                NSPoint::new(4., 4.),
                NSSize::new(16., 16.),
            ));
            ink.colorWithAlphaComponent(0.22).setStroke();
            ring.setLineWidth(1.7);
            ring.stroke();
            ink.setStroke();
            let arc = NSBezierPath::bezierPath();
            let angle = if reduced {
                90.
            } else {
                90. - (time % 900) as f64 / 900. * 360.
            };
            arc.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(
                centre,
                8.,
                angle,
                angle - 90.,
                true,
            );
            arc.setLineWidth(1.7);
            arc.stroke();
        } else if tone == Tone::Done {
            let f = if reduced {
                1.
            } else {
                let x =
                    (time.saturating_sub(model.done_at.unwrap_or(0)) as f64 / 360.).clamp(0., 1.);
                crate::agent_launcher::ease_out(x)
            };
            let path = NSBezierPath::bezierPath();
            path.moveToPoint(NSPoint::new(7., 12.));
            if f < 0.4 {
                path.lineToPoint(NSPoint::new(7. + 3. * f / 0.4, 12. - 3. * f / 0.4));
            } else {
                path.lineToPoint(NSPoint::new(10., 9.));
                path.lineToPoint(NSPoint::new(
                    10. + 7. * (f - 0.4) / 0.6,
                    9. + 7. * (f - 0.4) / 0.6,
                ));
            }
            path.setLineWidth(1.7);
            path.stroke();
            if model.unseen {
                NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                    NSPoint::new(20., 19.),
                    NSSize::new(3., 3.),
                ))
                .fill();
            }
        } else if tone == Tone::Attention {
            let alpha = if reduced || !model.attention_unseen() || model.running_count() == 0 {
                1.
            } else {
                crate::agent_launcher::pulse_opacity((time % 1400) as f64 / 1400.)
            };
            ink.colorWithAlphaComponent(alpha).setFill();
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                NSPoint::new(8.5, 8.5),
                NSSize::new(7., 7.),
            ))
            .fill();
        } else {
            toolbar_symbol("sparkles", "Agent", &t)
                .drawInRect(NSRect::new(NSPoint::new(5., 5.), NSSize::new(14., 14.)));
        }
        if count > 1 {
            native_color(t.colors.fill).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                NSRect::new(NSPoint::new(26., 3.), NSSize::new(20., 18.)),
                9.,
                9.,
            )
            .fill();
            let label = NSString::from_str(&count.to_string());
            let attrs = objc2_foundation::NSDictionary::from_slices(
                &[unsafe { NSFontAttributeName }, unsafe {
                    NSForegroundColorAttributeName
                }],
                &[
                    &*NSFont::systemFontOfSize(10.) as &AnyObject,
                    &*native_color(t.colors.muted) as &AnyObject,
                ],
            );
            unsafe {
                label.drawAtPoint_withAttributes(NSPoint::new(30., 6.), Some(&attrs));
            }
        }
        if !reduced && let Some(at) = model.receiving_at {
            let x = time.saturating_sub(at) as f64 / 420.;
            if x < 1. {
                ink.colorWithAlphaComponent((1. - x) * 0.28).setStroke();
                let p = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                    NSPoint::new(1., 1.),
                    NSSize::new(22., 22.),
                ));
                p.setLineWidth(3.);
                p.stroke();
            }
        }
        NSGraphicsContext::restoreGraphicsState_class();
        objc2::runtime::Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(width, 24.), false, &block)
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
                    image(model, 81000, &t).drawInRect(NSRect::new(
                        NSPoint::new(20., y),
                        NSSize::new(if model.running_count() > 1 { 48. } else { 24. }, 24.),
                    ));
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
