//! Layer queries beyond the kernel's synchronous surface: the remaining
//! evaluation answers the parity corpus records (`docs/engine/corpus.md`,
//! "Evaluation dumps"), so `pm-cli parity eval --wasm` can compare every
//! dumped value through this module (docs/engine/phase1-gate.md).
//!
//! | Op | Legacy call |
//! |---|---|
//! | `glyphLayout` | `PM.textLayout(L, T)` (units by character, word, line) |
//! | `textRasterGeometry` | `PM.textRasterGeometry(L, 1, T)`: box, anchor, density, selection |
//! | `textAnimation` | `textAnimationState` and `animatedGlyphs` over `PM.textLayout` |
//! | `captionBounds` | `PM.captionBounds(L, T)` |
//! | `sourceTime` | `sourceTime(PM, L, T)`; `retiming: "legacy"` for `retiming.ts`'s step |
//! | `compositionScene` | the 3D scene description of the composition at `T` |
//! | `expressionError` | `PM.expressionErrors.get(prop)` for `(comp, layer id, key)` |
//!
//! Replies use the same envelope as every query (`{"ok"}`, `{"throw"}`,
//! `{"host"}`, `{"fonts"}`, `{"ink"}`).

use pm_eval::retime::Retiming;
use pm_eval::{JsThrow, JsValue, Scope};
use pm_model::{Node, Value};
use pm_text::compat::{GlyphLayout, Unit};

use crate::engine::{Fonts, InkProbe, QueryResult, Unanswered, host_only};
use crate::reply::Reply;

fn get<'v>(request: &'v Value, key: &str) -> Option<&'v Value> {
    request.as_object().and_then(|m| m.get(key))
}

fn content(out: &mut Reply, c: &pm_eval::evaluator::Content) {
    out.raw("{");
    let mut first = true;
    for (k, v) in c {
        if matches!(v, JsValue::Undefined) {
            continue;
        }
        if !first {
            out.raw(",");
        }
        first = false;
        out.js_str(k);
        out.raw(":");
        out.js(v);
    }
    out.raw("}");
}

fn unit(out: &mut Reply, u: &Unit, characters: bool) {
    let mut first = true;
    out.raw("{");
    out.key(&mut first, "text");
    out.str(&u.text);
    for (k, v) in [("x", u.x), ("y", u.y), ("w", u.w), ("line", u.line as f64)] {
        out.key(&mut first, k);
        out.num(v);
    }
    if characters {
        out.key(&mut first, "lineUnit");
        match u.line_unit {
            Some(i) => out.num(i as f64),
            None => out.raw("null"),
        }
        out.key(&mut first, "word");
        match u.word {
            Some(i) => out.num(i as f64),
            None => out.raw("null"),
        }
    }
    out.key(&mut first, "sourceStart");
    out.num(u.source_start as f64);
    out.key(&mut first, "index");
    out.num(u.index as f64);
    out.raw("}");
}

/// `PM.textLayout`'s answer as the legacy object.
pub(crate) fn glyph_layout(out: &mut Reply, l: &GlyphLayout) {
    let mut first = true;
    out.raw("{");
    for (name, units, characters) in [
        ("characters", &l.characters, true),
        ("words", &l.words, false),
        ("lines", &l.lines, false),
    ] {
        out.key(&mut first, name);
        out.raw("[");
        for (i, u) in units.iter().enumerate() {
            if i > 0 {
                out.raw(",");
            }
            unit(out, u, characters);
        }
        out.raw("]");
    }
    out.raw("}");
}

fn rect(out: &mut Reply, items: &[(&str, f64)]) {
    let mut first = true;
    out.raw("{");
    for (k, v) in items {
        out.key(&mut first, k);
        out.num(*v);
    }
    out.raw("}");
}

