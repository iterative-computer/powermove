//! Powermove ▸ Check for Updates… through Sparkle 2, and What's New (S112).
//!
//! A release bundle carries `Contents/Frameworks/Sparkle.framework` and, in its
//! Info.plist, the appcast (`SUFeedURL`) and the EdDSA key that signs every
//! archive (`SUPublicEDKey`). The framework is loaded at the first editor window
//! and Sparkle owns the rest: scheduled checks, the update window with its
//! notes, download, EdDSA and code-signature verification, and replacement on
//! quit or relaunch through the app's normal quit (dirty projects still ask).
//!
//! Only a shipping build on a shipping bundle id may start Sparkle against a
//! public HTTPS feed. Development builds, lane and profile launches start it only
//! against a loopback feed (a local proof), so they never contact the real feed.
//! What's New reads its release notes from the development fixture
//! `<profile>/update-feed.json` and never touches the network.
use crate::alert::{Alert, Button};
use gpui::*;
use objc2::{
    MainThreadMarker, MainThreadOnly, Message, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_app_kit::{NSAlert, NSColor, NSFont, NSScrollView, NSTextView, NSWindow};
use objc2_foundation::{NSBundle, NSPoint, NSRange, NSRect, NSSize, NSString};
use pm_services::{
    Result as ServiceResult, ServiceErrorCode,
    storage::ServiceStorage,
    surface_backing::maintenance::{Release, ReleaseClaim, ReleaseHost, WhatsNew},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const SEEN: &str = "app.whats-new.seen";
const FIXTURE: &str = "update-feed.json";
pub const RELEASES_PAGE: &str = "https://github.com/iterative-computer/powermove/releases";
/// The shipped app's identity: V1 and V2 stable share it, so V1's Squirrel.Mac
/// updater swaps V2 in place (packaging/package.py `IDS["cutover"]`).
pub const PRODUCTION_ID: &str = "com.zellzoi.powermove";

/// What the development fixture holds: the release history for What's New.
#[derive(Clone, Default, Deserialize)]
pub struct Feed {
    /// GitHub release objects (`tag_name`, `name`, `body`, `published_at`, …).
    #[serde(default)]
    pub releases: Vec<serde_json::Value>,
}
impl Feed {
    fn releases(&self) -> Vec<Release> {
        self.releases
            .iter()
            .filter_map(|r| {
                Some(Release {
                    version: r["tag_name"].as_str()?.to_owned(),
                    draft: r["draft"].as_bool().unwrap_or(false),
                    prerelease: r["prerelease"].as_bool().unwrap_or(false),
                    metadata: r.clone(),
                })
            })
            .collect()
    }
}

/// The development fixture, when this is a development build that has one.
fn fixture(profile: &Path) -> Option<PathBuf> {
    let path = profile.join(FIXTURE);
    (cfg!(debug_assertions) && path.is_file()).then_some(path)
}
/// The running app's version: the bundle's, else the crate's.
pub fn current_version() -> String {
    info_string("CFBundleShortVersionString").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into())
}
fn info_string(key: &str) -> Option<String> {
    NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str(key))
        .and_then(|v| v.downcast::<NSString>().ok())
        .map(|v| v.to_string())
}
fn read_feed(path: &Path) -> std::result::Result<Feed, ServiceErrorCode> {
    let bytes = std::fs::read(path).map_err(|_| ServiceErrorCode::Unavailable)?;
    serde_json::from_slice(&bytes).map_err(|_| ServiceErrorCode::InvalidData)
}
fn storage(profile: &Path) -> ServiceResult<ServiceStorage> {
    ServiceStorage::open(profile.join("updates"))
}

#[derive(Default)]
struct State {
    whats_new: WhatsNew,
}
impl Global for State {}

