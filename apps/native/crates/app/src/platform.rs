//! AppKit sheets are asynchronous. No modal run loop or UI-thread file IO.
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::NSString;
use std::path::PathBuf;

// `UTType` is looked up by name for open panels; load its framework.
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

static TERMINATE: std::sync::OnceLock<(async_channel::Sender<()>, async_channel::Receiver<()>)> =
    std::sync::OnceLock::new();
static TERMINATE_HEARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Call first in `main`, before any thread exists: every thread inherits
/// SIGTERM blocked, and one waits for it, so SIGTERM runs the quit path
/// ([`terminations`]) instead of killing the process mid-write.
pub fn route_sigterm() {
    ignore_user_signals();
    let tx = TERMINATE
        .get_or_init(|| async_channel::bounded(1))
        .0
        .clone();
    // SAFETY: plain signal-set calls on a local set, before other threads.
    let set = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    };
    let _ = std::thread::Builder::new()
        .name("pm-sigterm".into())
        .spawn(move || {
            loop {
                let mut signal = 0;
                // SAFETY: waits on the blocked set built above.
                if unsafe { libc::sigwait(&set, &mut signal) } != 0 {
                    continue;
                }
                if !TERMINATE_HEARD.load(std::sync::atomic::Ordering::Acquire) {
                    // No window yet, so nothing is open to close.
                    std::process::exit(0);
                }
                let _ = tx.try_send(());
            }
        });
}
/// SIGUSR1 and SIGUSR2 terminate a process by default, and other tools send
/// them to apps they take for Electron (to open its inspector): the editor
/// would vanish without saving. A no-op handler, not `SIG_IGN`, so a child
/// process gets the default back at `exec`.
fn ignore_user_signals() {
    extern "C" fn ignore(_: libc::c_int) {}
    for signal in [libc::SIGUSR1, libc::SIGUSR2] {
        // SAFETY: installs an async-signal-safe handler that does nothing.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = ignore as extern "C" fn(libc::c_int) as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// SIGTERMs from [`route_sigterm`], for the quit coordinator.
pub fn terminations() -> async_channel::Receiver<()> {
    TERMINATE_HEARD.store(true, std::sync::atomic::Ordering::Release);
    TERMINATE
        .get_or_init(|| async_channel::bounded(1))
        .1
        .clone()
}

/// An open sheet that takes `.cube` files.
pub fn cube_file(window: &NSWindow) -> async_channel::Receiver<Option<PathBuf>> {
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str("Choose a .cube LUT")));
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&objc2_foundation::NSArray::from_retained_slice(&[
        NSString::from_str("cube"),
    ])));
    let retained = panel.clone();
    let done = block2::RcBlock::new(move |result: NSModalResponse| {
        let path = (result == NSModalResponseOK)
            .then(|| retained.URL())
            .flatten()
            .and_then(|u| u.path())
            .map(|p| PathBuf::from(p.to_string()));
        let _ = tx.try_send(path);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &done);
    rx
}

pub fn paths(window: &NSWindow, save: bool) -> async_channel::Receiver<Option<PathBuf>> {
    paths_named(window, save, "Project.pmv")
}
pub fn paths_named(
    window: &NSWindow,
    save: bool,
    name: &str,
) -> async_channel::Receiver<Option<PathBuf>> {
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    if save {
        let panel = NSSavePanel::savePanel(mtm);
        panel.setNameFieldStringValue(&NSString::from_str(name));
        let retained = panel.clone();
        let done = block2::RcBlock::new(move |result: NSModalResponse| {
            let path = (result == NSModalResponseOK)
                .then(|| retained.URL())
                .flatten()
                .and_then(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()));
            let _ = tx.try_send(path);
        });
        panel.beginSheetModalForWindow_completionHandler(window, &done);
    } else {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        let retained = panel.clone();
        let done = block2::RcBlock::new(move |result: NSModalResponse| {
            let path = (result == NSModalResponseOK)
                .then(|| retained.URL())
                .flatten()
                .and_then(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()));
            let _ = tx.try_send(path);
        });
        panel.beginSheetModalForWindow_completionHandler(window, &done);
    }
    rx
}

