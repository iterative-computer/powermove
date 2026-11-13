//! The fonts installed on this Mac change while the app runs: a font is
//! installed, removed, activated by a font manager, or registered by the
//! process. CoreText says so with one notification, posted on the main run
//! loop. This module counts them ([`epoch`]): the registry retires what it
//! resolved before a change, its generation and the broker's move on, so
//! every renderer installs fresh fonts and nothing keyed on the generation
//! (a frame cache) outlives the fonts it was drawn with.
//!
//! A font file rewritten in place by its owner's tool changes no
//! registration, so CoreText says nothing: the registries note the version
//! of each file they read ([`note_read`]), and [`refresh`] counts a file
//! that is another version now as a change like any other.

use crate::file_version::FileVersion;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Called after a change; answers whether to be called again.
type Listener = Box<dyn Fn() -> bool + Send + Sync>;
static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

/// How many font registration changes this process has seen ([`watch`]).
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

/// Font files the registries of this process have read, each with the
/// version read.
static READ: Mutex<Vec<(PathBuf, FileVersion)>> = Mutex::new(Vec::new());
/// How many times the installed fonts were listed ([`available_files`]).
static LISTINGS: AtomicU64 = AtomicU64::new(0);

/// A registry read the font file at `path` when it was `version` (it keeps
/// those bytes until the installed fonts change).
pub fn note_read(path: &Path, version: FileVersion) {
    let path = one_spelling(path.to_owned());
    let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
    match read.iter_mut().find(|(p, _)| *p == path) {
        Some(known) => known.1 = version,
        None => read.push((path, version)),
    }
}

/// CoreText names one file by more than one path (a font registered from
/// `/private/tmp/…` comes back as `/tmp/…` from its face): the lists and
/// the files read are compared by the file's resolved path.
fn one_spelling(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// The font files read so far, with the version each was read at.
pub fn files_read() -> Vec<(PathBuf, FileVersion)> {
    READ.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Looks at the font files read so far (one look each, whatever is
/// installed): a file that is another version now was rewritten or replaced
/// without any registration changing, and counts as a change of the
/// installed fonts ([`changed`]): the registries read it again. Whether any
/// was. Safe on any thread; listeners run on it.
pub fn refresh() -> bool {
    let stale = {
        let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
        let before = read.len();
        read.retain(|(path, version)| FileVersion::of(path) == Some(*version));
        read.len() != before
    };
    if stale {
        changed();
    }
    stale
}

/// How many times the installed fonts have been listed in this process
/// ([`available_files`]: every installed font file, hundreds of them).
pub fn listings() -> u64 {
    LISTINGS.load(Ordering::Relaxed)
}

/// Calls `listener` after every change, on the thread that saw it (the main
/// thread for CoreText's own; it must not block), for as long as it answers
/// true.
pub fn on_change(listener: impl Fn() -> bool + Send + Sync + 'static) {
    watch();
    LISTENERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Box::new(listener));
}

/// A change happened: what the notification does.
pub fn changed() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
    LISTENERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|listener| listener());
}

/// Subscribes to CoreText's registered-fonts-changed notification, once
/// per process. Its deliveries need the main run loop.
pub fn watch() {
    #[cfg(all(target_os = "macos", feature = "system"))]
    {
        use objc2_core_foundation::{CFNotificationCenter, CFNotificationSuspensionBehavior};
        static ONCE: std::sync::Once = std::sync::Once::new();
        unsafe extern "C-unwind" fn notified(
            _: *mut CFNotificationCenter,
            _: *mut std::ffi::c_void,
            _: *const objc2_core_foundation::CFNotificationName,
            _: *const std::ffi::c_void,
            _: *const objc2_core_foundation::CFDictionary,
        ) {
            changed();
        }
        ONCE.call_once(|| {
            let Some(center) = CFNotificationCenter::local_center() else {
                return;
            };
            // The local centre is the process's default notification centre,
            // where CoreText posts. The observer is never removed.
            unsafe {
                center.add_observer(
                    std::ptr::null(),
                    Some(notified),
                    Some(objc2_core_text::kCTFontManagerRegisteredFontsChangedNotification),
                    std::ptr::null(),
                    CFNotificationSuspensionBehavior::DeliverImmediately,
                );
            }
        });
    }
}

/// Every font file CoreText resolves from now, sorted: system, user and
/// session fonts, fonts activated from anywhere else, and those the process
/// registered. Current at once after a change (the notification follows).
pub fn available_files() -> Vec<PathBuf> {
    LISTINGS.fetch_add(1, Ordering::Relaxed);
    #[cfg(all(target_os = "macos", feature = "system"))]
    {
        use objc2_core_foundation::CFURL;
        let urls = unsafe { objc2_core_text::CTFontManagerCopyAvailableFontURLs() };
        let urls: &objc2_core_foundation::CFArray<CFURL> = unsafe { urls.cast_unchecked() };
        let mut files: Vec<PathBuf> = urls
            .iter()
            .filter_map(|url| url.to_file_path())
            .map(one_spelling)
            .collect();
        files.sort();
        files.dedup();
        files
    }
    #[cfg(not(all(target_os = "macos", feature = "system")))]
    Vec::new()
}

/// Delivers CoreText's pending change notifications ([`watch`]): runs the
/// calling thread's run loop until it has nothing ready. For the main
/// thread of a process that has no run loop of its own (a job's child).
pub fn pump() {
    #[cfg(all(target_os = "macos", feature = "system"))]
    {
        use objc2_core_foundation::{CFRunLoop, CFRunLoopRunResult, kCFRunLoopDefaultMode};
        // A handful of turns at most: each handles one ready source.
        for _ in 0..64 {
            let turned = CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0., true);
            if turned != CFRunLoopRunResult::HandledSource {
                break;
            }
        }
    }
}

