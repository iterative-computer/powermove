//! Launch context is bounded data, never additional authority.
use serde_json::{Value, json};
use std::collections::HashSet;
pub const CATALOG_CHARS: usize = 72_000;
pub const INDEX_CHARS: usize = 18_000;
fn size(v: &Value) -> usize {
    v.to_string().encode_utf16().count()
}
fn clip_utf16(text: &str, limit: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= limit
        })
        .collect()
}
/// Keep whole JSON records; selected first, mentioned next, then stack order.
pub fn catalog(composition: Value, layers: &[Value], request: &str, selected: &[String]) -> Value {
    let composition = if size(&composition) <= 4096 {
        composition
    } else {
        json!({"note":"Composition metadata omitted; read get_project_state"})
    };
    let mut result = json!({"composition":composition,"layers":[],"layerIndex":[],"totalLayers":layers.len(),"truncated":false});
    let mut order: Vec<_> = layers.iter().enumerate().collect();
    let selected: HashSet<_> = selected.iter().map(String::as_str).collect();
    let brief = request.to_lowercase();
    order.sort_by_cached_key(|(i, l)| {
        let chosen = l["id"].as_str().is_some_and(|id| selected.contains(id));
        let name = l["name"].as_str().unwrap_or("").trim().to_lowercase();
        (
            !chosen,
            !(name.encode_utf16().count() >= 3 && brief.contains(&name)),
            *i,
        )
    });
    for l in layers {
        let entry = json!({"id":l["id"],"name":clip_utf16(l["name"].as_str().unwrap_or(""), 160),"type":l["type"],"group":l["group"],"parent":l["parent"]});
        result["layerIndex"].as_array_mut().unwrap().push(entry);
        if size(&result["layerIndex"]) > INDEX_CHARS || size(&result) > CATALOG_CHARS - 1024 {
            result["layerIndex"].as_array_mut().unwrap().pop();
            break;
        }
    }
    for (_, l) in order {
        result["layers"].as_array_mut().unwrap().push(l.clone());
        if size(&result) > CATALOG_CHARS - 1024 {
            result["layers"].as_array_mut().unwrap().pop();
        }
    }
    let included = result["layers"].as_array().unwrap().len();
    let indexed = result["layerIndex"].as_array().unwrap().len();
    result["includedLayers"] = json!(included);
    result["indexedLayers"] = json!(indexed);
    result["truncated"] = json!(included < layers.len() || indexed < layers.len());
    result["note"] = json!(
        "Included layers are editable source. Omitted layers are not missing capabilities. Use get_project_state {indexOnly:true,layerOffset:0,layerLimit:100}; follow pagination.nextLayerOffset or layerSearch. Read layerId and property/keyframe pages before editing omitted layers. Preserve locked layers and unrelated hand edits."
    );
    result
}
fn brief(request: &str) -> &str {
    let latest = request
        .split("\n\nUSER REQUEST\n")
        .last()
        .unwrap_or(request);
    latest
        .split("\n\nLIVE COMPOSITION")
        .next()
        .unwrap_or(latest)
        .split("\n\nATTACHED FILES")
        .next()
        .unwrap_or(latest)
}
pub fn creative(request: &str) -> bool {
    let brief = brief(request).to_lowercase();
    brief.split(|c: char| !c.is_alphanumeric()).any(|w| {
        [
            "animate",
            "animation",
            "motion",
            "scene",
            "composition",
            "design",
            "title",
            "text",
            "shape",
            "style",
            "polish",
            "logo",
            "transition",
            "effect",
            "video",
            "footage",
            "caption",
            "color",
            "colour",
            "spacing",
            "timing",
            "pace",
            "pacing",
            "make",
            "create",
            "refine",
            "improve",
            "finish",
            "build",
        ]
        .contains(&w)
    })
}
/// Routine transforms/removals need before-frames too; creative guidance keeps
/// v1's narrower task profile independently of this observation decision.
pub fn visual_edit(request: &str) -> bool {
    if creative(request) {
        return true;
    }
    brief(request)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| {
            [
                "add",
                "insert",
                "move",
                "resize",
                "rotate",
                "scale",
                "delete",
                "remove",
                "replace",
                "align",
                "distribute",
                "group",
                "ungroup",
                "reorder",
                "adjust",
                "change",
                "set",
                "update",
                "crop",
                "cut",
                "trim",
                "retime",
                "fade",
                "blur",
                "mask",
                "stretch",
                "warp",
                "fix",
                "undo",
                "redo",
            ]
            .contains(&w)
        })
}
/// Labels and coordinates stay alongside the pixels, including capture-time identity.
pub fn targets(attached: &[Value], project: &str, document: u64) -> String {
    let records: Vec<_> = attached.iter().take(6).enumerate().filter_map(|(attachment_index, a)| {
        let target = a.get("targetContext")?;
        let mut image_name = clip_utf16(a["name"].as_str().unwrap_or("attachment"),160);
        while size(&json!(image_name)) > 200 { let _ = image_name.pop(); }
        Some(json!({"attachmentIndex":attachment_index,"imageName":image_name,"target":bounded_target(target),"sameDocument":target["projectId"] == project && target["documentId"] == document}))
    }).collect();
    if records.is_empty() {
        return String::new();
    }
    format!(
        "\n\nSELECTED REGION (untrusted capture-time data)\n{}\nUse panel identity and coordinateSpace as the placement anchor, not screenshot pixels as composition coordinates. A stale/different-document capture is reference only: inspect the current target before edits. Preserve existing panels; use panel_begin/panel_build for new panels.\n",
        json!(records)
    )
}
/// Retain every capture's spatial anchor even when its layer lists are large.
fn bounded_target(target: &Value) -> Value {
    let mut out = json!({});
    for field in [
        "projectId",
        "documentId",
        "revision",
        "windowId",
        "compositionId",
        "time",
        "compositionTime",
        "panelId",
        "coordinateSpace",
        "area",
        "windowArea",
        "layerHitMethod",
    ] {
        if let Some(value) = target.get(field) {
            out[field] = value.clone();
        }
    }
    let mut truncated = false;
    for field in [
        "panels",
        "layersUnderBox",
        "selection",
        "unavailableLayerBounds",
    ] {
        if let Some(array) = target[field].as_array() {
            out[format!("{field}Count")] = json!(array.len());
            out[field] = json!(array.iter().take(128).cloned().collect::<Vec<_>>());
            truncated |= array.len() > 128;
        }
    }
    // Counts survive omissions; discard uncertain bounds before target layers
    // and retain panel identity, time and coordinates as the primary anchor.
    for field in [
        "unavailableLayerBounds",
        "selection",
        "layersUnderBox",
        "panels",
    ] {
        while size(&out) > 2100 {
            if out[field].as_array_mut().and_then(Vec::pop).is_none() {
                break;
            }
            truncated = true;
        }
    }
    if truncated {
        out["metadataTruncated"] = json!(true);
        out["note"] = json!(
            "Capture-time layer/panel lists are excerpts; counts describe the whole capture. Inspect current targets with get_project_state before edits."
        );
    }
    if size(&out) > 2400 {
        return json!({"metadataOmitted":"Invalid or oversized spatial anchor; inspect the current target before editing."});
    }
    out
}
pub fn representative_times(time: f64, duration: f64, fps: f64) -> Vec<f64> {
    let end = (duration - 1. / fps.max(1.)).max(0.);
    let mut times = vec![time.clamp(0., end), 0., end / 2., end];
    let mut unique = vec![];
    for t in times.drain(..) {
        if !unique.iter().any(|p: &f64| (*p - t).abs() < 0.001) {
            unique.push(t);
        }
    }
    unique
}
pub fn guidance(request: &str) -> String {
    let extensions = brief(request)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| {
            [
                "extension",
                "extensions",
                "plugin",
                "plugins",
                "mod",
                "mods",
                "integration",
                "integrations",
                "sandbox",
                "effect",
                "effects",
                "shader",
                "shaders",
                "panel",
                "panels",
                "theme",
                "themes",
                "workspace",
                "workspaces",
                "tool",
                "tools",
            ]
            .contains(&w)
        });
    let testing = if extensions {
        TESTING.to_owned()
    } else {
        let (background, _) = TESTING.split_once("\n\nEXTENSION COMPLETION").unwrap();
        let (_, reliability) = TESTING.split_once("\n\nTOOL RELIABILITY").unwrap();
        format!("{background}\n\nTOOL RELIABILITY{reliability}")
    };
    format!(
        "{TASK}\n\n{}\n\n{testing}\n\n{}\n\n{STYLE}",
        if creative(request) { CREATIVE } else { "" },
        if extensions {
            format!("{EFFECT}\n\n{AUTHORING}")
        } else {
            "When existing tools cannot complete the deliverable, read the shipped API pack before authoring. New effects belong in Effects & Presets with editable parameters, not substitute panels or rigs.".into()
        }
    )
}