/// While held, macOS neither App Naps nor throttles the app, so a project that is
/// opening keeps going when the window is behind others or the screen locks.
/// Idle system sleep stays allowed.
pub struct Busy(Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>);
impl Busy {
    pub fn begin(reason: &str) -> Self {
        let options = objc2_foundation::NSActivityOptions::UserInitiatedAllowingIdleSystemSleep;
        Self(
            objc2_foundation::NSProcessInfo::processInfo()
                .beginActivityWithOptions_reason(options, &NSString::from_str(reason)),
        )
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        // SAFETY: the token beginActivity returned, ended once.
        unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.0) }
    }
}

/// Hands a web link to the default browser.
pub fn open_url(url: &str) -> bool {
    objc2_foundation::NSURL::URLWithString(&NSString::from_str(url))
        .is_some_and(|url| objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url))
}

/// Import Media: several items at once, filtered to `extensions` when it has any.
pub fn import_paths(
    window: &NSWindow,
    folders: bool,
    multiple: bool,
    extensions: &[&str],
) -> async_channel::Receiver<Vec<PathBuf>> {
    choose_paths(window, folders, multiple, extensions, "Import")
}

/// The general pasteboard's HTML and PNG, for Figma paste. Main thread: the
/// pasteboard is read where AppKit expects it, once per paste.
pub fn pasteboard_figma() -> Option<(String, Option<Vec<u8>>)> {
    let board = NSPasteboard::generalPasteboard();
    let html = board
        .stringForType(unsafe { NSPasteboardTypeHTML })?
        .to_string();
    if !html.contains("(figma)") {
        return None;
    }
    let png = board
        .dataForType(unsafe { NSPasteboardTypePNG })
        .map(|data| data.to_vec());
    Some((html, png))
}

/// Files for the Agent panel's Add Attachments.
pub fn attach_paths(window: &NSWindow) -> async_channel::Receiver<Vec<PathBuf>> {
    choose_paths(window, false, true, &[], "Attach")
}

#[cfg(debug_assertions)]
thread_local! {
    static REVIEW_ANSWER: std::cell::RefCell<Option<Vec<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}
/// Development builds: the next open panel answers `paths` without showing.
/// The system panel takes no background input, so on-screen reviews of
/// Import, Locate File… and the rest choose their files here.
#[cfg(debug_assertions)]
pub fn answer_next_open_panel(paths: Vec<PathBuf>) {
    REVIEW_ANSWER.with(|a| *a.borrow_mut() = Some(paths));
}

