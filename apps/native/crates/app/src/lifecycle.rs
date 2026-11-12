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
    /// An internal error stopped the project: it closes without asking and
    /// without saving; its journal is kept for the next open to recover.
    Stopped,
}
pub fn close_state(p: Option<&Published>, opening: bool, saving: bool) -> CloseState {
    if saving || p.is_some_and(|p| p.saving) {
        CloseState::Saving
    } else if p.is_some_and(|p| p.fault.is_some()) {
        CloseState::Stopped
    } else if opening || p.is_some_and(|p| p.entry.is_some()) {
        CloseState::Busy
    } else if p.is_some_and(|p| p.dirty) {
        CloseState::Dirty
    } else {
        CloseState::Clean
    }
}
/// What a Quit waits for at a window that cannot close this moment, when
/// that is work which ends on its own: a save in flight, a project still
/// opening, another project operation. The quit goes on when it has ended
/// (a second Quit offers to quit now meanwhile). `None`: nothing to wait
/// for, or the window is asking the person something (a recovery choice, an
/// alert, an open edit), which only they can end.
fn quit_waits_for(
    state: CloseState,
    opening: bool,
    asking: bool,
    fenced: bool,
) -> Option<&'static str> {
    match state {
        CloseState::Saving => Some("finishing the save"),
        CloseState::Busy if opening && !asking => Some("finishing opening the project"),
        CloseState::Busy | CloseState::Stopped => None,
        CloseState::Dirty | CloseState::Clean if fenced => Some("finishing a project operation"),
        CloseState::Dirty | CloseState::Clean => None,
    }
}
struct QuitCoordinator {
    _delegate: objc2::rc::Retained<crate::platform::QuitDelegate>,
    controllers: Vec<gpui::WeakEntity<Controller>>,
    queue: Vec<gpui::WeakEntity<Controller>>,
    quitting: bool,
    /// The person chose Discard Changes for every project at once.
    discard: bool,
    /// The running-export question is on screen.
    asking_exports: bool,
    /// A second Quit's prompt is showing: the window and its sheet, ended
    /// before the app terminates (AppKit refuses to while a sheet is up).
    asking_now: Option<(
        objc2::rc::Retained<objc2_app_kit::NSWindow>,
        objc2::rc::Retained<objc2_app_kit::NSWindow>,
    )>,
    host: std::sync::Arc<crate::mcp::Host>,
}
impl gpui::Global for QuitCoordinator {}
pub fn allow_quit(cx: &App) {
    let delegate = &cx.global::<QuitCoordinator>()._delegate;
    delegate.allow();
    delegate.resolve_quit_events(false);
}
fn cancel_quit(cx: &mut App) {
    cx.global::<QuitCoordinator>()
        ._delegate
        .resolve_quit_events(true);
    let controllers = {
        let q = cx.global_mut::<QuitCoordinator>();
        q.quitting = false;
        q.discard = false;
        q.asking_now = None;
        q.queue.clear();
        q.host.closing.store(false, Ordering::Release);
        q.controllers.clone()
    };
    // Extensions a hung one made the quit stop come back with the window,
    // once the update that cancelled (a close sheet's answer, inside one
    // controller) has ended: a controller can't be updated within itself.
    cx.defer(move |cx| {
        for weak in controllers {
            let _ = weak.update(cx, |v, cx| {
                if v.closing_note.take().is_some() {
                    cx.notify();
                }
                if v.extensions.stopped_for_quit {
                    v.start_extensions(cx);
                }
            });
        }
    });
}
/// The exports a quit would stop: every export that has not ended.
fn running_exports(
    tasks: &[pm_promedia::tasks::Task],
) -> Vec<(pm_promedia::tasks::TaskId, String)> {
    tasks
        .iter()
        .filter(|t| t.kind == pm_promedia::tasks::TaskKind::Export && !t.state.terminal())
        .map(|t| (t.id, t.title.clone()))
        .collect()
}
/// The question a quit asks while an export runs: quitting would stop it.
fn export_quit_alert(titles: &[String]) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button, Key};
    let (message, button) = match titles {
        [one] if !one.is_empty() => (
            format!("“{one}” is still exporting."),
            "Stop Export and Quit",
        ),
        [_] => ("An export is still running.".into(), "Stop Export and Quit"),
        many => (
            format!("{} exports are still running.", many.len()),
            "Stop Exports and Quit",
        ),
    };
    Alert::new(
        message,
        "If you quit now, the export stops and its unfinished file is removed. Your projects are not affected.",
        vec![
            Button::new("Cancel", false).default().key(Key::Escape),
            Button::new(button, true),
        ],
    )
}
pub fn begin_quit(cx: &mut App) {
    let q = cx.global_mut::<QuitCoordinator>();
    if q.quitting {
        return ask_quit_now(cx);
    }
    if q.asking_exports {
        return;
    }
    // An export that is running is asked about before anything closes.
    let exporting = running_exports(&crate::export_coordinator::tasks().lock().unwrap().all());
    let window = q
        .controllers
        .iter()
        .find_map(|weak| weak.upgrade())
        .map(|c| c.read(cx).host.borrow().window.clone());
    if let (false, Some(window)) = (exporting.is_empty(), window) {
        cx.global_mut::<QuitCoordinator>().asking_exports = true;
        let titles: Vec<String> = exporting.iter().map(|(_, title)| title.clone()).collect();
        let answer = export_quit_alert(&titles).sheet(&window);
        cx.spawn(async move |cx| {
            let stop = answer.recv().await.ok().flatten() == Some(true);
            cx.update(|cx| {
                cx.global_mut::<QuitCoordinator>().asking_exports = false;
                if stop {
                    for (id, _) in exporting {
                        crate::export_coordinator::cancel_export(id);
                    }
                    start_quit(cx);
                } else {
                    cancel_quit(cx);
                }
            });
        })
        .detach();
        return;
    }
    start_quit(cx);
}
fn start_quit(cx: &mut App) {
    let q = cx.global_mut::<QuitCoordinator>();
    q.quitting = true;
    q.queue = q.controllers.clone();
    q.host.closing.store(true, Ordering::Release);
    let controllers = q.controllers.clone();
    let dirty: Vec<_> = controllers
        .iter()
        .filter_map(|weak| weak.upgrade())
        .filter(|entity| entity.read(cx).dirty())
        .collect();
    let asked: Vec<_> = dirty.iter().map(Entity::entity_id).collect();
    let q = cx.global_mut::<QuitCoordinator>();
    q.queue = questions_first(std::mem::take(&mut q.queue), |weak| {
        asked.contains(&weak.entity_id())
    });
    let [first, _, ..] = &dirty[..] else {
        return next_quit(cx);
    };
    // S002: two or more dirty projects get one question before any per-project sheet.
    use crate::surfaces::documents::{Action, review_quit};
    let rx = review_quit(dirty.len()).sheet(&first.read(cx).host.borrow().window);
    cx.spawn(async move |cx| {
        let choice = rx.recv().await.ok().flatten();
        cx.update(|cx| match choice {
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
/// The order a quit takes its windows in (from the end of the list): those
/// with unsaved changes before any clean one. Their Save question is then on
/// screen at once instead of after every clean project has closed, and
/// Cancel leaves every window as it was.
fn questions_first<T>(windows: Vec<T>, unsaved: impl Fn(&T) -> bool) -> Vec<T> {
    let (asked, clean): (Vec<T>, Vec<T>) = windows.into_iter().partition(|w| unsaved(w));
    clean.into_iter().chain(asked).collect()
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
    // The window session's queued writes land first; a failed one is asked
    // about while the library is still held, so Cancel leaves all as it was.
    cx.spawn(async move |cx| {
        let flushed = cx
            .background_executor()
            .spawn(async { pm_shell::session::flush() })
            .await;
        cx.update(|cx| match flushed {
            Ok(()) => finish_quit(cx),
            Err(error) => ask_session_failed(&error, cx),
        });
    })
    .detach();
}
/// What a second Quit does while the first one runs.
#[derive(Debug, PartialEq)]
enum SecondQuit {
    /// The person is being asked something (a save question), or already
    /// asked to quit now: the question on screen is the answer.
    Ignore,
    /// The quit waits for this work: offer to quit now.
    Ask(String),
}
fn second_quit(asking_now: bool, waiting: Option<String>) -> SecondQuit {
    match waiting {
        Some(note) if !asking_now => SecondQuit::Ask(note),
        _ => SecondQuit::Ignore,
    }
}
/// The prompt of a second Quit while the first waits for `note`.
fn quit_now_alert(note: &str) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button, Key};
    let work = note.split_once(": ").map_or(note, |(_, work)| work);
    let work = work.trim_end_matches('…');
    Alert::new(
        "Powermove is still finishing before it quits.",
        format!(
            "Still {work}. If you quit now, no work is lost: the next time you open the project, Powermove offers to recover your unsaved changes."
        ),
        vec![
            Button::new("Keep Waiting", false)
                .default()
                .key(Key::Escape),
            Button::new("Quit Now", true),
        ],
    )
}
/// A second Quit while the first waits on work (archiving, saving, the
/// library): says what it waits for and offers to quit now. Quitting now
/// flushes the journals and leaves everything as a crash would: the journal
/// keeps unsaved work, consistent with whichever file a running save left,
/// and the next open offers to recover it.
fn ask_quit_now(cx: &mut App) {
    let q = cx.global::<QuitCoordinator>();
    let waiting = q.controllers.iter().find_map(|weak| {
        let c = weak.upgrade()?;
        let note = c.read(cx).closing_note.clone()?;
        Some((note, c.read(cx).host.borrow().window.clone()))
    });
    let (note, window) = match waiting {
        Some((note, window)) => (Some(note), Some(window)),
        None => (None, None),
    };
    let (SecondQuit::Ask(note), Some(window)) = (second_quit(q.asking_now.is_some(), note), window)
    else {
        return;
    };
    let mut sheet = None;
    let answer = quit_now_alert(&note).sheet_with(&window, |alert| sheet = Some(alert.window()));
    cx.global_mut::<QuitCoordinator>().asking_now = sheet.map(|sheet| (window, sheet));
    cx.spawn(async move |cx| {
        let now = answer.recv().await.ok().flatten() == Some(true);
        let kernel = cx.update(|cx| {
            let q = cx.global_mut::<QuitCoordinator>();
            q.asking_now = None;
            (now && q.quitting).then(|| q.host.kernel.clone())
        });
        let Some(kernel) = kernel else {
            return;
        };
        quit_as_is(kernel, note, cx).await;
    })
    .detach();
}
/// Quits without the work the quit was waiting for, leaving everything as a
/// crash would: every edit the kernel acknowledged is already written to its
/// journal and is made durable here without waiting for the busy kernel.
/// Saves journal their save point before replacing a file, so the journal
/// replays onto whatever the disk holds, and the next open offers it.
async fn quit_as_is(kernel: KernelHandle, note: String, cx: &mut gpui::AsyncApp) {
    let synced = cx
        .background_executor()
        .spawn(async move { kernel.sync_journals() })
        .await;
    cx.update(|cx| {
        let q = cx.global_mut::<QuitCoordinator>();
        // The quit may have been cancelled (a failed save) meanwhile.
        if !q.quitting {
            return;
        }
        q._delegate.resolve_quit_events(false);
        if let Err(e) = synced {
            eprintln!("Quit Now: the journal could not be flushed to disk: {e}");
        }
        eprintln!("Quit Now: {note}");
        std::process::exit(0);
    });
}
/// The quit alert when the project library could not be written or released.
/// Nothing in a window can fix it by then (every project's own close was
/// answered), so the person is never left with a quit that only refuses.
fn quit_failed_alert(reason: &str) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button, Key};
    // The kernel's account chooses the sentence and goes to the Error Log.
    let why = crate::surfaces::documents::WriteFailure::of(reason).sentence();
    Alert::new(
        "Powermove couldn’t finish saving before quitting.",
        format!(
            "{why} If you quit anyway, no work is lost: the next time you open a project with unsaved changes, Powermove offers to recover them. Window ▸ Error Log has the details."
        ),
        vec![
            Button::new("Cancel", false).default().key(Key::Escape),
            Button::new("Quit Anyway", true),
        ],
    )
}
/// The quit alert for a window session that couldn't be written.
fn session_failed_alert(error: &str) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button, Key};
    Alert::new(
        "Powermove couldn’t save your window arrangement.",
        format!("{error}. If you quit anyway, your windows open as they were last saved."),
        vec![
            Button::new("Cancel", false).default().key(Key::Escape),
            Button::new("Quit Anyway", true),
        ],
    )
}
fn ask_session_failed(error: &str, cx: &mut App) {
    let window = cx
        .global::<QuitCoordinator>()
        .controllers
        .iter()
        .find_map(|weak| weak.upgrade())
        .map(|c| c.read(cx).host.borrow().window.clone());
    let Some(window) = window else {
        return finish_quit(cx);
    };
    let answer = session_failed_alert(error).sheet(&window);
    cx.spawn(async move |cx| {
        let quit = answer.recv().await.ok().flatten() == Some(true);
        cx.update(|cx| {
            if quit {
                finish_quit(cx)
            } else {
                cancel_quit(cx)
            }
        });
    })
    .detach();
}
/// Compacts and releases the library, then quits.
fn finish_quit(cx: &mut App) {
    let host = cx.global::<QuitCoordinator>().host.clone();
    for weak in cx.global::<QuitCoordinator>().controllers.clone() {
        let _ = weak.update(cx, |v, cx| {
            v.closing_note = Some("Quitting: writing the project library…".into());
            cx.notify();
        });
    }
    cx.spawn(async move |cx| {
        let mut result = host.kernel.shell(ShellCall::PrepareQuit).await;
        if let Some(Ok(ShellReply::Compacted { .. })) = &result {
            // A clean quit leaves library.lock released, not to go stale.
            if let Some(Err(e)) = host.kernel.shell(ShellCall::ReleaseLibrary).await {
                result = Some(Err(e));
            }
        }
        if let Some(Ok(ShellReply::Compacted { .. })) = &result {
            // The Error Log's store: this quit was not a crash.
            crate::error_log::session_ended().await;
        }
        cx.update(|cx| match result {
            // Recovery copies are durable library entries; they show in the
            // library next launch, so they never hold the quit.
            Some(Ok(ShellReply::Compacted { .. })) => {
                // A second Quit's prompt would hold the termination.
                if let Some((window, sheet)) = cx.global_mut::<QuitCoordinator>().asking_now.take()
                {
                    window.endSheet_returnCode(&sheet, objc2_app_kit::NSModalResponseAbort);
                }
                allow_quit(cx);
                cx.quit();
            }
            other => {
                let reason = reason(other);
                eprintln!("Couldn’t quit: {reason}. Your work is kept.");
                crate::error_log::shared(cx).borrow_mut().push(
                    "Quit",
                    &format!("The project library couldn’t be written before quitting: {reason}"),
                );
                let q = cx.global::<QuitCoordinator>();
                let kernel = q.host.kernel.clone();
                let window = q
                    .controllers
                    .iter()
                    .find_map(|weak| weak.upgrade())
                    .map(|c| c.read(cx).host.borrow().window.clone());
                let Some(window) = window else {
                    return cancel_quit(cx);
                };
                let answer = quit_failed_alert(&reason).sheet(&window);
                cx.spawn(async move |cx| {
                    if answer.recv().await.ok().flatten() == Some(true) {
                        quit_as_is(kernel, format!("Quitting: {reason}"), cx).await;
                    } else {
                        cx.update(cancel_quit);
                    }
                })
                .detach();
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
            asking_exports: false,
            asking_now: None,
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
            // The key window is where MCP acts.
            if window.is_window_active() {
                v.mcp.activate_window(v.window_key);
            }
            if window.is_window_active()
                && !v.close_pending
                && !v.opening
                && v.session
                    .as_ref()
                    .is_some_and(|s| v.kernel.published(&s.project).is_some())
            {
                v.mcp.bind_window(v.window_key, v.session.clone());
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
pub(crate) fn reason(reply: Option<Result<ShellReply, pm_kernel::KernelFailure>>) -> String {
    match reply {
        Some(Err(e)) => e.to_string(),
        Some(Ok(ShellReply::Closed {
            recovery_copy: Some(_),
        })) => "the file couldn’t be written, so a recovered copy is in the library".into(),
        _ => "the project library didn’t answer".into(),
    }
}
/// The status line while a close or quit waits for `work`.
fn closing(intent: Intent, work: &str) -> String {
    match intent {
        Intent::Quit => format!("Quitting: {work}…"),
        Intent::Window => format!("Closing: {work}…"),
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
        self.closing_note = None;
        self.close_operation.take();
        if self
            .session
            .as_ref()
            .is_some_and(|s| self.kernel.published(&s.project).is_none())
        {
            self.session = None;
        }
        self.mcp.bind_window(self.window_key, self.session.clone());
        cancel_quit(cx);
        self.mcp.closing.store(false, Ordering::Release);
        cx.notify();
    }
    /// Commits an open text edit on the canvas, as v1 flushed edits before a
    /// save or close; `then` runs once the project holds no open entry (or
    /// after three seconds). `false` when nothing was being edited.
    pub(crate) fn finish_edits_then(
        &mut self,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) -> bool {
        let Some(gizmos) = self.builtins.gizmos.clone() else {
            return false;
        };
        if !gizmos.update(cx, |g, cx| g.finish_text(true, cx)) {
            return false;
        }
        let kernel = self.kernel.clone();
        let project = self.session.as_ref().map(|s| s.project.clone());
        cx.spawn(async move |this, cx| {
            for _ in 0..60 {
                let open = project
                    .as_ref()
                    .and_then(|p| kernel.published(p))
                    .is_some_and(|p| p.entry.is_some());
                let editing = this
                    .update(cx, |v, cx| {
                        v.builtins
                            .gizmos
                            .as_ref()
                            .is_some_and(|g| g.read(cx).text_open().is_some())
                    })
                    .unwrap_or(false);
                if !open && !editing {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            let _ = this.update(cx, |v, cx| then(v, cx));
        })
        .detach();
        true
    }
    pub fn request_close(&mut self, intent: Intent, cx: &mut Context<Self>) {
        if self.close_pending {
            if intent == Intent::Quit {
                cancel_quit(cx);
            }
            return;
        }
        if self.finish_edits_then(cx, move |v, cx| v.request_close(intent, cx)) {
            return;
        }
        if intent == Intent::Quit && self.stop_hung_extensions_for_quit() {
            // The stopped realm's open entry is sealed on its owner thread;
            // the quit goes on once it has gone (or after three seconds).
            let kernel = self.kernel.clone();
            let project = self.session.as_ref().map(|s| s.project.clone());
            cx.spawn(async move |this, cx| {
                for _ in 0..60 {
                    let open = project
                        .as_ref()
                        .and_then(|p| kernel.published(p))
                        .is_some_and(|p| p.entry.is_some());
                    if !open {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                }
                let _ = this.update(cx, |v, cx| v.request_close(Intent::Quit, cx));
            })
            .detach();
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
        // A second Quit's own prompt on this window (Quit Now, while the
        // quit waits) is not a question that holds the close: it goes once
        // there is nothing left to wait for.
        let quit_now_here = intent == Intent::Quit
            && cx
                .global::<QuitCoordinator>()
                .asking_now
                .as_ref()
                .is_some_and(|(window, _)| **window == *self.host.borrow().window);
        let asking = self.recovery_pending
            || (!quit_now_here && !crate::alert::dismiss_notice(&self.host.borrow().window));
        let state = close_state(p.as_deref(), self.opening || asking, self.save_pending);
        // A quit is not refused because of work that ends on its own: it
        // waits (saying what for) and goes on when the work has ended.
        let fenced = matches!(state, CloseState::Dirty | CloseState::Clean)
            && self.mcp.operation_in_progress();
        if intent == Intent::Quit
            && let Some(work) = quit_waits_for(state, self.opening, asking, fenced)
        {
            self.closing_note = Some(closing(Intent::Quit, work));
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let _ = this.update(cx, |v, cx| {
                    // Cancelled meanwhile: nothing to go on with.
                    if cx.global::<QuitCoordinator>().quitting {
                        v.request_close(Intent::Quit, cx);
                    }
                });
            })
            .detach();
            return;
        }
        if quit_now_here
            && let Some((window, sheet)) = cx.global_mut::<QuitCoordinator>().asking_now.take()
        {
            window.endSheet_returnCode(&sheet, objc2_app_kit::NSModalResponseAbort);
        }
        match state {
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
        if state == CloseState::Stopped || p.as_ref().is_none_or(|p| !p.dirty) {
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
        let rx = unsaved(&name).sheet(&self.host.borrow().window);
        cx.spawn(async move |this, cx| {
            let choice = rx.recv().await.ok().flatten();
            let _ = this.update(cx, |v, cx| match choice {
                Some(Action::Save) => {
                    v.close_after_save = Some(intent);
                    v.closing_note = Some(closing(intent, "saving"));
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
        self.closing_note = Some(closing(intent, "archiving changes"));
        cx.spawn(async move |this, cx| {
            let result = discard_close(&kernel, session).await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(()) => {
                    v.session = None;
                    v.mcp.bind_window(v.window_key, None);
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
                    v.mcp.bind_window(v.window_key, v.session.clone());
                    v.cancel_close(cx);
                }
            });
        })
        .detach();
    }
    pub fn complete_close(&mut self, intent: Intent, cx: &mut Context<Self>) {
        let kernel = self.kernel.clone();
        let session = self.session.clone();
        // A discarded close has already released its session; its project
        // is still published here until the close finishes.
        let had_project = session.is_some() || self.published.is_some();
        self.closing_note = Some(closing(intent, "finishing save and recovery data"));
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
                    // The playback feed owns the viewer's placement binding,
                    // whose drawableSize observer retains the layer and its
                    // drawable pool. Nothing else wakes the feed now (the
                    // display link is paused and the retired viewer sends
                    // nothing), so tell it the viewer is gone.
                    if let Some(wake) = &v.playback_wake {
                        let _ = wake.try_send(());
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
                    v.mcp.bind_window(v.window_key, None);
                    v.close_operation.take();
                    // Panel owners dispose asynchronously, and GPUI keeps
                    // the previous frame until the next draw. Collect on the
                    // UI thread once those references have had time to go;
                    // the render worker cannot collect this thread's heap.
                    cx.spawn(async move |_, cx| {
                        for delay in [250, 1750] {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(delay))
                                .await;
                            cx.update(|_| pm_engine_service::memory::pressure_relief());
                        }
                    })
                    .detach();
                    if intent == Intent::Quit {
                        v.closing_note = Some(closing(Intent::Quit, "finishing up"));
                        cx.defer(next_quit);
                    } else if had_project && crate::entry::editors(cx).len() <= 1 {
                        // v1: closing the last project shows Home; the
                        // window stays, its session forgets the project.
                        v.mcp.closing.store(false, Ordering::Release);
                        v.close_pending = false;
                        v.closing_note = None;
                        v.home = false;
                        v.start_builtins(cx);
                        let layer = metal::MetalLayer::new();
                        if pm_engine_service::layers::configure_layer(
                            objc2::MainThreadMarker::new().unwrap(),
                            &layer,
                            &v.engine.metal_device(),
                        )
                        .is_ok()
                        {
                            v.host.borrow_mut().replace_viewer_layer(layer);
                            v.main_output.set(None);
                        }
                        v.message = "No project".into();
                        if let Some(shell) = v.shell.as_ref().and_then(gpui::WeakEntity::upgrade) {
                            shell.update(cx, |s, _| s.bind_project("shell-untitled".into()));
                        }
                        cx.notify();
                    } else if let Some(handle) = v.close_window {
                        if let Some(shell) = v.shell.as_ref().and_then(gpui::WeakEntity::upgrade) {
                            shell.update(cx, |s, _| s.forget_session());
                        }
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

    /// A second Quit while the first waits on work names that work and
    /// offers to quit now; while a question is on screen it is the answer.
    #[test]
    fn a_second_quit_offers_to_quit_now_while_the_first_waits() {
        assert_eq!(second_quit(false, None), SecondQuit::Ignore);
        assert_eq!(
            second_quit(true, Some("Quitting: saving…".into())),
            SecondQuit::Ignore
        );
        let note = closing(Intent::Quit, "archiving changes");
        assert_eq!(note, "Quitting: archiving changes…");
        assert_eq!(
            second_quit(false, Some(note.clone())),
            SecondQuit::Ask(note.clone())
        );
        let alert = quit_now_alert(&note);
        assert_eq!(alert.labels(), ["Keep Waiting", "Quit Now"]);
        assert!(alert.buttons[0].default && !alert.buttons[0].action);
        assert!(alert.buttons[1].action);
        assert!(
            alert
                .informative
                .starts_with("Still archiving changes. If you quit now")
        );
        assert!(alert.informative.contains("no work is lost"));
    }

    #[test]
    fn a_window_session_that_fails_to_save_asks_before_quitting() {
        // A session file the writer must not overwrite (a newer version):
        // the flush quit runs fails, and the quit alert carries why.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, br#"{"version":9,"windows":[]}"#).unwrap();
        pm_shell::session::write(&path, pm_shell::session::Change::Remove(1));
        let error = pm_shell::session::flush().unwrap_err();
        let _ = pm_shell::session::take_error();
        let alert = session_failed_alert(&error);
        assert_eq!(alert.labels(), ["Cancel", "Quit Anyway"]);
        assert!(alert.buttons[0].default && !alert.buttons[0].action);
        assert!(alert.buttons[1].action);
        assert!(alert.informative.contains("Unsupported window session"));
    }
    /// A Quit at a window that is opening a project, saving, or behind
    /// another project operation waits for it; one at a window that asks
    /// the person something, or has an edit open, does not.
    #[test]
    fn a_quit_waits_for_work_that_ends_on_its_own() {
        use CloseState::*;
        assert_eq!(
            quit_waits_for(Busy, true, false, false),
            Some("finishing opening the project")
        );
        assert_eq!(
            quit_waits_for(Saving, false, false, false),
            Some("finishing the save")
        );
        assert_eq!(
            quit_waits_for(Clean, false, false, true),
            Some("finishing a project operation")
        );
        assert_eq!(
            quit_waits_for(Dirty, false, false, true),
            Some("finishing a project operation")
        );
        // A recovery choice or an alert on screen, an open edit, a stopped
        // project, and a window that can simply close.
        assert_eq!(quit_waits_for(Busy, true, true, false), None);
        assert_eq!(quit_waits_for(Busy, false, false, false), None);
        assert_eq!(quit_waits_for(Stopped, false, false, false), None);
        assert_eq!(quit_waits_for(Clean, false, false, false), None);
        assert_eq!(quit_waits_for(Dirty, false, false, false), None);
    }
    /// A quit while an export runs asks first: Cancel is the default, and
    /// only exports that have not ended count.
    #[test]
    fn a_quit_during_an_export_asks_before_stopping_it() {
        use pm_promedia::tasks::{BackgroundTasks, ProgressUnit, TaskKind, TaskSpec};
        let mut tasks = BackgroundTasks::default();
        let mut add = |kind, title: &str| {
            tasks
                .enqueue(
                    "w".into(),
                    "p".into(),
                    TaskSpec {
                        title: title.into(),
                        kind,
                        unit: ProgressUnit::Frames,
                        priority: 0,
                        total: 10,
                    },
                )
                .unwrap()
        };
        let export = add(TaskKind::Export, "Launch Film");
        add(TaskKind::Proxy, "clip");
        let done = add(TaskKind::Export, "Old");
        tasks.cancel(done).unwrap();
        let running = running_exports(&tasks.all());
        assert_eq!(running, [(export, "Launch Film".to_owned())]);
        let alert = export_quit_alert(&["Launch Film".to_owned()]);
        assert_eq!(alert.message, "“Launch Film” is still exporting.");
        assert_eq!(alert.labels(), ["Cancel", "Stop Export and Quit"]);
        assert!(alert.buttons[0].default && !alert.buttons[0].action);
        assert!(alert.buttons[1].action);
        let two = export_quit_alert(&["a".to_owned(), "b".to_owned()]);
        assert_eq!(two.labels(), ["Cancel", "Stop Exports and Quit"]);
    }
    /// A quit asks every window with unsaved changes before it closes a
    /// clean one (windows are taken from the end).
    #[test]
    fn a_quit_asks_about_unsaved_work_before_closing_clean_windows() {
        let unsaved = |w: &&str| w.ends_with('*');
        assert_eq!(
            questions_first(vec!["a*", "b", "c", "d*"], unsaved),
            ["b", "c", "a*", "d*"]
        );
        assert_eq!(questions_first(vec!["a", "b"], unsaved), ["a", "b"]);
    }
    /// A quit the project library refuses is asked about, never only
    /// refused: Cancel is the default, and quitting anyway is offered with
    /// what happens to unsaved work.
    #[test]
    fn a_quit_the_library_refuses_offers_to_quit_anyway() {
        let alert = quit_failed_alert("Permission denied (os error 13)");
        assert_eq!(alert.labels(), ["Cancel", "Quit Anyway"]);
        assert!(alert.buttons[0].default && !alert.buttons[0].action);
        assert!(alert.buttons[1].action);
        assert!(
            alert
                .informative
                .starts_with("Powermove isn’t allowed to write there. If you quit anyway")
        );
        assert!(!alert.informative.contains("os error"));
        assert!(alert.informative.contains("offers to recover"));
    }
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
    /// A quit that ends with a project no window shows still open in the
    /// kernel (an agent's project nobody showed, an open nothing adopted)
    /// is not refused: its file is not written without anyone asking, its
    /// unsaved work stays in its journal for the next open to offer, and
    /// the library and its file lock are released.
    #[test]
    fn a_quit_defers_a_project_no_window_shows_and_releases_the_library() {
        let (dir, actor, session) = fixture();
        let k = actor.handle();
        let file = dir.path().join("unshown.pmv");
        answer(
            &k,
            ShellCall::SaveAs {
                session,
                path: file.clone(),
                replace: false,
            },
        );
        let saved = std::fs::read(&file).unwrap();
        edit(&k);
        assert!(k.published("test").unwrap().dirty);
        // What Quit sends once every window closed its project.
        answer(&k, ShellCall::PrepareQuit);
        answer(&k, ShellCall::ReleaseLibrary);
        assert!(k.published("test").is_none(), "the session is released");
        assert_eq!(std::fs::read(&file).unwrap(), saved, "the file is as saved");
        let lock = pm_kernel::locks::project_lock_path(&file);
        drop(pm_kernel::locks::FileLock::acquire(&lock).expect("its file lock is released"));
        assert!(
            !pm_kernel::journal::sessions(&dir.path().join("journals/test")).is_empty(),
            "the unsaved edit is kept for recovery"
        );
        actor.shutdown();
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
    /// An injected kernel panic stops the project mid-edit: the window
    /// closes it without a save question (even with an edit open), nothing
    /// is written over the saved project, and Reopen's calls bring back
    /// exactly the acknowledged work through the recovery prompt.
    #[test]
    fn a_stopped_project_closes_unasked_and_reopens_to_its_journal() {
        let (dir, actor, session) = fixture();
        let k = actor.handle();
        let stored = || std::fs::read(dir.path().join("store/project.test.json")).unwrap();
        let base = stored();
        edit(&k);
        let acknowledged = k.published("test").unwrap();
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
        k.arm_fault(pm_kernel::fault_injection::Stage::AfterApply, "test");
        assert!(
            k.call(
                "test",
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some("Opacity".into()),
                        body: WriteBody::Commands(vec![
                            CompCommand::Set {
                                patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                    name: Some("Half applied".into()),
                                    ..Default::default()
                                }),
                            }
                            .into(),
                        ]),
                        expected_revision: None,
                    },
                ),
            )
            .wait()
            .is_none()
        );
        let p = k.published("test").unwrap();
        assert!(p.fault.is_some());
        assert!(
            p.entry.is_some() && p.dirty,
            "an edit is open, unsaved work"
        );
        assert_eq!(close_state(Some(&p), false, false), CloseState::Stopped);
        let source = crate::session::reopen_source(&p, dir.path()).unwrap();
        // complete_close's call.
        assert!(matches!(
            answer(&k, ShellCall::Close(session)),
            ShellReply::Closed {
                recovery_copy: None
            }
        ));
        assert_eq!(
            stored(),
            base,
            "closing wrote nothing of the stopped project"
        );
        // reopen_stopped → open_named → the recovery prompt's Recover.
        let ShellReply::RecoveryRequired { ticket, .. } = answer(
            &k,
            ShellCall::PrepareOpen {
                id: "test".into(),
                source,
                electron_profile: None,
            },
        ) else {
            panic!("reopening offers recovery")
        };
        answer(
            &k,
            ShellCall::DecideRecovery {
                ticket,
                decision: RecoveryDecision::Recover,
            },
        );
        let recovered = k.published("test").unwrap();
        assert!(recovered.fault.is_none());
        assert_eq!(
            recovered.snapshot.document.to_value().unwrap(),
            acknowledged.snapshot.document.to_value().unwrap()
        );
        assert_eq!(recovered.history.undo_label.as_deref(), Some("Rename"));
        // Quitting after Later instead: compaction keeps only the
        // acknowledged work, never the half-applied edit.
        let reopened = recovered.session.as_ref().unwrap().handle.clone();
        answer(&k, ShellCall::DeferClose(reopened));
        answer(&k, ShellCall::PrepareQuit);
        let text = String::from_utf8(stored()).unwrap();
        assert!(
            text.contains("Rejected edit") && !text.contains("Half applied"),
            "{text}"
        );
        actor.shutdown();
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