/// Whether this build may start Sparkle against `feed`. A public feed needs an
/// HTTPS URL, a shipping build (no debug assertions, no profile or lane
/// launch) and a shipping bundle id; a loopback feed (a local proof) is the
/// only one a development or lane build may use, and never on a shipping id.
pub(crate) fn feed_allowed(feed: &str, has_key: bool, bundle: &str, shipping: bool) -> bool {
    let Ok(url) = url::Url::parse(feed) else {
        return false;
    };
    let shipping_id = bundle == PRODUCTION_ID || bundle == pm_first_launch::BETA_BUNDLE_ID;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if !has_key || url.username() != "" || url.password().is_some() {
        return false;
    }
    if loopback {
        return matches!(url.scheme(), "http" | "https") && !shipping_id;
    }
    url.scheme() == "https" && shipping && shipping_id
}
/// A development, lane or profile launch: never a person's own app.
fn development_launch() -> bool {
    cfg!(debug_assertions)
        || std::env::args().any(|a| a == "--profile")
        || ["PM_NATIVE_PROFILE", "PM_NATIVE_REQUIRE_PROFILE", "PM_NATIVE_NO_ORIGINAL"]
            .iter()
            .any(|name| std::env::var_os(name).is_some())
}

/// Sparkle's standard controller, once loaded; main thread only.
struct Sparkle(Option<Retained<AnyObject>>);
impl Global for Sparkle {}

/// Loads the bundled Sparkle and starts its updater (scheduled checks run as
/// the Info.plist says). Without a framework, feed or key, or when this build
/// may not use the feed, nothing loads and Check for Updates says so.
fn start_sparkle(cx: &mut App) {
    if cx.has_global::<Sparkle>() {
        return;
    }
    let controller = sparkle_controller();
    cx.set_global(Sparkle(controller));
}
fn sparkle_controller() -> Option<Retained<AnyObject>> {
    let feed = info_string("SUFeedURL")?;
    let has_key = info_string("SUPublicEDKey").is_some_and(|k| !k.trim().is_empty());
    let bundle = crate::platform::bundle_id().unwrap_or_default();
    if !feed_allowed(&feed, has_key, &bundle, !development_launch()) {
        eprintln!("powermove: updates are off in this build (feed not allowed here)");
        return None;
    }
    let frameworks = NSBundle::mainBundle().privateFrameworksPath()?;
    let path = PathBuf::from(frameworks.to_string()).join("Sparkle.framework");
    let framework = NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?))?;
    // SAFETY: loads the bundle's own signed Sparkle.framework (its code runs here).
    if !unsafe { framework.load() } {
        eprintln!("powermove: Sparkle.framework did not load; updates are off");
        return None;
    }
    let class = AnyClass::get(c"SPUStandardUpdaterController")?;
    // SAFETY: Sparkle 2's public initializer; delegates are optional (nil).
    unsafe {
        let none = std::ptr::null::<AnyObject>();
        let object: *mut AnyObject = msg_send![class, alloc];
        let object: *mut AnyObject = msg_send![
            object,
            initWithStartingUpdater: Bool::YES,
            updaterDelegate: none,
            userDriverDelegate: none
        ];
        Retained::from_raw(object)
    }
}

fn say(window: &NSWindow, message: &str, informative: &str) {
    let alert = Alert::new(message, informative, vec![Button::new("OK", ()).default()]);
    let _ = alert.sheet(window);
}

/// Powermove ▸ Check for Updates…: Sparkle's own check, window and notes.
pub fn check_now(_profile: PathBuf, window: Retained<NSWindow>, cx: &mut App) {
    start_sparkle(cx);
    match cx.try_global::<Sparkle>().and_then(|s| s.0.clone()) {
        // SAFETY: SPUStandardUpdaterController's IBAction; sender may be nil.
        Some(controller) => unsafe {
            let _: () = msg_send![&*controller, checkForUpdates: std::ptr::null::<AnyObject>()];
        },
        None => say(
            &window,
            "Couldn’t check for updates.",
            "This build of Powermove doesn’t update itself.",
        ),
    }
}

fn persist(key: &'static str, value: serde_json::Value, cx: &mut App) {
    let Some(profile) = cx.try_global::<Profile>().map(|p| p.0.clone()) else {
        return;
    };
    cx.background_executor()
        .spawn(async move {
            if let Ok(mut s) = storage(&profile) {
                let _ = s.set(key, value);
            }
        })
        .detach();
}
struct Profile(PathBuf);
impl Global for Profile {}