/// An open panel sheet whose default button reads `prompt`.
pub fn choose_paths(
    window: &NSWindow,
    folders: bool,
    multiple: bool,
    extensions: &[&str],
    prompt: &str,
) -> async_channel::Receiver<Vec<PathBuf>> {
    use objc2::{msg_send, runtime::AnyObject};
    use objc2_foundation::NSArray;
    let (tx, rx) = async_channel::bounded(1);
    #[cfg(debug_assertions)]
    if let Some(paths) = REVIEW_ANSWER.with(|a| a.borrow_mut().take()) {
        let _ = tx.try_send(paths);
        return rx;
    }
    let mtm = MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(!folders);
    panel.setCanChooseDirectories(folders);
    panel.setAllowsMultipleSelection(multiple);
    panel.setPrompt(Some(&NSString::from_str(prompt)));
    panel.setCanCreateDirectories(folders);
    let types: Vec<Retained<AnyObject>> = extensions
        .iter()
        .filter_map(|extension| unsafe {
            let extension = NSString::from_str(extension);
            msg_send![objc2::class!(UTType), typeWithFilenameExtension: &*extension]
        })
        .collect();
    if !types.is_empty() {
        let types = NSArray::from_retained_slice(&types);
        unsafe {
            let _: () = msg_send![&*panel, setAllowedContentTypes: &*types];
        }
    }
    let retained = panel.clone();
    let done = block2::RcBlock::new(move |result: NSModalResponse| {
        let paths = if result == NSModalResponseOK {
            retained
                .URLs()
                .iter()
                .filter_map(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .collect()
        } else {
            vec![]
        };
        let _ = tx.try_send(paths);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &done);
    rx
}

/// This process's bundle identifier; none for a bare binary.
pub fn bundle_id() -> Option<String> {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}
/// The `UTType` of a file, read from the file itself.
fn content_type(path: &std::path::Path) -> Option<Retained<objc2::runtime::AnyObject>> {
    use objc2_foundation::{NSArray, NSURL, NSURLContentTypeKey};
    let url = NSURL::from_file_path(path)?;
    let keys = NSArray::from_slice(&[unsafe { NSURLContentTypeKey }]);
    let values = url.resourceValuesForKeys_error(&keys).ok()?;
    values.objectForKey(unsafe { NSURLContentTypeKey })
}
/// Whether `kind` is an image, movie or sound and nothing that runs.
fn media_type(kind: &objc2::runtime::AnyObject) -> bool {
    use objc2::{msg_send, runtime::Bool};
    let conforms = |identifier: &str| -> bool {
        let identifier = NSString::from_str(identifier);
        let other: Option<Retained<objc2::runtime::AnyObject>> =
            unsafe { msg_send![objc2::class!(UTType), typeWithIdentifier: &*identifier] };
        other.is_some_and(|other| {
            let yes: Bool = unsafe { msg_send![kind, conformsToType: &*other] };
            yes.as_bool()
        })
    };
    ["public.image", "public.movie", "public.audio"]
        .iter()
        .any(|t| conforms(t))
        && ![
            "public.executable",
            "com.apple.application",
            "com.apple.bundle",
            "public.directory",
        ]
        .iter()
        .any(|t| conforms(t))
}
/// A file an agent's reply names, made safe to show: every link resolved, a
/// regular file with no execute bit, whose own content type (read from the
/// resolved file, not the name the reply wrote) is an image, movie or sound.
/// Disk and Launch Services work: never on the main thread.
pub fn media_document(path: &std::path::Path) -> Result<PathBuf, &'static str> {
    use std::os::unix::fs::PermissionsExt;
    const MOVED: &str = "That file couldn’t be found. It may have moved.";
    const REFUSED: &str = "That file isn’t an image, video or sound, so it wasn’t opened.";
    let resolved = std::fs::canonicalize(path).map_err(|_| MOVED)?;
    let metadata = std::fs::symlink_metadata(&resolved).map_err(|_| MOVED)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 != 0 {
        return Err(REFUSED);
    }
    match content_type(&resolved) {
        Some(kind) if media_type(&kind) => Ok(resolved),
        _ => Err(REFUSED),
    }
}
/// Opens a checked media file ([`media_document`]) as a document in the
/// default viewer for its content type, never through a URL Launch Services
/// could run as an app.
pub fn open_document(path: &std::path::Path) -> bool {
    use objc2::msg_send;
    use objc2_foundation::{NSArray, NSURL};
    let (Some(url), Some(kind)) = (NSURL::from_file_path(path), content_type(path)) else {
        return false;
    };
    let workspace = NSWorkspace::sharedWorkspace();
    let viewer: Option<Retained<NSURL>> =
        unsafe { msg_send![&*workspace, URLForApplicationToOpenContentType: &*kind] };
    let Some(viewer) = viewer else {
        return false;
    };
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
        &NSArray::from_retained_slice(&[url]),
        &viewer,
        &configuration,
        None,
    );
    true
}
/// Selects `path` in Finder.
pub fn reveal(path: &std::path::Path) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSURL};
    if let Some(url) = NSURL::from_file_path(path) {
        NSWorkspace::sharedWorkspace()
            .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    }
}
pub fn history_labels(undo: Option<&str>, redo: Option<&str>) {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    let Some(root) = app.mainMenu() else { return };
    for top in root.itemArray() {
        if top.title().to_string() != "Edit" {
            continue;
        }
        if let Some(menu) = top.submenu() {
            for (index, (verb, label)) in [("Undo", undo), ("Redo", redo)].into_iter().enumerate() {
                if let Some(item) = menu.itemAtIndex(index as isize) {
                    item.setTitle(&NSString::from_str(
                        &label.map_or(verb.to_string(), |l| format!("{verb} {l}")),
                    ));
                    item.setEnabled(label.is_some());
                }
            }
        }
    }
}

