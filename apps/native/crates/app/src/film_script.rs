//! The film's requests, as the agent makes them: each change is the MCP
//! tool calls an agent run made for that request, replayed through the
//! agent's own connection on the film's project (`demo_editor`). Recorded
//! edits, not a live agent; nothing leaves the film's temporary profile.
use pm_mcp::transport::ToolConnection;
use pm_shell::stage::Change;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Calls one of the agent's tools; its JSON answer.
fn call(tools: &mut crate::mcp::Connection, name: &str, args: Value) -> anyhow::Result<Value> {
    let result = tools
        .invoke(name, args)
        .map_err(|e| anyhow::anyhow!("{name}: {e:?}"))?;
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    anyhow::ensure!(result["isError"] != true, "{name}: {text}");
    Ok(serde_json::from_str(text).unwrap_or(Value::String(text.to_owned())))
}

/// `call`, again while it answers `waiting` (what was just published is
/// still loading), as an agent tries again after a moment.
fn call_when_loaded(
    tools: &mut crate::mcp::Connection,
    name: &str,
    args: Value,
    waiting: &str,
) -> anyhow::Result<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match call(tools, name, args.clone()) {
            Err(error)
                if error.to_string().contains(waiting) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

pub fn apply(
    tools: &mut crate::mcp::Connection,
    profile: &Path,
    change: Change,
) -> anyhow::Result<()> {
    let _ = profile;
    // The agent's extension changes ask for a staging folder, as a run that
    // writes code does.
    let writes_code = change != Change::ClassicLayout;
    let started = call(
        tools,
        "start_session",
        json!({"prompt": change.prompt(), "includeExtensionStaging": writes_code}),
    )?;
    let staging = started["stagingDirectory"].as_str().map(PathBuf::from);
    let result = match (change, staging) {
        (Change::ClassicLayout, _) => classic_layout(tools),
        (_, None) => Err(anyhow::anyhow!("the session gave no staging folder")),
        (Change::GlassTimeline, Some(stage)) => glass_timeline(tools, &stage),
        (Change::LightLeak, Some(stage)) => light_leak(tools, &stage),
        (Change::BeatMarkers, Some(stage)) => beat_markers(tools, &stage),
    };
    // The run ends either way, as an agent's does: kept when it worked.
    let finished = call(tools, "finish_session", json!({"commit": result.is_ok()}));
    result?;
    finished.map(|_| ())
}

/// The extension the agent writes for `id`: its manifest and source.
fn extension(id: &str) -> (Value, &'static str, &'static str) {
    match id {
        GLASS => (
            json!({"id": GLASS, "name": "Glass Timeline", "version": "1.0.0", "apiVersion": 4,
                "entry": "index.tsx", "author": "agent",
                "description": "Clips drawn as glass: soft corners, a lit edge and a sheen."}),
            "index.tsx",
            GLASS_SOURCE,
        ),
        LEAK => (
            json!({"id": LEAK, "name": "Light Leak", "version": "1.0.0", "apiVersion": 3,
                "entry": "index.ts", "contributes": ["transitions"], "permissions": [], "author": "agent",
                "description": "A warm light leak that blooms across a cut."}),
            "index.ts",
            LEAK_SOURCE,
        ),
        BEATS => (
            json!({"id": BEATS, "name": "Beat Markers", "version": "1.0.0", "apiVersion": 4,
                "entry": "index.tsx", "author": "agent",
                "description": "The music's beats over the composition, the current one lit."}),
            "index.tsx",
            BEATS_SOURCE,
        ),
        _ => unreachable!("the film writes three extensions"),
    }
}
/// Writes the extension `id` in the staging folder, as the agent's file
/// tools do: its folder (named by its id), manifest and source.
fn stage(stage: &Path, id: &str) -> anyhow::Result<()> {
    let (manifest, entry, source) = extension(id);
    let dir = stage.join(id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    std::fs::write(dir.join(entry), source)?;
    Ok(())
}
fn publish(tools: &mut crate::mcp::Connection, id: &str, summary: &str) -> anyhow::Result<Value> {
    call(
        tools,
        "publish_extensions",
        json!({"extensions": [{"id": id, "action": "created", "summary": summary}]}),
    )
}

/// The glass timeline's extension id.
pub const GLASS: &str = "glass-timeline";
/// "Make the timeline feel like glass": a v4 extension that softens the
/// clips' corners (`timeline.chrome`) and lays a sheen on every clip
/// (`timeline.paint`): a lit top edge, a highlight falling off down the
/// upper half, a darker foot.
fn glass_timeline(tools: &mut crate::mcp::Connection, staging: &Path) -> anyhow::Result<()> {
    stage(staging, GLASS)?;
    publish(
        tools,
        GLASS,
        "Glass clips on the timeline: soft corners, a lit top edge and a sheen",
    )?;
    Ok(())
}
const GLASS_SOURCE: &str = r##"// Glass clips: each clip keeps its colour; light sits on it as on a slab
// of tinted glass: a lit top edge, a sheen falling off down its upper part,
// thin side rims and a shaded foot.
type Clip = { id: string; width: number; height: number };

const RADIUS = 5;
const WHITE = "#ffffff";
const BLACK = "#000000";

function bands(w: number, top: number, height: number, color: string, from: number, to: number, steps: number) {
  const out: unknown[] = [];
  const step = height / steps;
  for (let i = 0; i < steps; i++) {
    const t = steps === 1 ? 0 : i / (steps - 1);
    out.push({ rect: [1, top + i * step, w - 2, step], color, opacity: from + (to - from) * t });
  }
  return out;
}

function glass(clip: Clip) {
  const w = clip.width;
  const h = clip.height;
  if (w < 2 * RADIUS + 2 || h < 8) return [];
  return [
    // The sheen over the upper part, brightest at the top.
    ...bands(w, 1, h * 0.38, WHITE, 0.2, 0.03, 4),
    // The foot in shade.
    ...bands(w, h * 0.72, h * 0.28 - 1, BLACK, 0.04, 0.2, 3),
    // The rims: lit along the top, faint down the sides, dark at the foot.
    { rect: [RADIUS, 0.5, w - 2 * RADIUS, 1], color: WHITE, opacity: 0.75 },
    { rect: [0.5, RADIUS, 1, h - 2 * RADIUS], color: WHITE, opacity: 0.3 },
    { rect: [w - 1.5, RADIUS, 1, h - 2 * RADIUS], color: WHITE, opacity: 0.18 },
    { rect: [RADIUS, h - 1.5, w - 2 * RADIUS, 1], color: BLACK, opacity: 0.4 },
  ];
}

export function activate({ timeline }: { timeline: any }) {
  timeline.chrome({ clipRadius: RADIUS });
  timeline.paint((clips: Clip[]) => {
    const out: Record<string, unknown[]> = {};
    for (const clip of clips) out[clip.id] = glass(clip);
    return out;
  });
}

export const view = () => <text>Clips on the timeline are drawn as glass.</text>;
"##;

/// The light leak's extension and transition ids.
pub const LEAK: &str = "light-leak";
pub const LEAK_TRANSITION: &str = "light-leak.transition";
/// "Write me a light-leak transition": a v3 extension registering a GLSL
/// transition (v4 has no transition API), then the transition set on the
/// cut between Dusk and City (4 s) as an edit, through `apply_commands`.
fn light_leak(tools: &mut crate::mcp::Connection, staging: &Path) -> anyhow::Result<()> {
    stage(staging, LEAK)?;
    publish(tools, LEAK, "A light-leak transition")?;
    call(tools, "get_project_state", json!({"indexOnly": true}))?;
    call_when_loaded(
        tools,
        "apply_commands",
        json!({"commands": [{"type": "set_transition", "layer": "Dusk", "edge": "out",
            "pairedWith": "City", "transition": {"type": LEAK_TRANSITION, "dur": 1.2}}]}),
        "Not available",
    )?;
    Ok(())
}
const LEAK_SOURCE: &str = r##"import type { PowermoveAPI } from 'powermove';

// A warm leak of light that blooms over the outgoing shot, washes across
// the frame and burns off into the incoming one.
export default function activate(api: PowermoveAPI) {
  api.transitions.register({
    id: 'light-leak.transition',
    label: 'Light Leak',
    params: [
      { k: 'amount', label: 'Amount', def: 1, min: 0, max: 1.5, step: 0.01 },
    ],
    frag: `
      vec4 a = texture(u_from, v_st);
      vec4 b = texture(u_to, v_st);
      float p = clamp(u_prog, 0.0, 1.0);
      float bell = sin(p * 3.14159265);
      // A soft blob drifting across, and a long band behind it.
      vec2 c = vec2(mix(-0.1, 1.1, p), 0.28 + 0.18 * sin(p * 2.6));
      vec2 d = (v_st - c) * vec2(1.0, 1.8);
      float blob = exp(-dot(d, d) * 7.0);
      float band = exp(-pow(v_st.x * 0.8 + v_st.y * 0.45 - mix(-0.4, 1.6, p), 2.0) * 16.0);
      float edge = smoothstep(0.55, 1.0, 1.0 - v_st.y) * 0.25;
      float leak = clamp((blob * 1.1 + band * 0.45 + edge * blob) * bell * u_amount, 0.0, 0.92);
      vec3 warm = mix(vec3(1.0, 0.33, 0.08), vec3(1.0, 0.8, 0.55), blob);
      vec4 base = mix(a, b, smoothstep(0.4, 0.6, p));
      vec3 lit = 1.0 - (1.0 - base.rgb) * (1.0 - warm * leak);
      o = vec4(lit, base.a);
    `,
  });
}
"##;

/// The beat-markers panel's extension id.
pub const BEATS: &str = "beat-markers";
/// "Build me a beat-markers panel": the new-panel flow an agent follows
/// (`panel_begin`, the panel written in the stage, `publish_extensions`,
/// `panel_build` plan/commit/ready, `open_panel`).
fn beat_markers(tools: &mut crate::mcp::Connection, staging: &Path) -> anyhow::Result<()> {
    let begun = call(
        tools,
        "panel_begin",
        json!({"title": "Beat Markers", "placement": {"beside": "fxbrowser"}, "size_hint": 140,
            "planned": [{"kind": "markers", "label": "Beats"}]}),
    )?;
    let build = begun["build_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("panel_begin gave no build_id: {begun}"))?
        .to_owned();
    call(
        tools,
        "panel_build",
        json!({"build_id": build, "event": "plan", "row": 0, "kind": "markers", "label": "Beats"}),
    )?;
    stage(staging, BEATS)?;
    publish(tools, BEATS, "A beat-markers panel")?;
    call_when_loaded(
        tools,
        "panel_build",
        json!({"build_id": build, "event": "commit", "row": 0, "panelId": BEATS}),
        "Publish the new panel first",
    )?;
    call(
        tools,
        "panel_build",
        json!({"build_id": build, "event": "ready", "panelId": BEATS}),
    )?;
    call(tools, "open_panel", json!({"panelId": BEATS}))?;
    Ok(())
}
const BEATS_SOURCE: &str = r##"import { useEffect, useState } from "powermove/hooks";

// The film's music: 100 BPM in 4/4 over the 8-second composition.
const BPM = 100;
const SECONDS = 8;
const BEAT = 60 / BPM;
const COUNT = Math.floor(SECONDS / BEAT);

let host: any;
export function activate(api: { host: any }) {
  host = api.host;
}

function Marker({ index, lit }: { index: number; lit: boolean }) {
  const bar = index % 4 === 0;
  const colour = lit ? { token: "colors.accent" } : { token: "colors.text" };
  return (
    <div style={{ flexGrow: 1, flexBasis: 0, display: "flex", flexDirection: "column",
      alignItems: "center", gap: 6 }}>
      <div style={{ width: bar ? 4 : 2, height: bar ? 30 : 20, borderRadius: 2,
        backgroundColor: colour, opacity: lit ? 1 : bar ? 0.55 : 0.28 }} />
      <text style={{ fontSize: 10, color: colour, opacity: lit ? 1 : 0.45 }}>
        {bar ? `${index / 4 + 1}` : "·"}
      </text>
    </div>
  );
}

export function view() {
  const [time, setTime] = useState(0);
  useEffect(() => {
    let stop: (() => void) | undefined;
    host?.subscribe("beat-markers:time", "time", (v: any) => setTime(Number(v?.time ?? 0)))
      .then((s: () => void) => (stop = s))
      .catch(() => {});
    return () => stop?.();
  }, []);
  const current = Math.floor(time / BEAT) % COUNT;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: 12 }}>
      <text style={{ opacity: 0.6 }}>{`${BPM} BPM · 4/4 · beat ${current + 1}`}</text>
      <div style={{ display: "flex", flexDirection: "row", alignItems: "flex-end" }}>
        {Array.from({ length: COUNT }, (_, i) => <Marker index={i} lit={i === current} />)}
      </div>
    </div>
  );
}
"##;

