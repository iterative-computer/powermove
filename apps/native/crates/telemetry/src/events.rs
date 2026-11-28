//! Everything Powermove's telemetry sends, in one file.
//!
//! A reviewer reads this module to see every event and property that leaves
//! the machine. Nothing else is sent: the telemetry thread serializes only
//! these types, the super properties below, and the exception, crash and
//! hang payloads built in `live::exception` from addresses and binary names.
//!
//! Never sent: project content, layer, file, media or composition names, file
//! paths, prompts or agent text, extension source, local extension names, or
//! e-mail. Strings are `&'static str` from the app's own tables (command,
//! panel, tool and effect ids of built-ins), Store extension ids (public
//! catalogue ids), or values cleaned by [`clean_id`]. The distinct id is a
//! random install id kept in the profile; a Powermove Account sign-in sends
//! one `$identify` with the account's opaque id (never its e-mail).
use serde::Serialize;

/// One event, by its PostHog name (`event`) and properties.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "properties", rename_all = "snake_case")]
pub enum Event {
    AppLaunched(AppLaunched),
    AppQuit(AppQuit),
    OnboardingStep(OnboardingStep),
    OnboardingFinished(OnboardingFinished),
    ProjectOpened(ProjectOpened),
    ProjectCreated(ProjectCreated),
    ProjectSaved(ProjectSaved),
    CommandInvoked(CommandInvoked),
    PanelOpened(PanelOpened),
    ToolSelected(ToolSelected),
    EffectAdded(EffectAdded),
    LayerAdded(LayerAdded),
    ExportStarted(Export),
    ExportFinished(Export),
    AgentRunFinished(AgentRunFinished),
    StoreExtensionInstalled(StoreExtension),
    StoreExtensionRemoved(StoreExtension),
    SettingsChanged(SettingsChanged),
    /// Built by the telemetry thread every 5 minutes and on quit.
    PerfSummary(Box<PerfSummary>),
    /// Built by the hang watchdog.
    Hang(Hang),
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::AppLaunched(_) => "app_launched",
            Event::AppQuit(_) => "app_quit",
            Event::OnboardingStep(_) => "onboarding_step",
            Event::OnboardingFinished(_) => "onboarding_finished",
            Event::ProjectOpened(_) => "project_opened",
            Event::ProjectCreated(_) => "project_created",
            Event::ProjectSaved(_) => "project_saved",
            Event::CommandInvoked(_) => "command_invoked",
            Event::PanelOpened(_) => "panel_opened",
            Event::ToolSelected(_) => "tool_selected",
            Event::EffectAdded(_) => "effect_added",
            Event::LayerAdded(_) => "layer_added",
            Event::ExportStarted(_) => "export_started",
            Event::ExportFinished(_) => "export_finished",
            Event::AgentRunFinished(_) => "agent_run_finished",
            Event::StoreExtensionInstalled(_) => "store_extension_installed",
            Event::StoreExtensionRemoved(_) => "store_extension_removed",
            Event::SettingsChanged(_) => "settings_changed",
            Event::PerfSummary(_) => "perf_summary",
            Event::Hang(_) => "hang",
        }
    }
}

