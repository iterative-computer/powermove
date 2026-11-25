//! Every grader on synthetic passing and failing inputs.
use pm_aero_eval::artifacts::*;
use pm_aero_eval::graders::{Context, GraderSpec};
use pm_aero_eval::image::{self, Rgba};
use pm_aero_eval::verdict::Status;
use serde_json::{Value, json};
use std::path::Path;

fn spec(v: Value) -> GraderSpec {
    serde_json::from_value(v).expect("grader spec")
}

fn grade(g: &GraderSpec, a: &Artifacts, dir: &Path) -> pm_aero_eval::verdict::Verdict {
    let data = pm_aero_eval::data_root();
    g.grade(&Context {
        artifacts: a,
        run_dir: dir,
        data_root: &data,
    })
}

fn layer(id: &str, name: &str, ty: &str, from: f64, dur: f64) -> Value {
    json!({"id": id, "name": name, "type": ty, "from": from, "dur": dur, "on": true, "fx": [], "d": {}})
}

fn interview(layers: Vec<Value>) -> Value {
    json!({"proj": {"id": "interview", "layers": layers, "markers": []}})
}

fn takes() -> Vec<Value> {
    vec![
        layer("t1", "Take 1", "video", 0.0, 2.0),
        layer("t2", "Take 2", "video", 2.0, 2.0),
        layer("t3", "Take 3", "video", 4.0, 2.0),
        layer("b", "B-roll", "video", 6.0, 2.0),
        layer("lt", "Lower third", "text", 0.5, 4.0),
    ]
}

fn cut_spec() -> GraderSpec {
    spec(json!({"grader": "project", "assertions": [
        {"name": "flub gone", "count": {"select": {"name": "Take 2"}, "expect": {"eq": 0}}},
        {"name": "order", "order": {"select": {"type": "video"}, "names": ["Take 1", "B-roll", "Take 3"]}},
        {"name": "no gaps", "contiguous": {"select": {"type": "video"}, "start": 0}},
        {"name": "lower third", "unchanged": {"select": {"name": "Lower third"}}}
    ]}))
}

#[test]
fn project_assertions_pass_on_a_correct_cut() {
    let after = interview(vec![
        layer("t1", "Take 1", "video", 0.0, 2.0),
        layer("b", "B-roll", "video", 2.0, 2.0),
        layer("t3", "Take 3", "video", 4.0, 2.0),
        layer("lt", "Lower third", "text", 0.5, 4.0),
    ]);
    let a = Artifacts {
        project_before: Some(interview(takes())),
        project_after: Some(after),
        ..Default::default()
    };
    let v = grade(&cut_spec(), &a, Path::new("/"));
    assert_eq!(v.status, Status::Pass, "{:#?}", v.evidence);
    assert_eq!(v.evidence.len(), 4);
}

#[test]
fn project_assertions_fail_with_evidence_on_a_wrong_cut() {
    // Take 2 removed but the gap left open, and the lower third moved.
    let mut lt = layer("lt", "Lower third", "text", 1.0, 4.0);
    lt["name"] = json!("Lower third");
    let after = interview(vec![
        layer("t1", "Take 1", "video", 0.0, 2.0),
        layer("t3", "Take 3", "video", 4.0, 2.0),
        layer("b", "B-roll", "video", 6.0, 2.0),
        lt,
    ]);
    let a = Artifacts {
        project_before: Some(interview(takes())),
        project_after: Some(after),
        ..Default::default()
    };
    let v = grade(&cut_spec(), &a, Path::new("/"));
    assert_eq!(v.status, Status::Fail);
    let ev = v.evidence.join("\n");
    assert!(ev.contains("ok: flub gone"), "{ev}");
    assert!(
        ev.contains("FAIL: order: order [\"Take 1\", \"Take 3\", \"B-roll\"]"),
        "{ev}"
    );
    assert!(ev.contains("Take 1 gap before Take 3 by 2.000s"), "{ev}");
    assert!(
        ev.contains("FAIL: lower third: Lower third changed"),
        "{ev}"
    );
}

