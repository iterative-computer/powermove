//! Product telemetry: anonymous usage, crashes, hangs and frame pacing, sent
//! to PostHog by release builds only.
//!
//! Every event and property is in [`events`]. The main thread only ever does
//! a `try_send` into a bounded channel (or an atomic add for frame times);
//! one background thread batches, sends, retries and keeps an outbox in the
//! profile for offline sends. Without the `telemetry` cargo feature every
//! call here is an empty inline function and no network code is compiled.
//!
//! When it runs ([`decide`]): built with the feature, the person shares
//! (Settings › Privacy), `PM_NO_TELEMETRY` is unset, the app has windows (not
//! MCP or an engine job), was not started with `--profile` or as a lane or
//! development bundle, and is the shipping bundle. `PM_TELEMETRY_DEV=1`
//! turns it on in any feature build, every event marked `is_internal`.
pub mod events;
pub mod main_thread;
#[cfg(feature = "telemetry")]
mod prefs;

#[cfg(feature = "telemetry")]
mod live;

pub use events::*;
use std::path::PathBuf;
use std::sync::Arc;

/// The project's public client key (a write-only ingestion token, like the
/// key in a web page; not a secret: AGENTS.md "public client/site keys").
pub const PROJECT_TOKEN: &str = "phc_mjXkSLMUHZjZknpg6SDFfGdtme6v5uSvjQG4SeF79Bbt"; // gitleaks:allow (public PostHog client key)
pub const HOST: &str = "https://us.i.posthog.com";
/// Settings › Privacy › What We Collect.
pub const DOCS_URL: &str =
    "https://github.com/iterative-computer/powermove/blob/main/docs/telemetry.md";
/// The onboarding's one quiet line about it (the welcome window, PM-703).
pub const NOTICE: &str = "Powermove shares anonymous usage and crash reports to get better. You can turn this off in Settings › Privacy.";

/// The shipping bundles and their channel.
pub const SHIPPING: [(&str, &str); 2] = [
    ("com.zellzoi.powermove", "release"),
    ("com.zellzoi.powermove.native-beta", "beta"),
];

