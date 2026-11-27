//! Powermove ▸ Check for Updates… (S111) and What's New (S112), over pm-services
//! `updater` (feed admission) and `surface_backing::maintenance` (state).
//!
//! The feed and the What's New marker are read and written off the main
//! thread; the sheets are native alerts with the notes in a scrolling text view.
//! A build routes to a feed only through `TransitionGate`: shipped builds wait
//! for the release lane's verified transition receipt and stay Gated until
//! then. Development builds read a local fixture, `<profile>/update-feed.json`
//! (as `cage-report.json` turns on the cage report), and never touch the network.
//! Nothing here downloads or installs: staging belongs to the release
//! updater, so Install Update reports that the update couldn't be downloaded.
use crate::alert::{Alert, Button};
use gpui::*;
use objc2::{
    MainThreadMarker, MainThreadOnly, Message, msg_send, rc::Retained, runtime::AnyObject,
};
use objc2_app_kit::{NSAlert, NSColor, NSFont, NSScrollView, NSTextView, NSWindow};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use pm_services::{
    Result as ServiceResult, ServiceErrorCode,
    storage::ServiceStorage,
    surface_backing::maintenance::{
        Release, ReleaseClaim, ReleaseHost, UpdateHost, UpdateStage, UpdateState, Updates, WhatsNew,
    },
    updater::{FeedMetadata, FeedRoute, TransitionGate, UpdateFeed},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const SKIPPED: &str = "app.updates.skipped";
const SEEN: &str = "app.whats-new.seen";
const FIXTURE: &str = "update-feed.json";
pub const RELEASES_PAGE: &str = "https://github.com/iterative-computer/powermove/releases";

/// What a feed source holds: the offered build and the release history.
#[derive(Clone, Default, Deserialize)]
pub struct Feed {
    #[serde(default)]
    pub latest: Option<FeedMetadata>,
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
struct Fetched(Option<FeedMetadata>);
impl UpdateFeed for Fetched {
    fn metadata(&mut self, _: FeedRoute) -> ServiceResult<Option<FeedMetadata>> {
        Ok(self.0.take())
    }
}

/// The development fixture, when this is a development build that has one.
fn fixture(profile: &Path) -> Option<PathBuf> {
    let path = profile.join(FIXTURE);
    (cfg!(debug_assertions) && path.is_file()).then_some(path)
}
fn gate(fixture: bool) -> TransitionGate {
    TransitionGate {
        packaged_macos: fixture,
        macos_major: macos_major(),
        migration_complete: true,
        // Shipped builds: set by the release updater's verified receipt.
        transition_release_verified: fixture,
    }
}
fn macos_major() -> u32 {
    objc2_foundation::NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion as u32
}
/// The running app's version: the bundle's, else the crate's.
pub fn current_version() -> String {
    objc2_foundation::NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString"))
        .and_then(|v| v.downcast::<NSString>().ok())
        .map(|v| v.to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into())
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
    updates: Updates,
    whats_new: WhatsNew,
    loaded: bool,
}
impl Global for State {}

fn say(window: &NSWindow, message: &str, informative: &str, critical: bool) {
    let alert = Alert::new(message, informative, vec![Button::new("OK", ()).default()]);
    let alert = if critical { alert.critical() } else { alert };
    let _ = alert.sheet(window);
}

/// Powermove ▸ Check for Updates…
pub fn check_now(profile: PathBuf, window: Retained<NSWindow>, cx: &mut App) {
    let state = cx.default_global::<State>();
    if matches!(state.updates.state, UpdateState::Ready { .. }) {
        return say(
            &window,
            "An update is ready.",
            "Quit Powermove normally to install it. Your session will be saved before quitting.",
            false,
        );
    }
    let request = match state.updates.begin_check(true) {
        Ok(request) => request,
        Err(_) => {
            return say(
                &window,
                "An update is already in progress.",
                "Powermove will let you know when it’s ready.",
                false,
            );
        }
    };
    let load = !std::mem::replace(&mut state.loaded, true);
    let source = fixture(&profile);
    let work = cx.background_executor().spawn(async move {
        let skipped = load
            .then(|| storage(&profile).ok()?.setting(SKIPPED).ok().flatten())
            .flatten()
            .and_then(|v| v.as_str().map(str::to_owned));
        let feed = source.as_deref().map(read_feed);
        (skipped, feed)
    });
    cx.spawn(async move |cx| {
        let (skipped, feed) = work.await;
        cx.update(|cx| {
            let state = cx.global_mut::<State>();
            if skipped.is_some() {
                state.updates.skipped_version = skipped;
            }
            let fixture = feed.is_some();
            let current = current_version();
            let result = match feed {
                Some(Err(code)) => Err(code),
                Some(Ok(feed)) => state
                    .updates
                    .check(&request, gate(fixture), &current, &mut Fetched(feed.latest))
                    .map_err(|e| e.code),
                None => state
                    .updates
                    .check(&request, gate(false), &current, &mut Fetched(None))
                    .map_err(|e| e.code),
            };
            let state = state.updates.state.clone();
            show(result.err(), state, &current, &window, cx);
        });
    })
    .detach();
}

fn show(
    failure: Option<ServiceErrorCode>,
    state: UpdateState,
    current: &str,
    window: &NSWindow,
    cx: &mut App,
) {
    match (failure, state) {
        (Some(ServiceErrorCode::Offline), _) => say(
            window,
            "Couldn’t check for updates.",
            "Check your internet connection, then try again.",
            false,
        ),
        (Some(_), _) => say(
            window,
            "Couldn’t check for updates.",
            "Try again later. You can keep using Powermove.",
            false,
        ),
        (None, UpdateState::Gated) => say(
            window,
            "Couldn’t check for updates.",
            "This build of Powermove doesn’t update itself.",
            false,
        ),
        (None, UpdateState::Available(metadata)) => offer(metadata, current, window, cx),
        _ => say(
            window,
            "Powermove is up to date.",
            &format!("You’re running {current}."),
            false,
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Choice {
    Install,
    Later,
    Skip,
}
/// The update sheet: notes scroll in the reading style; Install Update is
/// the default and Skip This Version sits at the leading edge.
fn offer(metadata: FeedMetadata, current: &str, window: &NSWindow, cx: &mut App) {
    let t = pm_shell::tokens();
    let notes = metadata
        .notes
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Fixes and improvements.".into());
    let alert = Alert::new(
        "A New Version of Powermove Is Available",
        format!("{} is now available. You have {current}.", metadata.name),
        vec![
            Button::new("Install Update", Choice::Install).default(),
            Button::new("Remind Me Later", Choice::Later).cancel(),
            Button::new("Skip This Version", Choice::Skip),
        ],
    );
    let accessory = notes_view(&[(None, None, notes)], &t, t.metrics.sheet_width);
    let rx = alert.sheet_with(window, |native| set_accessory(native, &accessory));
    let window = window.retain();
    cx.spawn(async move |cx| {
        let choice = rx.recv().await.ok().flatten();
        cx.update(|cx| {
            let updates = &mut cx.global_mut::<State>().updates;
            match choice {
                Some(Choice::Install) => {
                    let staged = updates.stage(&mut NoStaging);
                    if staged.is_err() {
                        say(
                            &window,
                            "Couldn’t download the update.",
                            "Try again later. You can keep using Powermove.",
                            false,
                        );
                    }
                }
                Some(Choice::Skip) => {
                    if let Ok(version) = updates.skip_available() {
                        persist(SKIPPED, serde_json::json!(version), cx);
                    }
                }
                _ => {
                    let _ = updates.remind_later();
                }
            }
        });
    })
    .detach();
}
/// No updater can stage a download in this build (release lane): honest
/// failure, nothing fetched.
struct NoStaging;
impl UpdateHost for NoStaging {
    fn stage_verified(
        &mut self,
        _: &FeedMetadata,
        _: &mut dyn FnMut(UpdateStage) -> ServiceResult<()>,
    ) -> ServiceResult<String> {
        Err(pm_services::ServiceError::message(
            ServiceErrorCode::Unavailable,
            "no update staging in this build",
        ))
    }
    fn request_normal_quit(&mut self) -> ServiceResult<bool> {
        Ok(false)
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
            r#"{"latest":{"version":"9.0.0","name":"Powermove 9","size_bytes":1,"minimum_macos":14,"notes":"- Faster"},
                "releases":[{"tag_name":"v9.0.0","name":"Powermove 9","body":"- Faster"},
                            {"tag_name":"v8.9.0-beta.1","prerelease":true},{"name":"no tag"}]}"#,
        )
        .unwrap();
        let releases = feed.releases();
        assert_eq!(releases.len(), 2);
        assert!(releases[1].prerelease);
        assert_eq!(feed.latest.unwrap().name, "Powermove 9");
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