#[test]
fn project_grader_without_the_saved_project_is_unavailable() {
    let v = grade(&cut_spec(), &Artifacts::default(), Path::new("/"));
    assert_eq!(v.status, Status::Unavailable);
}

fn with_effect(ty: &str, amount: Value) -> Value {
    let mut l = layer("bg", "Background plate", "image", 0.0, 6.0);
    l["fx"] = json!([{"id": "f1", "type": ty, "on": true, "p": {"amount": amount}}]);
    interview(vec![l])
}

#[test]
fn effect_parameters_read_channels_and_new_effects_resolve() {
    let g = spec(json!({"grader": "project", "assertions": [
        {"name": "half vignette", "effect": {"select": {"name": "Background plate"}, "type": "vignette",
          "params": {"amount": {"between": [35, 65]}}}}]}));
    let mut a = Artifacts {
        project_after: Some(with_effect(
            "vignette",
            json!({"v": 50, "kf": [], "expr": null}),
        )),
        ..Default::default()
    };
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Pass);
    a.project_after = Some(with_effect(
        "vignette",
        json!({"v": 90, "kf": [], "expr": null}),
    ));
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Fail);

    // $new:effect matches only an effect registered by a changed extension.
    let g = spec(json!({"grader": "project", "assertions": [
        {"name": "new effect", "effect": {"select": {"name": "Background plate"}, "type": "$new:effect"}}]}));
    a.project_after = Some(with_effect("halftone.effect", json!(4)));
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Fail);
    a.extensions.changed = vec![ChangedExtension {
        id: "halftone".into(),
        action: "created".into(),
    }];
    a.extensions.registered = vec![Registration {
        kind: "effect".into(),
        id: "halftone.effect".into(),
        extension: "halftone".into(),
        params: 1,
    }];
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Pass);
}

#[test]
fn animated_transition_root_and_any_of_checks() {
    let mut bg = layer("bg", "Background plate", "image", 0.0, 6.0);
    bg["fx"] = json!([{"id": "b", "type": "blur", "on": true, "p": {"amount": {"v": 0, "expr": null,
        "kf": [{"t": 1.0, "v": 24}, {"t": 0.0, "v": 0}]}}}]);
    let mut b = layer("b", "Shot B", "image", 3.0, 3.0);
    b["transitionIn"] = json!({"type": "push.transition", "dur": 0.5, "p": {}});
    let mut after = interview(vec![bg, layer("a", "Shot A", "image", 0.0, 3.0), b]);
    after["proj"]["markers"] = json!([{"t": 0, "name": "Tap"}]);
    let a = Artifacts {
        project_after: Some(after),
        ..Default::default()
    };
    let g = spec(json!({"grader": "project", "assertions": [
        {"name": "blur builds", "animated": {"select": {"name": "Background plate"}, "effect": "blur", "param": "amount",
            "first": {"t": {"le": 0.15}, "v": {"le": 2}}, "last": {"t": {"between": [0.6, 1.6]}, "v": {"ge": 8}}}},
        {"name": "push on the cut", "transition": {"select": {"type": "image"}, "type": "push.transition", "dur": {"approx": 0.5, "tol": 0.05}}},
        {"name": "marker", "root": {"path": "/markers", "expect": {"len_ge": 1}}},
        {"name": "slides", "any-of": [
            {"animated": {"select": {"name": "Shot A"}, "param": "position.x"}},
            {"animated": {"select": {"name": "Background plate"}, "effect": "blur", "param": "amount"}}]}
    ]}));
    let v = grade(&g, &a, Path::new("/"));
    assert_eq!(v.status, Status::Pass, "{:#?}", v.evidence);
    // A transition from an extension that is not loaded (`missing`) does not count.
    let mut a2 = a.clone();
    a2.project_after.as_mut().unwrap()["proj"]["layers"][2]["transitionIn"]["missing"] =
        json!(true);
    let g2 = spec(json!({"grader": "project", "assertions": [
        {"name": "push", "transition": {"select": {"type": "image"}, "type": "push.transition"}}]}));
    assert_eq!(grade(&g2, &a2, Path::new("/")).status, Status::Fail);
}