/// How this process was started, for [`init`].
pub struct Config {
    pub profile: PathBuf,
    pub app_version: String,
    /// The commit the build was made from.
    pub build: String,
    pub bundle_id: Option<String>,
    /// MCP, an engine job or another process without windows.
    pub headless: bool,
    /// Started with `--profile`, a lane flag, or from a bundle marked as a
    /// lane or development bundle.
    pub development: bool,
    /// `cfg!(debug_assertions)` of the app.
    pub debug_build: bool,
    /// Displays and the largest backing scale, read on the main thread.
    pub displays: (u32, f32),
    /// Bytes the Metal device holds, read on the telemetry thread.
    pub metal_bytes: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is sent this session, for this reason.
    Off(&'static str),
    /// A shipping build on a person's Mac.
    Production { channel: &'static str },
    /// `PM_TELEMETRY_DEV=1`: sent, every event marked `is_internal`.
    Internal,
}

/// Whether this session sends, from the build, the environment and how it
/// was started (the sharing choice is separate and can change while it
/// runs).
pub fn decide(feature: bool, env: &dyn Fn(&str) -> Option<String>, config: &Config) -> Mode {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
    if !feature {
        return Mode::Off("built without telemetry");
    }
    if set("PM_NO_TELEMETRY") {
        return Mode::Off("PM_NO_TELEMETRY");
    }
    if config.headless {
        return Mode::Off("headless");
    }
    if env("PM_TELEMETRY_DEV").as_deref() == Some("1") {
        return Mode::Internal;
    }
    if config.debug_build || config.development {
        return Mode::Off("development build");
    }
    // Harnesses that launch the app (pm-gui, acceptance, soak).
    if set("PM_NATIVE_NO_ORIGINAL") || set("PM_NATIVE_REQUIRE_PROFILE") || set("PM_NATIVE_PROFILE")
    {
        return Mode::Off("launched by a harness");
    }
    match SHIPPING
        .iter()
        .find(|(id, _)| config.bundle_id.as_deref() == Some(*id))
    {
        Some((_, channel)) => Mode::Production { channel },
        None => Mode::Off("not the shipping bundle"),
    }
}

/// Starts telemetry for this session (call once, on the main thread, before
/// the first window); returns what was decided. Nothing is sent or written
/// until Settings' sharing value arrives ([`set_sharing`], from the kernel
/// when the library attaches): until then events wait in memory only.
pub fn init(config: Config) -> Mode {
    let mode = decide(
        cfg!(feature = "telemetry"),
        &|name| std::env::var(name).ok(),
        &config,
    );
    #[cfg(feature = "telemetry")]
    if !matches!(mode, Mode::Off(_)) {
        live::start(config, mode);
    }
    #[cfg(not(feature = "telemetry"))]
    let _ = (config.metal_bytes, config.displays);
    mode
}

/// Settings' value: unknown until the library attaches, then on or off.
static SHARING: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The person's choice in Settings › Privacy (false until Settings is read).
pub fn sharing() -> bool {
    SHARING.load(std::sync::atomic::Ordering::Relaxed) == 1
}

/// Settings › Privacy's value: when the library attaches, and on every
/// change. The kernel reports off at once and on only once Settings has
/// saved it. Off stops sending at once, drops what is waiting (in memory and
/// on disk) and anything captured before; on resumes.
pub fn set_sharing(on: bool) {
    let value = if on { 1 } else { 2 };
    let changed = SHARING.swap(value, std::sync::atomic::Ordering::Relaxed) != value;
    #[cfg(feature = "telemetry")]
    if changed {
        live::set_sharing(on);
    }
    #[cfg(not(feature = "telemetry"))]
    let _ = changed;
}

/// Whether events are being taken (running, and Settings hasn't said off):
/// call sites that would do work to build an event check this first.
#[inline]
pub fn enabled() -> bool {
    #[cfg(feature = "telemetry")]
    return live::taking();
    #[cfg(not(feature = "telemetry"))]
    false
}

/// Telemetry runs this session (whatever Settings says now: sharing can be
/// turned on later).
pub fn running() -> bool {
    #[cfg(feature = "telemetry")]
    return live::running();
    #[cfg(not(feature = "telemetry"))]
    false
}

/// Extensions this app installed from the Store catalogue (their public
/// ids, kept in `telemetry.json`): only these are ever named in events;
/// every other extension, a folder with a Store marker included, is "local".
static STORE_EXTENSIONS: std::sync::RwLock<std::collections::BTreeSet<String>> =
    std::sync::RwLock::new(std::collections::BTreeSet::new());

pub fn is_store_extension(id: &str) -> bool {
    STORE_EXTENSIONS.read().is_ok_and(|set| set.contains(id))
}

#[cfg(feature = "telemetry")]
fn remember_store_extensions(ids: &[String]) {
    if let Ok(mut set) = STORE_EXTENSIONS.write() {
        *set = ids.iter().cloned().collect();
    }
}

/// The Store window installed `id`, the public id it picked from the
/// catalogue: remembered as a Store install, and `store_extension_installed`.
/// `fresh`: a new install (`store_extension_installed`); an update only
/// records the provenance.
pub fn store_installed(id: &str, fresh: bool) {
    #[cfg(feature = "telemetry")]
    live::store_installed(id, fresh);
    #[cfg(not(feature = "telemetry"))]
    let _ = (id, fresh);
}

/// The Store window removed `id`: `store_extension_removed` only when this
/// app had installed it from the Store.
pub fn store_removed(id: &str) {
    #[cfg(feature = "telemetry")]
    live::store_removed(id);
    #[cfg(not(feature = "telemetry"))]
    let _ = id;
}

/// Queues one event (never blocks; dropped and counted when the queue is
/// full).
#[inline]
pub fn capture(event: Event) {
    #[cfg(feature = "telemetry")]
    live::capture(event);
    #[cfg(not(feature = "telemetry"))]
    let _ = event;
}

/// The app is on screen: times from the process start to the first window
/// and to the first frame a person can use.
#[inline]
pub fn launched(first_window_ms: u64, interactive_ms: u64) {
    #[cfg(feature = "telemetry")]
    live::launched(first_window_ms, interactive_ms);
    #[cfg(not(feature = "telemetry"))]
    let _ = (first_window_ms, interactive_ms);
}

/// Milliseconds since this process started (for [`launched`]).
pub fn since_process_start_ms() -> u64 {
    #[cfg(feature = "telemetry")]
    return live::since_process_start_ms();
    #[cfg(not(feature = "telemetry"))]
    0
}

/// One submitted frame and how long it took to build (an atomic add into
/// the histogram of what the person is doing now).
#[inline]
pub fn frame(ms: f32) {
    #[cfg(feature = "telemetry")]
    live::perf::frame(live::perf::context(), ms);
    #[cfg(not(feature = "telemetry"))]
    let _ = ms;
}

/// A drag (`1`) or a scroll (`2`) is under way (frame context).
#[inline]
pub fn note_input(kind: u8) {
    #[cfg(feature = "telemetry")]
    live::perf::note_input(kind);
    #[cfg(not(feature = "telemetry"))]
    let _ = kind;
}

/// A viewer frame was asked for while playing or scrubbing (frame
/// context), and when a played one reached the display (host seconds), for
/// the achieved playback rate.
#[inline]
pub fn note_viewer_frame(playing: bool, scrubbing: bool, displayed_at: Option<f64>) {
    #[cfg(feature = "telemetry")]
    live::perf::note_viewer_frame(playing, scrubbing, displayed_at);
    #[cfg(not(feature = "telemetry"))]
    let _ = (playing, scrubbing, displayed_at);
}

/// The frame rate playback aims for (the playing composition's rate).
#[inline]
pub fn note_playback_target(fps: f32) {
    #[cfg(feature = "telemetry")]
    live::perf::note_playback_target(fps);
    #[cfg(not(feature = "telemetry"))]
    let _ = fps;
}

/// Whether the app is the active app (hangs count only then: a background
/// app's timers can be held by App Nap).
#[inline]
pub fn set_active(active: bool) {
    #[cfg(feature = "telemetry")]
    live::set_active(active);
    #[cfg(not(feature = "telemetry"))]
    let _ = active;
}

/// A Powermove Account signed in: joins this install to the account's
/// other installs by its opaque id (never the e-mail).
pub fn identify(account_id: &str) {
    #[cfg(feature = "telemetry")]
    live::identify(account_id);
    #[cfg(not(feature = "telemetry"))]
    let _ = account_id;
}

/// The app is quitting: sends `app_quit` and the last frame summary, and
/// waits at most a moment for them to leave (what doesn't is kept for the
/// next launch).
pub fn quit(clean: bool) {
    #[cfg(feature = "telemetry")]
    live::quit(clean);
    #[cfg(not(feature = "telemetry"))]
    let _ = clean;
}

/// Development builds with `PM_TELEMETRY_DEV=1`: what the forced-failure
/// checks need (`PM_TELEMETRY_DEV_FAIL=panic|abort|segv|hang` after launch).
pub fn dev_failure() -> Option<String> {
    (std::env::var("PM_TELEMETRY_DEV").as_deref() == Ok("1"))
        .then(|| std::env::var("PM_TELEMETRY_DEV_FAIL").ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sol P0 (w99 review): sharing is Settings' value, never a default:
    /// nothing is on until Settings says so, and off can't be reversed by
    /// telemetry's own files.
    /// (With the feature, the live end-to-end test checks this on the
    /// running worker.)
    #[cfg(not(feature = "telemetry"))]
    #[test]
    fn sharing_waits_for_settings_and_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("telemetry.json"), br#"{"sharing":true}"#).unwrap();
        let mut c = config("com.powermove.native.dev.w99");
        c.profile = dir.path().into();
        let _ = init(c);
        assert!(!sharing());
        set_sharing(false);
        assert!(!sharing());
        std::fs::remove_file(dir.path().join("telemetry.json")).unwrap();
        assert!(!sharing());
        set_sharing(true);
        assert!(sharing());
    }

    fn config(bundle: &str) -> Config {
        Config {
            profile: "/p".into(),
            app_version: "2.0.0".into(),
            build: "abc".into(),
            bundle_id: Some(bundle.into()),
            headless: false,
            development: false,
            debug_build: false,
            displays: (1, 2.),
            metal_bytes: None,
        }
    }

    #[test]
    fn only_a_shipping_build_on_a_persons_mac_sends() {
        let none = |_: &str| None;
        let ship = config("com.zellzoi.powermove");
        assert_eq!(
            decide(true, &none, &ship),
            Mode::Production { channel: "release" }
        );
        assert_eq!(
            decide(true, &none, &config("com.zellzoi.powermove.native-beta")),
            Mode::Production { channel: "beta" }
        );
        assert!(matches!(decide(false, &none, &ship), Mode::Off(_)));
        assert!(matches!(
            decide(true, &none, &config("com.powermove.native.dev.w99")),
            Mode::Off(_)
        ));
        for change in [
            |c: &mut Config| c.headless = true,
            |c: &mut Config| c.development = true,
            |c: &mut Config| c.debug_build = true,
            |c: &mut Config| c.bundle_id = None,
        ] {
            let mut c = config("com.zellzoi.powermove");
            change(&mut c);
            assert!(matches!(decide(true, &none, &c), Mode::Off(_)));
        }
        for var in [
            "PM_NO_TELEMETRY",
            "PM_NATIVE_NO_ORIGINAL",
            "PM_NATIVE_PROFILE",
            "PM_NATIVE_REQUIRE_PROFILE",
        ] {
            let env = move |name: &str| (name == var).then(|| "1".to_owned());
            assert!(matches!(decide(true, &env, &ship), Mode::Off(_)), "{var}");
        }
    }

    #[test]
    fn the_dev_switch_sends_as_internal_but_never_headless_or_opted_out_by_env() {
        let dev = |name: &str| (name == "PM_TELEMETRY_DEV").then(|| "1".to_owned());
        let mut lane = config("com.powermove.native.dev.w99");
        lane.development = true;
        lane.debug_build = true;
        assert_eq!(decide(true, &dev, &lane), Mode::Internal);
        assert!(matches!(decide(false, &dev, &lane), Mode::Off(_)));
        lane.headless = true;
        assert!(matches!(decide(true, &dev, &lane), Mode::Off(_)));
        let both = |name: &str| {
            matches!(name, "PM_TELEMETRY_DEV" | "PM_NO_TELEMETRY").then(|| "1".to_owned())
        };
        lane.headless = false;
        assert!(matches!(decide(true, &both, &lane), Mode::Off(_)));
    }
}
