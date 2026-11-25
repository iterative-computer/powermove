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
/// the first window). Reads the sharing choice; returns what was decided.
pub fn init(config: Config) -> Mode {
    let state = prefs::load(&config.profile);
    let mode = decide(
        cfg!(feature = "telemetry"),
        &|name| std::env::var(name).ok(),
        &config,
    );
    SHARING.store(state.sharing, std::sync::atomic::Ordering::Relaxed);
    let _ = PROFILE.set(config.profile.clone());
    #[cfg(feature = "telemetry")]
    if !matches!(mode, Mode::Off(_)) {
        live::start(config, state, mode);
    }
    #[cfg(not(feature = "telemetry"))]
    let _ = (config.metal_bytes, config.displays);
    mode
}

static SHARING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static PROFILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The person's choice in Settings › Privacy.
pub fn sharing() -> bool {
    SHARING.load(std::sync::atomic::Ordering::Relaxed)
}

/// Turns sharing on or off. Off stops sending at once and deletes what is
/// waiting to be sent; on resumes. The choice is saved off the main thread.
pub fn set_sharing(on: bool) {
    if SHARING.swap(on, std::sync::atomic::Ordering::Relaxed) == on {
        return;
    }
    #[cfg(feature = "telemetry")]
    if live::set_sharing(on) {
        return;
    }
    // No telemetry thread this session: only the choice is saved.
    if let Some(profile) = PROFILE.get().cloned() {
        std::thread::spawn(move || {
            let mut state = prefs::load(&profile);
            state.sharing = on;
            let _ = prefs::store(&profile, &state);
        });
    }
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
pub fn launched(first_window_ms: u64, interactive_ms: u64, first_run: bool) {
    #[cfg(feature = "telemetry")]
    live::launched(first_window_ms, interactive_ms, first_run);
    #[cfg(not(feature = "telemetry"))]
    let _ = (first_window_ms, interactive_ms, first_run);
}

/// Milliseconds since this process started (for [`launched`]).
pub fn since_process_start_ms() -> u64 {
    #[cfg(feature = "telemetry")]
    return live::since_process_start_ms();
    #[cfg(not(feature = "telemetry"))]
    0
}

/// One drawn frame and how long it took (an atomic add).
#[inline]
pub fn frame(context: PerfContext, ms: f32) {
    #[cfg(feature = "telemetry")]
    live::perf::frame(context, ms);
    #[cfg(not(feature = "telemetry"))]
    let _ = (context, ms);
}

/// A stretch of playback that ended: frames presented over its length, and
/// the rate the project asked for.
#[inline]
pub fn playback(frames_presented: u64, seconds: f64, target_fps: f32) {
    #[cfg(feature = "telemetry")]
    live::perf::playback(frames_presented, seconds, target_fps);
    #[cfg(not(feature = "telemetry"))]
    let _ = (frames_presented, seconds, target_fps);
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