// Preserve GPUI's existing application delegate while intercepting termination
// before its non-cancellable shutdown callback. All other selectors forward.
use objc2::{
    DefinedClass, define_class, msg_send,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use std::cell::Cell;
pub struct QuitState {
    previous: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
    sender: async_channel::Sender<()>,
    allowed: Cell<bool>,
    /// Quit Apple Events waiting for the quit's outcome (suspension ids).
    suspended: std::cell::RefCell<Vec<usize>>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNativeQuitDelegate"]
    #[ivars = QuitState]
    pub struct QuitDelegate;
    unsafe impl NSObjectProtocol for QuitDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            selector == objc2::sel!(applicationShouldTerminate:)
                || unsafe { msg_send![&*self.ivars().previous, respondsToSelector: selector] }
        }
    }
    unsafe impl NSApplicationDelegate for QuitDelegate {
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _: &NSApplication) -> NSApplicationTerminateReply {
            if self.ivars().allowed.get() { NSApplicationTerminateReply::TerminateNow }
            else { let _=self.ivars().sender.try_send(()); NSApplicationTerminateReply::TerminateCancel }
        }
    }
    impl QuitDelegate {
        /// The Quit Apple Event (Dock, Activity Monitor, `osascript … quit`,
        /// logout). AppKit drops it while a window shows a sheet, so a hung
        /// extension's D11 sheet would hold the app open; the event goes to
        /// the quit coordinator directly instead, which ends that sheet.
        #[unsafe(method(handleQuitEvent:withReplyEvent:))]
        fn handle_quit_event(&self, _event: *mut AnyObject, _reply: *mut AnyObject) {
            if self.ivars().allowed.get() {
                let mtm = MainThreadMarker::new().unwrap();
                NSApplication::sharedApplication(mtm).terminate(None);
                return;
            }
            // The sender hears the outcome: quit, or cancelled (-128) when the
            // person cancels a save prompt (`QuitDelegate::resolve_quit_events`).
            let id: *mut Suspension =
                unsafe { msg_send![apple_events(), suspendCurrentAppleEvent] };
            if !id.is_null() {
                self.ivars().suspended.borrow_mut().push(id as usize);
            }
            let _ = self.ivars().sender.try_send(());
        }
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self, _: Sel) -> *const AnyObject {
            &*self.ivars().previous as *const _ as *const AnyObject
        }
    }
);
pub fn quit_guard() -> (Retained<QuitDelegate>, async_channel::Receiver<()>) {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let (sender, rx) = async_channel::bounded(1);
    let previous = app.delegate().expect("GPUI application delegate");
    let this = QuitDelegate::alloc(mtm).set_ivars(QuitState {
        previous,
        sender,
        allowed: Cell::new(false),
        suspended: Default::default(),
    });
    let guard: Retained<QuitDelegate> = unsafe { objc2::msg_send![super(this), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*guard)));
    // kCoreEventClass 'aevt', kAEQuitApplication 'quit'.
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const QUIT_APPLICATION: u32 = u32::from_be_bytes(*b"quit");
    unsafe {
        let manager = apple_events();
        let _: () = msg_send![
            manager,
            setEventHandler: &*guard,
            andSelector: objc2::sel!(handleQuitEvent:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: QUIT_APPLICATION
        ];
    }
    (guard, rx)
}
impl QuitDelegate {
    pub fn allow(&self) {
        self.ivars().allowed.set(true);
    }
    /// Answers the Quit Apple Events the quit kept waiting: success, or the
    /// user-cancelled error (-128) when the quit was cancelled.
    pub fn resolve_quit_events(&self, cancelled: bool) {
        const ERROR_NUMBER: u32 = u32::from_be_bytes(*b"errn");
        const USER_CANCELED: i32 = -128;
        for id in self.ivars().suspended.borrow_mut().drain(..) {
            let id = id as *mut Suspension;
            unsafe {
                let manager = apple_events();
                if cancelled {
                    let reply: *mut AnyObject =
                        msg_send![manager, replyAppleEventForSuspensionID: id];
                    let error: *mut AnyObject = msg_send![
                        objc2::class!(NSAppleEventDescriptor),
                        descriptorWithInt32: USER_CANCELED
                    ];
                    if !reply.is_null() && !error.is_null() {
                        let _: () =
                            msg_send![reply, setParamDescriptor: error, forKeyword: ERROR_NUMBER];
                    }
                }
                let _: () = msg_send![manager, resumeWithSuspensionID: id];
            }
        }
    }
}
/// `NSAppleEventManagerSuspensionID`. Debug builds check message encodings,
/// and a plain `void *` made every Quit Apple Event (Dock, logout) abort.
#[repr(C)]
pub struct Suspension {
    _opaque: [u8; 0],
}
unsafe impl objc2::RefEncode for Suspension {
    const ENCODING_REF: objc2::Encoding = objc2::Encoding::Pointer(&objc2::Encoding::Struct(
        "__NSAppleEventManagerSuspension",
        &[],
    ));
}
fn apple_events() -> *mut AnyObject {
    unsafe { msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager] }
}

