//! How a project reaches a window from outside an editor: Finder, a drop on
//! the Dock icon, `open`, `powermove://` links (pm-services `entrypoints`) and
//! Project Library ▸ Open in New Window.
//!
//! A project that is already open brings its window forward. Otherwise the
//! project opens in the frontmost editor when that window is empty, and in a
//! new window when it isn't: windows own their projects, so an open from
//! Finder never replaces the one a person is working in.
use crate::session::Controller;
use gpui::*;
use pm_kernel::session::ShellReply;
use pm_services::entrypoints::{AuthLink, EntryHook, EntryQueue};
use std::{collections::VecDeque, path::PathBuf};

/// What a new editor window opens instead of restoring its last project.
pub enum Pending {
    File(PathBuf),
    /// A project an agent opened while its window showed another: already
    /// open in the kernel, this window shows it.
    Session(pm_kernel::session::SessionHandle),
    /// A project the Project Library's worker already opened.
    Adopt {
        reply: Box<ShellReply>,
        name: String,
        file: Option<PathBuf>,
    },
}

#[derive(Default)]
struct Entries {
    queue: EntryQueue,
    /// Set once the first editor window exists (first-launch copy is done).
    ready: bool,
    next_window: VecDeque<Pending>,
    editors: Vec<(WeakEntity<Controller>, AnyWindowHandle)>,
}
impl Global for Entries {}

