//! Editor window session: which windows were open, where, in which native tab
//! groups, and with which project/workspace. Restored at launch. The document
//! itself (kernel state) is not stored here; `project` is only the identity the
//! layout store and the app's lifecycle use.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        mpsc::{self, Receiver, Sender},
    },
};

/// Most windows (tabs included) a launch reopens: a session that grew past
/// it (lane runs, a crash loop) opens its key window's group and the most
/// recently opened others, never hundreds (PM-400).
pub const MAX_RESTORED: usize = 24;
/// The project a window showing only Home records.
pub const NO_PROJECT: &str = "shell-untitled";
/// A launch plan without windows that showed only Home (one stays when
/// nothing else would), and at most `MAX_RESTORED` windows: the key
/// window's group first, then whole groups from the most recently opened.
fn bounded(plan: Vec<WindowState>, key: Option<u64>) -> Vec<WindowState> {
    let home = |w: &WindowState| w.project.is_empty() || w.project == NO_PROJECT;
    let mut plan: Vec<WindowState> = if plan.iter().all(home) {
        plan.into_iter().take(1).collect()
    } else {
        plan.into_iter().filter(|w| !home(w)).collect()
    };
    let group = |w: &WindowState| w.tab_group.unwrap_or(w.id);
    let ids: Vec<u64> = plan.iter().map(|w| w.id).collect();
    for w in &mut plan {
        // A tab whose group's first window was left out leads its group.
        if w.tab_group.is_some_and(|g| !ids.contains(&g)) {
            w.tab_group = None;
        }
    }
    if plan.len() <= MAX_RESTORED {
        return plan;
    }
    let key_group = key
        .and_then(|k| plan.iter().find(|w| w.id == k))
        .map(group);
    // Groups in the order they were opened (a group's place: its last window).
    let mut groups: Vec<u64> = Vec::new();
    for w in &plan {
        let g = group(w);
        groups.retain(|x| *x != g);
        groups.push(g);
    }
    let size = |g: u64| plan.iter().filter(|w| group(w) == g).count();
    let mut kept: Vec<u64> = key_group.into_iter().collect();
    let mut count = key_group.map_or(0, size);
    for g in groups.into_iter().rev() {
        if kept.contains(&g) {
            continue;
        }
        if count + size(g) > MAX_RESTORED {
            continue;
        }
        count += size(g);
        kept.push(g);
    }
    plan.retain(|w| kept.contains(&group(w)));
    // The key window's group alone can be larger still.
    plan.truncate(MAX_RESTORED);
    plan
}
/// Smallest frame that is restored as-is; smaller frames are enlarged to this.
pub const MIN_FRAME: [f64; 2] = [640., 400.];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Stable per-session window id (not an AppKit window number).
    pub id: u64,
    /// AppKit screen frame: x, y (bottom-left origin), width, height.
    pub frame: [f64; 4],
    pub project: String,
    pub workspace: String,
    /// Id of the first window of the native tab group this window belongs to.
    #[serde(default)]
    pub tab_group: Option<u64>,
    /// Its place in that group's strip: a relaunch puts the tabs back in
    /// this order (opening order is not the strip's after a reorder).
    #[serde(default)]
    pub tab_index: usize,
    /// The pages (Settings, the Store) open in its strip.
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub fullscreen: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    /// Front-to-back order is not tracked; this is opening order.
    pub windows: Vec<WindowState>,
    /// Id of the key window when last saved.
    #[serde(default)]
    pub key: Option<u64>,
    /// Every entry was an open window when the file was last written: a
    /// quit kept only the windows still open, and a launch kept only the
    /// windows it restored. A file without it comes from a build that
    /// restored one window and never took the others out, so its entries
    /// are history, not what was open (PM-392).
    #[serde(default)]
    pub complete: bool,
}
impl Session {
    pub fn next_id(&self) -> u64 {
        self.windows.iter().map(|w| w.id).max().map_or(1, |m| m + 1)
    }
    /// Inserts or replaces the window with the same id, keeping opening order.
    pub fn upsert(&mut self, w: WindowState) {
        self.version = 1;
        match self.windows.iter_mut().find(|o| o.id == w.id) {
            Some(o) => *o = w,
            None => self.windows.push(w),
        }
    }
    pub fn remove(&mut self, id: u64) {
        self.windows.retain(|w| w.id != id);
        let followers: Vec<u64> = self
            .windows
            .iter()
            .filter(|w| w.tab_group == Some(id))
            .map(|w| w.id)
            .collect();
        if let Some(&leader) = followers.first() {
            for w in &mut self.windows {
                if followers.contains(&w.id) {
                    w.tab_group = (w.id != leader).then_some(leader);
                }
            }
        }
        if self.key == Some(id) {
            self.key = None;
        }
    }
    /// Only the windows `ids` stay (the windows open at a quit); the file is
    /// then complete.
    pub fn keep(&mut self, ids: &[u64]) {
        self.windows.retain(|w| ids.contains(&w.id));
        let kept: Vec<u64> = self.windows.iter().map(|w| w.id).collect();
        for w in &mut self.windows {
            if w.tab_group.is_some_and(|g| !kept.contains(&g)) {
                w.tab_group = None;
            }
        }
        if self.key.is_some_and(|k| !kept.contains(&k)) {
            self.key = None;
        }
        self.complete = true;
    }
    /// The windows a launch reopens. From a complete file, every window
    /// (`all`, the system's "Close windows when quitting an app" off) or the
    /// key window's arrangement; from an earlier build's file, only the key
    /// window's arrangement, which is all that build reopened.
    pub fn launch_plan(&self, tabs: bool, all: bool) -> Vec<WindowState> {
        let all = all && self.complete;
        let plan = if tabs {
            self.restore_plan_tabs(all)
        } else {
            self.restore_plan(all)
        };
        bounded(plan, self.key)
    }
    /// Separate windows mode: the windows to reopen. With `all == false` (the system's "Close windows when
    /// quitting an app" setting) only the key window's arrangement is restored.
    /// Each entry has a sane frame; tab members follow their leader.
    pub fn restore_plan(&self, all: bool) -> Vec<WindowState> {
        let list: Vec<WindowState> = if all {
            self.windows.clone()
        } else {
            self.key
                .and_then(|k| self.windows.iter().find(|w| w.id == k))
                .or_else(|| self.windows.first())
                .cloned()
                .map(|w| WindowState {
                    tab_group: None,
                    ..w
                })
                .into_iter()
                .collect()
        };
        // One project per window: a tab group saved by an earlier version
        // reopens as one window, showing the project that was in front (the
        // key window when it is in the group, else the group's first). The
        // others are not lost: their projects stay in the library (Recent).
        let key = self.key;
        let group = |w: &WindowState| w.tab_group.unwrap_or(w.id);
        let mut kept: Vec<WindowState> = vec![];
        for w in &list {
            let g = group(w);
            match kept.iter().position(|k| group(k) == g) {
                Some(i) if Some(w.id) == key => kept[i] = w.clone(),
                Some(_) => {}
                None => kept.push(w.clone()),
            }
        }
        for w in &mut kept {
            w.frame = sane_frame(w.frame);
            w.tab_group = None;
        }
        kept
    }
    /// Tabs mode: the windows to reopen with their tab groups as they were
    /// (`all == false`, the system's "Close windows when quitting": the key
    /// window with its tabs). Each entry has a sane frame; a group's leader
    /// comes before its members, so `addTabbedWindow` always has the leader
    /// open.
    pub fn restore_plan_tabs(&self, all: bool) -> Vec<WindowState> {
        let mut list: Vec<WindowState> = if all {
            self.windows.clone()
        } else {
            let group = |w: &WindowState| w.tab_group.unwrap_or(w.id);
            let key = self
                .key
                .and_then(|k| self.windows.iter().find(|w| w.id == k))
                .or_else(|| self.windows.first())
                .map(group);
            self.windows
                .iter()
                .filter(|w| Some(group(w)) == key)
                .cloned()
                .collect()
        };
        let ids: Vec<u64> = list.iter().map(|w| w.id).collect();
        for w in &mut list {
            w.frame = sane_frame(w.frame);
            if w.tab_group.is_some_and(|g| g == w.id || !ids.contains(&g)) {
                w.tab_group = None;
            }
        }
        // Leaders first, then each group's tabs in their strip's order (a
        // tab joins at its group's end).
        list.sort_by_key(|w| (w.tab_group.is_some(), w.tab_index));
        list
    }
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        anyhow::ensure!(s.version == 1, "Unsupported window session");
        let mut ids = std::collections::BTreeSet::new();
        for w in &s.windows {
            anyhow::ensure!(ids.insert(w.id), "Duplicate window id");
            anyhow::ensure!(
                w.frame.iter().all(|v| v.is_finite()),
                "Invalid window frame"
            );
            anyhow::ensure!(!w.workspace.is_empty(), "Missing workspace");
        }
        Ok(s)
    }
    /// Atomic replacement, like the layout store.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = path.with_extension("json.pending");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            f.sync_all()?;
        }
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
/// One window's change to the shared session file.
#[derive(Clone, Debug)]
pub enum Change {
    /// The window's state, and whether it is the key window.
    Upsert { state: WindowState, key: bool },
    /// The person closed the window.
    Remove(u64),
    /// A quit: only these windows (the ones open) stay.
    Keep(Vec<u64>),
    /// A launch: the windows it restores, the file complete.
    Replace(Session),
}
enum Job {
    Change(PathBuf, Change),
    Flush(Sender<Result<(), String>>),
}
struct Writer {
    jobs: Mutex<Sender<Job>>,
    error: Mutex<Option<String>>,
    /// Held by `pause` (tests): the writer waits before each batch.
    gate: Mutex<()>,
}
fn writer() -> &'static Writer {
    static WRITER: OnceLock<Writer> = OnceLock::new();
    WRITER.get_or_init(|| {
        let (jobs, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name("pm-window-session".into())
            .spawn(move || serve(inbox))
            .expect("window session writer");
        Writer {
            jobs: Mutex::new(jobs),
            error: Mutex::new(None),
            gate: Mutex::new(()),
        }
    })
}
/// The writer: every window's changes, read-modify-written in the order they
/// were asked for, a burst at a time, never on the main thread (N8).
fn serve(inbox: Receiver<Job>) {
    // The first write that failed since the last flush, for that flush.
    let mut failed: Option<String> = None;
    while let Ok(first) = inbox.recv() {
        let _gate = writer().gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut batch = vec![first];
        batch.extend(inbox.try_iter());
        let mut changes: Vec<(PathBuf, Change)> = Vec::new();
        for job in batch {
            match job {
                Job::Change(path, change) => changes.push((path, change)),
                Job::Flush(done) => {
                    if let Err(e) = apply(std::mem::take(&mut changes)) {
                        failed.get_or_insert(e);
                    }
                    let _ = done.send(failed.take().map_or(Ok(()), Err));
                }
            }
        }
        if let Err(e) = apply(changes) {
            failed.get_or_insert(e);
        }
    }
}
fn apply(changes: Vec<(PathBuf, Change)>) -> Result<(), String> {
    let mut result = Ok(());
    let mut paths: Vec<&PathBuf> = Vec::new();
    for (path, _) in &changes {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    for path in paths {
        if let Err(e) = update(path, changes.iter().filter(|(p, _)| p == path)) {
            let message = format!("Window session could not be saved: {e}");
            *writer().error.lock().unwrap() = Some(message.clone());
            if result.is_ok() {
                result = Err(message);
            }
        }
    }
    result
}
/// A malformed or newer file is reported and never overwritten.
fn update<'a>(
    path: &Path,
    changes: impl Iterator<Item = &'a (PathBuf, Change)>,
) -> anyhow::Result<()> {
    let mut session = if path.exists() {
        Session::load(path)?
    } else {
        Session::default()
    };
    for (_, change) in changes {
        match change {
            Change::Upsert { state, key } => {
                let id = state.id;
                session.upsert(state.clone());
                if *key {
                    session.key = Some(id);
                }
            }
            Change::Remove(id) => session.remove(*id),
            Change::Keep(ids) => session.keep(ids),
            Change::Replace(whole) => session = whole.clone(),
        }
    }
    session.version = 1;
    session.save(path)
}
/// Queues `change` to the session file at `path`; returns at once.
pub fn write(path: &Path, change: Change) {
    let _ = writer()
        .jobs
        .lock()
        .unwrap()
        .send(Job::Change(path.to_owned(), change));
}
/// Waits until every change queued before it has been written (quit,
/// tests); the first failure since the last flush, if any write failed.
pub fn flush() -> Result<(), String> {
    let stopped = || "The window session writer stopped.".to_owned();
    let (done, wait) = mpsc::channel();
    writer()
        .jobs
        .lock()
        .unwrap()
        .send(Job::Flush(done))
        .map_err(|_| stopped())?;
    wait.recv().map_err(|_| stopped())?
}
/// The last write's failure, once.
pub fn take_error() -> Option<String> {
    writer().error.lock().unwrap().take()
}
/// Holds the writer until the guard drops (tests).
#[doc(hidden)]
pub fn pause() -> std::sync::MutexGuard<'static, ()> {
    writer().gate.lock().unwrap_or_else(|e| e.into_inner())
}
/// Enlarges tiny frames; the caller still constrains the frame to a screen.
pub fn sane_frame(f: [f64; 4]) -> [f64; 4] {
    if !f.iter().all(|v| v.is_finite()) {
        return [0., 0., MIN_FRAME[0], MIN_FRAME[1]];
    }
    [f[0], f[1], f[2].max(MIN_FRAME[0]), f[3].max(MIN_FRAME[1])]
}
/// Moves `frame` fully onto the screen it overlaps most (or the first screen),
/// shrinking it if it is larger than that screen's visible frame.
pub fn constrain_to_screens(frame: [f64; 4], screens: &[[f64; 4]]) -> [f64; 4] {
    let Some(screen) = screens
        .iter()
        .max_by(|a, b| overlap(frame, **a).total_cmp(&overlap(frame, **b)))
        .filter(|s| overlap(frame, **s) > 0.)
        .or(screens.first())
    else {
        return frame;
    };
    let w = frame[2].min(screen[2]);
    let h = frame[3].min(screen[3]);
    let x = frame[0].clamp(screen[0], screen[0] + screen[2] - w);
    let y = frame[1].clamp(screen[1], screen[1] + screen[3] - h);
    [x, y, w, h]
}
fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    w.max(0.) * h.max(0.)
}