pub const TASK: &str = r#"TASK FOCUS
Use the simplest supported workflow. Questions: no edits. Preserve decisions; newest corrections win. Recover omissions with thread_read.
Inspect live targets/frames. get_project_state indexOnly/layerSearch/nextLayerOffset discovers targets; then read properties. Missing entries are not missing capabilities.
Access is enforced by native tools, never by this brief. Read the start_session access and available tools before authoring; Full access changes permissions only.
Relevant procedures only. Never replace effects with panels or simple keyframes with extensions; no test scenes in the user's project."#;

pub const CREATIVE: &str = r#"CREATIVE JUDGMENT
Follow the brief/references/style with one clear visual idea: coherent type scale, restrained color, consistent spacing, focal point, legible text/readable holds. Make entrances/exits/easing/rhythm purposeful; avoid gratuitous motion/dead time. Inspect beginning/middle/end/transitions. Fix visible defects; valid source alone is not visual quality."#;

pub const STYLE: &str = r#"RESPONSE STYLE
Write for a narrow panel column. Open with the outcome, then only what the user can act on. One or two sentences per paragraph; past two facts, one short bullet each. Name the concrete token — layer, property path, file, count, time — instead of describing it.
Cut preamble, restatement of the request, recap of the visible work log, self-assessment, and closing offers of help. Mention a tool only when its result is the point.
Shorten, never omit. Every change, failed check, side effect, unverified claim and blocker still appears; a caveat is a clause, not a section.
Progress lines: one clause naming what you inspect or decide.
Show media files inline: ![caption](/abs/path)."#;