fn health(state: &str) -> ExtensionReport {
    ExtensionReport {
        changed: vec![ChangedExtension {
            id: "softglow".into(),
            action: "updated".into(),
        }],
        windows: vec!["headless".into()],
        health: vec![ExtensionHealth {
            id: "softglow".into(),
            window: "headless".into(),
            state: state.into(),
            error: (state != "ok")
                .then(|| "0:14: 'luma' : no matching overloaded function found".into()),
        }],
        registered: vec![Registration {
            kind: "effect".into(),
            id: "softglow.effect".into(),
            extension: "softglow".into(),
            params: 3,
        }],
    }
}

#[test]
fn extensions_load_in_every_window() {
    let g = spec(
        json!({"grader": "extensions-load", "must_change": ["softglow"],
        "require": [{"kind": "effect", "id": "softglow.effect", "params_ge": 3}]}),
    );
    let mut a = Artifacts {
        extensions: health("ok"),
        ..Default::default()
    };
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Pass);

    a.extensions = health("error");
    let v = grade(&g, &a, Path::new("/"));
    assert_eq!(v.status, Status::Fail);
    assert!(
        v.evidence.iter().any(|e| e.contains("luma")),
        "{:?}",
        v.evidence
    );

    // A second window with no report: not evidence of loading.
    a.extensions = health("ok");
    a.extensions.windows.push("window-2".into());
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Unavailable);

    // Nothing changed: the fix was never made.
    a.extensions = health("ok");
    a.extensions.changed.clear();
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Fail);

    // $new needs a registration owned by a changed extension.
    let g = spec(
        json!({"grader": "extensions-load", "require": [{"kind": "transition", "id": "$new"}]}),
    );
    a.extensions = health("ok");
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Fail);
}

fn write(dir: &Path, name: &str, img: &Rgba) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("frames")).unwrap();
    let rel = Path::new("frames").join(name);
    std::fs::write(dir.join(&rel), img.encode_png()).unwrap();
    rel
}

#[test]
fn frames_compare_within_a_perceptual_tolerance() {
    let t = tempfile::tempdir().unwrap();
    let g = spec(
        json!({"grader": "frames", "times": [1.0], "reference": {"solid": [120, 120, 120]},
        "tolerance": {"mean": 2.0, "p95": 5.0}}),
    );
    // One level off (a codec round trip) passes.
    let mut a = Artifacts {
        frames: vec![Frame {
            time: 1.0,
            png: write(t.path(), "a.png", &Rgba::solid(64, 36, [121, 120, 119])),
        }],
        ..Default::default()
    };
    let v = grade(&g, &a, t.path());
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    // A visible shift fails.
    a.frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "b.png", &Rgba::solid(64, 36, [140, 120, 100])),
    }];
    let v = grade(&g, &a, t.path());
    assert_eq!(v.status, Status::Fail);
    assert!(v.evidence[0].contains("ΔE2000 mean"), "{:?}", v.evidence);
    // A small defect over 10 % of the frame fails on p95 even when the mean is low.
    let mut img = Rgba::solid(100, 100, [120, 120, 120]);
    for y in 0..10 {
        for x in 0..100 {
            img.set(x, y, [255, 0, 255, 255]);
        }
    }
    a.frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "c.png", &img),
    }];
    assert_eq!(grade(&g, &a, t.path()).status, Status::Fail);
    // No frame at that time: unavailable, never a pass.
    a.frames.clear();
    assert_eq!(grade(&g, &a, t.path()).status, Status::Unavailable);
    // Reference file missing: unavailable with the reason.
    let g = spec(
        json!({"grader": "frames", "times": [1.0], "reference": {"file": "references/none/x.png"}}),
    );
    a.frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "d.png", &Rgba::solid(64, 36, [0, 0, 0])),
    }];
    let v = grade(&g, &a, t.path());
    assert_eq!(v.status, Status::Unavailable);
    assert!(v.evidence[0].contains("render references first"));
}