#[cfg(test)]
mod writer_tests {
    use super::*;

    fn window(id: u64, x: f64) -> WindowState {
        WindowState {
            id,
            frame: [x, 0., 800., 600.],
            project: "p".into(),
            workspace: "design".into(),
            tab_group: None,
            tab_index: 0,
            pages: Vec::new(),
            fullscreen: false,
        }
    }

    /// One test: the writer and its error are the process's.
    #[test]
    fn writes_leave_the_caller_at_once_land_in_order_and_never_overwrite_a_bad_file() {
        let dir = std::env::temp_dir().join(format!("pm-session-writer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("session.json");
        {
            // The writer is held: a write is queued, not done, when it returns.
            let _held = pause();
            for (id, x) in [(1, 10.), (2, 20.), (1, 30.)] {
                write(
                    &path,
                    Change::Upsert {
                        state: window(id, x),
                        key: id == 2,
                    },
                );
            }
            write(&path, Change::Remove(2));
            write(
                &path,
                Change::Upsert {
                    state: window(3, 40.),
                    key: false,
                },
            );
            assert!(!path.exists(), "a write blocked its caller on the disk");
        }
        flush().unwrap();
        let s = Session::load(&path).unwrap();
        assert_eq!(s.windows, [window(1, 30.), window(3, 40.)]);
        assert_eq!(s.key, None, "the closed key window is no longer key");
        assert!(take_error().is_none());
        // A newer or malformed file is reported and left as it is.
        let newer = b"{\"version\":9,\"windows\":[]}";
        std::fs::write(&path, newer).unwrap();
        write(&path, Change::Remove(1));
        // The flush itself says so: quit asks before going on.
        assert!(flush().is_err_and(|e| e.contains("could not be saved")));
        assert!(take_error().is_some_and(|e| e.contains("could not be saved")));
        // Said once: the next flush with nothing failing is fine.
        assert!(flush().is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), newer);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
#[cfg(test)]
mod upgrade_tests {
    use super::*;

    fn window(id: u64, project: &str, tab_group: Option<u64>) -> WindowState {
        WindowState {
            id,
            frame: [10., 10., 800., 600.],
            project: project.into(),
            workspace: "design".into(),
            tab_group,
            tab_index: 0,
            pages: Vec::new(),
            fullscreen: false,
        }
    }
    /// An earlier build's file: 21 entries, of which only the key window was
    /// open (that build reopened one window and never took the rest out).
    fn history() -> Session {
        let mut windows = vec![
            window(1, "paper", None),
            window(2, "a", Some(1)),
            window(3, "b", Some(1)),
            window(4, "c", None),
        ];
        windows.extend((5..=21).map(|id| window(id, &format!("old{id}"), None)));
        Session {
            version: 1,
            windows,
            key: Some(4),
            complete: false,
        }
    }

    #[test]
    fn a_file_from_an_earlier_build_reopens_only_its_key_windows_arrangement() {
        // The file an earlier build wrote has no `complete`.
        let old: Session = serde_json::from_str(
            r#"{"version":1,"windows":[{"id":4,"frame":[0,0,800,600],"project":"c","workspace":"design"}],"key":4}"#,
        )
        .unwrap();
        assert!(!old.complete);
        let s = history();
        for tabs in [true, false] {
            for all in [true, false] {
                let plan = s.launch_plan(tabs, all);
                assert_eq!(
                    plan.iter().map(|w| w.id).collect::<Vec<_>>(),
                    [4],
                    "tabs {tabs}, all {all}"
                );
            }
        }
        // Key window in a tab group: the group, as the earlier build had it.
        let s = Session {
            key: Some(2),
            ..history()
        };
        let plan = s.launch_plan(true, true);
        assert_eq!(plan.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn a_complete_file_reopens_every_window_it_holds() {
        let mut s = history();
        s.complete = true;
        assert_eq!(s.launch_plan(true, true).len(), 21);
        assert_eq!(s.launch_plan(false, true).len(), 19, "a group is one window");
    }

    #[test]
    fn a_quit_keeps_the_open_windows_only_and_marks_the_file_complete() {
        let mut s = history();
        s.keep(&[2, 3, 4]);
        assert!(s.complete);
        assert_eq!(s.windows.iter().map(|w| w.id).collect::<Vec<_>>(), [2, 3, 4]);
        // The group's leader went: its tabs no longer point at it.
        assert!(s.windows.iter().all(|w| w.tab_group.is_none()));
        assert_eq!(s.key, Some(4));
        s.keep(&[2]);
        assert_eq!(s.key, None);
        // A round trip keeps the mark.
        let back: Session = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert!(back.complete);
    }
}
#[cfg(test)]
mod order_tests {
    use super::*;
    #[test]
    fn tabs_come_back_in_their_strips_order_whatever_order_they_opened_in() {
        let tab = |id: u64, group: Option<u64>, index: usize| WindowState {
            id,
            frame: [10., 10., 800., 600.],
            project: format!("p{id}"),
            workspace: "design".into(),
            tab_group: group,
            tab_index: index,
            pages: Vec::new(),
            fullscreen: false,
        };
        // Opened 1, 2, 3, 4; the strip was then reordered to 1, 4, 2, 3.
        let s = Session {
            version: 1,
            windows: vec![tab(1, None, 0), tab(2, Some(1), 2), tab(3, Some(1), 3), tab(4, Some(1), 1)],
            key: Some(4),
            complete: true,
        };
        let plan = s.launch_plan(true, true);
        assert_eq!(plan.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 4, 2, 3]);
    }
}
#[cfg(test)]
mod bound_tests {
    use super::*;
    fn w(id: u64, project: &str, tab_group: Option<u64>) -> WindowState {
        WindowState {
            id,
            frame: [10., 10., 800., 600.],
            project: project.into(),
            workspace: "design".into(),
            tab_group,
            tab_index: 0,
            pages: Vec::new(),
            fullscreen: false,
        }
    }
    #[test]
    fn a_session_of_seventy_windows_reopens_its_projects_and_no_home_windows() {
        // PM-400's profile: 70 windows, 65 of them Home.
        let mut windows: Vec<WindowState> = (1..=65).map(|id| w(id, NO_PROJECT, None)).collect();
        windows.extend((66..=70).map(|id| w(id, &format!("p{id}"), None)));
        let s = Session { version: 1, windows, key: Some(3), complete: true };
        for tabs in [true, false] {
            let plan = s.launch_plan(tabs, true);
            assert_eq!(plan.iter().map(|w| w.id).collect::<Vec<_>>(), [66, 67, 68, 69, 70]);
        }
        // Home alone: one window, not 65.
        let s = Session {
            version: 1,
            windows: (1..=65).map(|id| w(id, NO_PROJECT, None)).collect(),
            key: None,
            complete: true,
        };
        assert_eq!(s.launch_plan(true, true).len(), 1);
    }
    #[test]
    fn at_most_a_bounded_number_of_windows_reopen_the_key_windows_group_first() {
        // 40 windows; the key window's group (1 with tabs 2 and 3) opened first.
        let mut windows = vec![w(1, "a", None), w(2, "b", Some(1)), w(3, "c", Some(1))];
        windows.extend((4..=40).map(|id| w(id, &format!("p{id}"), None)));
        let s = Session { version: 1, windows, key: Some(2), complete: true };
        let plan = s.launch_plan(true, true);
        assert_eq!(plan.len(), MAX_RESTORED);
        let ids: Vec<u64> = plan.iter().map(|w| w.id).collect();
        assert!(ids.starts_with(&[1]) && ids.contains(&2) && ids.contains(&3));
        // The most recently opened others fill the rest.
        assert!(ids.contains(&40) && !ids.contains(&4));
    }
}
