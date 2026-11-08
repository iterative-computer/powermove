//! Native document close policy; the kernel remains the durability owner.
use crate::session::Controller;
use gpui::{App, Context, Entity, Window};
use pm_kernel::{KernelHandle, Published, session::*};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Window,
    Quit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseState {
    Busy,
    Saving,
    Dirty,
    Clean,
}
pub fn close_state(p: Option<&Published>, opening: bool, saving: bool) -> CloseState {
    if saving || p.is_some_and(|p| p.saving) {
        CloseState::Saving
    } else if opening || p.is_some_and(|p| p.entry.is_some()) {
        CloseState::Busy
    } else if p.is_some_and(|p| p.dirty) {
        CloseState::Dirty
    } else {
        CloseState::Clean
    }
}
struct QuitCoordinator {
    _delegate: objc2::rc::Retained<crate::platform::QuitDelegate>,
    controllers: Vec<gpui::WeakEntity<Controller>>,
    queue: Vec<gpui::WeakEntity<Controller>>,
    quitting: bool,
    /// The person chose Discard Changes for every project at once.
    discard: bool,
    host: std::sync::Arc<crate::mcp::Host>,
}
impl gpui::Global for QuitCoordinator {}
pub fn allow_quit(cx: &App) {
    cx.global::<QuitCoordinator>()._delegate.allow();
}
fn cancel_quit(cx: &mut App) {
    {
        let q = cx.global_mut::<QuitCoordinator>();
        q.quitting = false;
        q.discard = false;
        q.queue.clear();
        q.host.closing.store(false, Ordering::Release);
    }
}
pub fn begin_quit(cx: &mut App) {
    let q = cx.global_mut::<QuitCoordinator>();
    if q.quitting {
        return;
    }
    q.quitting = true;
    q.queue = q.controllers.clone();
    q.host.closing.store(true, Ordering::Release);
    let controllers = q.controllers.clone();
    let dirty: Vec<_> = controllers
        .iter()
        .filter_map(|weak| weak.upgrade())
        .filter(|entity| entity.read(cx).dirty())
        .collect();
    let [first, _, ..] = &dirty[..] else {
        return next_quit(cx);
    };
    // S002: two or more dirty projects get one question before any per-project sheet.
    use crate::surfaces::documents::{Action, review_quit};
    let alert = review_quit(dirty.len());
    let labels: Vec<_> = alert.buttons.iter().map(|b| b.label).collect();
    let rx = crate::platform::choice(
        &first.read(cx).host.borrow().window,
        &alert.message,
        &alert.informative,
        &labels,
    );
    cx.spawn(async move |cx| {
        let choice = rx.recv().await.ok();
        cx.update(|cx| match alert.action_at(choice) {
            Some(Action::ReviewChanges) => next_quit(cx),
            Some(Action::DiscardChanges) => {
                cx.global_mut::<QuitCoordinator>().discard = true;
                next_quit(cx);
            }
            _ => cancel_quit(cx),
        });
    })
    .detach();
}
fn next_quit(cx: &mut App) {
    while let Some(weak) = cx.global_mut::<QuitCoordinator>().queue.pop() {
        if weak
            .update(cx, |v, cx| v.request_close(Intent::Quit, cx))
            .is_ok()
        {
            return;
        }
    }
    let host = cx.global::<QuitCoordinator>().host.clone();
    cx.spawn(async move |cx| {
        let mut result = host.kernel.shell(ShellCall::PrepareQuit).await;
        if let Some(Ok(ShellReply::Compacted { .. })) = &result {
            // A clean quit leaves library.lock released, not to go stale.
            if let Some(Err(e)) = host.kernel.shell(ShellCall::ReleaseLibrary).await {
                result = Some(Err(e));
            }
        }
        cx.update(|cx| match result {
            // Recovery copies are durable library entries; they show in the
            // library next launch, so they never hold the quit.
            Some(Ok(ShellReply::Compacted { .. })) => {
                allow_quit(cx);
                cx.quit();
            }
            other => {
                let message = format!("Couldn’t quit: {}. Your work is kept.", reason(other));
                eprintln!("{message}");
                let controllers = cx.global::<QuitCoordinator>().controllers.clone();
                cancel_quit(cx);
                for weak in controllers {
                    let _ = weak.update(cx, |v, cx| {
                        v.message = message.clone();
                        cx.notify();
                    });
                }
            }
        });
    })
    .detach();
}
pub fn install(entity: &Entity<Controller>, window: &mut Window, cx: &mut App) {
    if cx.try_global::<QuitCoordinator>().is_none() {
        let (delegate, rx) = crate::platform::quit_guard();
        let host = entity.read(cx).mcp.clone();
        cx.set_global(QuitCoordinator {
            _delegate: delegate,
            controllers: Vec::new(),
            queue: Vec::new(),
            quitting: false,
            discard: false,
            host,
        });
        cx.spawn(async move |cx| {
            while rx.recv().await.is_ok() {
                cx.update(begin_quit);
            }
        })
        .detach();
        // SIGTERM (kill, logout, a shutdown script) quits like Quit does.
        let terminations = crate::platform::terminations();
        cx.spawn(async move |cx| {
            while terminations.recv().await.is_ok() {
                cx.update(begin_quit);
            }
        })
        .detach();
    }
    cx.global_mut::<QuitCoordinator>()
        .controllers
        .push(entity.downgrade());
    entity.update(cx, |v, cx| {
        v.close_window = Some(window.window_handle());
        cx.observe_window_activation(window, |v, window, _| {
            if window.is_window_active()
                && !v.close_pending
                && !v.opening
                && v.session
                    .as_ref()
                    .is_some_and(|s| v.kernel.published(&s.project).is_some())
            {
                v.mcp.bind(v.session.clone());
            }
        })
        .detach();
    });
    let weak = entity.downgrade();
    window.on_window_should_close(cx, move |_, cx| {
        let _ = weak.update(cx, |v, cx| v.request_close(Intent::Window, cx));
        false
    });
}
/// The kernel archives rejected work and releases the session atomically.
pub async fn discard_close(kernel: &KernelHandle, session: SessionHandle) -> Result<(), String> {
    expect_closed(kernel.shell(ShellCall::DiscardClose(session)).await)
}
fn expect_closed(
    reply: Option<Result<ShellReply, pm_kernel::KernelFailure>>,
) -> Result<(), String> {
    match reply {
        Some(Ok(ShellReply::Closed {
            recovery_copy: None,
        })) => Ok(()),
        other => Err(format!(
            "Couldn’t close: {}. Your work is kept.",
            reason(other)
        )),
    }
}
/// A person-readable reason for a kernel reply that wasn't the expected one.
fn reason(reply: Option<Result<ShellReply, pm_kernel::KernelFailure>>) -> String {
    match reply {
        Some(Err(e)) => e.to_string(),
        Some(Ok(ShellReply::Closed {
            recovery_copy: Some(_),
        })) => "the file couldn’t be written, so a recovered copy is in the library".into(),
        _ => "the project library didn’t answer".into(),
    }
}
impl Controller {
    fn dirty(&self) -> bool {
        self.session
            .as_ref()
            .and_then(|s| self.kernel.published(&s.project))
            .is_some_and(|p| p.dirty)
    }
    pub fn cancel_close(&mut self, cx: &mut Context<Self>) {
        self.close_pending = false;
        self.close_operation.take();
        if self
            .session
            .as_ref()
            .is_some_and(|s| self.kernel.published(&s.project).is_none())
        {
            self.session = None;
        }
        self.mcp.bind(self.session.clone());
        cancel_quit(cx);
        self.mcp.closing.store(false, Ordering::Release);
        cx.notify();
    }
    pub fn request_close(&mut self, intent: Intent, cx: &mut Context<Self>) {
        if self.close_pending {
            if intent == Intent::Quit {
                cancel_quit(cx);
            }
            return;
        }
        if intent == Intent::Window && cx.global::<QuitCoordinator>().quitting {
            self.message = "Finish the quit decision first.".into();
            cx.notify();
            return;
        }
        let p = self
            .session
            .as_ref()
            .and_then(|s| self.kernel.published(&s.project));
        if p.is_none() {
            self.session = None;
        }
        match close_state(
            p.as_deref(),
            self.opening
                || self.recovery_pending
                || self.host.borrow().window.attachedSheet().is_some(),
            self.save_pending,
        ) {
            CloseState::Saving => {
                self.message =
                    "Saving in progress. Please wait for it to finish before closing.".into();
                cancel_quit(cx);
                cx.notify();
                return;
            }
            CloseState::Busy => {
                self.message =
                    "Finish the active edit, open or recovery decision before closing.".into();
                cancel_quit(cx);
                cx.notify();
                return;
            }
            _ => {}
        }
        let Some(operation) = self.mcp.try_operation() else {
            self.message =
                "A project operation is in progress. Try closing again when it finishes.".into();
            cancel_quit(cx);
            cx.notify();
            return;
        };
        self.close_operation = Some(operation);
        self.close_pending = true;
        self.mcp.closing.store(true, Ordering::Release);
        if p.as_ref().is_none_or(|p| !p.dirty) {
            self.complete_close(intent, cx);
            return;
        }
        if intent == Intent::Quit && cx.global::<QuitCoordinator>().discard {
            self.discard_and_close(intent, cx);
            return;
        }
        use crate::surfaces::documents::{Action, unsaved};
        let name = p
            .as_ref()
            .and_then(|p| {
                p.snapshot
                    .root()?
                    .get("name")
                    .and_then(crate::node_string)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "Untitled".into());
        let alert = unsaved(&name);
        let labels: Vec<_> = alert.buttons.iter().map(|b| b.label).collect();
        let rx = crate::platform::choice(
            &self.host.borrow().window,
            &alert.message,
            &alert.informative,
            &labels,
        );
        cx.spawn(async move |this, cx| {
            let choice = rx.recv().await.ok();
            let _ = this.update(cx, |v, cx| match alert.action_at(choice) {
                Some(Action::Save) => {
                    v.close_after_save = Some(intent);
                    v.save(None, cx);
                }
                Some(Action::DontSave) => v.discard_and_close(intent, cx),
                _ => v.cancel_close(cx),
            });
        })
        .detach();
    }
    fn discard_and_close(&mut self, intent: Intent, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            self.complete_close(intent, cx);
            return;
        };
        let kernel = self.kernel.clone();
        self.message = "Archiving changes…".into();
        cx.spawn(async move |this, cx| {
            let result = discard_close(&kernel, session).await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(()) => {
                    v.session = None;
                    v.mcp.bind(None);
                    v.complete_close(intent, cx);
                }
                Err(e) => {
                    // If a post-release step failed, retain the window and show the failure.
                    v.message = e;
                    if let Some(old) = v.session.as_ref()
                        && let Some(p) = v.kernel.published(&old.project)
                    {
                        v.session = p.session.as_ref().map(|s| s.handle.clone());
                    }
                    v.mcp.bind(v.session.clone());
                    v.cancel_close(cx);
                }
            });
        })
        .detach();
    }
    pub fn complete_close(&mut self, intent: Intent, cx: &mut Context<Self>) {
        let kernel = self.kernel.clone();
        let session = self.session.clone();
        self.message = "Finishing save and recovery data…".into();
        cx.spawn(async move |this, cx| {
            let result = async {
                if let Some(session) = session {
                    expect_closed(kernel.shell(ShellCall::Close(session)).await)?;
                }
                if intent == Intent::Window {
                    match kernel.shell(ShellCall::DurabilityBarrier).await {
                        Some(Ok(ShellReply::Durable)) => {}
                        other => {
                            return Err(format!(
                                "Couldn’t finish closing: {}. Your work is kept.",
                                reason(other)
                            ));
                        }
                    }
                }
                Ok::<_, String>(())
            }
            .await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(()) => {
                    if let Some(viewer) = v.viewer.take() {
                        viewer.retire();
                    }
                    if let Some(popout) = v.popout.take() {
                        popout.shutdown();
                    }
                    v.session = None;
                    v.published = None;
                    v.media_refresh.reset();
                    v.media = Default::default();
                    v.media_base = Default::default();
                    v.mirror = None;
                    v.rows.clear();
                    v.saving_note = false;
                    v.selected = None;
                    v.path = None;
                    v.host
                        .borrow()
                        .set_document_metadata("Untitled", "", false, None);
                    v.mcp.bind(None);
                    v.close_operation.take();
                    if intent == Intent::Quit {
                        v.message = "Closed. Finishing quit…".into();
                        cx.defer(next_quit);
                    } else if let Some(handle) = v.close_window {
                        v.mcp.closing.store(false, Ordering::Release);
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, w, _| w.remove_window());
                        });
                    }
                }
                Err(e) => {
                    v.message = e;
                    v.cancel_close(cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::*;
    use std::{
        future::Future,
        sync::Arc,
        task::{Context as TaskContext, Poll, Wake, Waker},
    };
    fn wait<F: Future>(f: F) -> F::Output {
        struct Notify(std::thread::Thread);
        impl Wake for Notify {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }
        let w = Waker::from(Arc::new(Notify(std::thread::current())));
        let mut cx = TaskContext::from_waker(&w);
        let mut f = std::pin::pin!(f);
        loop {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => std::thread::park(),
            }
        }
    }
    fn answer(k: &KernelHandle, c: ShellCall) -> ShellReply {
        k.shell(c).wait().unwrap().unwrap()
    }
    fn fixture() -> (tempfile::TempDir, KernelActor, SessionHandle) {
        let dir = tempfile::tempdir().unwrap();
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(dir.path().join("journals")),
            ..Default::default()
        });
        let k = actor.handle();
        crate::mcp::attach(&k, dir.path()).unwrap();
        let ShellReply::Opened {session,..}=answer(&k,ShellCall::Open {id:"test".into(),source:SessionSource::New(pm_model::json::parse(r##"{"id":"test","name":"Base","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap()),electron_profile:None}) else{panic!()};
        answer(&k, ShellCall::Save(session.clone()));
        (dir, actor, session)
    }
    fn edit(k: &KernelHandle) {
        k.call_blocking(
            "test",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Rename".into()),
                    body: WriteBody::Commands(vec![
                        CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                name: Some("Rejected edit".into()),
                                ..Default::default()
                            }),
                        }
                        .into(),
                    ]),
                    expected_revision: None,
                },
            ),
        )
        .unwrap();
    }
    #[test]
    fn dont_save_library_archives_and_quit_never_resaves_rejected_work() {
        let (dir, actor, session) = fixture();
        let k = actor.handle();
        let base = std::fs::read(dir.path().join("store/project.test.json")).unwrap();
        edit(&k);
        wait(discard_close(&k, session)).unwrap();
        assert!(
            std::fs::read_dir(dir.path().join("journals/test/discarded"))
                .unwrap()
                .next()
                .is_some()
        );
        answer(&k, ShellCall::PrepareQuit);
        let stored = std::fs::read(dir.path().join("store/project.test.json")).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&stored).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&base).unwrap()
        );
        actor.shutdown();
    }
    #[test]
    fn dont_save_file_keeps_original_bytes_after_quit() {
        let (dir, actor, session) = fixture();
        let k = actor.handle();
        let path = dir.path().join("project.pmv");
        answer(
            &k,
            ShellCall::SaveAs {
                session: session.clone(),
                path: path.clone(),
                replace: false,
            },
        );
        let base = std::fs::read(&path).unwrap();
        edit(&k);
        wait(discard_close(&k, session)).unwrap();
        answer(&k, ShellCall::PrepareQuit);
        assert_eq!(std::fs::read(path).unwrap(), base);
        actor.shutdown();
    }
    #[test]
    fn save_flight_refuses_close_and_quit_then_worker_completion_allows_them() {
        let (dir, actor, session) = fixture();
        let k = actor.handle();
        let path = dir.path().join("project.pmv");
        edit(&k);
        let save = k
            .prepare_save(session.clone(), Some(path))
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(
            close_state(k.published("test").as_deref(), false, false),
            CloseState::Saving
        );
        assert!(
            k.shell(ShellCall::Close(session.clone()))
                .wait()
                .unwrap()
                .is_err()
        );
        assert!(k.shell(ShellCall::PrepareQuit).wait().unwrap().is_err());
        k.finish_save(save.run()).wait().unwrap().unwrap();
        assert_eq!(
            close_state(k.published("test").as_deref(), false, false),
            CloseState::Clean
        );
        answer(&k, ShellCall::Close(session));
        answer(&k, ShellCall::PrepareQuit);
        actor.shutdown();
    }
    #[test]
    fn dirty_and_active_edit_require_explicit_choice_or_deferral() {
        let (_dir, actor, _session) = fixture();
        let k = actor.handle();
        edit(&k);
        let p = k.published("test").unwrap();
        assert_eq!(close_state(Some(&p), false, false), CloseState::Dirty);
        assert_eq!(close_state(Some(&p), true, false), CloseState::Busy);
        assert_eq!(close_state(Some(&p), false, true), CloseState::Saving);
        k.call_blocking(
            "test",
            Request::new(
                Caller::native(),
                GestureCall::Begin {
                    label: "Drag".into(),
                },
            ),
        )
        .unwrap();
        assert_eq!(
            close_state(k.published("test").as_deref(), false, false),
            CloseState::Busy
        );
        actor.shutdown();
    }
}
