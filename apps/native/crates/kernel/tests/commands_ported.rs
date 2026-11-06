//! Commands ported from `origin/main`, against values worked out by hand from
//! the pinned sources. Every case goes through `execute`, undo and redo.
use std::sync::Arc;

use pm_kernel::commands::comps::PrecomposeMode;
use pm_kernel::commands::text::{SplitMode, TextCommand};
use pm_kernel::commands::transform::TransformCommand;
use pm_kernel::text_host::{SharedTextHost, TextHost};
use pm_kernel::{
    Command, CompCommand, DocumentStore, LayerCommand, MediaCommand, Origin, PropertyCommand,
};
use pm_model::json::{Value, parse};

fn layer(id: &str, kind: &str, extra: &str) -> String {
    format!(
        r##"{{"id":"{id}","type":"{kind}","name":"{id}","from":0,"dur":5,"on":true,"lock":false,
        "parent":null,"group":null,"blend":"normal","mblur":false,"fx":[],"masks":[],
        "p":{{"position.x":{{"v":100,"kf":[]}},"position.y":{{"v":50,"kf":[]}},
        "anchor.x":{{"v":0,"kf":[]}},"anchor.y":{{"v":0,"kf":[]}},
        "scale.x":{{"v":100,"kf":[]}},"scale.y":{{"v":100,"kf":[]}},
        "rotation":{{"v":0,"kf":[]}},"opacity":{{"v":100,"kf":[]}}}},
        "d":{{"w":40,"h":20,"color":"#FF0000"}}{extra}}}"##
    )
}
fn store(layers: &[String]) -> DocumentStore {
    let text = format!(
        r##"{{"id":"p","name":"Ported","compId":"root","w":200,"h":100,"fps":30,"dur":10,
        "bg":"#101010","assets":{{"a1":{{"name":"Clip","folder":"f1"}},"a2":{{"name":"Still"}}}},
        "assetFolders":{{"f1":{{"id":"f1","name":"One","parent":null}},
        "f2":{{"id":"f2","name":"Two","parent":null}}}},
        "comps":{{}},"layers":[{}]}}"##,
        layers.join(",")
    );
    DocumentStore::new_untitled(parse(&text).unwrap())
}
fn apply(s: &mut DocumentStore, command: Command) {
    let before = s.root().unwrap().to_value();
    s.execute(command, Origin::Interface)
        .unwrap()
        .expect("the command changes the document");
    let after = s.root().unwrap().to_value();
    assert_ne!(before, after);
    s.undo().unwrap();
    assert_eq!(s.root().unwrap().to_value(), before, "one Undo restores");
    s.redo().unwrap();
    assert_eq!(s.root().unwrap().to_value(), after, "Redo replays");
}
fn root(s: &DocumentStore) -> Value {
    s.root().unwrap().to_value()
}
fn host_layers(s: &DocumentStore) -> Vec<Value> {
    let v = root(s);
    let id = v.get("compId").unwrap().as_str().unwrap().to_owned();
    v.get("comps")
        .and_then(|c| c.get(&id))
        .or(Some(&v))
        .and_then(|c| c.get("layers"))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}
fn comp(s: &DocumentStore, id: &str) -> Value {
    root(s).get("comps").unwrap().get(id).unwrap().clone()
}
fn find<'a>(layers: &'a [Value], id: &str) -> &'a Value {
    layers
        .iter()
        .find(|l| text(l, "id") == id)
        .unwrap_or_else(|| panic!("no layer {id}"))
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap()
}
fn channel(l: &Value, key: &str) -> f64 {
    l.get("p").unwrap().get(key).unwrap().get("v").unwrap().as_f64().unwrap()
}

fn precompose(ids: &[&str], mode: PrecomposeMode, adjust: bool) -> Command {
    CompCommand::Precompose {
        layer_ids: ids.iter().map(|s| (*s).into()).collect(),
        new_comp_id: "C2".into(),
        precomp_layer_id: "P".into(),
        name: None,
        mode,
        adjust_duration: adjust,
        time: 0.,
    }
    .into()
}