#[test]
fn the_committed_grade_reference_matches_itself_and_not_the_cast_plate() {
    let t = tempfile::tempdir().unwrap();
    let data = pm_aero_eval::data_root();
    let chart =
        Rgba::decode_png(&std::fs::read(data.join("references/grade-match/chart.png")).unwrap())
            .unwrap();
    assert_eq!((chart.width, chart.height), (1280, 720));
    let g = spec(
        json!({"grader": "frames", "times": [1.0], "reference": {"file": "references/grade-match/chart.png"},
        "tolerance": {"mean": 4.0, "p95": 9.0}}),
    );
    let a = Artifacts {
        frames: vec![Frame {
            time: 1.0,
            png: write(t.path(), "ok.png", &chart),
        }],
        ..Default::default()
    };
    assert_eq!(grade(&g, &a, t.path()).status, Status::Pass);
    // The ungraded plate (lifted, cool) must not pass.
    let doc = std::fs::read(data.join("fixtures/projects/grade-match.pmv")).unwrap();
    let cast = embedded_png(&doc, "cast");
    let a = Artifacts {
        frames: vec![Frame {
            time: 1.0,
            png: write(t.path(), "cast.png", &cast),
        }],
        ..Default::default()
    };
    let v = grade(&g, &a, t.path());
    assert_eq!(v.status, Status::Fail, "{:?}", v.evidence);
}