/// The first editor window after a launch that changed the version shows
/// What's New once, when no other sheet is up; the seen marker is written
/// when it is shown. Offline or no notes: nothing shows and it retries next launch.
pub fn after_launch(profile: PathBuf, window: Retained<NSWindow>, cx: &mut App) {
    if cx.has_global::<Profile>() {
        return;
    }
    cx.set_global(Profile(profile.clone()));
    start_sparkle(cx);
    let source = fixture(&profile);
    let work = cx.background_executor().spawn(async move {
        let seen = storage(&profile)
            .ok()
            .and_then(|s| s.setting(SEEN).ok().flatten())
            .and_then(|v| v.as_str().map(str::to_owned));
        let releases = source
            .as_deref()
            .and_then(|p| read_feed(p).ok())
            .map(|f| f.releases());
        (seen, releases)
    });
    cx.spawn(async move |cx| {
        let (seen, releases) = work.await;
        cx.update(|cx| {
            let mut host = Prefetched {
                seen,
                releases,
                marked: None,
            };
            let current = current_version();
            let key = format!("{:p}", Retained::as_ptr(&window));
            let claim = cx
                .default_global::<State>()
                .whats_new
                .pending(&mut host, &key, &current)
                .ok()
                .flatten();
            if let Some(version) = host.marked.take() {
                persist(SEEN, serde_json::json!(version), cx);
            }
            if let Some(claim) = claim {
                present_when_free(claim, host.seen.unwrap_or_default(), window, cx);
            }
        });
    })
    .detach();
}
/// Waits for the window to be free of other sheets (recovery, onboarding).
fn present_when_free(
    claim: ReleaseClaim,
    previous: String,
    window: Retained<NSWindow>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while window.attachedSheet().is_some() {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
        }
        cx.update(|cx| whats_new(claim, &previous, &window, cx));
    })
    .detach();
}
fn whats_new(claim: ReleaseClaim, previous: &str, window: &NSWindow, cx: &mut App) {
    let t = pm_shell::tokens();
    let entries: Vec<_> = claim
        .releases
        .iter()
        .map(|r| {
            let m = &r.metadata;
            let name = m["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .unwrap_or(&r.version);
            let notes = m["body"]
                .as_str()
                .filter(|b| !b.trim().is_empty())
                .unwrap_or("Fixes and improvements.");
            (
                Some(name.to_owned()),
                m["published_at"].as_str().and_then(long_date),
                notes.to_owned(),
            )
        })
        .collect();
    let alert = Alert::new(
        format!("What’s New in Powermove {}", claim.current),
        format!("Powermove updated from {previous} to {}.", claim.current),
        vec![
            Button::new("Continue", false).default().cancel(),
            Button::new("View on GitHub", true),
        ],
    );
    let accessory = notes_view(&entries, &t, t.metrics.sheet_width_wide);
    let rx = alert.sheet_with(window, |native| set_accessory(native, &accessory));
    // Marked seen now that it's on screen, even if a quit dismisses it.
    let mut host = Prefetched::default();
    if cx
        .global_mut::<State>()
        .whats_new
        .shown(&mut host, &claim)
        .is_ok()
        && let Some(version) = host.marked
    {
        persist(SEEN, serde_json::json!(version), cx);
    }
    cx.spawn(async move |_| {
        if rx.recv().await.ok().flatten() == Some(true) {
            crate::platform::open_url(RELEASES_PAGE);
        }
    })
    .detach();
}
/// `2026-11-04T…` in the system's long date style.
fn long_date(iso: &str) -> Option<String> {
    let day = iso.get(..10)?;
    let formatter = objc2_foundation::NSDateFormatter::new();
    formatter.setDateFormat(Some(&NSString::from_str("yyyy-MM-dd")));
    let date = formatter.dateFromString(&NSString::from_str(day))?;
    formatter.setDateFormat(None);
    formatter.setDateStyle(objc2_foundation::NSDateFormatterStyle::LongStyle);
    formatter.setTimeStyle(objc2_foundation::NSDateFormatterStyle::NoStyle);
    Some(formatter.stringFromDate(&date).to_string())
}

/// The What's New marker and release history, read before the check and
/// written after it, so no file work happens on the main thread.
#[derive(Default)]
struct Prefetched {
    seen: Option<String>,
    releases: Option<Vec<Release>>,
    marked: Option<String>,
}
impl ReleaseHost for Prefetched {
    fn seen_version(&mut self) -> ServiceResult<Option<String>> {
        Ok(self.seen.clone())
    }
    fn releases(&mut self) -> ServiceResult<Vec<Release>> {
        self.releases.clone().ok_or_else(|| {
            pm_services::ServiceError::message(ServiceErrorCode::Offline, "no release notes")
        })
    }
    fn mark_seen(&mut self, version: &str) -> ServiceResult<()> {
        self.marked = Some(version.into());
        Ok(())
    }
}

/// Release notes as one scrolling text view: each release's name and date,
/// then its Markdown (inline: links work, nothing remote loads).
fn notes_view(
    entries: &[(Option<String>, Option<String>, String)],
    t: &pm_design::Tokens,
    width: f32,
) -> Retained<NSScrollView> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let frame = NSRect::new(
        NSPoint::new(0., 0.),
        NSSize::new(width as f64, t.metrics.palette_height as f64),
    );
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
    scroll.setHasVerticalScroller(true);
    scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
    scroll.setDrawsBackground(false);
    let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), frame);
    text.setEditable(false);
    text.setSelectable(true);
    text.setDrawsBackground(false);
    text.setVerticallyResizable(true);
    text.setHorizontallyResizable(false);
    unsafe {
        let storage: *mut AnyObject = msg_send![&*text, textStorage];
        for (index, (name, date, notes)) in entries.iter().enumerate() {
            let gap = if index == 0 { "" } else { "\n\n" };
            if let Some(name) = name {
                append(
                    storage,
                    &format!("{gap}{name}\n"),
                    &NSFont::boldSystemFontOfSize(t.typography.title as f64),
                    &NSColor::labelColor(),
                );
            }
            if let Some(date) = date {
                append(
                    storage,
                    &format!("{date}\n"),
                    &NSFont::systemFontOfSize(t.typography.minimum as f64),
                    &NSColor::secondaryLabelColor(),
                );
            }
            append_markdown(storage, notes, t.typography.reading as f64);
        }
    }
    scroll.setDocumentView(Some(&text));
    scroll
}
unsafe fn append(storage: *mut AnyObject, text: &str, font: &NSFont, colour: &NSColor) {
    unsafe {
        let ns = NSString::from_str(text);
        let piece: *mut AnyObject = msg_send![objc2::class!(NSMutableAttributedString), alloc];
        let piece: *mut AnyObject = msg_send![piece, initWithString: &*ns];
        style(piece, font, colour);
        let _: () = msg_send![storage, appendAttributedString: piece];
        let _: () = msg_send![piece, release];
    }
}
unsafe fn style(piece: *mut AnyObject, font: &NSFont, colour: &NSColor) {
    unsafe {
        let length: usize = msg_send![piece, length];
        let range = NSRange::new(0, length);
        let font_key = NSString::from_str("NSFont");
        let colour_key = NSString::from_str("NSColor");
        let _: () = msg_send![piece, addAttribute: &*font_key, value: font, range: range];
        let _: () = msg_send![piece, addAttribute: &*colour_key, value: colour, range: range];
    }
}
/// Inline Markdown keeps line breaks and links; anything unparseable reads as text.
unsafe fn append_markdown(storage: *mut AnyObject, markdown: &str, size: f64) {
    unsafe {
        let font = NSFont::systemFontOfSize(size);
        let colour = NSColor::labelColor();
        let options: *mut AnyObject =
            msg_send![objc2::class!(NSAttributedStringMarkdownParsingOptions), new];
        // NSAttributedStringMarkdownInterpretedSyntaxInlineOnlyPreservingWhitespace
        let _: () = msg_send![options, setInterpretedSyntax: 2isize];
        let ns = NSString::from_str(markdown);
        let parsed: *mut AnyObject = msg_send![objc2::class!(NSMutableAttributedString), alloc];
        let parsed: *mut AnyObject = msg_send![
            parsed,
            initWithMarkdownString: &*ns,
            options: options,
            baseURL: std::ptr::null::<AnyObject>(),
            error: std::ptr::null_mut::<*mut AnyObject>()
        ];
        let _: () = msg_send![options, release];
        if parsed.is_null() {
            append(storage, markdown, &font, &colour);
            return;
        }
        style(parsed, &font, &colour);
        let _: () = msg_send![storage, appendAttributedString: parsed];
        let _: () = msg_send![parsed, release];
    }
}
fn set_accessory(native: &NSAlert, view: &NSScrollView) {
    native.setAccessoryView(Some(view));
}

