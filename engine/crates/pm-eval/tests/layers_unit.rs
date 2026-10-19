//! Unit tests for the layer queries the TypeScript vectors and the corpus do
//! not reach: picking, the group compositing boundary and transitions
//! (private to the legacy compositor), the engine's retiming step (section 5
//! rule 5), the retiming cache, host-only answers, and model scene placement.

use pm_eval::bounds::{HOST_ONLY, LayerHost, NoHost, PickOptions};
use pm_eval::retime::{RetimeCache, Retiming};
use pm_eval::text_geom::{MeasuredLine, TextMeasure};
use pm_eval::{Comp, Evaluator, JsValue, Scope};
use pm_model::json::{Value, parse};
use pm_model::{Document, JsString, Node};
use pm_text::compat::{GlyphLayout, TextContent};

#[path = "support/pm_expr_engine.rs"]
mod pm_expr_engine;

/// An evaluator that runs expressions through `pm-expr` (legacy dialect).
fn with_expressions() -> Evaluator {
    Evaluator::new().with_expressions(std::sync::Arc::new(pm_expr_engine::PmExprEngine))
}

/// Text measurement that is never called (the documents have no text).
struct NoText;

impl TextMeasure for NoText {
    fn measure_lines(&mut self, _: &TextContent) -> Vec<MeasuredLine> {
        unreachable!("no text layers")
    }
    fn plain_width(&mut self, _: &TextContent, _: &str) -> f64 {
        unreachable!("no text layers")
    }
    fn glyph_layout(&mut self, _: &TextContent) -> GlyphLayout {
        unreachable!("no text layers")
    }
    fn spaced_width(&mut self, _: &TextContent, _: &str) -> f64 {
        unreachable!("no text layers")
    }
}

fn doc(text: &str) -> Document {
    Document::new(Some(parse(text).unwrap()), 1)
}

fn ch(v: f64) -> String {
    format!(r#"{{"v":{v},"kf":[],"expr":null}}"#)
}

fn layer(id: &str, ty: &str, x: f64, y: f64, extra: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"{ty}","from":0,"dur":10,"on":true,"p":{{"position.x":{},"position.y":{},"anchor.x":{},"anchor.y":{},"scale.x":{},"scale.y":{},"rotation":{},"opacity":{}}},"d":{{"w":100,"h":100}}{extra}}}"#,
        ch(x),
        ch(y),
        ch(0.0),
        ch(0.0),
        ch(100.0),
        ch(100.0),
        ch(0.0),
        ch(100.0)
    )
}

fn project(layers: &[String]) -> String {
    format!(
        r#"{{"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":10,"layers":[{}],"comps":{{}},"params":{{}}}}"#,
        layers.join(",")
    )
}

fn id_of(n: &Node) -> String {
    pm_eval::node::to_js_string(n.get("id")).to_string_lossy()
}

#[test]
fn pick_finds_the_topmost_unlocked_layer() {
    let d = doc(&project(&[
        layer("top", "solid", 0.0, 0.0, r#","lock":true"#),
        layer("mid", "solid", 50.0, 50.0, ""),
        layer("audio", "audio", 0.0, 0.0, ""),
        layer("low", "solid", 0.0, 0.0, ""),
    ]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let opts = PickOptions::default();
    // (60, 60) is inside "mid" (50…150) and "low" (0…100); "top" is locked.
    let hit = scope
        .pick(60.0, 60.0, 1.0, &opts, &mut NoText, &mut NoHost)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("mid"));
    let hit = scope
        .pick(10.0, 10.0, 1.0, &opts, &mut NoText, &mut NoHost)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("low"));
    let locked = PickOptions {
        include_locked: true,
        ..PickOptions::default()
    };
    let hit = scope
        .pick(10.0, 10.0, 1.0, &locked, &mut NoText, &mut NoHost)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("top"));
    assert!(
        scope
            .pick(500.0, 300.0, 1.0, &opts, &mut NoText, &mut NoHost)
            .unwrap()
            .is_none()
    );
    // Outside the layer's time span nothing is active.
    assert!(
        scope
            .pick(10.0, 10.0, 20.0, &opts, &mut NoText, &mut NoHost)
            .unwrap()
            .is_none()
    );
}

