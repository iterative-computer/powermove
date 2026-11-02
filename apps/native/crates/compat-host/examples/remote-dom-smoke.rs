//! Synthetic production-applier probe. Owns and closes its AppKit window.
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString};
use pm_compat_host::*;
use pm_remote_dom::{
    bounded_channel,
    protocol::{PageMessage, Session},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct Inbox {
    events: RefCell<Vec<HostEvent>>,
    feedback: mpsc::SyncSender<Value>,
}
impl SyncHandler for Inbox {
    fn call(&self, _: &RequestContext, _: &str, _: Value) -> Result<Value> {
        Err(Error::Forbidden)
    }
    fn event(&self, event: HostEvent) {
        self.events.borrow_mut().push(event);
    }
    fn remote_dom(&self, _: &RequestContext, payload: Value) -> Result<()> {
        self.feedback
            .try_send(payload)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Error::Backpressure,
                mpsc::TrySendError::Disconnected(_) => Error::StaleGeneration,
            })
    }
}
fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mtm = MainThreadMarker::new().ok_or("main thread required")?;
    let bundle = std::fs::read_to_string(std::env::args().nth(1).ok_or("page bundle required")?)?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.finishLaunching();
    let (input, feedback) = mpsc::sync_channel(8);
    let inbox = Rc::new(Inbox {
        events: RefCell::new(vec![]),
        feedback: input,
    });
    let mut host = MacDomHost::new_remote_dom(
        mtm,
        Trust::Trusted,
        inbox.clone(),
        &mut StoreDataStores::default(),
        &bundle,
    )?;
    host.create(PanelSpec {
        id: 1,
        label: "Synthetic remote DOM".into(),
    })?;
    let rect = Rect::new(0., 0., 320., 180.);
    host.attach(
        1,
        PanelSlot {
            id: 1,
            bounds: rect,
            clip: rect,
            reading_order: 0,
        },
    )?;
    host.present(rect)?;
    let window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(80., 80.), NSSize::new(320., 180.)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Synthetic remote DOM"));
    window.setContentView(Some(&host.accessibility_view()));
    window.makeKeyAndOrderFront(None);

    let (mut page, mutations) = bounded_channel(8)?;
    let (mut realm, received) = bounded_channel(8)?;
    let (failure, failures) = mpsc::sync_channel(1);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let worker = thread::spawn(move || {
        let result = (|| -> Result<()> {
            let session = Session {
                panel: "synthetic".into(),
                generation: 1,
            };
            let mut router = RemoteDomPanel::new(1, session.clone())?;
            router.enqueue(
                json!({"version":1,"session":session,"seq":1,"caused_by":null,"ops":[
                {"op":"create","id":6,"kind":"element","name":"output","namespace":null},
                {"op":"create","id":7,"kind":"text","name":"","namespace":null},
                {"op":"text","id":7,"value":"Synthetic WebKit remote DOM"},
                {"op":"insert","id":7,"parent":6,"before":null},
                {"op":"insert","id":6,"parent":4,"before":null}]}),
                &mut page,
            )?;
            while !stopped.load(Ordering::Relaxed) {
                match feedback.recv_timeout(Duration::from_millis(10)) {
                    Ok(value) => router.receive(value, &mut realm)?,
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = failure.try_send(error);
        }
    });
    let started = Instant::now();
    let mut ready = false;
    let mut pending = None;
    let result = (|| -> std::result::Result<(), Box<dyn std::error::Error>> {
        while started.elapsed() < Duration::from_secs(15) {
            let event = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::dateWithTimeIntervalSinceNow(0.01)),
                unsafe { NSDefaultRunLoopMode },
                true,
            );
            if let Some(event) = event {
                app.sendEvent(&event);
            }
            app.updateWindows();
            host.tick();
            let events = std::mem::take(&mut *inbox.events.borrow_mut());
            for event in events {
                match event {
                    HostEvent::Ready { .. } => ready = true,
                    HostEvent::Failure(reason) => return Err(reason.into()),
                    _ => (),
                }
            }
            if let Ok(error) = failures.try_recv() {
                return Err(error.into());
            }
            if ready {
                if pending.is_none() {
                    pending = mutations.try_recv().ok();
                }
                if let Some(batch) = &pending {
                    match batch.present(&mut host) {
                        Ok(()) => pending = None,
                        Err(Error::Backpressure) => (),
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            while let Ok(message) = received.try_recv() {
                if matches!(message, PageMessage::Applied { seq: 1, .. }) {
                    println!("remote_dom_webkit_applied=1");
                    return Ok(());
                }
            }
        }
        Err("WebKit acknowledgement timeout".into())
    })();
    window.close();
    drop(host);
    stop.store(true, Ordering::Relaxed);
    if worker.join().is_err() {
        return Err("routing worker failed".into());
    }
    result
}
fn main() {
    if let Err(error) = objc2::rc::autoreleasepool(|_| run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