/// "Put my tools where After Effects had them": the Project panel with
/// Effect Controls under it on the left, Effects & Presets over Properties
/// on the right, a taller timeline.
fn classic_layout(tools: &mut crate::mcp::Connection) -> anyhow::Result<()> {
    call(
        tools,
        "set_panel_layout",
        json!({
            "name": "Classic",
            "docks": [
                {"id": "left", "size": 300, "panels": [{"id": "assets", "size": 260}, {"id": "layer-effects", "flex": true}]},
                {"id": "center", "panels": [{"id": "viewer", "flex": true}]},
                {"id": "right", "size": 300, "panels": [{"id": "fxbrowser", "size": 300}, {"id": "inspector", "flex": true}, {"id": BEATS, "size": 130}]},
                {"id": "bottom", "size": 330, "panels": [{"id": "timeline", "flex": true}]},
            ],
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// What the film's agent writes passes the check `publish_extensions`
    /// runs (compiled with the app's compiler and activated in a throwaway
    /// runtime), so the film's changes fail only if the app does.
    #[test]
    fn the_films_extensions_pass_the_publish_check() {
        let root = tempfile::Builder::new()
            .prefix("pm-film-check-")
            .tempdir()
            .unwrap();
        for id in [GLASS, LEAK, BEATS] {
            stage(root.path(), id).unwrap();
            let raw = extension(id).0;
            let manifest = pm_compat_runner::manifest::parse(&raw).unwrap();
            crate::extension_authoring::check(&root.path().join(id), &manifest)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }
}