#[test]
fn pick_skips_layers_in_locked_groups_and_follows_3d_depth() {
    let d = doc(&project(&[
        layer("g", "group", 0.0, 0.0, r#","lock":true"#),
        layer("inside", "solid", 0.0, 0.0, r#","group":"g""#),
        // A 3D layer pushed back behind a nearer one: depth order puts the
        // nearer (smaller Z) layer first, whatever the stack order.
        layer("far", "solid", 0.0, 0.0, r#","threeD":true"#).replace(
            r#""opacity""#,
            r#""position.z":{"v":400,"kf":[],"expr":null},"opacity""#,
        ),
        layer("near", "solid", 0.0, 0.0, r#","threeD":true"#),
    ]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let opts = PickOptions::default();
    let hit = scope
        .pick(10.0, 10.0, 0.0, &opts, &mut NoText, &mut NoHost)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("near"));
}

/// A geometry host without model assets or an editor camera.
#[derive(Default)]
struct ObjectHost {
    scenes: pm_eval::query3d::SceneCache,
}

impl pm_eval::query3d::Geometry3d for ObjectHost {
    fn model(&mut self, _: &str) -> pm_eval::query3d::ModelAsset {
        pm_eval::query3d::ModelAsset::Missing
    }
    fn view_camera(&self) -> Option<pm_geom3d::query::Camera> {
        None
    }
    fn scenes(&mut self) -> &mut pm_eval::query3d::SceneCache {
        &mut self.scenes
    }
}

impl LayerHost for ObjectHost {
    fn geometry(&mut self) -> Option<&mut dyn pm_eval::query3d::Geometry3d> {
        Some(self)
    }
}

#[test]
fn object_picks_raycast_the_scene() {
    // A unit box at the origin seen by the studio camera covers the canvas
    // center, not its corner.
    let p = format!(
        r#"{{"position.x":{0},"position.y":{0},"position.z":{0},"rotation.x":{0},"rotation.y":{0},"rotation":{0},"scale.x":{1},"scale.y":{1},"scale.z":{1},"opacity":{1}}}"#,
        ch(0.0),
        ch(100.0)
    );
    let object = format!(
        r#"{{"id":"model","type":"extension","from":0,"dur":10,"on":true,"threeD":true,"p":{p},"d":{{"definition":"powermove.3d.object","data":{{"object":{{"source":{{"primitive":"box"}},"material":{{"p":{{"opacity":{}}}}}}}}}}}}}"#,
        ch(1.0)
    );
    let d = doc(&project(&[object, layer("below", "solid", 0.0, 0.0, "")]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    // The synthetic layer has no name. JS `parseScene` fills the default.
    let evaluated = scope.composition_scene(0.0).unwrap();
    let Value::Object(scene) = &evaluated.scene else {
        panic!("scene")
    };
    let Some(Value::Array(objects)) = scene.get("objects") else {
        panic!("objects")
    };
    let Value::Object(first) = &objects[0] else {
        panic!("object")
    };
    assert_eq!(first.get("name"), Some(&Value::String("Object".into())));
    let opts = PickOptions::default();
    let mut host = ObjectHost::default();
    let hit = scope
        .pick(320.0, 180.0, 0.0, &opts, &mut NoText, &mut host)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("model"));
    let hit = scope
        .pick(10.0, 10.0, 0.0, &opts, &mut NoText, &mut host)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("below"));
    let err = scope
        .pick(10.0, 10.0, 0.0, &opts, &mut NoText, &mut NoHost)
        .unwrap_err();
    assert_eq!(
        err.kind, HOST_ONLY,
        "a 3D pick without geometry is never silently a miss"
    );
    // Local View hides objects outside the set from picking.
    let local = [JsValue::from("other")];
    let opts = PickOptions {
        local_view: Some(&local),
        ..PickOptions::default()
    };
    let hit = scope
        .pick(320.0, 180.0, 0.0, &opts, &mut NoText, &mut host)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), None);
    let hit = scope
        .pick(10.0, 10.0, 0.0, &opts, &mut NoText, &mut NoHost)
        .unwrap();
    assert_eq!(hit.map(id_of).as_deref(), Some("below"));
}

#[test]
fn group_compositing_boundary() {
    let known = |t: &JsString| t.as_str() == Some("fade");
    let cases = [
        ("", false),
        (r#","p":{"opacity":{"v":50,"kf":[],"expr":null}}"#, true),
        (r#","blend":"multiply""#, true),
        (r#","blend":"normal""#, false),
        (r#","mblur":true"#, true),
        (r#","matteSource":"x""#, true),
        (r#","fx":[{"id":"f","type":"blur","on":true,"p":{}}]"#, true),
        (
            r#","fx":[{"id":"f","type":"blur","on":true,"missing":true,"p":{}}]"#,
            false,
        ),
        (
            r#","fx":[{"id":"f","type":"blur","on":false,"p":{}}]"#,
            false,
        ),
        (r#","masks":[{"id":"m","on":true}]"#, true),
        (r#","masks":[{"id":"m","on":false}]"#, false),
        (r#","transitionIn":{"type":"fade","dur":1}"#, true),
        (r#","transitionIn":{"type":"unknown","dur":1}"#, false),
        (r#","transitionOut":{"type":"fade","dur":1}"#, false),
    ];
    for (extra, want) in cases {
        let mut g = format!(
            r#"{{"id":"g","type":"group","from":0,"dur":10,"on":true,"p":{{"opacity":{}}}{extra}}}"#,
            ch(100.0)
        );
        if extra.starts_with(r#","p":"#) {
            g = format!(r#"{{"id":"g","type":"group","from":0,"dur":10,"on":true{extra}}}"#);
        }
        let d = doc(&project(&[
            g,
            layer("a", "solid", 0.0, 0.0, r#","group":"g","from":0.5,"dur":3"#),
        ]));
        let comp = Comp::resolve(&d, "C").unwrap();
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let g = &comp.layers()[0];
        // The group's span is its member's (0.5…3.5); a transition in is
        // active at 1, a transition out is not.
        assert_eq!(
            scope
                .group_creates_compositing_boundary(g, 1.0, true, &known)
                .unwrap(),
            want,
            "{extra}"
        );
    }
    // Motion blur counts only when the pass renders it.
    let g = format!(
        r#"{{"id":"g","type":"group","from":0,"dur":10,"on":true,"mblur":true,"p":{{"opacity":{}}}}}"#,
        ch(100.0)
    );
    let d = doc(&project(&[g]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    assert!(
        !scope
            .group_creates_compositing_boundary(&comp.layers()[0], 1.0, false, &known)
            .unwrap()
    );
}

#[test]
fn transition_edges_use_the_group_span() {
    let g = r#"{"id":"g","type":"group","from":0,"dur":10,"on":true,"p":{},"transitionIn":{"type":"fade","dur":0},"transitionOut":{"type":"fade","dur":0.5}}"#;
    let d = doc(&project(&[
        g.to_owned(),
        layer("a", "solid", 0.0, 0.0, r#","group":"g","from":2,"dur":2"#),
    ]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let g = &comp.layers()[0];
    let known = |_: &JsString| true;
    // `Number(dur) || .5`: a zero duration means the default half second.
    let t = scope.active_transition(g, 2.25, &known).unwrap().unwrap();
    assert!(t.edge_in);
    assert_eq!(t.progress, 0.5);
    let t = scope.active_transition(g, 3.75, &known).unwrap().unwrap();
    assert!(!t.edge_in);
    assert_eq!(t.progress, 0.5);
    assert!(scope.active_transition(g, 3.0, &known).unwrap().is_none());
}

fn video(speed: &str) -> String {
    format!(
        r#"{{"id":"v","type":"video","from":0.5,"dur":20,"on":true,"p":{{}},"d":{{"trim":2,"speed":{speed}}}}}"#
    )
}

#[test]
fn engine_retiming_integrates_constant_and_linear_speed_exactly() {
    // A keyframed speed that is constant: both steps integrate it exactly.
    let constant = r#"{"v":2,"kf":[{"t":0,"v":2,"i":"a"},{"t":10,"v":2,"i":"b"}],"expr":null}"#;
    let d = doc(&project(&[video(constant)]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let l = &comp.layers()[0];
    for t in [0.0, 0.5, 0.6, 1.5, 3.25, 7.0] {
        let want = (t - 0.5f64).max(0.0) * 2.0 + 2.0;
        let engine = scope
            .source_time_with(l, t, Retiming::Engine, None)
            .unwrap();
        let legacy = scope
            .source_time_with(l, t, Retiming::Legacy, None)
            .unwrap();
        assert!(
            (engine - want).abs() < 1e-12,
            "engine {engine} want {want} at {t}"
        );
        assert!(
            (legacy - want).abs() < 1e-12,
            "legacy {legacy} want {want} at {t}"
        );
    }

    // A linear ramp from 1 to 3 over 10 s: the midpoint rule integrates a
    // linear speed exactly on any grid, so frame-aligned engine answers are
    // exact; between frames the partial step is exact too.
    let ramp = r#"{"v":1,"kf":[{"t":0,"v":1,"i":"a","eo":[0,0],"ei":[1,1]},{"t":10,"v":3,"i":"b","eo":[0,0],"ei":[1,1]}],"expr":null}"#;
    let d = doc(&project(&[video(ramp)]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let l = &comp.layers()[0];
    let mut cache = RetimeCache::new();
    for t in [0.5, 0.5 + 1.0 / 30.0, 1.0, 2.0 + 1.0 / 60.0, 5.5, 10.0] {
        let local = t - 0.5;
        let want = local + local * local / 10.0 + 2.0;
        let engine = scope
            .source_time_with(l, t, Retiming::Engine, None)
            .unwrap();
        let cached = scope
            .source_time_with(l, t, Retiming::Engine, Some(&mut cache))
            .unwrap();
        assert!(
            (engine - want).abs() < 1e-9,
            "engine {engine} want {want} at {t}"
        );
        assert_eq!(
            engine.to_bits(),
            cached.to_bits(),
            "the cache changes nothing"
        );
    }
}

#[test]
fn engine_retiming_uses_the_owning_composition_rate() {
    // An eased speed is not integrated exactly; the engine's answer depends
    // only on the owning composition's rate, the legacy one on the open
    // composition's (at least 60 Hz).
    let eased = r#"{"v":1,"kf":[{"t":0,"v":0.5,"i":"a","eo":[0.8,0],"ei":[0.2,1]},{"t":4,"v":4,"i":"b","eo":[0.8,0],"ei":[0.2,1]}],"expr":null}"#;
    let nested = format!(
        r#"{{"id":"P","compId":"C","w":640,"h":360,"fps":60,"dur":10,"layers":[],"params":{{}},"comps":{{"N":{{"id":"N","w":640,"h":360,"fps":24,"dur":10,"layers":[{}]}}}}}}"#,
        video(eased)
    );
    let d = doc(&nested);
    let open = Comp::resolve(&d, "C").unwrap();
    let inner = Comp::resolve(&d, "N").unwrap();
    let mut ev = Evaluator::new();
    let l = &inner.layers()[0];
    let t = 3.0;
    let mut scope = Scope::new(&mut ev, &inner, &open);
    let engine = scope
        .source_time_with(l, t, Retiming::Engine, None)
        .unwrap();
    let legacy = scope
        .source_time_with(l, t, Retiming::Legacy, None)
        .unwrap();
    // Reference: a fine midpoint integral.
    let mut ev2 = Evaluator::new();
    let mut s2 = Scope::new(&mut ev2, &inner, &open);
    let speed = l.get("d").unwrap().get("speed").unwrap();
    let n = 200_000;
    let local = t - 0.5;
    let mut sum = 0.0;
    for i in 0..n {
        let at = 0.5 + (i as f64 + 0.5) * local / n as f64;
        sum += s2
            .channel_p(l, Some(speed), at, "c.speed")
            .unwrap()
            .to_number()
            * local
            / n as f64;
    }
    let reference = sum + 2.0;
    assert!(
        (engine - reference).abs() < 2e-3,
        "engine {engine} reference {reference}"
    );
    assert!(
        (legacy - reference).abs() < 2e-3,
        "legacy {legacy} reference {reference}"
    );
    assert_ne!(
        engine.to_bits(),
        legacy.to_bits(),
        "24 fps steps differ from 120 Hz steps"
    );
    // Opening another composition does not move the engine's answer.
    let other_open = Comp::resolve(&d, "N").unwrap();
    let mut ev3 = Evaluator::new();
    let mut s3 = Scope::new(&mut ev3, &inner, &other_open);
    assert_eq!(
        s3.source_time_with(l, t, Retiming::Engine, None)
            .unwrap()
            .to_bits(),
        engine.to_bits()
    );
}

/// A project at `fps` with one video layer whose speed is eased and carries
/// an expression (the case the corpus lacked: its only variable speed is a
/// linear ramp, which every midpoint grid integrates exactly).
fn eased_expression_speed(fps: f64) -> String {
    let speed = r#"{"v":1,"kf":[{"t":0,"v":0.5,"i":"a","eo":[0.8,0],"ei":[0.2,1]},{"t":4,"v":4,"i":"b","eo":[0.8,0],"ei":[0.2,1]}],"expr":"value * (1 + 0.1 * Math.sin(time * 7))"}"#;
    format!(
        r#"{{"id":"P","compId":"C","w":640,"h":360,"fps":{fps},"dur":10,"layers":[{}],"comps":{{}},"params":{{}}}}"#,
        video(speed)
    )
}

#[test]
fn engine_retiming_matches_today_at_the_root() {
    // Amendment 16: two midpoint samples per frame of the owning composition,
    // at no less than 60 Hz. At the root that is today's step, so the engine
    // answers bit for bit what `retiming.ts` answers, on and off the frame
    // grid, before the layer starts, and past the eased keys.
    for fps in [30.0, 24.0, 29.97, 60.0, 120.0, 23.976, 59.94] {
        let d = doc(&eased_expression_speed(fps));
        let comp = Comp::resolve(&d, "C").unwrap();
        let mut ev = with_expressions();
        let mut scope = Scope::open(&mut ev, &comp);
        let l = &comp.layers()[0];
        let mut cache = RetimeCache::new();
        let mut checked = 0;
        for i in 0..400 {
            // Frame-aligned, off-grid, and irrational offsets over 0…8 s.
            let t = match i % 3 {
                0 => (i / 3) as f64 / fps,
                1 => i as f64 * 0.0201 + 0.0003,
                _ => i as f64 * std::f64::consts::E / 136.0,
            };
            let engine = scope
                .source_time_with(l, t, Retiming::Engine, None)
                .unwrap();
            let legacy = scope
                .source_time_with(l, t, Retiming::Legacy, None)
                .unwrap();
            let cached = scope
                .source_time_with(l, t, Retiming::Engine, Some(&mut cache))
                .unwrap();
            assert_eq!(
                engine.to_bits(),
                legacy.to_bits(),
                "{fps} fps at t = {t}: engine {engine} legacy {legacy}"
            );
            assert_eq!(
                engine.to_bits(),
                cached.to_bits(),
                "the cache changes nothing"
            );
            checked += 1;
        }
        assert_eq!(checked, 400);
    }
    // The expression takes part: without it the answers differ.
    let with = doc(&eased_expression_speed(30.0));
    let without = doc(&eased_expression_speed(30.0)
        .replace(r#""value * (1 + 0.1 * Math.sin(time * 7))""#, "null"));
    let answer = |d: &Document| {
        let comp = Comp::resolve(d, "C").unwrap();
        let mut ev = with_expressions();
        let mut scope = Scope::open(&mut ev, &comp);
        let l = &comp.layers()[0];
        scope
            .source_time_with(l, 3.3, Retiming::Engine, None)
            .unwrap()
    };
    assert_ne!(answer(&with), answer(&without));
}

#[test]
fn engine_retiming_of_a_nested_composition_is_its_own_rate() {
    // A 24 fps composition nested in a 60 fps open one integrates at its own
    // rate: the answer equals today's with the nested composition open, and
    // differs from today's with the 60 fps composition open (the one allowed
    // difference of amendment 16).
    let speed = r#"{"v":1,"kf":[{"t":0,"v":0.5,"i":"a","eo":[0.8,0],"ei":[0.2,1]},{"t":4,"v":4,"i":"b","eo":[0.8,0],"ei":[0.2,1]}],"expr":"value * (1 + 0.1 * Math.sin(time * 7))"}"#;
    let nested = format!(
        r#"{{"id":"P","compId":"C","w":640,"h":360,"fps":60,"dur":10,"layers":[],"params":{{}},"comps":{{"N":{{"id":"N","w":640,"h":360,"fps":24,"dur":10,"layers":[{}]}}}}}}"#,
        video(speed)
    );
    let d = doc(&nested);
    let open = Comp::resolve(&d, "C").unwrap();
    let inner = Comp::resolve(&d, "N").unwrap();
    let l = &inner.layers()[0];
    let mut differs = 0;
    for i in 0..200 {
        let t = 0.5 + i as f64 * 0.0373;
        let mut ev = with_expressions();
        let engine = Scope::new(&mut ev, &inner, &open)
            .source_time_with(l, t, Retiming::Engine, None)
            .unwrap();
        let mut ev = with_expressions();
        let own = Scope::open(&mut ev, &inner)
            .source_time_with(l, t, Retiming::Legacy, None)
            .unwrap();
        let mut ev = with_expressions();
        let legacy = Scope::new(&mut ev, &inner, &open)
            .source_time_with(l, t, Retiming::Legacy, None)
            .unwrap();
        assert_eq!(engine.to_bits(), own.to_bits(), "t = {t}");
        if engine.to_bits() != legacy.to_bits() {
            differs += 1;
        }
    }
    assert!(
        differs > 150,
        "60 Hz and 120 Hz steps differ: {differs} of 200"
    );
}

fn bounds_of<'d>(scope: &mut Scope<'_, 'd>, layer: &'d Node) -> Option<pm_eval::bounds::Bounds> {
    scope.bounds(layer, 0.0, &mut NoText, &mut NoHost).unwrap()
}

#[test]
fn bounds_by_type() {
    let null = r#"{"id":"n","type":"null","from":0,"dur":10,"on":true,"p":{},"d":{}}"#.to_owned();
    let image =
        r#"{"id":"i","type":"image","from":0,"dur":10,"on":true,"p":{},"d":{"w":0,"h":50}}"#
            .to_owned();
    let shape = r#"{"id":"s","type":"shape","from":0,"dur":10,"on":true,"p":{},"d":{"shape":"rect","w":100,"h":40,"stroke":6}}"#.to_owned();
    let adjustment =
        r#"{"id":"a","type":"adjustment","from":0,"dur":10,"on":true,"p":{},"d":{}}"#.to_owned();
    let sized = r#"{"id":"z","type":"solid","from":0,"dur":10,"on":true,"p":{"anchor.x":{"v":50,"kf":[],"expr":null}},"d":{"w":200,"h":100,"sizeAnchorBounds":{"x0":0,"y0":0,"x1":100,"y1":100}}}"#.to_owned();
    let d = doc(&project(&[null, image, shape, adjustment, sized]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let layers = comp.layers();
    let n = bounds_of(&mut scope, &layers[0]).unwrap();
    assert_eq!(
        (n.x0, n.y0, n.x1, n.y1, n.ax),
        (0.0, 0.0, 100.0, 100.0, 0.0)
    );
    // An image without a width takes the open composition's, centered.
    let i = bounds_of(&mut scope, &layers[1]).unwrap();
    assert_eq!(
        (i.x0, i.y0, i.w, i.h, i.ax, i.ay),
        (-320.0, -25.0, 640.0, 50.0, 320.0, 25.0)
    );
    // Shapes include half the stroke; the anchor sits at the raster center.
    let s = bounds_of(&mut scope, &layers[2]).unwrap();
    assert_eq!((s.x0, s.y0, s.x1, s.y1), (-53.0, -23.0, 53.0, 23.0));
    assert_eq!(
        (s.ax, s.ay),
        ((100.0 + 2.0 * 7.0) / 2.0, (40.0 + 2.0 * 7.0) / 2.0)
    );
    assert!(
        bounds_of(&mut scope, &layers[3]).is_none(),
        "adjustment layers have no bounds"
    );
    // A size anchor keeps the anchor at the same relative point: anchor x 50
    // of the reference 0…100 is the middle, which is 100 in 0…200.
    let z = bounds_of(&mut scope, &layers[4]).unwrap();
    assert_eq!((z.x0, z.x1, z.ax), (-50.0, 150.0, 50.0));
}

#[test]
fn scene_camera_aims_through_the_parent_space() {
    let parent = format!(
        r#"{{"id":"rig","type":"null","from":0,"dur":10,"on":true,"threeD":true,"p":{{"position.x":{},"position.y":{},"position.z":{}}},"d":{{}}}}"#,
        ch(10.0),
        ch(0.0),
        ch(0.0)
    );
    let camera = format!(
        r#"{{"id":"cam","type":"extension","from":0,"dur":10,"on":true,"threeD":true,"parent":"rig","p":{{"position.x":{},"position.y":{},"position.z":{}}},"d":{{"definition":"powermove.3d.camera","data":{{"camera":{{"projection":"perspective","p":{{"targetX":{},"targetY":{},"targetZ":{}}}}},"environment":{{"p":{{}}}}}}}}}}"#,
        ch(0.0),
        ch(1.0),
        ch(5.0),
        ch(0.0),
        ch(0.0),
        ch(0.0)
    );
    let d = doc(&project(&[camera, parent]));
    let comp = Comp::resolve(&d, "C").unwrap();
    let mut ev = Evaluator::new();
    let mut scope = Scope::open(&mut ev, &comp);
    let pose = scope.scene_camera(0.0).unwrap();
    assert_eq!(pose.position, [10.0, 1.0, 5.0]);
    assert_eq!(
        pose.target,
        [10.0, 0.0, 0.0],
        "the stored aim lives in the parent's space"
    );
    assert_eq!(pose.up, [0.0, 1.0, 0.0]);
    let m = pm_eval::scene3d::camera_matrix(&pose);
    // The camera looks down its −Z axis toward the target.
    let z = [m[8], m[9], m[10]];
    let to_eye = [0.0, 1.0, 5.0];
    let len = (1.0f64 + 25.0).sqrt();
    for i in 0..3 {
        assert!((z[i] - to_eye[i] / len).abs() < 1e-12);
    }
    let scene = scope.composition_scene(0.0).unwrap();
    assert!(scene.recipe_parts.is_empty());
    let pm_model::json::Value::Object(s) = &scene.scene else {
        unreachable!()
    };
    let lights = s
        .get_key(&JsString::from("lights"))
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(lights.len(), 1, "no light layer: the studio sun");
    assert_eq!(scope.clip_time(&comp.layers()[0], -1.0), 0.0);
}
