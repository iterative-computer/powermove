//! Editor window session: which windows were open, where, in which native tab
//! groups, and with which project/workspace. Restored at launch. The document
//! itself (kernel state) is not stored here; `project` is only the identity the
//! layout store and the app's lifecycle use.
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    /// Windows to reopen. With `all == false` (the system's "Close windows when
    /// quitting an app" setting) only the key window's arrangement is restored.
    /// Each entry has a sane frame; tab members follow their leader.
    pub fn restore_plan(&self, all: bool) -> Vec<WindowState> {
        let mut list: Vec<WindowState> = if all {
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
        let ids: Vec<u64> = list.iter().map(|w| w.id).collect();
        for w in &mut list {
            w.frame = sane_frame(w.frame);
            if w.tab_group.is_some_and(|g| g == w.id || !ids.contains(&g)) {
                w.tab_group = None;
            }
        }
        // Leaders first so `addTabbedWindow` always has its leader open.
        list.sort_by_key(|w| w.tab_group.is_some());
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
