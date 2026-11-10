//! Panic injection on the transaction path, for the tests that prove a
//! contained panic never leaves a half-applied project behind. Compiled to
//! nothing outside tests and the `fault-injection` feature.

/// Where on the transaction path a panic is injected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// In a write, before the document changes.
    BeforeApply,
    /// After the document changed, before history records it.
    AfterApply,
    /// After history recorded it, before the journal has it.
    AfterRecord,
    /// After the journal has it, before publication.
    BeforePublish,
    /// While publishing, before the publication is visible.
    InPublish,
}

#[cfg(any(test, feature = "fault-injection"))]
mod armed {
    use super::Stage;
    use std::cell::RefCell;
    use std::sync::Mutex;

    static ARMED: Mutex<Vec<(Stage, String)>> = Mutex::new(Vec::new());

    thread_local! {
        static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// The next call on `project` that reaches `stage` panics (once).
    pub fn arm(stage: Stage, project: &str) {
        ARMED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((stage, project.to_owned()));
    }

    pub(crate) fn enter(project: Option<&str>) {
        CURRENT.with(|c| *c.borrow_mut() = project.map(str::to_owned));
    }

    pub(crate) fn hit(stage: Stage) {
        let Some(project) = CURRENT.with(|c| c.borrow().clone()) else {
            return;
        };
        let mut armed = ARMED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = armed.iter().position(|(s, p)| *s == stage && *p == project) {
            armed.remove(i);
            drop(armed);
            panic!("injected fault at {stage:?}");
        }
    }
}

#[cfg(any(test, feature = "fault-injection"))]
pub use armed::arm;
#[cfg(any(test, feature = "fault-injection"))]
pub(crate) use armed::{enter, hit};

#[cfg(not(any(test, feature = "fault-injection")))]
#[inline(always)]
pub(crate) fn enter(_: Option<&str>) {}
#[cfg(not(any(test, feature = "fault-injection")))]
#[inline(always)]
pub(crate) fn hit(_: Stage) {}
