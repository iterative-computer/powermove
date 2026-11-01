//! A view-bound display link; no GPUI invalidation in the callback. One queued
//! tick bounds latency. Invalidate before releasing the target on window close.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObject, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use std::{
    cell::Cell,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};

#[derive(Clone, Copy)]
pub struct Tick {
    pub sequence: u64,
    pub timestamp: f64,
    pub target: f64,
}
struct State {
    sender: SyncSender<Tick>,
    sequence: Cell<u64>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMS3DisplayTarget"]
    #[ivars = State]
    struct DisplayTarget;
    impl DisplayTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            let s = self.ivars();
            let sequence = s.sequence.get();
            s.sequence.set(sequence + 1);
            let _ = s.sender.try_send(Tick { sequence, timestamp: link.timestamp(), target: link.targetTimestamp() });
        }
    }
);
pub struct Pacer {
    link: Retained<CADisplayLink>,
    _target: Retained<DisplayTarget>,
}
impl Pacer {
    pub fn new(view: &NSView, hz: f64) -> (Self, Receiver<Tick>) {
        let (sender, receiver) = sync_channel(1);
        let this = DisplayTarget::alloc(MainThreadMarker::new().unwrap()).set_ivars(State {
            sender,
            sequence: Cell::new(0),
        });
        let target: Retained<DisplayTarget> = unsafe { msg_send![super(this), init] };
        let link = unsafe { view.displayLinkWithTarget_selector(&target, sel!(tick:)) };
        link.setPreferredFrameRateRange(CAFrameRateRange::new(hz as f32, hz as f32, hz as f32));
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        (
            Self {
                link,
                _target: target,
            },
            receiver,
        )
    }
}
impl Drop for Pacer {
    fn drop(&mut self) {
        self.link.invalidate();
    }
}