/// PNG sequences choose a parent directory; publication owns a fresh child
/// folder, so existing frames are never overwritten.
/// Development builds only: `--save-dir <dir>` (`PM_NATIVE_REVIEW_SAVE_DIR`)
/// answers the export destination with `<dir>/<suggested name>` and shows no
/// panel. The save panel runs out of process, so background review checks
/// can't drive it; this lets them prove the sheet, render and file end to end.
fn review_destination(name: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("PM_NATIVE_REVIEW_SAVE_DIR").filter(|_| cfg!(debug_assertions))?;
    let name = PathBuf::from(name).file_name()?.to_owned();
    Some(PathBuf::from(dir).join(name))
}
pub fn export_destination(
    window: &NSWindow,
    folder: bool,
    name: &str,
) -> async_channel::Receiver<Option<PathBuf>> {
    if let Some(path) = review_destination(name) {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(Some(path));
        return rx;
    }
    if !folder {
        return paths_named(window, true, name);
    }
    let (tx, rx) = async_channel::bounded(1);
    let mtm = MainThreadMarker::new().unwrap();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    let retained = panel.clone();
    // The suggestion is authored by the Export model, never a pathname.
    let name = PathBuf::from(name).file_name().map(|s| s.to_owned());
    let done = block2::RcBlock::new(move |result: NSModalResponse| {
        let path = (result == NSModalResponseOK)
            .then(|| retained.URL())
            .flatten()
            .and_then(|u| u.path())
            .map(|p| PathBuf::from(p.to_string()));
        let path = path
            .zip(name.clone())
            .map(|(parent, name)| parent.join(name));
        let _ = tx.try_send(path);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &done);
    rx
}

/// The AppKit window behind a GPUI window (a panel's own window, for sheets).
pub fn ns_window(window: &gpui::Window) -> Option<Retained<NSWindow>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let view: Retained<objc2_app_kit::NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast())? };
    view.window()
}

#[cfg(test)]
mod signal_tests {
    #[test]
    fn a_stray_sigusr1_or_sigusr2_does_not_end_the_app() {
        super::ignore_user_signals();
        // SAFETY: raising a signal whose handler was just installed.
        unsafe {
            assert_eq!(libc::raise(libc::SIGUSR1), 0);
            assert_eq!(libc::raise(libc::SIGUSR2), 0);
        }
        // Still here. A child gets the default back at exec.
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -USR1 $$; sleep 1"])
            .status()
            .unwrap();
        assert!(!status.success(), "the default action ends the child");
    }
}

#[cfg(test)]
mod media_tests {
    use super::media_document;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn a_reply_path_opens_only_a_real_media_file_never_what_a_link_points_at_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let write = |name: &str, bytes: &[u8], mode: u32| {
            let path = root.join(name);
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            path
        };
        let png = write("still.png", b"\x89PNG\r\n\x1a\n", 0o644);
        let script = write("run.sh", b"#!/bin/sh\necho hi\n", 0o755);
        let text = write("notes.txt", b"hello", 0o644);
        write("marked.png", b"\x89PNG\r\n\x1a\n", 0o755);
        let app = root.join("Fake.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        write("Fake.app/Contents/MacOS/Fake", b"#!/bin/sh\n", 0o755);
        std::fs::create_dir_all(root.join("folder.png")).unwrap();
        for (name, target) in [
            ("preview.png", png.clone()),
            ("app.png", root.join("Fake.app")),
            ("tool.png", script.clone()),
            ("words.png", text.clone()),
            ("chain.png", root.join("preview.png")),
        ] {
            symlink(&target, root.join(name)).unwrap();
        }
        assert_eq!(media_document(&png), Ok(png.clone()));
        // A link (or a chain of them) opens the file it resolves to.
        assert_eq!(media_document(&root.join("preview.png")), Ok(png.clone()));
        assert_eq!(media_document(&root.join("chain.png")), Ok(png.clone()));
        // The name says .png; what it resolves to decides.
        for refused in [
            "app.png",
            "tool.png",
            "words.png",
            "marked.png",
            "folder.png",
            "Fake.app",
        ] {
            assert_eq!(
                media_document(&root.join(refused)),
                Err("That file isn’t an image, video or sound, so it wasn’t opened."),
                "{refused}"
            );
        }
        assert!(media_document(&root.join("gone.png")).is_err());
    }
}