/// URLs from LaunchServices. Files queue until the library is attached.
pub fn receive(urls: Vec<String>, cx: &mut App) {
    for raw in urls {
        // Development builds: on-screen reviews reach a menu command without
        // pressing a menu item (which could bring the app forward).
        #[cfg(debug_assertions)]
        if let Some(command) = raw.strip_prefix("powermove://review/command/") {
            pm_shell::send_menu_command(command);
            continue;
        }
        #[cfg(debug_assertions)]
        if let Some(rest) = raw.strip_prefix("powermove://review/skim/") {
            let editors = cx.default_global::<Entries>().editors.clone();
            if let Some((editor, _)) = editors.first() {
                crate::skimming::review_sweep(editor.clone(), rest, cx);
            }
            continue;
        }
        #[cfg(debug_assertions)]
        if let Some(rest) = raw.strip_prefix("powermove://review/library-row/") {
            review_library_row(rest, cx);
            continue;
        }
        // `review/viewer/<View menu title>`: the viewer's render actions.
        #[cfg(debug_assertions)]
        if let Some(title) = raw.strip_prefix("powermove://review/viewer/") {
            let title = url::form_urlencoded::parse(format!("x={title}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            let editors = cx.default_global::<Entries>().editors.clone();
            if let Some(action) = crate::viewer2::review_action(&title)
                && let Some(editor) = editors.into_iter().find_map(|(e, _)| e.upgrade())
            {
                editor.update(cx, |v, cx| v.viewer_action(action, cx));
            }
            continue;
        }
        #[cfg(debug_assertions)]
        if let Some(rest) = raw.strip_prefix("powermove://review/files/") {
            review_files(rest, cx);
            continue;
        }
        let failure = admit(&mut cx.default_global::<Entries>().queue, &raw);
        if let Some(text) = failure {
            crate::error_log::shared(cx).borrow_mut().push("Open", text);
        }
    }
    drain(cx);
}

/// `<project name>/<context-menu label>`, percent-encoded, on Home.
#[cfg(debug_assertions)]
fn review_library_row(rest: &str, cx: &mut App) {
    let decode = |s: &str| {
        url::form_urlencoded::parse(format!("x={s}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    };
    let Some((name, label)) = rest.split_once('/').map(|(n, l)| (decode(n), decode(l))) else {
        return;
    };
    // The window showing Home, else any window with one.
    let editors = editors(cx);
    let target = editors
        .iter()
        .find(|e| e.read(cx).home_visible)
        .or(editors.first())
        .and_then(|e| e.read(cx).library.clone());
    if let Some(library) = target {
        library.update(cx, |v, cx| v.review_row_menu(&name, &label, cx));
    }
}

/// `<drop|sequence|figma|answer>/<paths>`, percent-encoded and newline-separated,
/// on the first editor: what a Finder drop on the window, or the open panel of
/// File ▸ Import Image Sequence…, hands over; `answer` is what the next open
/// panel (Import, Locate File…) returns. The system open panel takes no
/// background input, so reviews choose files here.
#[cfg(debug_assertions)]
fn review_files(rest: &str, cx: &mut App) {
    let Some((route, paths)) = rest.split_once('/') else {
        return;
    };
    let paths: Vec<PathBuf> = url::form_urlencoded::parse(format!("x={paths}").as_bytes())
        .next()
        .map(|(_, v)| v.lines().map(PathBuf::from).collect())
        .unwrap_or_default();
    if route == "answer" {
        crate::platform::answer_next_open_panel(paths);
        return;
    }
    let route = route.to_owned();
    let editors = cx.default_global::<Entries>().editors.clone();
    if let Some(editor) = editors.into_iter().find_map(|(e, _)| e.upgrade()) {
        editor.update(cx, |v, cx| match route.as_str() {
            "drop" => v.import_dropped(paths, cx),
            "sequence" => {
                let at = Some(v.time());
                v.choose_sequence(paths, true, at, None, cx)
            }
            // What Paste reads from the pasteboard, from a file: reviews
            // never overwrite the person's clipboard.
            "figma" => {
                if let Some(html) = paths.first().and_then(|p| std::fs::read_to_string(p).ok()) {
                    v.paste_figma_html(html, None, cx);
                }
            }
            _ => {}
        });
    }
}

/// Queues one URL; the Error Log line when it can't be opened.
fn admit(queue: &mut EntryQueue, raw: &str) -> Option<&'static str> {
    match url::Url::parse(raw) {
        Ok(url) if url.scheme() == "file" => match url.to_file_path() {
            Ok(path) => queue
                .push_file(path)
                .err()
                .map(|_| "Powermove opens Powermove projects (.pmv files)."),
            Err(()) => Some("That file couldn’t be opened."),
        },
        // Never log the link itself: a sign-in link carries a token.
        _ => queue
            .push_link(raw)
            .err()
            .map(|_| "Ignored a powermove:// link that is not a sign-in."),
    }
}

/// Called for every editor window the app creates.
pub fn register(controller: &Entity<Controller>, window: AnyWindowHandle, cx: &mut App) {
    let entries = cx.default_global::<Entries>();
    entries
        .editors
        .retain(|(editor, _)| editor.upgrade().is_some());
    entries.editors.push((controller.downgrade(), window));
    if !entries.ready {
        entries.ready = true;
        cx.defer(|cx| {
            let packaged = crate::platform::bundle_id().as_deref()
                == Some(pm_update_policy::update_distribution::PRODUCTION_ID);
            let _ = EntryQueue::register(packaged, false, &mut Hook { cx });
            drain(cx);
        });
    }
}

/// The editor in the window with this id, if it is still open.
pub fn editor(window: u64, cx: &App) -> Option<Entity<Controller>> {
    cx.try_global::<Entries>()?
        .editors
        .iter()
        .find(|(_, w)| w.window_id().as_u64() == window)
        .and_then(|(e, _)| e.upgrade())
}

/// Every editor window's controller that is still open.
pub fn editors(cx: &App) -> Vec<Entity<Controller>> {
    cx.try_global::<Entries>()
        .map(|e| e.editors.iter().filter_map(|(e, _)| e.upgrade()).collect())
        .unwrap_or_default()
}

/// The new window's opening, taken once by the window factory.
pub fn take_for_new_window(cx: &mut App) -> Option<Pending> {
    cx.default_global::<Entries>().next_window.pop_front()
}

pub fn open_in_new_window(pending: Pending, cx: &mut App) {
    cx.default_global::<Entries>()
        .next_window
        .push_back(pending);
    // Not inside the caller's entity update: the factory updates new entities.
    cx.defer(pm_shell::open_editor);
}

/// A project opened in a window that shows one (from Home, File ▸ Open, a
/// dropped file, an agent): a window of its own joined to that window's
/// tabs, as v1 added a tab. The project the window shows stays open and
/// untouched; nothing is saved or closed without its own question.
pub fn open_in_new_tab(pending: Pending, leader: u64, cx: &mut App) {
    cx.default_global::<Entries>()
        .next_window
        .push_back(pending);
    cx.defer(move |cx| pm_shell::open_editor_tab(cx, leader));
}

fn drain(cx: &mut App) {
    if !cx.default_global::<Entries>().ready {
        return;
    }
    let mut queue = std::mem::take(&mut cx.global_mut::<Entries>().queue);
    // The hook never fails: an open reports its own failure (S004 alert and
    // Error Log row) in the window it was given.
    let _ = queue.drain(&mut Hook { cx });
    cx.global_mut::<Entries>().queue = queue;
}

struct Hook<'a> {
    cx: &'a mut App,
}
impl EntryHook for Hook<'_> {
    /// The Powermove Cloud sign-in waiting in its browser step takes it; a
    /// link nothing waits for is ignored, as v1.2.0 does, with a log line.
    fn auth(&mut self, link: &AuthLink) -> pm_services::Result<()> {
        if !crate::cloud::receive_link(link.state(), link.token(), self.cx) {
            crate::error_log::shared(self.cx)
                .borrow_mut()
                .push("Open", "Ignored a sign-in link: no sign-in is waiting.");
        }
        Ok(())
    }
    fn open_project(&mut self, path: &std::path::Path) -> pm_services::Result<()> {
        let cx = &mut *self.cx;
        let id = crate::session::stable_id(path);
        let editors = cx.global::<Entries>().editors.clone();
        let live: Vec<_> = editors
            .iter()
            .filter_map(|(editor, window)| Some((editor.upgrade()?, *window)))
            .collect();
        // Already open: that window comes forward (only if the app is active).
        if let Some((editor, _)) = live
            .iter()
            .find(|(e, _)| e.read(cx).session.as_ref().is_some_and(|s| s.project == id))
        {
            gpui_macos::child_window::raise_if_active(&editor.read(cx).host.borrow().window);
            return Ok(());
        }
        let active = cx.active_window();
        let empty = |e: &Entity<Controller>, cx: &App| {
            let v = e.read(cx);
            v.session.is_none() && !v.opening
        };
        let target = live
            .iter()
            .filter(|(e, _)| empty(e, cx))
            .min_by_key(|(_, w)| Some(*w) != active)
            .map(|(e, _)| e.clone());
        match target {
            Some(editor) => {
                let path = path.to_path_buf();
                editor.update(cx, |v, cx| v.open_file(path, cx));
            }
            None => open_in_new_window(Pending::File(path.to_path_buf()), cx),
        }
        Ok(())
    }
    /// Only the shipped bundle claims `powermove://`, as v1.2.0's packaged app
    /// does. Development and beta bundles never touch LaunchServices defaults.
    fn register_associations(&mut self, _extension: &str, scheme: &str) -> pm_services::Result<()> {
        self.cx.register_url_scheme(scheme).detach();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryQueue, admit};
    #[test]
    fn finder_files_and_links_are_admitted_and_others_explained() {
        let mut queue = EntryQueue::default();
        assert_eq!(admit(&mut queue, "file:///tmp/Launch%20Film.pmv"), None);
        assert_eq!(admit(&mut queue, "file:///tmp/Launch%20Film.PMV"), None);
        assert_eq!(admit(&mut queue, "powermove://auth?state=s&token=t"), None);
        assert_eq!(queue.pending(), (1, 2));
        assert_eq!(
            admit(&mut queue, "file:///tmp/notes.txt"),
            Some("Powermove opens Powermove projects (.pmv files).")
        );
        for link in [
            "powermove://store/x",
            "powermove://auth?state=s",
            "nonsense",
        ] {
            assert_eq!(
                admit(&mut queue, link),
                Some("Ignored a powermove:// link that is not a sign-in.")
            );
        }
        assert_eq!(queue.pending(), (1, 2));
    }
}
