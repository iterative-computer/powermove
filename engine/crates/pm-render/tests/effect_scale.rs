//! A viewer frame at another resolution than the composition's is the Full
//! picture resampled, never a different picture. A playing viewer draws at
//! half resolution (Auto), and the viewer draws at its drawable's size (a
//! resized window, a Retina display); an effect whose cell, radius or offset
//! in pixels followed that raster would change size between playing and
//! paused, or with the window.
//!
//! Every built-in effect, two synthetic extensions (a cell grid in `v_px`,
//! and a pattern in `gl_FragCoord`, which takes the composition-density
//! replay) and, when the shader corpus is present (as in `shader_gate.rs`),
//! every effect definition it captured, run on an adjustment layer over a
//! textured scene at Full (scale 1) and at [`JUDGED`] scales.
//!
//! What is measured: both frames box-filtered onto one grid, a quarter of
//! the coarser frame's resolution (a sixth of the 0.75 frame's, whose
//! quarter is no integer box), then blurred by [1 2 1]² on both sides; the
//! mean over pixels of the largest channel difference, on values clamped to
//! [0, 1]. The grid and blur absorb sampling (an edge moved by part of a
//! pixel, per-pixel noise); a cell, radius or offset of another size moves
//! structure several pixels wide and does not average out.
//!
//! The bound, [`BOUND`] (half an 8-bit code on average), comes from the
//! measured table (lane notes): with the fix every effect without sub-pixel
//! detail is at most 0.0012; before it, every effect the fix changes was at
//! least 0.0023 (a cell twice as large: 0.02 to 0.12). Shaders whose own
//! detail is finer than the coarser frame's pixel (a one-pixel Sobel or
//! derivative, scanlines closer than a pixel, a two-to-four-pixel dot pitch,
//! a glow pyramid, per-pixel grain) cannot match Full by any coordinate
//! mapping: [`FINE_DETAIL`] names each by a hash of its source with its own
//! ceiling, 1.25 times its measured residual, so a scale bug in any of them
//! (before the fix 1.5 to 9 times their ceilings) still fails. A quarter of
//! the composition (0.25) is printed, not judged: a one-pixel kernel cannot
//! be drawn at a quarter pixel.

mod support;

use std::collections::HashMap;
use std::path::PathBuf;

use pm_protocol::color::Precision;
use pm_protocol::common::{Seq, Size};
use pm_protocol::definitions::{DefinitionKind, DefinitionOrigin, DefinitionSnapshot};
use pm_render::{MapResolver, RenderRequest, Renderer, builtin_definitions};
use serde_json::{Value, json};
use support::doc::*;
use support::{repo_root, ticks};

const W: u32 = 384;
const H: u32 = 224;
/// Viewer scales against Full: playback at Half, drawables smaller and
/// larger than the composition.
const JUDGED: [f64; 4] = [0.5, 0.75, 1.5, 2.0];
/// Printed only (see the module notes).
const REPORTED: f64 = 0.25;
const BOUND: f64 = 0.002;

/// Shaders with detail finer than the coarser frame's pixel: the FNV-1a 64
/// of the GLSL source, the ceiling, the idiom.
const FINE_DETAIL: &[(&str, f64, &str)] = &[
    ("325d469a2f494e25", 0.026, "one-pixel Sobel"),
    ("aad8c2457eeb196d", 0.021, "scanlines closer than a pixel"),
    ("78948447dede5359", 0.013, "per-pixel derivatives"),
    ("1ed1f263635e8a6b", 0.013, "per-pixel derivatives"),
    ("84bf670c5b7c5791", 0.013, "fragment-coordinate halftone"),
    ("7cdd93aab6f434bc", 0.012, "one-pixel bevel taps"),
    ("6910cdfedc547489", 0.009, "one-pixel strokes"),
    ("098c1b18049b7d2f", 0.008, "one-pixel threshold walk"),
    ("e934ac83079fc24b", 0.007, "dot pitch of a few pixels"),
    ("0486bfffb0f616c0", 0.007, "one-pixel strokes"),
    ("329ae58e05bdb1e8", 0.006, "glow pyramid"),
    ("778f8c7aedb58e95", 0.006, "glow pyramid"),
    ("faf5f0f0b82b05fc", 0.003, "per-pixel grain"),
];