/// Answers the ops above; `None` for any other op.
pub(crate) fn dispatch<'d>(
    op: &str,
    scope: &mut Scope<'_, 'd>,
    fonts: &mut Fonts,
    layer: Option<&'d Node>,
    t: f64,
    request: &Value,
    out: &mut Reply,
) -> Option<QueryResult> {
    macro_rules! try_js {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(e) if host_only(&e) => return Some(Err(Unanswered::Host)),
                Err(e) => return Some(Ok(Err(e))),
            }
        };
    }
    macro_rules! layer {
        () => {
            match layer {
                Some(l) => l,
                None => {
                    return Some(Ok(Err(JsThrow::type_error(
                        "Cannot read properties of undefined (reading 'p')",
                    ))));
                }
            }
        };
    }
    // Text answers: report fonts the mirror lacks, and ink without outlines.
    macro_rules! text_checks {
        ($probe:expr) => {{
            let ink = $probe.ink;
            if fonts.incomplete() {
                return Some(Err(Unanswered::Fonts));
            }
            if ink && !fonts.outlines {
                return Some(Err(Unanswered::Ink));
            }
        }};
    }
    match op {
        "glyphLayout" => {
            let l = layer!();
            fonts.clear_reports();
            let mut probe = InkProbe {
                inner: &mut fonts.measure,
                ink: false,
            };
            let layout = scope.text_layout(l, t, &mut probe);
            text_checks!(probe);
            let layout = try_js!(layout);
            glyph_layout(out, &layout);
        }
        "textRasterGeometry" => {
            let l = layer!();
            fonts.clear_reports();
            let mut probe = InkProbe {
                inner: &mut fonts.measure,
                ink: false,
            };
            let g = scope.text_geometry(l, t, &mut probe);
            text_checks!(probe);
            let g = try_js!(g);
            let mut first = true;
            out.raw("{");
            for (k, v) in [
                ("w", g.w),
                ("h", g.h),
                ("anchorX", g.anchor_x),
                ("anchorY", g.anchor_y),
                ("density", g.density(1.0)),
            ] {
                out.key(&mut first, k);
                out.num(v);
            }
            out.key(&mut first, "selection");
            let s = g.selection;
            rect(
                out,
                &[
                    ("x0", s.x0),
                    ("y0", s.y0),
                    ("x1", s.x1),
                    ("y1", s.y1),
                    ("w", s.w),
                    ("h", s.h),
                ],
            );
            out.raw("}");
        }
        "textAnimation" => {
            let l = layer!();
            fonts.clear_reports();
            let mut probe = InkProbe {
                inner: &mut fonts.measure,
                ink: false,
            };
            let d = try_js!(scope.resolved_text_content(l, t));
            let layout = scope.text_layout(l, t, &mut probe);
            text_checks!(probe);
            let layout = try_js!(layout);
            let state = try_js!(scope.text_animation_state(l, t, &layout));
            let glyphs = if pm_eval::text_level2::is_level2(l) {
                scope.level2_glyphs(&d, &layout, &state)
            } else {
                scope.animated_glyphs(&d, &layout, &state)
            };
            // The style keys every glyph reports: base typography plus every
            // key a range style sets (the capture's `styleKeys`).
            let mut keys: Vec<pm_model::JsString> =
                ["font", "weight", "size", "color", "italic", "tracking"]
                    .iter()
                    .map(|k| pm_model::JsString::from(*k))
                    .collect();
            for s in &state.styles {
                for k in s.p.keys() {
                    if !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
            }
            let mut first = true;
            out.raw("{");
            out.key(&mut first, "animators");
            out.raw("[");
            for (i, a) in state.animators.iter().enumerate() {
                if i > 0 {
                    out.raw(",");
                }
                let mut f = true;
                out.raw("{");
                out.key(&mut f, "id");
                out.js(&a.id);
                out.key(&mut f, "mode");
                out.str(a.mode.name());
                out.key(&mut f, "unit");
                out.str(a.unit.name());
                out.key(&mut f, "values");
                content(out, &a.values);
                out.key(&mut f, "weights");
                out.nums(&a.weights);
                out.raw("}");
            }
            out.raw("]");
            out.key(&mut first, "styles");
            out.raw("[");
            for (i, s) in state.styles.iter().enumerate() {
                if i > 0 {
                    out.raw(",");
                }
                content(out, &s.item);
            }
            out.raw("]");
            out.key(&mut first, "glyphs");
            out.raw("[");
            for (i, g) in glyphs.iter().enumerate() {
                if i > 0 {
                    out.raw(",");
                }
                let u = &g.unit;
                let mut f = true;
                out.raw("{");
                out.key(&mut f, "index");
                out.num(u.index as f64);
                out.key(&mut f, "text");
                out.str(&u.text);
                out.key(&mut f, "line");
                out.num(u.line as f64);
                out.key(&mut f, "word");
                match u.word {
                    Some(w) => out.num(w as f64),
                    None => out.raw("null"),
                }
                for (k, v) in [("x", u.x), ("y", u.y), ("w", u.w)] {
                    out.key(&mut f, k);
                    out.num(v);
                }
                out.key(&mut f, "matrix");
                out.nums(&g.matrix);
                out.key(&mut f, "opacity");
                out.num(g.opacity);
                out.key(&mut f, "blur");
                out.num(g.blur);
                out.key(&mut f, "color");
                match &g.color {
                    Some(c) => out.js_str(c),
                    None => out.raw("null"),
                }
                out.key(&mut f, "tracking");
                out.num(g.tracking);
                out.key(&mut f, "trackingShift");
                out.num(g.tracking_shift);
                out.key(&mut f, "style");
                out.raw("{");
                for (j, k) in keys.iter().enumerate() {
                    if j > 0 {
                        out.raw(",");
                    }
                    out.js_str(k);
                    out.raw(":");
                    match g.style.get(k) {
                        None | Some(JsValue::Undefined) => out.raw("null"),
                        Some(v) => out.js(v),
                    }
                }
                out.raw("}}");
            }
            out.raw("]}");
        }
        "captionBounds" => {
            let l = layer!();
            fonts.clear_reports();
            let mut probe = InkProbe {
                inner: &mut fonts.measure,
                ink: false,
            };
            let b = try_js!(scope.caption_bounds(l, t, &mut probe));
            text_checks!(probe);
            rect(
                out,
                &[
                    ("x0", b.x0),
                    ("y0", b.y0),
                    ("x1", b.x1),
                    ("y1", b.y1),
                    ("w", b.x1 - b.x0),
                    ("h", b.y1 - b.y0),
                    ("ax", 0.0),
                    ("ay", 0.0),
                ],
            );
        }
        "sourceTime" => {
            let l = layer!();
            let mode = match get(request, "retiming").and_then(Value::as_str) {
                Some("legacy") => Retiming::Legacy,
                _ => Retiming::Engine,
            };
            out.num(try_js!(scope.source_time_with(l, t, mode, None)));
        }
        "compositionScene" => {
            let scene = try_js!(scope.composition_scene(t));
            if !scene.recipe_parts.is_empty() {
                // Model group recipes need pm-geom3d (Phase 3c).
                return Some(Err(Unanswered::Host));
            }
            out.value(&scene.scene);
        }
        "expressionError" => {
            let s = |k: &str| {
                get(request, k)
                    .map(Value::to_js_string)
                    .map(|v| v.to_string_lossy())
                    .unwrap_or_default()
            };
            match scope
                .ev
                .expression_error(&s("comp"), &s("layerId"), &s("key"))
            {
                Some(m) => out.str(m),
                None => out.raw("null"),
            }
        }
        _ => return None,
    }
    Some(Ok(Ok(())))
}
