//! 2.5D layers: bounds and picks agree with what the renderer displays
//! (docs/engine/composite.md "2.5D"; plan section 8).
//!
//! `pm_eval`'s `bounds` and `pick` are ports of today's `GL.bounds` and
//! `GL.pick`, checked against the TypeScript in pm-eval's layer parity
//! vectors (3D layers of every type among them). This test closes the
//! loop with the display: for every 2.5D layer (the 3D switch or a 3D
//! group) of the closure captures' space projects, the pixels the layer
//! draws (the frame with it versus the frame with its opacity at 0) are
//! picked as that layer (or a layer drawn over it there), at every captured
//! sample time; what a layer draws lies inside the region picked as it;
//! and for content that fills its bounds (solids, shader and extension
//! layers) the picked region is what it draws. Motion-blurred layers smear
//! past their center-time bounds, as today's do; only their center-time
//! picks are checked against their drawn pixels' core.
//!
//! Reads the newest `compclose-*` and `compclose3-*` captures (or
//! `PM_ENGINE_CORPUS`); passes with a note when there are none. Runs
//! without the harness (the CoreText registry needs the main thread).

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_eval::bounds::LayerHost;
    use pm_eval::{Comp, Evaluator, Scope, node};
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::{RenderRequest, Renderer};
    use support::*;

    struct NoHost;
    impl LayerHost for NoHost {}

    let captures: Vec<_> = match std::env::var("PM_ENGINE_CORPUS") {
        Ok(p) => p.split(',').map(std::path::PathBuf::from).collect(),
        Err(_) => ["compclose-", "compclose3-"]
            .iter()
            .filter_map(|p| newest_capture(p))
            .collect(),
    };
    if captures.is_empty() {
        eprintln!("note: no compclose capture; 2.5D picks not measured");
        return;
    }
    let only = [
        "space25d/vector",
        "space25d/text",
        "space25d/captions",
        "space25d/generated",
    ];
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .expect("built-in definitions");
    let fonts =
        || pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new());
    renderer.set_text_fonts(Some(Box::new(text::MainThreadFonts(fonts(), None))));
    let mut measure = fonts();
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for capture in &captures {
        for dir in projects(capture) {
            let p = load_project(capture, &dir);
            if !only.iter().any(|o| p.name == *o) || !p.missing_media.is_empty() {
                continue;
            }
            let root = p.document.root().unwrap();
            let comp = Comp::resolve(&p.document, &p.composition).unwrap();
            // The composition's visual 2.5D layers.
            let layers: Vec<&pm_model::Node> = node::array(node::member(Some(root), "layers"))
                .unwrap_or(&[])
                .iter()
                .collect();
            let mut ev = Evaluator::new()
                .with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
            let mut targets = Vec::new();
            let mut blurred_ids: Vec<String> = Vec::new();
            {
                let mut scope = Scope::open(&mut ev, &comp);
                for l in &layers {
                    let ty = node::to_js_string(node::member(Some(l), "type")).to_string_lossy();
                    if ["group", "null", "audio", "adjustment"].contains(&ty.as_str())
                        || pm_eval::parenting::Role3d::of(l).is_some()
                    {
                        continue;
                    }
                    if scope.is_3d_layer(l).unwrap() {
                        let id = pm_render_layer_id(l);
                        let mut chain = vec![*l];
                        chain.extend(scope.group_ancestors(l).unwrap());
                        if chain
                            .iter()
                            .any(|n| node::truthy(node::member(Some(n), "mblur")))
                        {
                            blurred_ids.push(id.clone());
                        }
                        targets.push((id, ty));
                    }
                }
            }
            for s in &p.samples {
                let request =
                    RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
                let draw = |r: &mut Renderer, doc: &pm_model::Document| {
                    let frame = r
                        .render(doc, &p.composition, ticks(s.time), &request, &p.resolver)
                        .unwrap();
                    rgba8(&r.read_rgba(&frame).unwrap())
                };
                let full = draw(&mut renderer, &p.document);
                let (w, h) = (s.width as usize, s.height as usize);
                let mut masks = Vec::new();
                for (id, _) in &targets {
                    let hidden = draw(&mut renderer, &with_opacity_zero(&p.document, id));
                    let mask: Vec<bool> = full
                        .chunks_exact(4)
                        .zip(hidden.chunks_exact(4))
                        .map(|(a, b)| a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 8))
                        .collect();
                    masks.push(mask);
                }
                // `GL.pick` at every pixel center near what the layers draw
                // (24 pixels around it), and on a 3-pixel grid elsewhere.
                let mut near = vec![false; w * h];
                for m in &masks {
                    for (i, v) in m.iter().enumerate() {
                        near[i] |= *v;
                    }
                }
                for _ in 0..24 {
                    near = pm_render::parity::dilate(&near, w, h);
                }
                let mut hits: Vec<Option<String>> = vec![None; w * h];
                let mut probed = vec![false; w * h];
                {
                    let mut scope = Scope::open(&mut ev, &comp);
                    for y in 0..h {
                        for x in 0..w {
                            let i = y * w + x;
                            if !near[i] && (x % 3 != 0 || y % 3 != 0) {
                                continue;
                            }
                            probed[i] = true;
                            hits[i] = scope
                                .pick(
                                    x as f64 + 0.5,
                                    y as f64 + 0.5,
                                    s.time,
                                    &Default::default(),
                                    &mut measure,
                                    &mut NoHost,
                                )
                                .unwrap()
                                .map(pm_render_layer_id);
                        }
                    }
                }
                for (k, (id, ty)) in targets.iter().enumerate() {
                    let mask = &masks[k];
                    // Drawn pixels away from the antialiased edge (and, for
                    // a blurred layer, from the smear past its center-time
                    // bounds).
                    let mut inner = erode(mask, w, h);
                    if blurred_ids.contains(id) {
                        for _ in 0..4 {
                            inner = erode(&inner, w, h);
                        }
                    }
                    let (mut drawn, mut missed) = (0usize, Vec::new());
                    let (mut pick_box, mut draw_box) = (Box2::default(), Box2::default());
                    for y in 0..h {
                        for x in 0..w {
                            let i = y * w + x;
                            if mask[i] {
                                draw_box.add(x, y);
                            }
                            if !probed[i] {
                                continue;
                            }
                            let hit = hits[i].clone();
                            if hit.as_deref() == Some(id.as_str()) {
                                pick_box.add(x, y);
                            }
                            if !inner[i] {
                                continue;
                            }
                            drawn += 1;
                            let ok = match &hit {
                                Some(h) if h == id => true,
                                // A layer drawn over this one here.
                                Some(h) => targets
                                    .iter()
                                    .position(|(t, _)| t == h)
                                    .is_some_and(|j| masks[j][i]),
                                None => false,
                            };
                            if !ok {
                                missed.push((x, y, hit));
                            }
                        }
                    }
                    checked += drawn;
                    let fills = matches!(ty.as_str(), "solid" | "shader" | "extension");
                    let blurred = blurred_ids.contains(id);
                    let box_ok = if drawn == 0 || blurred {
                        true
                    } else if fills {
                        pick_box.within(&draw_box, 2) && draw_box.within(&pick_box, 2)
                    } else {
                        draw_box.within(&pick_box, 2)
                    };
                    println!(
                        "{:<22} t={:<5} {:<28} {:<10} drawn {:>6} unpicked {:>4} drawn box {:?} picked box {:?}",
                        p.name,
                        s.time,
                        id,
                        ty,
                        drawn,
                        missed.len(),
                        draw_box,
                        pick_box
                    );
                    if !missed.is_empty() || !box_ok {
                        failures.push(format!(
                            "{} t={} {id}: {} drawn pixels not picked (first {:?}), boxes drawn {:?} picked {:?}",
                            p.name,
                            s.time,
                            missed.len(),
                            missed.first(),
                            draw_box,
                            pick_box
                        ));
                    }
                }
            }
        }
    }
    level2_text_on_planes(&mut failures);
    println!(
        "\n{checked} drawn pixels checked; {} failures",
        failures.len()
    );
    for f in &failures {
        println!("  FAILED {f}");
    }
    assert!(checked > 0, "no 2.5D layer was checked");
    assert!(
        failures.is_empty(),
        "{} layers whose picks differ from what is drawn",
        failures.len()
    );
}

