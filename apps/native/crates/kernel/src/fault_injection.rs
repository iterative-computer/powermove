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
    use super::Stage;
    use std::cell::RefCell;

    // Both live on the kernel actor's own thread: a panic armed for one
    // actor (`KernelHandle::arm_fault`) is never taken by another actor in
    // the same process that happens to edit a project of the same id.
    thread_local! {
        static ARMED: RefCell<Vec<(Stage, String)>> = const { RefCell::new(Vec::new()) };
        static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// On the actor thread: its next call on `project` that reaches `stage`
    /// panics (once).
    pub(crate) fn arm(stage: Stage, project: String) {
        ARMED.with(|a| a.borrow_mut().push((stage, project)));
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
            let i = a.iter().position(|(s, p)| *s == stage && *p == project)?;
            Some(a.remove(i))
        });
        if armed.is_some() {
            panic!("injected fault at {stage:?}");
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