fn fnv64(source: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in source.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// An ASCII-style extension: cells of `u_cell` pixels, each a dot sized by
/// the brightness at its centre, its edge two pixels wide (one pixel of the
/// Half frame: a narrower edge is sub-pixel detail, see the module notes).
const CELLS: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nin vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nuniform float u_cell;\nout vec4 o;\nvoid main(){ vec2 cell = floor(v_px / u_cell); vec4 c = texture(u_tex, (cell + 0.5) * u_cell / u_res); float d = length(v_px - (cell + 0.5) * u_cell); float l = dot(c.rgb, vec3(0.299, 0.587, 0.114)); float r = (0.2 + 0.3 * l) * u_cell; o = vec4(c.rgb * clamp((r - d) / 2.0 + 0.5, 0.0, 1.0), c.a); }\n";

/// A pattern in physical fragment coordinates (period 25 pixels): it can
/// only follow the frame's density by drawing at the composition's.
const FRAGMENT: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nout vec4 o;\nvoid main(){ float k = 0.5 + 0.5 * sin(gl_FragCoord.x * 0.25) * sin(gl_FragCoord.y * 0.25); o = texture(u_tex, v_st) * vec4(vec3(0.25 + 0.75 * k), 1.0); }\n";

fn extension(id: &str, source: &str, params: Value, origin: DefinitionOrigin) -> DefinitionSnapshot {
    let mut def: DefinitionSnapshot = serde_json::from_value(json!({
        "id": id, "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": source, "bodyLineOffset": 0, "bodyUtf16Len": source.encode_utf16().count(), "bodyUtf8Len": source.len() },
        "params": params,
        "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap();
    def.origin = origin;
    def
}

/// A background, a rotated image, two solids, and the effect (if any) on an
/// adjustment layer over all of them.
fn scene(effect: Option<&str>) -> (pm_model::Document, MapResolver) {
    let dur = 2.0;
    let mut adjustment = layer("adjust", "adjustment", json!({}), 0.0, 0.0, dur);
    if let Some(effect) = effect {
        adjustment["fx"] = json!([{ "id": "e", "type": effect, "on": true, "p": {} }]);
    }
    let mut chip = layer(
        "chip",
        "solid",
        json!({ "color": "#3FCF8E", "w": 70, "h": 50 }),
        220.0,
        20.0,
        dur,
    );
    set(&mut chip, "rotation", json!(12));
    let mut image = layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "stretch", "w": 200, "h": 112 }),
        50.0,
        40.0,
        dur,
    );
    set(&mut image, "rotation", json!(-8));
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#E8E2D0", "w": 120, "h": 60 }),
        10.0,
        110.0,
        dur,
    );
    let mut doc = project(
        W,
        H,
        dur,
        solid_fill("#2A2F45"),
        vec![adjustment, chip, image, back],
    );
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert(
        "still".into(),
        repo_root().join("engine/fixtures/still-alpha.png"),
    );
    (load(&doc), resolver)
}

/// A viewer frame at `scale`, or `None` when the effect did not draw
/// natively (refused, routed or failed: the shader gate judges those).
fn frame(
    r: &mut Renderer,
    doc: &pm_model::Document,
    res: &MapResolver,
    scale: f64,
) -> Option<Vec<f32>> {
    let req = RenderRequest {
        viewer: true,
        ..RenderRequest::at_scale(Size::new(W, H), scale, Precision::Float16)
    };
    let frame = r.render(doc, "Ptest-root", ticks(0.5), &req, res).ok()?;
    if !frame.failures.is_empty() {
        return None;
    }
    r.read_rgba(&frame).ok()
}