/// A smoke test of `compat` level 2 text on 2.5D planes (rasterized in
/// layer space at the plane's density and resampled through it; no legacy
/// reference exists): a 3D layer with no rotation draws what the same flat
/// layer draws, and a turned one draws inside the region picked as it.
#[cfg(target_os = "macos")]
fn level2_text_on_planes(failures: &mut Vec<String>) {
    use pm_eval::bounds::LayerHost;
    use pm_eval::{Comp, Evaluator, Scope};
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::{RenderRequest, Renderer};
    use serde_json::json;
    use support::doc::{layer, no_fill, project, set};

    struct NoHost;
    impl LayerHost for NoHost {}

    let families = ["SF Pro Display"];
    let mut registry = pm_text::registry::CoreTextRegistry::new();
    let l2 = support::text::level2_fonts(&mut registry, &families);
    let measure = pm_eval::text_measure::CompatMeasure::new(registry);
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
        measure,
        Some(l2),
    ))));
    let mut pick_measure =
        pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new());
    let (w, h) = (640usize, 360usize);
    let render = |r: &mut Renderer, three_d: bool, rotation_y: f64, scale: f64| {
        let d = json!({
            "text": "Level two on a plane", "font": "SF Pro Display", "weight": 600, "size": 40,
            "tracking": 0, "leading": 1.2, "align": "center", "italic": false, "color": "#FFFFFF",
            "boxWidth": 0, "boxHeight": 0
        });
        let mut l = layer("t", "text", d, 320.25, 180.5, 10.0);
        l["compat"] = json!(2);
        l["threeD"] = json!(three_d);
        set(&mut l, "rotation.y", json!(rotation_y));
        set(&mut l, "scale.x", json!(scale * 100.0));
        set(&mut l, "scale.y", json!(scale * 100.0));
        let doc = project(w as u32, h as u32, 10.0, no_fill(), vec![l]);
        let doc =
            pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1);
        let mut request = RenderRequest::sized(Size::new(w as u32, h as u32), Precision::Float32);
        request.background = false;
        let frame = r
            .render(
                &doc,
                "Ptest-root",
                support::ticks(0.0),
                &request,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        (doc, support::rgba8(&r.read_rgba(&frame).unwrap()))
    };
    let ink = |px: &[u8]| -> Vec<bool> { px.chunks_exact(4).map(|p| p[3] > 8).collect() };
    let bbox = |m: &[bool]| {
        let mut b = Box2::default();
        for (i, v) in m.iter().enumerate() {
            if *v {
                b.add(i % w, i / w);
            }
        }
        b
    };
    for scale in [1.0, 1.75] {
        // No rotation: the plane is the layer's own 2D placement.
        let (_, flat) = render(&mut renderer, false, 0.0, scale);
        let (_, plane) = render(&mut renderer, true, 0.0, scale);
        let (fb, pb) = (bbox(&ink(&flat)), bbox(&ink(&plane)));
        let mean: f64 = flat
            .chunks_exact(4)
            .zip(plane.chunks_exact(4))
            .map(|(a, b)| f64::from(a[3].abs_diff(b[3])))
            .sum::<f64>()
            / (w * h) as f64;
        println!(
            "level 2 text, scale {scale}: flat box {fb:?}, unturned plane box {pb:?}, mean alpha difference {mean:.4} CV"
        );
        if !(fb.within(&pb, 1) && pb.within(&fb, 1)) || mean > 0.5 {
            failures.push(format!("level 2 text on an unturned plane, scale {scale}: boxes {fb:?} vs {pb:?}, mean {mean:.4} CV"));
        }
        // Turned: what it draws lies inside the region picked as it.
        for rotation in [-40.0, 30.0] {
            let (doc, turned) = render(&mut renderer, true, rotation, scale);
            let comp = Comp::resolve(&doc, "Ptest-root").unwrap();
            let mut ev = Evaluator::new();
            let mut scope = Scope::open(&mut ev, &comp);
            let mask = ink(&turned);
            let inner = erode(&mask, w, h);
            let mut missed = 0;
            for (i, v) in inner.iter().enumerate() {
                if *v
                    && scope
                        .pick(
                            (i % w) as f64 + 0.5,
                            (i / w) as f64 + 0.5,
                            0.0,
                            &Default::default(),
                            &mut pick_measure,
                            &mut NoHost,
                        )
                        .unwrap()
                        .is_none()
                {
                    missed += 1;
                }
            }
            let drawn = inner.iter().filter(|v| **v).count();
            println!(
                "level 2 text, scale {scale}, rotation.y {rotation}: {drawn} drawn, {missed} outside the picked region, box {:?}",
                bbox(&mask)
            );
            if drawn == 0 || missed > 0 {
                failures.push(format!("level 2 text on a plane turned {rotation}°, scale {scale}: {missed} of {drawn} drawn pixels outside its pick region"));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn pm_render_layer_id(l: &pm_model::Node) -> String {
    pm_eval::node::to_js_string(pm_eval::node::member(Some(l), "id")).to_string_lossy()
}

/// The document with one layer at opacity 0 (in any composition).
#[cfg(target_os = "macos")]
fn with_opacity_zero(doc: &pm_model::Document, id: &str) -> pm_model::Document {
    fn visit(v: &mut pm_model::Value, id: &str) {
        match v {
            pm_model::Value::Object(o) => {
                let is_target = matches!(o.get("id"), Some(pm_model::Value::String(s)) if s.to_string_lossy() == id)
                    && o.get("type").is_some()
                    && o.get("p").is_some();
                if is_target && let Some(pm_model::Value::Object(p)) = o.get_mut("p") {
                    let mut ch = pm_model::json::Map::new();
                    ch.insert("v", pm_model::Value::Number(0.0));
                    ch.insert("kf", pm_model::Value::Array(Vec::new()));
                    ch.insert("expr", pm_model::Value::Null);
                    p.insert("opacity", pm_model::Value::Object(ch));
                }
                for (_, child) in o.iter_mut() {
                    visit(child, id);
                }
            }
            pm_model::Value::Array(a) => {
                for child in a.iter_mut() {
                    visit(child, id);
                }
            }
            _ => {}
        }
    }
    let mut root = doc.to_value().expect("a document");
    visit(&mut root, id);
    pm_model::Document::new(Some(root), doc.revision() + 1)
}

/// Pixels whose 8 neighbours are all set.
#[cfg(target_os = "macos")]
fn erode(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            out[y * w + x] = (0..9).all(|k| mask[(y + k / 3 - 1) * w + x + k % 3 - 1]);
        }
    }
    out
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct Box2 {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

#[cfg(target_os = "macos")]
impl Default for Box2 {
    fn default() -> Self {
        Box2 {
            x0: usize::MAX,
            y0: usize::MAX,
            x1: 0,
            y1: 0,
        }
    }
}

#[cfg(target_os = "macos")]
impl Box2 {
    fn add(&mut self, x: usize, y: usize) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }
    /// Whether this box lies inside `o` grown by `d` pixels.
    fn within(&self, o: &Box2, d: usize) -> bool {
        if self.x0 == usize::MAX {
            return true;
        }
        self.x0 + d >= o.x0 && self.y0 + d >= o.y0 && self.x1 <= o.x1 + d && self.y1 <= o.y1 + d
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