/// Sent with every event (and every exception), set once per launch.
#[derive(Debug, Clone, Serialize)]
pub struct SuperProperties {
    pub app_version: String,
    /// The commit the build was made from (short hash), or "unknown".
    pub build: String,
    /// "release", "beta" or "dev".
    pub channel: &'static str,
    /// e.g. "26.1".
    pub macos_version: String,
    /// e.g. "Apple M3 Max".
    pub chip: String,
    pub ram_bucket: &'static str,
    pub display_count: u32,
    pub display_max_scale: f32,
    /// Language code only, e.g. "en".
    pub locale: String,
    /// Region code only, e.g. "US".
    pub region: String,
    /// True for every event a development, lane or `PM_TELEMETRY_DEV` build
    /// sends; the project's test-account filter hides them.
    pub is_internal: bool,
    /// A random id per launch (PostHog sessions).
    #[serde(rename = "$session_id")]
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchKind {
    /// The first launch since the Mac started (no app binary pages cached).
    Cold,
    Warm,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppLaunched {
    pub kind: LaunchKind,
    pub launch_to_first_window_ms: u64,
    pub launch_to_interactive_ms: u64,
    pub first_run: bool,
    /// Filled in by the telemetry thread: the previous session ended without
    /// a clean quit (a crash, a force quit or a power loss).
    pub crash_last_session: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppQuit {
    pub session_minutes: u64,
    pub clean: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepAction {
    Shown,
    Completed,
    Skipped,
    Back,
}

/// Onboarding step names are the onboarding's own fixed ids (PM-703).
#[derive(Debug, Clone, Serialize)]
pub struct OnboardingStep {
    pub step: &'static str,
    pub action: StepAction,
    pub ms_on_step: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingOutcome {
    Completed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V1Import {
    /// No v1 library on this Mac.
    NotOffered,
    Offered,
    Accepted,
    Declined,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardingFinished {
    pub outcome: OnboardingOutcome,
    pub total_ms: u64,
    pub v1_import: V1Import,
    pub imported_projects: u32,
    pub imported_extensions: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenSource {
    New,
    Recent,
    File,
    ImportV1,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectOpened {
    pub source: OpenSource,
    pub open_ms: u64,
    pub layer_count: &'static str,
    pub comp_count: &'static str,
    pub duration: &'static str,
    pub fps: f32,
    pub resolution: &'static str,
    pub has_3d: bool,
    pub extension_effect_count: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectCreated {}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSaved {
    pub ms: u64,
    pub size: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Menu,
    Shortcut,
    Palette,
    Toolbar,
    ContextMenu,
}

/// Repeats of one command within a second are folded into one event with a
/// `count` by the telemetry thread.
#[derive(Debug, Clone, Serialize)]
pub struct CommandInvoked {
    /// The fixed command registry id (built-in commands only).
    pub command_id: &'static str,
    pub via: Via,
    pub count: u32,
}

/// A panel, effect or extension: a built-in by its id in the app's own
/// tables, a Store extension by its public catalogue id, anything else (a
/// person's own extension) as "local", never by its name.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Origin {
    BuiltIn(String),
    Store(String),
    Local(LocalMarker),
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalMarker {
    Local,
}

impl Origin {
    pub const LOCAL: Origin = Origin::Local(LocalMarker::Local);

    /// A built-in, by an id from the app's own tables (the panel list, the
    /// effect catalogue).
    pub fn builtin(id: &str) -> Origin {
        match clean_id(&id.to_ascii_lowercase()) {
            Some(_) => Origin::BuiltIn(id.to_owned()),
            None => Origin::LOCAL,
        }
    }

    /// An extension: by its id only when it came from the Store (see
    /// `pm_telemetry::set_store_extensions`), else "local".
    pub fn extension(id: &str) -> Origin {
        if crate::is_store_extension(id)
            && let Some(id) = clean_id(id)
        {
            return Origin::Store(id);
        }
        Origin::LOCAL
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PanelOpened {
    pub panel_id: Origin,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSelected {
    pub tool_id: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectAdded {
    pub effect_id: Origin,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerAdded {
    /// The layer's kind (e.g. "text", "shape", "video").
    pub kind: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    Cancelled,
}

/// `export_started` carries the job; `export_finished` adds the timing and
/// the result.
#[derive(Debug, Clone, Serialize)]
pub struct Export {
    /// The export format's fixed name ("prores", "mp4", "webm", "png", …).
    pub format: String,
    /// The codec or ProRes profile's fixed name.
    pub codec: String,
    pub resolution: &'static str,
    pub duration: &'static str,
    pub fps: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realtime_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Outcome>,
    /// A fixed error class, never the error's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    /// Powermove's own harness (Aero); Aerolite and Aero are plan names.
    Powermove,
    Codex,
    ClaudeCode,
}

/// The prompt and every message stay on the Mac.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRunFinished {
    pub harness: Harness,
    /// A public model id from [`model_id`]'s list, else "custom".
    pub model: &'static str,
    /// From [`effort_id`]'s list, else "unknown".
    pub effort: &'static str,
    pub result: Outcome,
    /// Model turns, when the harness reports them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
    pub duration_ms: u64,
    /// Tokens, when the provider reports them (input includes cached input).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreExtension {
    /// The extension's public Store catalogue id.
    pub extension_id: String,
}

/// Settings whose change is sent: this list only, with the new value.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "key", content = "value")]
pub enum SettingsChanged {
    /// "light", "dark" or "system".
    Theme(&'static str),
    /// "tabs" or "windows".
    ProjectWindows(&'static str),
    /// Sent when sharing is turned back on (never when it is turned off:
    /// nothing is sent after that).
    Telemetry(bool),
}

/// Frame pacing and memory over the last five minutes (or since the last
/// summary), per interaction context.
#[derive(Debug, Clone, Serialize, Default)]
pub struct PerfSummary {
    pub window_s: u64,
    /// Per context, flat for insights: `playing_frames`, `playing_p50_ms`,
    /// `playing_p95_ms`, `playing_p99_ms`, `playing_hitches_8ms`,
    /// `playing_hitches_16ms`, and so on for each context with frames.
    #[serde(flatten)]
    pub frames: Frames,
    /// Playback over the window: played frames that reached the display, a
    /// second, and the rate the playing composition asks for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_fps_achieved: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_fps_target: Option<f32>,
    pub peak_footprint_mb: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_metal_mb: Option<u64>,
    /// "nominal", "fair", "serious" or "critical".
    pub thermal_state_max: &'static str,
    pub on_battery: bool,
    /// Events the capture queue dropped because it was full.
    pub telemetry_dropped: u64,
}

/// One entry per [`PerfContext`] (in `PerfContext::ALL` order).
#[derive(Debug, Clone, Default)]
pub struct Frames(pub [Option<ContextFrames>; 5]);

impl Serialize for Frames {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(None)?;
        for (context, frames) in PerfContext::ALL.iter().zip(&self.0) {
            let Some(f) = frames else { continue };
            let name = context.name();
            map.serialize_entry(&format!("{name}_frames"), &f.frames)?;
            map.serialize_entry(&format!("{name}_p50_ms"), &f.p50_ms)?;
            map.serialize_entry(&format!("{name}_p95_ms"), &f.p95_ms)?;
            map.serialize_entry(&format!("{name}_p99_ms"), &f.p99_ms)?;
            map.serialize_entry(&format!("{name}_hitches_8ms"), &f.hitches_8ms)?;
            map.serialize_entry(&format!("{name}_hitches_16ms"), &f.hitches_16ms)?;
        }
        map.end()
    }
}

/// What the person was doing while a frame was made.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PerfContext {
    Paused,
    Playing,
    Scrubbing,
    Dragging,
    Scrolling,
}

impl PerfContext {
    pub fn name(self) -> &'static str {
        match self {
            PerfContext::Paused => "paused",
            PerfContext::Playing => "playing",
            PerfContext::Scrubbing => "scrubbing",
            PerfContext::Dragging => "dragging",
            PerfContext::Scrolling => "scrolling",
        }
    }
    pub const ALL: [PerfContext; 5] = [
        PerfContext::Paused,
        PerfContext::Playing,
        PerfContext::Scrubbing,
        PerfContext::Dragging,
        PerfContext::Scrolling,
    ];
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextFrames {
    pub frames: u64,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub hitches_8ms: u64,
    pub hitches_16ms: u64,
}

/// The main thread did not answer for more than 2 s, and what the person was
/// doing when it stopped: only values the other events already send.
#[derive(Debug, Clone, Serialize)]
pub struct Hang {
    /// "2-5s", "5-10s", "10-30s", "30s+", or "unrecovered" (the app was
    /// ended while it hung; sent at the next launch).
    pub duration: &'static str,
    /// The last command a person ran, by its `command_invoked` id ("none"
    /// before the first).
    pub last_command: &'static str,
    /// Playback was asked for or running.
    pub playing: bool,
    pub surface: Surface,
    /// The project of the last command, counted as `project_opened` counts
    /// it (absent without a project).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_count: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_3d: Option<bool>,
}

/// What the front window showed: a project, Home, the Store page, or the
/// Settings window.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    Other,
    Editor,
    Home,
    Store,
    Settings,
}

impl Surface {
    pub const ALL: [Surface; 5] = [
        Surface::Other,
        Surface::Editor,
        Surface::Home,
        Surface::Store,
        Surface::Settings,
    ];
}

/// Public model ids the app's harnesses offer. Anything else (a model a
/// person configured for their own provider) is "custom", never its name.
const MODELS: &[&str] = &[
    "gpt-6.2-sol",
    "gpt-6.1-sol",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-6-astra",
    "gpt-6.2-astra",
    "gpt-6.1-terra",
    "gpt-6",
    "gpt-6-mini",
    "gpt-5.6",
    "gpt-5.6-terra",
    "gpt-5.5",
    "gpt-5.5-pro",
    "gpt-5.5-chat",
    "gpt-5.4",
    "gpt-5.4-pro",
    "gpt-5.4-chat",
    "gpt-5.4-chat-latest",
    "gpt-oss",
    "o1",
    "o3",
    "o3-mini",
    "o4",
    "claude-fable-5-2",
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-5-5",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-haiku-5-5",
    "claude-haiku-4-5",
    "claude-haiku-4-5-20251001",
    "opus",
    "sonnet",
    "haiku",
    "fable",
];

/// A run's model, by the list above.
pub fn model_id(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw else { return "unknown" };
    MODELS
        .iter()
        .find(|m| raw.eq_ignore_ascii_case(m))
        .copied()
        .unwrap_or("custom")
}

/// A run's reasoning effort, by the efforts the harnesses take.
pub fn effort_id(raw: Option<&str>) -> &'static str {
    const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];
    raw.and_then(|r| EFFORTS.iter().find(|e| r.eq_ignore_ascii_case(e)).copied())
        .unwrap_or("unknown")
}

// Buckets: counts and sizes go out as ranges, never exact.

pub fn count_bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2..=5 => "2-5",
        6..=20 => "6-20",
        21..=100 => "21-100",
        101..=500 => "101-500",
        _ => "500+",
    }
}

pub fn duration_bucket(seconds: f64) -> &'static str {
    match seconds {
        s if s < 10. => "<10s",
        s if s < 30. => "10-30s",
        s if s < 60. => "30-60s",
        s if s < 300. => "1-5m",
        s if s < 1800. => "5-30m",
        _ => "30m+",
    }
}

/// By the longer side of the frame.
pub fn resolution_bucket(width: u32, height: u32) -> &'static str {
    match width.max(height) {
        0..=1280 => "720p",
        1281..=1920 => "1080p",
        1921..=2560 => "1440p",
        2561..=3840 => "4k",
        _ => "8k",
    }
}

pub fn size_bucket(bytes: u64) -> &'static str {
    const MB: u64 = 1 << 20;
    match bytes {
        b if b < MB => "<1MB",
        b if b < 10 * MB => "1-10MB",
        b if b < 100 * MB => "10-100MB",
        b if b < 1024 * MB => "100MB-1GB",
        _ => "1GB+",
    }
}

pub fn ram_bucket(bytes: u64) -> &'static str {
    match bytes >> 30 {
        0..=8 => "8GB",
        9..=16 => "16GB",
        17..=24 => "24GB",
        25..=36 => "36GB",
        37..=64 => "64GB",
        65..=128 => "128GB",
        _ => "128GB+",
    }
}

pub fn hang_bucket(seconds: f64) -> &'static str {
    match seconds {
        s if s < 5. => "2-5s",
        s if s < 10. => "5-10s",
        s if s < 30. => "10-30s",
        _ => "30s+",
    }
}

/// An id from a catalogue or a provider (a Store extension id, a model id):
/// lowercase letters, digits and `.-_:/@`, at most 80 characters. Anything
/// else (spaces, a path, a person's text) is refused.
pub fn clean_id(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id.len() <= 80
        && !id.starts_with('/')
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-:/@".contains(&b));
    ok.then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_by_name_with_only_their_properties() {
        let e = Event::CommandInvoked(CommandInvoked {
            command_id: "timeline.split",
            via: Via::Shortcut,
            count: 3,
        });
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["event"], e.name());
        assert_eq!(
            v["properties"],
            serde_json::json!({"command_id":"timeline.split","via":"shortcut","count":3})
        );
        let s =
            serde_json::to_value(Event::SettingsChanged(SettingsChanged::Theme("dark"))).unwrap();
        assert_eq!(
            s["properties"],
            serde_json::json!({"key":"theme","value":"dark"})
        );
        let p = serde_json::to_value(PanelOpened {
            panel_id: Origin::LOCAL,
        })
        .unwrap();
        assert_eq!(p, serde_json::json!({"panel_id":"local"}));
        let p = serde_json::to_value(PanelOpened {
            panel_id: Origin::builtin("timeline"),
        })
        .unwrap();
        assert_eq!(p, serde_json::json!({"panel_id":"timeline"}));
        let h = serde_json::to_value(Event::Hang(Hang {
            duration: "2-5s",
            last_command: "save",
            playing: false,
            surface: Surface::Store,
            layer_count: Some("6-20"),
            has_3d: Some(false),
        }))
        .unwrap();
        assert_eq!(
            h["properties"],
            serde_json::json!({"duration":"2-5s","last_command":"save","playing":false,
                "surface":"store","layer_count":"6-20","has_3d":false})
        );
    }

    #[test]
    fn ids_that_could_carry_a_persons_text_are_refused() {
        assert_eq!(clean_id("acme.glow-pro"), Some("acme.glow-pro".into()));
        assert_eq!(clean_id("gpt-6.2-sol"), Some("gpt-6.2-sol".into()));
        for bad in [
            "",
            "My Glow",
            "/Users/me/ext",
            "../x",
            "Glow",
            "a b",
            &"x".repeat(81),
        ] {
            assert_eq!(clean_id(bad), None, "{bad}");
        }
        assert_eq!(Origin::extension("acme.not-from-the-store"), Origin::LOCAL);
        assert_eq!(Origin::builtin("My Panel"), Origin::LOCAL);
    }

    /// Sol P1 (w99 review): a configured model string never leaves.
    #[test]
    fn model_ids_outside_the_allowlist_read_custom() {
        assert_eq!(model_id(Some("gpt-6.2-sol")), "gpt-6.2-sol");
        assert_eq!(model_id(Some("Claude-Opus-5-5")), "claude-opus-5-5");
        for private in [
            "name@example.com",
            "projects/private-model",
            "jane-finetune",
            "my-llama",
        ] {
            assert_eq!(model_id(Some(private)), "custom", "{private}");
        }
        assert_eq!(model_id(None), "unknown");
        assert_eq!(effort_id(Some("high")), "high");
        assert_eq!(effort_id(Some("whatever I typed")), "unknown");
    }

    #[test]
    fn buckets_cover_their_ranges() {
        assert_eq!(count_bucket(0), "0");
        assert_eq!(count_bucket(7), "6-20");
        assert_eq!(count_bucket(10_000), "500+");
        assert_eq!(resolution_bucket(1920, 1080), "1080p");
        assert_eq!(resolution_bucket(1080, 1920), "1080p");
        assert_eq!(resolution_bucket(3840, 2160), "4k");
        assert_eq!(size_bucket(5 << 20), "1-10MB");
        assert_eq!(ram_bucket(16 << 30), "16GB");
        assert_eq!(ram_bucket(36 << 30), "36GB");
        assert_eq!(hang_bucket(3.), "2-5s");
        assert_eq!(duration_bucket(45.), "30-60s");
    }
}
