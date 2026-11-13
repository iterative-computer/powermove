//! The fonts installed on this Mac change while the app runs: a font is
//! installed, removed, activated by a font manager, or registered by the
//! process. CoreText says so with one notification, posted on the main run
//! loop. This module counts them ([`epoch`]): the registry retires what it
//! resolved before a change, its generation and the broker's move on, so
//! every renderer installs fresh fonts and nothing keyed on the generation
//! (a frame cache) outlives the fonts it was drawn with.

use std::path::PathBuf;
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

/// Calls `listener` after every change, on the thread that saw it (the main
/// thread; it must not block), for as long as it answers true.
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
    #[cfg(all(target_os = "macos", feature = "system"))]
    {
        use objc2_core_foundation::CFURL;
        let urls = unsafe { objc2_core_text::CTFontManagerCopyAvailableFontURLs() };
        let urls: &objc2_core_foundation::CFArray<CFURL> = unsafe { urls.cast_unchecked() };
        let mut files: Vec<PathBuf> = urls.iter().filter_map(|url| url.to_file_path()).collect();
        files.sort();
        files.dedup();
        files
    }
    #[cfg(not(all(target_os = "macos", feature = "system")))]
    Vec::new()
}

/// The font files CoreText resolves from at one moment, each with its
/// length and modification time (nanoseconds): the fonts a render can
/// resolve to, by stable identity. A render in another process draws with
/// the same fonts when that process's inventory is this one
/// ([`Inventory::adopt`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Inventory(pub Vec<(PathBuf, u64, u64)>);

impl Inventory {
    /// This process's fonts now ([`available_files`]).
    pub fn current() -> Self {
        Inventory(
            available_files()
                .into_iter()
                .map(|path| {
                    let meta = std::fs::metadata(&path).ok();
                    let modified = meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
                    (path, meta.map_or(0, |m| m.len()), modified)
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
    /// difference.
    pub fn verify(&self) -> Result<(), String> {
        let now = Self::current();
        if now == *self {
            return Ok(());
        }
        let (ours, theirs): (std::collections::HashSet<_>, std::collections::HashSet<_>) =
            (now.0.iter().collect(), self.0.iter().collect());
        let name = |f: Option<&&(PathBuf, u64, u64)>| f.map(|f| f.0.display().to_string());
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