/// Pull an embedded PNG out of a PMV4 fixture by media id.
fn embedded_png(bytes: &[u8], id: &str) -> Rgba {
    let f = &bytes[bytes.len() - 64..];
    let at = f64::from_le_bytes(f[8..16].try_into().unwrap()) as usize;
    let len = u32::from_le_bytes(f[16..20].try_into().unwrap()) as usize;
    let index: Value = serde_json::from_slice(&bytes[at..at + len]).unwrap();
    let m = index["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap();
    let (o, l) = (
        m["offset"].as_u64().unwrap() as usize,
        m["length"].as_u64().unwrap() as usize,
    );
    Rgba::decode_png(&bytes[o..o + l]).unwrap()
}

fn split(left: [u8; 3], right: [u8; 3], at: u32) -> Rgba {
    let mut img = Rgba::solid(100, 50, right);
    for y in 0..50 {
        for x in 0..at {
            img.set(x, y, [left[0], left[1], left[2], 255]);
        }
    }
    img
}

#[test]
fn frame_stats_tell_a_push_from_a_dissolve() {
    let (dawn, dusk) = ([232, 116, 59], [31, 138, 138]);
    let g = spec(
        json!({"grader": "frame-stats", "times": [3.0, 3.1], "quantifier": "any", "checks": [
            {"stat": {"near-any": {"colors": [dawn, dusk], "tol": 8}}, "expect": {"ge": 0.9}},
            {"stat": {"near": {"rgb": dawn, "tol": 8}}, "expect": {"between": [0.15, 0.85]}},
            {"stat": {"near": {"rgb": dawn, "tol": 8}}, "region": {"x": 0, "y": 0, "w": 0.08, "h": 1}, "expect": {"ge": 0.9}},
            {"stat": {"near": {"rgb": dusk, "tol": 8}}, "region": {"x": 0.92, "y": 0, "w": 0.08, "h": 1}, "expect": {"ge": 0.9}}
        ]}),
    );
    let t = tempfile::tempdir().unwrap();
    // Mid-push: dawn on the left, dusk on the right, at 3.1 s only.
    let a = Artifacts {
        frames: vec![
            Frame {
                time: 3.0,
                png: write(t.path(), "p0.png", &Rgba::solid(100, 50, dawn)),
            },
            Frame {
                time: 3.1,
                png: write(t.path(), "p1.png", &split(dawn, dusk, 40)),
            },
        ],
        ..Default::default()
    };
    let v = grade(&g, &a, t.path());
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    assert!(v.evidence[0].contains("at 3.1s"));
    // A dissolve: every pixel a blend of the two, so nothing is near either.
    let blend = [131u8, 127, 98];
    let a = Artifacts {
        frames: vec![
            Frame {
                time: 3.0,
                png: write(t.path(), "d0.png", &Rgba::solid(100, 50, dawn)),
            },
            Frame {
                time: 3.1,
                png: write(t.path(), "d1.png", &Rgba::solid(100, 50, blend)),
            },
        ],
        ..Default::default()
    };
    assert_eq!(grade(&g, &a, t.path()).status, Status::Fail);
}

#[test]
fn frame_stats_relative_to_the_baseline() {
    let t = tempfile::tempdir().unwrap();
    let base = split([40, 40, 40], [200, 200, 200], 50);
    let warmer = split([60, 42, 30], [220, 200, 185], 50);
    let negative = split([215, 215, 215], [55, 55, 55], 50);
    let g = spec(json!({"grader": "frame-stats", "times": [1.0], "checks": [
        {"stat": "warmth", "relative": true, "expect": {"between": [0.02, 0.15]}},
        {"stat": "p10-l", "relative": true, "expect": {"ge": 1.5}},
        {"stat": "correlation-baseline", "expect": {"ge": 0.9}}]}));
    let mut a = Artifacts {
        baseline_frames: vec![Frame {
            time: 1.0,
            png: write(t.path(), "base.png", &base),
        }],
        frames: vec![Frame {
            time: 1.0,
            png: write(t.path(), "warm.png", &warmer),
        }],
        ..Default::default()
    };
    let v = grade(&g, &a, t.path());
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    // Unchanged: no warmth gained.
    a.frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "same.png", &base),
    }];
    assert_eq!(grade(&g, &a, t.path()).status, Status::Fail);
    // The mono fix: tones reversed relative to a negative baseline, no colour.
    let g = spec(json!({"grader": "frame-stats", "times": [1.0], "checks": [
        {"stat": "chroma", "expect": {"le": 3}}, {"stat": "correlation-baseline", "expect": {"le": -0.85}}]}));
    a.baseline_frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "neg.png", &negative),
    }];
    a.frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "pos.png", &base),
    }];
    assert_eq!(grade(&g, &a, t.path()).status, Status::Pass);
    a.frames = vec![Frame {
        time: 1.0,
        png: write(t.path(), "neg2.png", &negative),
    }];
    assert_eq!(grade(&g, &a, t.path()).status, Status::Fail);
    // Relative checks without a baseline are unavailable, not passes.
    a.baseline_frames.clear();
    let g = spec(json!({"grader": "frame-stats", "times": [1.0], "checks": [
        {"stat": "warmth", "relative": true, "expect": {"ge": 0}}]}));
    assert_eq!(grade(&g, &a, t.path()).status, Status::Unavailable);
}

fn timed(ms: Vec<f64>) -> Artifacts {
    let mut a = Artifacts {
        frame_times: vec![FrameTiming {
            target: "split.effect".into(),
            ms,
            method: "test".into(),
        }],
        ..Default::default()
    };
    a.extensions.changed = vec![ChangedExtension {
        id: "split".into(),
        action: "created".into(),
    }];
    a.extensions.registered = vec![Registration {
        kind: "effect".into(),
        id: "split.effect".into(),
        extension: "split".into(),
        params: 1,
    }];
    a
}

#[test]
fn frame_time_under_its_budget() {
    let g = spec(json!({"grader": "frame-time", "target": "$new:effect", "budget_ms": 4.0}));
    let v = grade(
        &g,
        &timed(vec![1.0, 1.2, 1.1, 9.0, 1.3, 1.2, 1.1]),
        Path::new("/"),
    );
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    assert_eq!(v.score, Some(1.2));
    assert_eq!(
        grade(&g, &timed(vec![5.0; 7]), Path::new("/")).status,
        Status::Fail
    );
    assert_eq!(
        grade(&g, &timed(vec![1.0; 3]), Path::new("/")).status,
        Status::Unavailable
    );
    let mut a = timed(vec![1.0; 7]);
    a.extensions.registered.clear();
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Fail);
}