/// The font files CoreText resolves from at one moment, each with its
/// version ([`FileVersion`]; the empty one for a file that cannot be looked
/// at): the fonts a render can resolve to, by stable identity. A render in
/// another process draws with the same fonts when that process's inventory
/// is this one ([`Inventory::adopt`]) and stays it ([`Fence`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Inventory(pub Vec<(PathBuf, FileVersion)>);

impl Inventory {
    /// This process's fonts now ([`available_files`]): a look at every
    /// installed font file.
    pub fn current() -> Self {
        Inventory(
            available_files()
                .into_iter()
                .map(|path| {
                    let version = FileVersion::of(&path).unwrap_or_default();
                    (path, version)
                })
                .collect(),
        )
    }

    /// Makes this process resolve from exactly these files: the ones it
    /// lacks (fonts another process activated for itself) are registered
    /// for this process only. Fails, saying what differs, when its fonts
    /// still are not these.
    pub fn adopt(&self) -> Result<(), String> {
        let now = Self::current();
        if now == *self {
            return Ok(());
        }
        let have: std::collections::HashSet<&PathBuf> = now.0.iter().map(|f| &f.0).collect();
        for (path, ..) in &self.0 {
            if !have.contains(path) {
                // Verified below, whatever CoreText answered.
                let _ = register_for_process(path);
            }
        }
        self.verify()
    }

    /// Whether this process's fonts are these now; if not, the first
    /// difference. A look at every installed font file.
    pub fn verify(&self) -> Result<(), String> {
        let now = Self::current();
        if now == *self {
            return Ok(());
        }
        let (ours, theirs): (std::collections::HashSet<_>, std::collections::HashSet<_>) =
            (now.0.iter().collect(), self.0.iter().collect());
        let name = |f: Option<&&(PathBuf, FileVersion)>| f.map(|f| f.0.display().to_string());
        Err(
            match (
                name(theirs.difference(&ours).min()),
                name(ours.difference(&theirs).min()),
            ) {
                (Some(missing), _) => format!("{missing} is not available as it was"),
                (None, Some(extra)) => format!("{extra} was not available before"),
                (None, None) => "the font list is in another order".to_owned(),
            },
        )
    }
}

/// Holds a job to the fonts it was asked to draw with, from the moment it
/// is made (when this process's inventory is the one asked for:
/// [`Inventory::adopt`], and then [`Inventory::verify`] once more after the
/// fence stands). [`Fence::hold`] is asked after each unit of work, before
/// its result is kept: whatever was drawn while the fence held was drawn
/// with those fonts.
pub struct Fence {
    asked: std::collections::HashMap<PathBuf, FileVersion>,
    epoch: u64,
}

impl Fence {
    /// Stands from now: call with pending change notifications delivered
    /// ([`pump`], or a running main run loop).
    pub fn new(asked: &Inventory) -> Self {
        Self {
            asked: asked.0.iter().cloned().collect(),
            epoch: epoch(),
        }
    }

    /// Whether the fonts are still the ones asked for; if not, why. The
    /// installed fonts have not changed since the fence was made: the
    /// count of changes is compared, never the list, so a font installed
    /// and removed again in between does not pass. And every font file this
    /// process has read is the version asked for (when it is among the
    /// installed ones) and is that version still. One look per font file
    /// read, whatever is installed; call with pending notifications
    /// delivered.
    pub fn hold(&self) -> Result<(), String> {
        if epoch() != self.epoch {
            return Err("the installed fonts changed".to_owned());
        }
        for (path, read) in files_read() {
            if self.asked.get(&path).is_some_and(|asked| *asked != read)
                || FileVersion::of(&path) != Some(read)
            {
                return Err(format!("{} is not the file it was", path.display()));
            }
        }
        Ok(())
    }
}

/// Makes the fonts of the file at `path` available to this process only
/// (nothing is installed: no other process sees them and they are gone when
/// it exits), as a font manager's activation is to one app. CoreText posts
/// its change notification.
pub fn register_for_process(path: &std::path::Path) -> Result<(), String> {
    scope_process(path, true)
}

/// Undoes [`register_for_process`].
pub fn unregister_for_process(path: &std::path::Path) -> Result<(), String> {
    scope_process(path, false)
}

fn scope_process(path: &std::path::Path, register: bool) -> Result<(), String> {
    #[cfg(all(target_os = "macos", feature = "system"))]
    {
        use objc2_core_text::{
            CTFontManagerRegisterFontsForURL, CTFontManagerScope,
            CTFontManagerUnregisterFontsForURL,
        };
        let url = objc2_core_foundation::CFURL::from_file_path(path)
            .ok_or_else(|| format!("{} is not a file path", path.display()))?;
        let ok = unsafe {
            if register {
                CTFontManagerRegisterFontsForURL(
                    &url,
                    CTFontManagerScope::Process,
                    std::ptr::null_mut(),
                )
            } else {
                CTFontManagerUnregisterFontsForURL(
                    &url,
                    CTFontManagerScope::Process,
                    std::ptr::null_mut(),
                )
            }
        };
        if ok {
            Ok(())
        } else {
            Err(format!(
                "CoreText refused to {} {}",
                if register { "register" } else { "unregister" },
                path.display()
            ))
        }
    }
    #[cfg(not(all(target_os = "macos", feature = "system")))]
    {
        let _ = (path, register);
        Err("the native font registry is macOS only".into())
    }
}