/// A frame at `scale` (values clamped to [0, 1]) box-filtered onto the grid
/// of `grid` output pixels per composition pixel.
fn on_grid(px: &[f32], scale: f64, grid: f64) -> Vec<f64> {
    let w = (f64::from(W) * scale).round() as usize;
    let k = (scale / grid).round() as usize;
    assert!(k >= 1 && ((scale / grid) - k as f64).abs() < 1e-9);
    let (gw, gh) = (w / k, px.len() / 4 / w / k);
    let mut out = vec![0.; gw * gh * 4];
    for y in 0..gh * k {
        for x in 0..gw * k {
            for c in 0..4 {
                out[((y / k) * gw + x / k) * 4 + c] +=
                    f64::from(px[(y * w + x) * 4 + c]).clamp(0., 1.) / (k * k) as f64;
            }
        }
    }
    out
}

/// A [1 2 1]² blur of a grid `w` wide (edges clamped).
fn binomial(px: &[f64], w: usize) -> Vec<f64> {
    let h = px.len() / 4 / w;
    let at = |x: isize, y: isize, c: usize| {
        px[((y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize) * 4 + c]
    };
    let mut out = vec![0.; px.len()];
    for y in 0..h as isize {
        for x in 0..w as isize {
            for c in 0..4 {
                let mut v = 0.;
                for (dy, wy) in [(-1, 1.), (0, 2.), (1, 1.)] {
                    for (dx, wx) in [(-1, 1.), (0, 2.), (1, 1.)] {
                        v += wx * wy * at(x + dx, y + dy, c);
                    }
                }
                out[(y as usize * w + x as usize) * 4 + c] = v / 16.;
            }
        }
    }
    out
}

/// The mean over pixels of the largest channel difference.
fn mean_difference(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() / 4;
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(a, b)| (0..4).map(|c| (a[c] - b[c]).abs()).fold(0., f64::max))
        .sum::<f64>()
        / n as f64
}

/// Full against a frame at `scale` as the module notes describe.
fn sampled_difference(full: &[f32], other: &[f32], scale: f64) -> f64 {
    let grid = if (scale - 0.75).abs() < 1e-9 {
        0.125
    } else {
        scale.min(1.0) / 4.
    };
    let gw = (f64::from(W) * grid).round() as usize;
    mean_difference(
        &binomial(&on_grid(full, 1.0, grid), gw),
        &binomial(&on_grid(other, scale, grid), gw),
    )
}

/// An effect's difference at each scale, [`REPORTED`] first.
struct Measured {
    id: String,
    source: String,
    at: Vec<(f64, f64)>,
}

/// Renders each definition of `ids` (with built-ins) at Full and each scale.
fn measure(definitions: Vec<DefinitionSnapshot>, ids: &[String]) -> Vec<Measured> {
    let mut set = builtin_definitions();
    set.definitions.extend(definitions);
    set.generation = Seq::new(1).unwrap();
    let sources: HashMap<String, String> = set
        .definitions
        .iter()
        .map(|d| (d.id.0.clone(), d.glsl.as_ref().map_or(String::new(), |g| g.source.clone())))
        .collect();
    let mut r = Renderer::headless().expect("a GPU");
    let _ = r.set_definitions(&set).unwrap();
    let mut out = Vec::new();
    'ids: for id in ids {
        let (doc, res) = scene(Some(id));
        let Some(full) = frame(&mut r, &doc, &res, 1.0) else {
            eprintln!("not drawn natively: {id}");
            continue;
        };
        let mut at = Vec::new();
        for scale in std::iter::once(REPORTED).chain(JUDGED) {
            let Some(other) = frame(&mut r, &doc, &res, scale) else {
                eprintln!("not drawn natively at {scale}: {id}");
                continue 'ids;
            };
            at.push((scale, sampled_difference(&full, &other, scale)));
        }
        out.push(Measured { id: id.clone(), source: sources[id].clone(), at });
    }
    out
}

