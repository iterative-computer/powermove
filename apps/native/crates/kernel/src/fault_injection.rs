//! Panic injection on the transaction path, for the tests that prove a
//! contained panic never leaves a half-applied project behind, and holds
//! there, for the tests that race other callers against a call caught
//! mid-way. Compiled to nothing outside tests and the `fault-injection`
//! feature.

/// A call held at a stage: `reached` is told when the actor gets there, and
/// the actor waits there until `release` answers (or is dropped).
#[cfg(any(test, feature = "fault-injection"))]
pub struct Hold {
    pub reached: std::sync::mpsc::Sender<()>,
    pub release: std::sync::mpsc::Receiver<()>,
}

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
    /// In the shared history budget, right after a record of some project
    /// (not necessarily the call's own) was trimmed.
    InHistoryTrim,
    /// In a save, after the file or library slot is written and before the
    /// journal is rebased on it.
    InSave,
    /// In a close, after its save and before the project is released.
    InClose,
}

#[cfg(any(test, feature = "fault-injection"))]
mod armed {
    use super::{Hold, Stage};
    use std::cell::RefCell;

    // Both live on the kernel actor's own thread: a panic armed for one
    // actor (`KernelHandle::arm_fault`) is never taken by another actor in
    // the same process that happens to edit a project of the same id.
    thread_local! {
        static ARMED: RefCell<Vec<(Stage, String, Option<Hold>)>> = const { RefCell::new(Vec::new()) };
        static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// On the actor thread: its next call on `project` that reaches `stage`
    /// panics, or with a `hold` waits there (once).
    pub(crate) fn arm(stage: Stage, project: String, hold: Option<Hold>) {
        ARMED.with(|a| a.borrow_mut().push((stage, project, hold)));
    }

    pub(crate) fn enter(project: Option<&str>) {
        CURRENT.with(|c| *c.borrow_mut() = project.map(str::to_owned));
    }

    /// `PM_KERNEL_INJECT=<Stage>:<n>` (feature builds only, for checking a
    /// running app): the n-th call on any project that reaches the stage
    /// panics, once.
    fn from_env(stage: Stage) -> bool {
        use std::sync::OnceLock;
        use std::sync::atomic::{AtomicU32, Ordering};
        static PLAN: OnceLock<Option<(String, AtomicU32)>> = OnceLock::new();
        let Some((name, left)) = PLAN.get_or_init(|| {
            let v = std::env::var("PM_KERNEL_INJECT").ok()?;
            let (name, n) = v.split_once(':')?;
            Some((name.to_owned(), AtomicU32::new(n.parse().ok()?)))
        }) else {
            return false;
        };
        name == &format!("{stage:?}")
            && left.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1)) == Ok(1)
    }

    pub(crate) fn hit(stage: Stage) {
        let Some(project) = CURRENT.with(|c| c.borrow().clone()) else {
            return;
        };
        if from_env(stage) {
            panic!("injected fault at {stage:?}");
        }
        let armed = ARMED.with(|a| {
            let mut a = a.borrow_mut();
            let i = a
                .iter()
                .position(|(s, p, _)| *s == stage && *p == project)?;
            Some(a.remove(i))
        });
        match armed {
            Some((_, _, Some(hold))) => {
                let _ = hold.reached.send(());
                let _ = hold.release.recv();
            }
            Some((_, _, None)) => panic!("injected fault at {stage:?}"),
            None => {}
        }
    }
}

#[cfg(any(test, feature = "fault-injection"))]
pub(crate) use armed::{arm, enter, hit};

#[cfg(not(any(test, feature = "fault-injection")))]
#[inline(always)]
pub(crate) fn enter(_: Option<&str>) {}
#[cfg(not(any(test, feature = "fault-injection")))]
#[inline(always)]
pub(crate) fn hit(_: Stage) {}