#[test]
fn precompose_moves_layers_and_keeps_stack_slot_and_settings() {
    let mut s = store(&[layer("a", "solid", ""), layer("b", "solid", ""), layer("c", "solid", "")]);
    apply(&mut s, precompose(&["b", "a"], PrecomposeMode::MoveAll, false));
    let host = host_layers(&s);
    let ids: Vec<&str> = host.iter().map(|l| text(l, "id")).collect();
    assert_eq!(ids, ["P", "c"]);
    let precomp = find(&host, "P");
    assert_eq!(text(precomp, "type"), "precomp");
    assert_eq!(text(precomp, "name"), "Pre-comp 1");
    assert_eq!(precomp.get("d").and_then(|d| d.get("comp")).and_then(Value::as_str), Some("C2"));
    assert_eq!(num(precomp, "dur"), 10.);
    for (key, value) in [
        ("anchor.x", 100.),
        ("anchor.y", 50.),
        ("position.x", 100.),
        ("position.y", 50.),
    ] {
        assert_eq!(channel(precomp, key), value, "{key}");
    }
    let inner = comp(&s, "C2");
    assert_eq!((num(&inner, "w"), num(&inner, "h"), num(&inner, "fps")), (200., 100., 30.));
    assert_eq!(num(&inner, "dur"), 10.);
    assert_eq!(text(&inner, "bg"), "#101010");
    let moved = inner.get("layers").unwrap().as_array().unwrap();
    assert_eq!(moved.len(), 2);
    assert_eq!(text(&moved[0], "id"), "a");
    assert_eq!(text(&moved[1], "id"), "b");
}