/// Every judged scale within the bound, or the effect's own ceiling.
fn assert_resampled(results: &[Measured], names: &HashMap<String, String>) {
    let mut over = Vec::new();
    for m in results {
        let hash = fnv64(&m.source);
        let fine = FINE_DETAIL.iter().find(|(h, ..)| *h == hash);
        let ceiling = fine.map_or(BOUND, |f| f.1);
        let name = names.get(&m.id).unwrap_or(&m.id);
        let row: Vec<String> = m.at.iter().map(|(s, d)| format!("{s}: {d:.4}")).collect();
        eprintln!(
            "MEASURED {name} [{hash}] ceiling {ceiling}{} | {}",
            fine.map_or(String::new(), |f| format!(" ({})", f.2)),
            row.join(" | ")
        );
        if m.at.iter().any(|(s, d)| *s != REPORTED && *d > ceiling) {
            over.push((name.clone(), ceiling, m.at.clone()));
        }
    }
    assert!(over.is_empty(), "a scaled viewer frame is not Full resampled: {over:?}");
}

#[test]
fn scaled_viewer_frames_of_pixel_sized_extensions_are_the_full_frame_resampled() {
    for origin in [DefinitionOrigin::Local, DefinitionOrigin::Store] {
        let definitions = vec![
            extension(
                "synthetic.cells",
                CELLS,
                json!([{ "type": "number", "key": "cell", "default": 24, "min": 1, "max": 100, "step": null }]),
                origin,
            ),
            extension("synthetic.fragment", FRAGMENT, json!([]), origin),
        ];
        let ids = ["synthetic.cells".to_string(), "synthetic.fragment".to_string()];
        let results = measure(definitions, &ids);
        assert_eq!(results.len(), 2, "the extensions draw natively");
        assert_resampled(&results, &HashMap::new());
    }
}

#[test]
fn scaled_viewer_frames_of_every_built_in_effect_are_the_full_frame_resampled() {
    let ids: Vec<String> = builtin_definitions()
        .definitions
        .iter()
        .filter(|d| d.kind == DefinitionKind::Effect)
        .map(|d| d.id.0.clone())
        .collect();
    let results = measure(Vec::new(), &ids);
    assert!(results.len() * 2 > ids.len(), "most built-ins draw natively");
    assert_resampled(&results, &HashMap::new());
}

fn corpus() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PM_SHADER_GATE_CORPUS") {
        return Some(PathBuf::from(p));
    }
    let base = PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/Application Support/Powermove Engine Corpus");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("shaders-"))
                && p.join("corpus.json").exists()
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

/// Every effect definition the shader corpus captured (real extensions
/// through the app's loader, the Store, generated programs), under unique
/// ids with their own origin. Without a corpus the test passes vacuously.
#[test]
fn scaled_viewer_frames_of_every_corpus_effect_are_the_full_frame_resampled() {
    let Some(dir) = corpus() else {
        eprintln!("SKIP: no shader corpus");
        return;
    };
    let corpus: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("corpus.json")).unwrap()).unwrap();
    let mut definitions = Vec::new();
    let mut ids = Vec::new();
    let mut names = HashMap::new();
    for (n, case) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        if case["kind"].as_str() != Some("effect") || case["definition"].is_null() {
            continue;
        }
        let Ok(mut def) =
            serde_json::from_value::<DefinitionSnapshot>(case["definition"].clone())
        else {
            continue;
        };
        if def.kind != DefinitionKind::Effect || def.origin == DefinitionOrigin::BuiltIn {
            continue;
        }
        let id = format!("scale.{n}");
        // Private cases (the real extension loader's) are named by their
        // index only.
        let shown = if case["private"].as_bool() == Some(true)
            || case["group"].as_str() == Some("user-setups")
        {
            id.clone()
        } else {
            format!("{id} {}", def.id.0)
        };
        names.insert(id.clone(), shown);
        def.id = id.as_str().into();
        definitions.push(def);
        ids.push(id);
    }
    let results = measure(definitions, &ids);
    assert!(!results.is_empty(), "corpus effects draw natively");
    assert_resampled(&results, &names);
}