#[cfg(test)]
mod tests {
    use super::{Feed, Prefetched, current_version};
    use pm_services::surface_backing::maintenance::{ReleaseHost, WhatsNew};
    #[test]
    fn fixture_feeds_read_github_release_objects() {
        let feed: Feed = serde_json::from_str(
            r#"{"releases":[{"tag_name":"v9.0.0","name":"Powermove 9","body":"- Faster"},
                            {"tag_name":"v8.9.0-beta.1","prerelease":true},{"name":"no tag"}]}"#,
        )
        .unwrap();
        let releases = feed.releases();
        assert_eq!(releases.len(), 2);
        assert!(releases[1].prerelease);
    }
    #[test]
    fn only_a_shipping_build_on_a_shipping_id_may_use_a_public_feed() {
        use super::{PRODUCTION_ID, feed_allowed};
        let beta = pm_first_launch::BETA_BUNDLE_ID;
        let public = "https://github.com/iterative-computer/powermove/releases/latest/download/appcast.xml";
        assert!(feed_allowed(public, true, PRODUCTION_ID, true));
        assert!(feed_allowed(public, true, beta, true));
        // Development, lane and profile launches never contact the real feed.
        assert!(!feed_allowed(public, true, PRODUCTION_ID, false));
        assert!(!feed_allowed(public, true, "com.powermove.native.dev", true));
        assert!(!feed_allowed(public, true, "com.powermove.native.dev.w122-release", false));
        // No key, plain HTTP, credentials or nonsense: never.
        assert!(!feed_allowed(public, false, PRODUCTION_ID, true));
        assert!(!feed_allowed("http://example.com/appcast.xml", true, PRODUCTION_ID, true));
        assert!(!feed_allowed("https://u:p@example.com/appcast.xml", true, PRODUCTION_ID, true));
        assert!(!feed_allowed("appcast.xml", true, PRODUCTION_ID, true));
        // A loopback feed is a local proof: development ids only.
        let local = "http://127.0.0.1:8765/appcast.xml";
        assert!(feed_allowed(local, true, "com.powermove.native.dev.w122-release", false));
        assert!(feed_allowed("http://[::1]:8765/appcast.xml", true, "com.powermove.native.dev", false));
        assert!(!feed_allowed(local, true, PRODUCTION_ID, true));
        assert!(!feed_allowed("http://localhost.example.com/appcast.xml", true, "com.powermove.native.dev", false));
    }
    #[test]
    fn whats_new_waits_for_notes_and_marks_a_fresh_install_seen() {
        let current = current_version();
        let mut fresh = Prefetched::default();
        assert!(
            WhatsNew::default()
                .pending(&mut fresh, "w", &current)
                .unwrap()
                .is_none()
        );
        assert_eq!(fresh.marked.as_deref(), Some(current.as_str()));
        let mut offline = Prefetched {
            seen: Some("0.0.1".into()),
            ..Default::default()
        };
        assert!(
            WhatsNew::default()
                .pending(&mut offline, "w", &current)
                .is_err()
        );
        assert!(offline.marked.is_none());
        let _ = offline.releases();
    }
}
