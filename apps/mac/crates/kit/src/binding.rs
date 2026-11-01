//! Generic typed edit adapter. Hosts provide their own command and document sink.
//! Keeping the model out of the kit lets extension panels use the same controls.
use crate::EditEvent;
use std::{cell::RefCell, rc::Rc};
pub trait EditSink<C> {
    /// Refuse when another transaction is active; do not mutate on refusal.
    fn begin(&mut self, label: &str, origin: &str) -> bool;
    fn dispatch(&mut self, command: C);
    fn commit(&mut self, label: &str);
    fn cancel(&mut self);
    fn apply(&mut self, commands: Vec<C>, label: &str, origin: &str);
}
pub struct CommandBinding<T, C, S> {
    sink: Rc<RefCell<S>>,
    pub label: String,
    pub origin: String,
    build: Rc<dyn Fn(&T) -> Vec<C>>,
    prepare: Option<Rc<dyn Fn()>>,
    active: bool,
}
impl<T, C, S: EditSink<C>> CommandBinding<T, C, S> {
    pub fn new(
        sink: Rc<RefCell<S>>,
        label: impl Into<String>,
        build: impl Fn(&T) -> Vec<C> + 'static,
    ) -> Self {
        Self {
            sink,
            label: label.into(),
            origin: "interface".into(),
            build: Rc::new(build),
            prepare: None,
            active: false,
        }
    }
    pub fn origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = origin.into();
        self
    }
    pub fn prepare(mut self, f: impl Fn() + 'static) -> Self {
        self.prepare = Some(Rc::new(f));
        self
    }
    /// Connect to the control's on_begin hook so a busy document aborts the gesture.
    pub fn begin(&mut self) -> bool {
        if self.active {
            return false;
        }
        if let Some(f) = &self.prepare {
            f();
        }
        self.active = self.sink.borrow_mut().begin(&self.label, &self.origin);
        self.active
    }
    /// Connect to on_edit. Begin is already handled by the refusal-capable hook.
    pub fn event(&mut self, event: &EditEvent<T>) {
        match event {
            EditEvent::Begin => {}
            EditEvent::Change(value) if self.active => {
                for c in (self.build)(value) {
                    self.sink.borrow_mut().dispatch(c);
                }
            }
            EditEvent::Commit(_) if self.active => {
                self.active = false;
                self.sink.borrow_mut().commit(&self.label);
            }
            EditEvent::Cancel(_) if self.active => {
                self.active = false;
                self.sink.borrow_mut().cancel();
            }
            _ => {}
        }
    }
    pub fn once(&mut self, value: &T) {
        if let Some(f) = &self.prepare {
            f();
        }
        self.sink
            .borrow_mut()
            .apply((self.build)(value), &self.label, &self.origin);
    }
}
/// Read the clock when building a command, not when constructing a panel.
pub enum TimeSource {
    Fixed(f64),
    Live(Rc<dyn Fn() -> f64>),
}
impl TimeSource {
    pub fn now(&self) -> f64 {
        match self {
            Self::Fixed(t) => *t,
            Self::Live(f) => f(),
        }
    }
}
/// Work-area endpoints preserve a frame between start/end and stay inside duration.
pub fn work_area(current: [f64; 2], index: usize, value: f64, duration: f64, fps: f64) -> [f64; 2] {
    let mut work = current;
    let frame = 1. / fps.max(1.);
    if index == 0 {
        work[0] = value.max(0.).min((work[1] - frame).max(0.));
    } else {
        work[1] = value.max(duration.min(work[0] + frame)).min(duration);
    }
    work
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Sink {
        busy: bool,
        log: Vec<String>,
    }
    impl EditSink<i32> for Sink {
        fn begin(&mut self, l: &str, o: &str) -> bool {
            if self.busy {
                return false;
            }
            self.busy = true;
            self.log.push(format!("begin:{l}:{o}"));
            true
        }
        fn dispatch(&mut self, c: i32) {
            self.log.push(format!("write:{c}"));
        }
        fn commit(&mut self, l: &str) {
            self.busy = false;
            self.log.push(format!("commit:{l}"));
        }
        fn cancel(&mut self) {
            self.busy = false;
            self.log.push("cancel".into());
        }
        fn apply(&mut self, c: Vec<i32>, l: &str, o: &str) {
            self.log.push(format!("once:{c:?}:{l}:{o}"));
        }
    }
    #[test]
    fn typed_command_gesture_and_click_cancel() {
        let sink = Rc::new(RefCell::new(Sink::default()));
        let mut b = CommandBinding::new(sink.clone(), "Scale", |v: &i32| vec![*v, *v / 2]);
        assert!(b.begin());
        assert!(!b.begin());
        b.event(&EditEvent::Begin);
        b.event(&EditEvent::Change(200));
        b.event(&EditEvent::Commit(200));
        assert_eq!(
            sink.borrow().log,
            vec![
                "begin:Scale:interface",
                "write:200",
                "write:100",
                "commit:Scale"
            ]
        );
        assert!(b.begin());
        b.event(&EditEvent::Cancel(200));
        assert_eq!(sink.borrow().log.last().unwrap(), "cancel");
        b.once(&80);
        assert_eq!(
            sink.borrow().log.last().unwrap(),
            "once:[80, 40]:Scale:interface"
        );
    }
    #[test]
    fn live_time_and_work_bounds() {
        let t = Rc::new(RefCell::new(1.));
        let clock = t.clone();
        let source = TimeSource::Live(Rc::new(move || *clock.borrow()));
        *t.borrow_mut() = 4.;
        assert_eq!(source.now(), 4.);
        assert_eq!(work_area([0., 5.], 0, 10., 5., 30.), [5. - 1. / 30., 5.]);
        assert_eq!(work_area([1., 5.], 1, 0., 5., 30.), [1., 1. + 1. / 30.]);
    }
}