#[test]
fn precompose_adjust_duration_shifts_moved_layers_to_zero() {
    let late = layer("a", "solid", "").replace(r#""from":0"#, r#""from":2"#);
    let mut s = store(&[late, layer("b", "solid", "")]);
    apply(&mut s, precompose(&["a"], PrecomposeMode::MoveAll, true));
    let host = host_layers(&s);
    assert_eq!(num(find(&host, "P"), "from"), 2.);
    assert_eq!(num(find(&host, "P"), "dur"), 5.);
    let inner = comp(&s, "C2");
    assert_eq!(num(&inner, "dur"), 5.);
    assert_eq!(num(&inner.get("layers").unwrap().as_array().unwrap()[0], "from"), 0.);
}

#[test]
fn precompose_splits_a_rig_across_the_boundary_and_clears_mattes() {
    let child = layer("kid", "solid", r#","matteSource":"out""#)
        .replace(r#""parent":null"#, r#""parent":"rig""#);
    let keeper = layer("keep", "solid", "").replace(r#""parent":null"#, r#""parent":"kid""#);
    let mut s = store(&[child, layer("rig", "null", ""), layer("out", "solid", ""), keeper]);
    apply(&mut s, precompose(&["kid"], PrecomposeMode::MoveAll, false));
    let inner = comp(&s, "C2");
    let moved = inner.get("layers").unwrap().as_array().unwrap();
    let kid = find(moved, "kid");
    let copy = text(kid, "parent");
    assert_ne!(copy, "rig", "the rig is a copy inside");
    assert_eq!(text(find(moved, copy), "type"), "null");
    assert!(kid.get("matteSource").unwrap().is_null(), "matte left behind");
    let host = host_layers(&s);
    let outside = text(find(&host, "keep"), "parent");
    assert_ne!(outside, "kid", "the outer child follows a copy of the moved parent");
    assert_eq!(text(find(&host, outside), "type"), "null");
    assert!(find(&host, "rig").get("parent").unwrap().is_null());
}

#[test]
fn precompose_refuses_a_partial_styled_group() {
    let at_origin = |l: String| {
        l.replace(r#""position.x":{"v":100"#, r#""position.x":{"v":0"#)
            .replace(r#""position.y":{"v":50"#, r#""position.y":{"v":0"#)
    };
    let group = at_origin(layer("g", "group", ""))
        .replace(r#""opacity":{"v":100"#, r#""opacity":{"v":40"#);
    let member = layer("m", "solid", "").replace(r#""group":null"#, r#""group":"g""#);
    let other = layer("n", "solid", "").replace(r#""group":null"#, r#""group":"g""#);
    let mut s = store(&[group, member, other]);
    s.execute(
        precompose(&["m"], PrecomposeMode::MoveAll, false),
        Origin::Interface,
    )
    .unwrap();
    // The identity group is safe: the precomp stays inside it, styled.
    let host = host_layers(&s);
    assert_eq!(text(find(&host, "P"), "group"), "g");
    let rotated = at_origin(layer("g", "group", ""))
        .replace(r#""opacity":{"v":100"#, r#""opacity":{"v":40"#)
        .replace(r#""rotation":{"v":0"#, r#""rotation":{"v":45"#);
    let mut s = store(&[
        rotated,
        layer("m", "solid", "").replace(r#""group":null"#, r#""group":"g""#),
    ]);
    let e = s
        .execute(precompose(&["m"], PrecomposeMode::MoveAll, false), Origin::Interface)
        .unwrap_err();
    assert!(e.to_string().contains("Select the whole styled group"), "{e}");
}

#[test]
fn leave_attributes_keeps_the_layer_and_moves_only_its_source() {
    let fx = r#","trim":1"#;
    let mut s = store(&[layer("a", "solid", fx)]);
    apply(&mut s, precompose(&["a"], PrecomposeMode::LeaveAttributes, false));
    let host = host_layers(&s);
    assert_eq!(host.len(), 1);
    let kept = find(&host, "a");
    assert_eq!(text(kept, "type"), "precomp");
    let d = kept.get("d").unwrap();
    assert_eq!(d.get("comp").and_then(Value::as_str), Some("C2"));
    assert_eq!((num(d, "w"), num(d, "h")), (40., 20.));
    // A solid's layer space already starts at its corner, like a pre-comp's.
    assert_eq!((channel(kept, "anchor.x"), channel(kept, "anchor.y")), (0., 0.));
    assert_eq!(channel(kept, "position.x"), 100., "the transform stays outside");
    let inner = comp(&s, "C2");
    assert_eq!((num(&inner, "w"), num(&inner, "h"), num(&inner, "dur")), (40., 20., 5.));
    let source = &inner.get("layers").unwrap().as_array().unwrap()[0];
    assert_eq!(text(source, "type"), "solid");
    assert_eq!(text(source.get("d").unwrap(), "color"), "#FF0000");
}

#[test]
fn leave_attributes_refuses_silently_for_several_layers_or_no_source() {
    let mut s = store(&[layer("a", "solid", ""), layer("g", "group", "")]);
    let revision = root(&s);
    let result = s
        .execute(
            precompose(&["a", "g"], PrecomposeMode::LeaveAttributes, false),
            Origin::Interface,
        )
        .unwrap();
    assert!(result.is_none());
    assert_eq!(root(&s), revision);
}

#[test]
fn center_anchor_moves_the_anchor_and_keeps_the_rotated_picture_still() {
    // Anchor (10, 5) on a 40x20 solid, whose bounds are (0, 0)..(40, 20): the
    // new anchor is (20, 10), so dx = 10, dy = 5. Scale 200, rotation 90 gives
    // a = 0, b = 2, c = -2, d = 0, and the position moves by
    // (a dx + c dy, b dx + d dy) = (-10, 20).
    let source = layer("a", "solid", "")
        .replace(r#""anchor.x":{"v":0"#, r#""anchor.x":{"v":10"#)
        .replace(r#""anchor.y":{"v":0"#, r#""anchor.y":{"v":5"#)
        .replace(r#""scale.x":{"v":100"#, r#""scale.x":{"v":200"#)
        .replace(r#""scale.y":{"v":100"#, r#""scale.y":{"v":200"#)
        .replace(r#""rotation":{"v":0"#, r#""rotation":{"v":90"#);
    let mut s = store(&[source, layer("lock", "solid", r#","lock":true"#)]);
    apply(
        &mut s,
        TransformCommand::CenterAnchor {
            layer_ids: vec!["a".into(), "lock".into()],
            time: 0.,
        }
        .into(),
    );
    let host = host_layers(&s);
    let a = find(&host, "a");
    let close = |got: f64, want: f64, what: &str| {
        assert!((got - want).abs() < 1e-9, "{what}: {got} != {want}");
    };
    close(channel(a, "anchor.x"), 20., "anchor x");
    close(channel(a, "anchor.y"), 10., "anchor y");
    close(channel(a, "position.x"), 90., "position x");
    close(channel(a, "position.y"), 70., "position y");
    assert_eq!(channel(find(&host, "lock"), "anchor.x"), 0., "locked layers stay");
}

#[test]
fn center_anchor_on_text_needs_fonts_without_a_host() {
    let mut s = store(&[layer("t", "text", "").replace(
        r##""d":{"w":40,"h":20,"color":"#FF0000"}"##,
        r##""d":{"text":"Hi","font":"Inter","size":20,"color":"#FFFFFF"}"##,
    )]);
    let e = s
        .execute(
            TransformCommand::CenterAnchor {
                layer_ids: vec!["t".into()],
                time: 0.,
            }
            .into(),
            Origin::Interface,
        )
        .unwrap_err();
    assert!(e.to_string().contains("needs fonts"), "{e}");
}

fn reset(channel: &str, default: Option<Value>) -> Command {
    PropertyCommand::Reset {
        layer_id: "a".into(),
        channel: channel.into(),
        default,
        time: 1.,
    }
    .into()
}

#[test]
fn reset_clears_keys_and_expression_then_writes_the_shipped_default() {
    let keyed = layer("a", "solid", "").replace(
        r#""position.x":{"v":100,"kf":[]}"#,
        r#""position.x":{"v":7,"expr":"time","kf":[{"i":"k1","t":0,"v":1,"e":"linear"},{"i":"k2","t":1,"v":2,"e":"linear"}]}"#,
    );
    let mut s = store(&[keyed]);
    apply(&mut s, reset("position.x", None));
    let host = host_layers(&s);
    let p = find(&host, "a").get("p").unwrap().get("position.x").unwrap().clone();
    assert_eq!(p.get("v").and_then(Value::as_f64), Some(100.), "half the comp width");
    assert!(p.get("kf").unwrap().as_array().unwrap().is_empty());
    assert!(!p.get("expr").is_some_and(Value::is_truthy));
}

#[test]
fn reset_uses_the_null_layer_table_and_a_caller_default() {
    let mut s = store(&[layer("a", "null", "")]);
    apply(&mut s, reset("opacity", None));
    assert_eq!(channel(find(&host_layers(&s), "a"), "opacity"), 0.);
    let mut s = store(&[layer("a", "solid", "").replace(
        r#""fx":[]"#,
        r#""fx":[{"id":"f1","type":"blur","on":true,"p":{"amount":{"v":9,"kf":[]}}}]"#,
    )]);
    apply(&mut s, reset("f1.amount", Some(Value::Number(4.))));
    let host = host_layers(&s);
    let amount = find(&host, "a").get("fx").unwrap().as_array().unwrap()[0]
        .get("p")
        .unwrap()
        .get("amount")
        .unwrap()
        .get("v")
        .unwrap()
        .as_f64();
    assert_eq!(amount, Some(4.));
}

fn folder(ids: &[&str], to: Option<&str>) -> Command {
    MediaCommand::SetAssetFolder {
        asset_ids: ids.iter().map(|s| (*s).into()).collect(),
        folder: to.map(str::to_owned),
    }
    .into()
}
fn asset_folder(s: &DocumentStore, id: &str) -> Option<String> {
    root(s)
        .get("assets")
        .and_then(|a| a.get(id))
        .and_then(|a| a.get("folder"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn set_asset_folder_moves_media_and_unknown_folders_mean_the_top_level() {
    let mut s = store(&[layer("a", "solid", "")]);
    apply(&mut s, folder(&["a1", "a2", "gone"], Some("f2")));
    assert_eq!(asset_folder(&s, "a1").as_deref(), Some("f2"));
    assert_eq!(asset_folder(&s, "a2").as_deref(), Some("f2"));
    apply(&mut s, folder(&["a1"], None));
    assert_eq!(asset_folder(&s, "a1"), None);
    apply(&mut s, folder(&["a2"], Some("missing")));
    assert_eq!(asset_folder(&s, "a2"), None);
    let same = root(&s);
    assert!(s.execute(folder(&["a2"], None), Origin::Interface).unwrap().is_none());
    assert_eq!(root(&s), same);
}

fn transform(json: &str, state: &str, selection: &[&str]) -> Command {
    LayerCommand::TransformLayers {
        transform: parse(json).unwrap(),
        state: parse(state).unwrap(),
        selection: selection.iter().map(|s| (*s).into()).collect(),
        time: 0.,
    }
    .into()
}

#[test]
fn transform_layers_staggers_in_order_with_frames_and_state() {
    let mut s = store(&[
        layer("a", "solid", ""),
        layer("b", "solid", ""),
        layer("c", "solid", r#","lock":true"#),
    ]);
    // index * state.step frames, in reverse stack order: b gets 0, a gets 1.
    apply(
        &mut s,
        transform(
            r#"{"selector":{"scope":"all"},"order":"reverseStack","edits":[
              {"path":"layer.from","value":{"op":"frames","args":[
                {"op":"multiply","args":[{"ref":"index"},{"state":"step"}]}]}},
              {"path":"properties.opacity","value":{"op":"subtract","args":[{"ref":"current"},50]}}]}"#,
            r#"{"step":3}"#,
            &[],
        ),
    );
    let host = host_layers(&s);
    assert_eq!(num(find(&host, "a"), "from"), 0.1, "3 frames at 30 fps");
    assert_eq!(num(find(&host, "b"), "from"), 0.);
    assert_eq!(num(find(&host, "c"), "from"), 0., "generated tools skip locked layers");
    assert_eq!(channel(find(&host, "a"), "opacity"), 50.);
    assert_eq!(channel(find(&host, "c"), "opacity"), 100.);
}

#[test]
fn transform_layers_aggregates_over_every_target_before_editing() {
    let late = layer("b", "solid", "").replace(r#""position.x":{"v":100"#, r#""position.x":{"v":160"#);
    let mut s = store(&[layer("a", "solid", ""), late]);
    apply(
        &mut s,
        transform(
            r#"{"edits":[{"path":"properties.position.x",
               "value":{"aggregate":"average","path":"properties.position.x"}}]}"#,
            "{}",
            &["a", "b"],
        ),
    );
    let host = host_layers(&s);
    assert_eq!(channel(find(&host, "a"), "position.x"), 130.);
    assert_eq!(channel(find(&host, "b"), "position.x"), 130.);
}

#[test]
fn transform_layers_reports_main_refusals_and_changes_nothing() {
    let mut s = store(&[layer("a", "solid", "")]);
    let refused = |s: &mut DocumentStore, c: Command| {
        s.execute(c, Origin::Interface).unwrap_err().to_string()
    };
    assert_eq!(
        refused(&mut s, transform(r#"{"edits":[{"path":"layer.id","value":1}]}"#, "{}", &["a"])),
        "The generated transformation is invalid"
    );
    assert_eq!(
        refused(&mut s, transform(r#"{"edits":[{"path":"layer.name","value":"x"}]}"#, "{}", &[])),
        "Select at least one unlocked layer"
    );
    assert_eq!(
        refused(
            &mut s,
            transform(r#"{"edits":[{"path":"layer.from","value":{"ref":"current"}}]}"#, "{}", &["a"])
        ),
        "This transformation would not change the selected source"
    );
}

fn text_layer(id: &str, text: &str) -> String {
    layer(id, "text", "").replace(
        r##""d":{"w":40,"h":20,"color":"#FF0000"}"##,
        &format!(
            r##""d":{{"text":"{text}","font":"Inter","size":20,"tracking":0,"leading":1.2,"align":"left","color":"#FFFFFF","animators":[]}}"##
        ),
    )
}

#[test]
fn animators_and_styles_append_and_remove_as_structured_content() {
    let mut s = store(&[text_layer("t", "Hello")]);
    let animator = parse(r#"{"name":"Fade","enabled":true,"mode":"stagger","p":{}}"#).unwrap();
    apply(
        &mut s,
        TextCommand::AddAnimator {
            layer_id: "t".into(),
            animator_id: "ta1".into(),
            animator,
        }
        .into(),
    );
    let host = host_layers(&s);
    let d = find(&host, "t").get("d").unwrap().clone();
    let list = d.get("animators").unwrap().as_array().unwrap();
    assert_eq!((text(&list[0], "id"), text(&list[0], "name")), ("ta1", "Fade"));
    apply(
        &mut s,
        TextCommand::RemoveAnimator {
            layer_id: "t".into(),
            animator_id: "ta1".into(),
        }
        .into(),
    );
    let host = host_layers(&s);
    assert!(find(&host, "t").get("d").unwrap().get("animators").unwrap().as_array().unwrap().is_empty());
    let e = s
        .execute(
            TextCommand::RemoveAnimator {
                layer_id: "t".into(),
                animator_id: "ta1".into(),
            }
            .into(),
            Origin::Interface,
        )
        .unwrap_err();
    assert!(e.to_string().contains("Text animator not found"), "{e}");
    apply(
        &mut s,
        TextCommand::SetStyleRange {
            layer_id: "t".into(),
            style_id: "ts1".into(),
            style: Some(parse(r#"{"start":0,"end":2,"p":{}}"#).unwrap()),
        }
        .into(),
    );
    apply(
        &mut s,
        TextCommand::SetStyleRange {
            layer_id: "t".into(),
            style_id: "ts1".into(),
            style: None,
        }
        .into(),
    );
}

/// Ten units per character, one line per paragraph.
struct Mono;
struct MonoContext;
impl pm_text::compat::Measure for MonoContext {
    fn width(&mut self, text: &[u16]) -> f64 {
        text.len() as f64 * 10.
    }
}
impl pm_eval::text_geom::TextMeasure for Mono {
    fn measure_lines(
        &mut self,
        d: &pm_text::compat::TextContent,
    ) -> Vec<pm_eval::text_geom::MeasuredLine> {
        pm_text::compat::text_source_lines(d, &mut MonoContext, d.line_height())
            .into_iter()
            .map(|line| {
                let width = line.text.len() as f64 * 10.;
                let ink = [0., -15., width, 5.];
                pm_eval::text_geom::MeasuredLine {
                    ink,
                    anchor_ink: ink,
                    text: String::from_utf16_lossy(&line.text),
                    width,
                }
            })
            .collect()
    }
    fn plain_width(&mut self, _: &pm_text::compat::TextContent, text: &str) -> f64 {
        text.encode_utf16().count() as f64 * 10.
    }
    fn glyph_layout(&mut self, d: &pm_text::compat::TextContent) -> pm_text::compat::GlyphLayout {
        pm_text::compat::glyph_layout(d, &mut MonoContext)
    }
    fn spaced_width(&mut self, _: &pm_text::compat::TextContent, text: &str) -> f64 {
        text.encode_utf16().count() as f64 * 10.
    }
}
struct MonoHost;
impl TextHost for MonoHost {
    fn measure(&self) -> Box<dyn pm_eval::text_geom::TextMeasure> {
        Box::new(Mono)
    }
    fn take_failure(&self) -> Option<String> {
        None
    }
}

#[test]
fn split_text_makes_grouped_pieces_beside_the_hidden_source() {
    let mut s = store(&[text_layer("t", "Hi yo"), layer("under", "solid", "")]);
    s.set_runtime_state(
        pm_kernel::state::StateSnapshot::seeded(),
        Some(SharedTextHost(Arc::new(MonoHost))),
    );
    apply(
        &mut s,
        TextCommand::Split {
            layer_ids: vec!["t".into()],
            group_ids: vec!["G".into()],
            mode: SplitMode::Words,
            time: 0.,
        }
        .into(),
    );
    let host = host_layers(&s);
    let group = find(&host, "G");
    assert_eq!(text(group, "type"), "group");
    assert_eq!(text(group, "name"), "t");
    let members: Vec<&Value> = host.iter().filter(|l| text(l, "group") == "G").collect();
    assert_eq!(members.len(), 3, "two words and the source");
    let source = find(&host, "t");
    assert_eq!(source.get("on").and_then(|on| on.get("v")), Some(&Value::Bool(false)));
    let first = members[0];
    let second = members[1];
    assert_eq!(first.get("d").unwrap().get("text").and_then(Value::as_str), Some("Hi"));
    assert_eq!(second.get("d").unwrap().get("text").and_then(Value::as_str), Some("yo"));
    assert_eq!(text(first, "name"), "t · 1 · Hi");
    // "Hi " is 30 units wide; the second piece's anchor moves left by that.
    assert_eq!(channel(first, "anchor.x") - channel(second, "anchor.x"), 30.);
    assert_eq!(text(host.last().unwrap(), "id"), "under", "the stack below is untouched");
}

#[test]
fn split_text_refuses_without_fonts_and_non_text_layers() {
    let mut s = store(&[text_layer("t", "Hi"), layer("a", "solid", "")]);
    let split = |id: &str| {
        Command::from(TextCommand::Split {
            layer_ids: vec![id.into()],
            group_ids: vec!["G".into()],
            mode: SplitMode::Characters,
            time: 0.,
        })
    };
    let e = s.execute(split("t"), Origin::Interface).unwrap_err();
    assert!(e.to_string().contains("needs fonts"), "{e}");
    let e = s.execute(split("a"), Origin::Interface).unwrap_err();
    assert!(e.to_string().contains("Not a text layer"), "{e}");
}