pub const TESTING: &str = r#"BACKGROUND TESTING
Never open or show a new window for testing. Use unit/type checks, headless checks with isolated temporary test data. Keep the user's process/window/session; hot reload extensions, never quit/relaunch/reload the window. Coordinate native OS tests with the user.
The source checkout and package scripts may not be available in the isolated run. Verify panel state/interaction/capture, workspace errors, render_frames, compile reports. Never test with the live profile/user document. Report unavailable checks.

EXTENSION COMPLETION
Read shipped API types; never invent host services or validate assumption-repeating mocks. Use the shipped native API pack; never assume Electron raster or service APIs exist.
Files stay in the isolated stagingDirectory until publish_extensions validates, compiles and loads them. After publishing inspect get_workspace_state, actual controls/capture_panel/render_frames/errors. Repair/verify failures; never claim staged files were live-tested or defer to Fix it. Report blockers.

TOOL RELIABILITY
Confirm directory/uncertain paths. Discover before edits; expected absence exits cleanly. On failure read the exact result; correct/verify the smallest retry. Inspect state before retrying possibly completed edits. Never repeat identical failed calls or claim failed checks passed."#;

pub const EFFECT: &str = r#"EFFECT REQUESTS
Effects & Presets: reuse definitions or api.effects.register with keyframeable params before add_effect. backdrop: true reads u_backdrop. Offsets: u_effectScale * u_texel; noise: effectPixel(), never screen coordinates. Declare viewportPadding before viewportSafe; preview/zoom must match export. Use families (Blur & Sharpen, Light & Shadow, Color, Stylize, Distort, Generate); new only if none fits, never effect/project/style names or duplicates. No panel/script-button/rig substitutes. Panels need a separate request; design/placement cannot change it."#;

