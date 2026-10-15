//! The `edit.*` requests: `pm-edit`'s kernels over the mirror
//! (docs/engine/pm-edit.md).
//!
//! A request names the composition (`comp`, `PM.curComp()`), the open one
//! (`open`), the time (`t`), the layers by index, and an id source (`uid`:
//! `{ base, start }`) for ids the kernel creates. The reply is
//! `{"ok":{"ops":[…],"next":n,…}}`: the canonical operations the renderer
//! applies to the live document inside its history transaction (contract 5's
//! JSON form), the id counter's next value, and the kernel's own answers. A
//! legacy throw is `{"throw":…}`; `{"host"}`, `{"fonts"}`, and `{"ink"}` mean
//! the renderer runs the legacy edit (three.js geometry, fonts not served
//! yet, ink without outlines), as for queries.
//!
//! Kernels never write the mirror: the renderer's own write reaches it
//! through the barrier like any other edit.

use std::sync::Arc;

use pm_edit::align::{self, Mode, Spread};
use pm_edit::anchor::{self, Kind};
use pm_edit::split_text;
use pm_edit::{Edit, IdSource, comp_bounds, groups, materials, pose};
use pm_eval::JsThrow;
use pm_eval::space3d::Rect;
use pm_model::ops::ops_to_value;
use pm_model::{Document, JsString, Value};

use crate::engine::{Fonts, InkProbe, QueryResult, RefuseHost, Unanswered, host_only};
use crate::expr::PmExpressions;
use crate::reply::Reply;

fn get<'v>(request: &'v Value, key: &str) -> Option<&'v Value> {
    request.as_object().and_then(|m| m.get(key))
}

fn number(request: &Value, key: &str) -> f64 {
    get(request, key)
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN)
}

/// A layer index of the edited composition.
fn index(v: Option<&Value>, count: usize) -> Result<usize, JsThrow> {
    match v.and_then(Value::as_f64) {
        Some(i) if i >= 0.0 && i.fract() == 0.0 && (i as usize) < count => Ok(i as usize),
        _ => Err(JsThrow::type_error("Layer not found")),
    }
}

fn indexes(v: Option<&Value>, count: usize) -> Result<Vec<usize>, JsThrow> {
    v.and_then(Value::as_array)
        .ok_or_else(|| JsThrow::type_error("Layer not found"))?
        .iter()
        .map(|i| index(Some(i), count))
        .collect()
}

/// `null` or a layer index.
fn optional_index(v: Option<&Value>, count: usize) -> Result<Option<usize>, JsThrow> {
    match v {
        None | Some(Value::Null) => Ok(None),
        other => index(other, count).map(Some),
    }
}

fn rect(v: Option<&Value>) -> Result<Rect, JsThrow> {
    let read = |k: &str| {
        v.and_then(|b| get(b, k))
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN)
    };
    if v.is_none() {
        return Err(JsThrow::type_error("bounds are required"));
    }
    Ok(Rect {
        x0: read("x0"),
        y0: read("y0"),
        x1: read("x1"),
        y1: read("y1"),
    })
}

fn kind(v: Option<&Value>) -> Result<Kind, JsThrow> {
    match v.and_then(Value::as_str) {
        Some("font") => Ok(Kind::Font),
        Some("size") => Ok(Kind::Size),
        _ => Err(JsThrow::type_error("kind must be 'font' or 'size'")),
    }
}

fn rect_reply(out: &mut Reply, r: &Rect) {
    out.raw("{\"x0\":");
    out.num(r.x0);
    out.raw(",\"y0\":");
    out.num(r.y0);
    out.raw(",\"x1\":");
    out.num(r.x1);
    out.raw(",\"y1\":");
    out.num(r.y1);
    out.raw("}");
}

fn moves_reply(out: &mut Reply, moves: &[align::Move]) {
    out.raw(",\"moves\":[");
    for (i, m) in moves.iter().enumerate() {
        if i > 0 {
            out.raw(",");
        }
        out.raw("{\"layer\":");
        out.num(m.layer as f64);
        out.raw(",\"channel\":");
        out.str(m.channel);
        out.raw(",\"value\":");
        out.num(m.value);
        out.raw("}");
    }
    out.raw("]");
}