#[test]
fn panels_are_judged_on_what_the_probe_observed() {
    let g = spec(
        json!({"grader": "panel", "panel": "$new:panel", "probes": [{"contains": ["Intro", "Drop"],
        "select": ["Clip"], "steps": [{"click": "Drop"}],
        "playhead": {"approx": 2.5, "tol": 0.04},
        "after": [{"name": "half speed", "field": {"select": {"name": "Clip"}, "path": "/d/speed", "expect": {"approx": 0.5}}}]}]}),
    );
    let mut clip = layer("c", "Clip", "video", 0.0, 2.0);
    let before = clip.clone();
    clip["d"] = json!({"speed": 0.5});
    let mut a = Artifacts::default();
    a.extensions.changed = vec![ChangedExtension {
        id: "markers".into(),
        action: "created".into(),
    }];
    a.extensions.registered = vec![Registration {
        kind: "panel".into(),
        id: "markers.panel".into(),
        extension: "markers".into(),
        params: 0,
    }];
    a.panels = vec![PanelObservation {
        panel: "markers.panel".into(),
        opened: true,
        probes: vec![ProbeObservation {
            text: vec!["Intro 0:00".into(), "Drop 0:02.5".into()],
            steps: vec!["ok".into()],
            playhead: Some(2.5),
            project_before: Some(interview(vec![before])),
            project_after: Some(interview(vec![clip])),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let v = grade(&g, &a, Path::new("/"));
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    a.panels[0].probes[0].steps = vec!["no control labelled \"Drop\"".into()];
    a.panels[0].probes[0].playhead = Some(0.0);
    let v = grade(&g, &a, Path::new("/"));
    assert_eq!(v.status, Status::Fail);
    assert!(v.evidence.iter().any(|e| e.contains("playhead 0.000s")));
    a.panels.clear();
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Unavailable);
}

fn theme(tokens: &[(&str, &str)]) -> Artifacts {
    let mut a = Artifacts {
        theme: Some(ThemeObservation {
            active: "studio.theme".into(),
            extension: Some("studio".into()),
            tokens: tokens
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }),
        ..Default::default()
    };
    a.extensions.changed = vec![ChangedExtension {
        id: "studio".into(),
        action: "created".into(),
    }];
    a.extensions.registered = vec![Registration {
        kind: "theme".into(),
        id: "studio.theme".into(),
        extension: "studio".into(),
        params: 0,
    }];
    a
}

#[test]
fn themes_meet_hue_lightness_and_contrast() {
    let g = spec(json!({"grader": "theme",
        "tokens": [{"token": "--pm-accent", "hue_between": [45, 95], "chroma_ge": 40}, {"token": "--pm-ground", "l_le": 22}],
        "contrast": [{"fg": "--pm-text", "bg": "--pm-ground", "ge": 7}]}));
    let amber = theme(&[
        ("--pm-accent", "#ffb000"),
        ("--pm-ground", "rgb(28, 24, 20)"),
        ("--pm-text", "#f4ede4"),
    ]);
    let v = grade(&g, &amber, Path::new("/"));
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    let blue = theme(&[
        ("--pm-accent", "#3b82f6"),
        ("--pm-ground", "#1c1814"),
        ("--pm-text", "#5a5048"),
    ]);
    let v = grade(&g, &blue, Path::new("/"));
    assert_eq!(v.status, Status::Fail);
    assert_eq!(
        v.evidence.iter().filter(|e| e.starts_with("FAIL")).count(),
        2,
        "{:?}",
        v.evidence
    );
    let unreadable = theme(&[
        ("--pm-accent", "var(--x)"),
        ("--pm-ground", "#1c1814"),
        ("--pm-text", "#fff"),
    ]);
    assert_eq!(grade(&g, &unreadable, Path::new("/")).status, Status::Fail);
    assert_eq!(
        grade(&g, &Artifacts::default(), Path::new("/")).status,
        Status::Unavailable
    );
}

fn evaluation(scores: &[(&str, f64, &str)]) -> Evaluation {
    Evaluation {
        evaluator: "fresh session".into(),
        rubric: "panel-native".into(),
        scores: scores
            .iter()
            .map(|(k, s, e)| {
                (
                    k.to_string(),
                    CriterionScore {
                        score: *s,
                        evidence: e.to_string(),
                    },
                )
            })
            .collect(),
    }
}

#[test]
fn rubric_scores_and_the_owner_spot_check() {
    let g = spec(json!({"grader": "rubric", "rubric": "rubrics/panel-native.json"}));
    let good = evaluation(&[
        ("works", 2.0, "clicking Drop moved the playhead"),
        ("native", 1.0, "close"),
        ("clarity", 2.0, "plain"),
        ("restraint", 1.0, "a count badge"),
    ]);
    let mut a = Artifacts {
        evaluation: Some(good.clone()),
        ..Default::default()
    };
    let v = grade(&g, &a, Path::new("/"));
    assert_eq!(v.status, Status::Pass, "{:?}", v.evidence);
    assert_eq!(v.score, Some((2.0 * 2.0 + 1.0 + 2.0 + 1.0) / 5.0));
    // A hard criterion below its floor fails whatever the mean.
    a.evaluation = Some(evaluation(&[
        ("works", 0.0, "nothing happens"),
        ("native", 2.0, "x"),
        ("clarity", 2.0, "x"),
        ("restraint", 2.0, "x"),
    ]));
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Fail);
    // Scores without evidence are not a verdict.
    a.evaluation = Some(evaluation(&[
        ("works", 2.0, ""),
        ("native", 2.0, "x"),
        ("clarity", 2.0, "x"),
        ("restraint", 2.0, "x"),
    ]));
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Unavailable);
    // The owner's spot-check decides, and a disagreement is recorded.
    a.evaluation = Some(good);
    a.spot_check = Some(SpotCheck {
        by: "owner".into(),
        pass: false,
        note: "looks web".into(),
        needs_fix: Some(true),
    });
    let v = grade(&g, &a, Path::new("/"));
    assert_eq!(v.status, Status::Fail);
    assert!(v.evidence.iter().any(|e| e.starts_with("DISAGREEMENT")));
    a.evaluation = None;
    a.spot_check = None;
    assert_eq!(grade(&g, &a, Path::new("/")).status, Status::Unavailable);
}

#[test]
fn ciede2000_matches_published_test_pairs() {
    // Sharma, Wu and Dalal (2005), pairs 1, 2, 7, 17 and 25.
    for (a, b, want) in [
        ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
        ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
        ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
        ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        (
            [60.2574, -34.0099, 36.2677],
            [60.4626, -34.1751, 39.4387],
            1.2644,
        ),
    ] {
        let got = image::delta_e2000(a, b);
        assert!((got - want).abs() < 1e-4, "{a:?} {b:?}: {got} vs {want}");
    }
    let white = image::lab([255, 255, 255]);
    assert!((white[0] - 100.0).abs() < 0.01 && white[1].abs() < 0.01 && white[2].abs() < 0.01);
    assert!((image::contrast_ratio([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
}

#[test]
fn pmv_reader_reads_fixtures_and_refuses_tampering() {
    let path = pm_aero_eval::data_root().join("fixtures/projects/interview.pmv");
    let mut bytes = std::fs::read(path).unwrap();
    let doc = pm_aero_eval::pmv::read_document(&bytes).unwrap();
    let p = pm_aero_eval::pmv::project_of(&doc);
    assert_eq!(p["id"], "interview");
    assert_eq!(p["layers"].as_array().unwrap().len(), 6);
    bytes[10] ^= 1;
    assert!(
        pm_aero_eval::pmv::read_document(&bytes)
            .unwrap_err()
            .contains("checksum")
    );
    assert_eq!(
        pm_aero_eval::pmv::read_document(b"{\"id\":\"p\"}").unwrap()["id"],
        "p"
    );
}