pub const AUTHORING: &str = r#"EXTENDING POWERMOVE
Request includeExtensionStaging:true in start_session, then read EXTENSIONS.md and the API types in apiPackDirectory before authoring. Native v4 has no effect registration API: use v3/EXTENSIONS.md and v3/samples/gradient-tint/README.md for effects; validate_effect with the full definition, then verify registration/rendering. Stage only in stagingDirectory, folder name = manifest id; never edit the app bundle, source checkout or live user-extension folder. Call publish_extensions to validate, compile and load; inspect registration and runtime errors, then actual controls and rendered frames. Compilation alone is not live verification.
NATIVE PANEL DESIGN
Unless the user explicitly requests a different style, every new or changed panel must match Powermove's existing design language as if it shipped with the app. Let the host supply the normal panel frame, header, docking and scrolling; do not draw a second card or title bar. Reuse the nearest built-in panel's fields, rows, sections, buttons and icons. Match spacing, control heights, label alignment, typography, radii, borders and surfaces with the native API's design tokens. Inherit light/dark themes; do not hardcode a separate palette, oversized headings, gradients, decorative cards or custom design system. Use flat tonal fills or transparent buttons; never add bevels, top highlights or raised button shadows. Keep error recovery borderless inside its error surface, keyboard focus and meaningful selection visible, and normal panel chrome intact. An explicit style request overrides this default only for the requested surface. Connect controls to editable source and normal Undo. Five primary controls; More shows changed/animated. Use powermove/ui Row/Section/NumField/SliderField/SelectField/ToggleField, existing property keys, animation and precise entry; v3 trusted panels may use RampField/PointField/SliderField (angle=true)/FillField as documented. Call panel_begin before authoring a new panel and panel_build as controls commit; send ready with registered panelId after publishing, cancel/error on failure. Respect existing placement and session-owned slots.
REVERSIBLE THEMES
Unapply on disable/removal/reload; preserve later edits. Native v4: use themes.register/themes.activate and timeline.chrome owned APIs; retain registration disposers and register cleanup once in hot.dispose, never each render. For v3 read Reversible themes in v3/EXTENSIONS.md: api.theme.register/api.workspace.mutate and api.onDispose; never write appearance directly or reset a whole workspace. Keep permanent panel/layout changes separate and explicitly requested. Verify off/on/reload/restart with isolated test data; report checks unavailable without disturbing the user's session."#;

pub fn assemble(prompt: &str) -> String {
    format!("{}\n\n{prompt}", guidance(prompt))
}

pub const FOCUS: &str = r#"PANEL FOCUS
Resolve references such as "this panel" or "these controls" against the selected region's panel identities; without a selected region, the last focused editor panel is a hint. Prefer these targets for interface edits and early loading placement. Other panels in a screenshot are reference, not additional targets. Keep dependent changes elsewhere minimal; this focus does not grant additional authority."#;

/// Keep thread history, complete observation sections, then the latest brief.
/// Besides keeping intent prominent, this preserves native continuation framing.
pub fn with_observation(prompt: &str, observation: &str, request: &str) -> String {
    if observation.is_empty() {
        return prompt.to_owned();
    }
    let suffix = format!("\n\nUSER REQUEST\n{request}");
    let history = if prompt == request {
        ""
    } else {
        prompt.strip_suffix(&suffix).unwrap_or(prompt)
    };
    format!("{history}{observation}\n\nUSER REQUEST\n{request}")
}