/// Runs one `edit.<name>` request.
pub(crate) fn dispatch(
    doc: &Document,
    fonts: &mut Fonts,
    name: &str,
    request: &Value,
    out: &mut Reply,
) -> QueryResult {
    let comp = get(request, "comp").and_then(Value::as_str).unwrap_or("");
    let open = get(request, "open").and_then(Value::as_str).unwrap_or(comp);
    let t = number(request, "t");
    let uid = get(request, "uid");
    let ids = IdSource::new(
        uid.and_then(|u| get(u, "base"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        uid.and_then(|u| get(u, "start"))
            .and_then(Value::as_f64)
            .filter(|n| *n >= 0.0 && n.fract() == 0.0)
            .unwrap_or(1.0) as u64,
    );
    let mut edit = match Edit::new(doc, comp, open, Some(Arc::new(PmExpressions)), ids) {
        Ok(e) => e,
        Err(e) => return Ok(Err(e)),
    };
    let count = edit.layers().len();
    fonts.clear_reports();
    let outlines = fonts.outlines;
    let mut probe = InkProbe {
        inner: &mut fonts.measure,
        ink: false,
    };
    let mut host = RefuseHost;
    // Members of the reply after `ops` and `next`, each starting with a comma.
    let mut extra = Reply::new();
    let result: Result<(), JsThrow> = (|| match name {
        "reparent" => {
            let layer = index(get(request, "layer"), count)?;
            let parent = optional_index(get(request, "parent"), count)?;
            pose::reparent(&mut edit, layer, parent, t)
        }
        "moveToGroup" => {
            let roots = indexes(get(request, "roots"), count)?;
            let target = optional_index(get(request, "target"), count)?;
            groups::move_to_group(&mut edit, &roots, target, t)
        }
        "ungroup" => {
            let gs = indexes(get(request, "groups"), count)?;
            let children = groups::ungroup(&mut edit, &gs, t)?;
            extra.raw(",\"ids\":[");
            for (i, id) in children.iter().enumerate() {
                if i > 0 {
                    extra.raw(",");
                }
                extra.js(id);
            }
            extra.raw("]");
            Ok(())
        }
        "groupPlacement" => {
            let group = index(get(request, "group"), count)?;
            let b = groups::group_placement(&mut edit, group, t, &mut probe, &mut host)?;
            extra.raw(",\"bounds\":");
            match b {
                Some(b) => rect_reply(&mut extra, &b.rect()),
                None => extra.raw("null"),
            }
            Ok(())
        }
        "align" => {
            let layers = indexes(get(request, "layers"), count)?;
            let mode = get(request, "mode")
                .and_then(Value::as_str)
                .and_then(Mode::parse)
                .ok_or_else(|| JsThrow::type_error("Unknown alignment"))?;
            let moves = align::align(&mut edit, &layers, mode, t, &mut probe, &mut host)?;
            moves_reply(&mut extra, &moves);
            Ok(())
        }
        "distribute" => {
            let layers = indexes(get(request, "layers"), count)?;
            let spread = get(request, "spread")
                .and_then(Value::as_str)
                .and_then(Spread::parse)
                .ok_or_else(|| JsThrow::type_error("Unknown distribution"))?;
            let moves = align::distribute(&mut edit, &layers, spread, t, &mut probe, &mut host)?;
            moves_reply(&mut extra, &moves);
            Ok(())
        }
        "captureFontAnchor" | "captureSizeAnchor" => {
            let layer = index(get(request, "layer"), count)?;
            let fields: Vec<JsString> = get(request, "fields")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(Value::to_js_string).collect())
                .unwrap_or_default();
            if name == "captureFontAnchor" {
                anchor::capture_font_anchor(&mut edit, layer, &fields, t, &mut probe)
            } else {
                anchor::capture_size_anchor(&mut edit, layer, &fields, t)
            }
        }
        "anchorReference" => {
            let layer = index(get(request, "layer"), count)?;
            let k = kind(get(request, "kind"))?;
            let r = anchor::anchor_reference(&mut edit, layer, k, t, &mut probe)?;
            extra.raw(",\"bounds\":");
            match r {
                Some(r) => rect_reply(&mut extra, &r),
                None => extra.raw("null"),
            }
            Ok(())
        }
        "anchorMoved" => {
            let layer = index(get(request, "layer"), count)?;
            let k = kind(get(request, "kind"))?;
            let bounds = rect(get(request, "bounds"))?;
            anchor::anchor_moved(&mut edit, layer, k, t, &bounds)
        }
        "splitText" => {
            let sources = indexes(get(request, "sources"), count)?;
            let mode = get(request, "mode")
                .and_then(Value::as_str)
                .and_then(split_text::Mode::parse)
                .ok_or_else(|| JsThrow::type_error("Unknown split mode"))?;
            let plans = split_text::split_text(&mut edit, &sources, mode, t, &mut probe)?;
            extra.raw(",\"plans\":[");
            for (i, plan) in plans.iter().enumerate() {
                if i > 0 {
                    extra.raw(",");
                }
                extra.raw("{\"source\":");
                extra.num(plan.source as f64);
                extra.raw(",\"children\":");
                extra.value(&Value::Array(plan.children.clone()));
                extra.raw("}");
            }
            extra.raw("]");
            Ok(())
        }
        "leaveAttributes" => {
            let layer = index(get(request, "layer"), count)?;
            let b = comp_bounds::leave_attributes(&mut edit, layer, t, &mut probe, &mut host)?;
            for (k, v) in [("w", b.w), ("h", b.h), ("ax", b.ax), ("ay", b.ay)] {
                extra.raw(",");
                extra.str(k);
                extra.raw(":");
                extra.num(v);
            }
            Ok(())
        }
        "syncPeers" => {
            let layer = index(get(request, "layer"), count)?;
            let channel = get(request, "channel")
                .and_then(Value::as_str)
                .ok_or_else(|| JsThrow::type_error("channel is required"))?;
            let n = materials::synchronize_peers(&mut edit, layer, channel)?;
            extra.raw(",\"count\":");
            extra.num(n as f64);
            Ok(())
        }
        other => Err(JsThrow::type_error(format!("unknown edit '{other}'"))),
    })();
    let ink = probe.ink;
    match result {
        Err(e) if host_only(&e) => return Err(Unanswered::Host),
        Err(e) => return Ok(Err(e)),
        Ok(()) => {}
    }
    if fonts.incomplete() {
        return Err(Unanswered::Fonts);
    }
    if ink && !outlines {
        return Err(Unanswered::Ink);
    }
    let next = edit.ids.peek();
    let ops = edit.finish();
    out.raw("{\"ops\":");
    out.value(&ops_to_value(&ops));
    out.raw(",\"next\":");
    out.num(next as f64);
    out.raw(&extra.text);
    out.special |= extra.special;
    out.raw("}");
    Ok(Ok(()))
}
