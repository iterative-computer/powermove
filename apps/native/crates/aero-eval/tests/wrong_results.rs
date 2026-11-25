//! Every task's shipped graders against a correct result and the cheapest
//! wrong results: the correct one passes, each wrong one fails. Wrong results
//! carry a positive owner spot-check, which decides only the rubric and never
//! rescues a programmatic failure. Frames are drawn here at 320x180 to model
//! what each result looks like; projects start from the real fixtures.
use pm_aero_eval::artifacts::*;
use pm_aero_eval::graders::{Context, GraderSpec};
use pm_aero_eval::image::Rgba;
use pm_aero_eval::suite::Suite;
use pm_aero_eval::verdict::{Status, Verdict};
use serde_json::{Value, json};
use std::path::Path;

const W: u32 = 320;
const H: u32 = 180;

fn suite() -> Suite {
    Suite::load(&pm_aero_eval::data_root()).unwrap()
}

fn fixture(name: &str) -> Value {
    let p = pm_aero_eval::data_root()
        .join("fixtures/projects")
        .join(name);
    pm_aero_eval::pmv::read_document(&std::fs::read(p).unwrap()).unwrap()
}

/// The layer named `name` in a document.
fn layer<'a>(doc: &'a mut Value, name: &str) -> &'a mut Value {
    doc["proj"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["name"] == name)
        .unwrap()
}

fn img(f: impl Fn(u32, u32) -> [u8; 3]) -> Rgba {
    let mut i = Rgba::solid(W, H, [0, 0, 0]);
    for y in 0..H {
        for x in 0..W {
            let p = f(x, y);
            i.set(x, y, [p[0], p[1], p[2], 255]);
        }
    }
    i
}

struct Case {
    dir: tempfile::TempDir,
    a: Artifacts,
    n: usize,
}

impl Case {
    fn new(before: Value) -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            a: Artifacts {
                project_after: Some(before.clone()),
                project_before: Some(before),
                ..Default::default()
            },
            n: 0,
        }
    }
    fn after(&mut self) -> &mut Value {
        self.a.project_after.as_mut().unwrap()
    }
    fn save(&mut self, i: &Rgba) -> std::path::PathBuf {
        std::fs::create_dir_all(self.dir.path().join("frames")).unwrap();
        self.n += 1;
        let rel = Path::new("frames").join(format!("f{}.png", self.n));
        std::fs::write(self.dir.path().join(&rel), i.encode_png()).unwrap();
        rel
    }
    fn frame(&mut self, t: f64, i: &Rgba) -> Frame {
        Frame {
            time: t,
            png: self.save(i),
        }
    }
    /// The run's frames and the baseline at the same times.
    fn frames(&mut self, times: &[f64], base: impl Fn(f64) -> Rgba, now: impl Fn(f64) -> Rgba) {
        for &t in times {
            let b = self.frame(t, &base(t));
            let n = self.frame(t, &now(t));
            self.a.baseline_frames.push(b);
            self.a.frames.push(n);
        }
    }
    /// An extension the run created, loading, registering `id`.
    fn ext(&mut self, ext: &str, action: &str, kind: &str, id: &str, params: usize) {
        let e = &mut self.a.extensions;
        e.changed.push(ChangedExtension {
            id: ext.into(),
            action: action.into(),
        });
        e.windows = vec!["headless".into()];
        e.health.push(ExtensionHealth {
            id: ext.into(),
            window: "headless".into(),
            state: "ok".into(),
            error: None,
        });
        e.registered.push(Registration {
            kind: kind.into(),
            id: id.into(),
            extension: ext.into(),
            params,
        });
    }
    fn timing(&mut self, id: &str) {
        self.a.frame_times.push(FrameTiming {
            target: id.into(),
            ms: vec![1.0; 7],
            method: "paired".into(),
        });
    }
    fn spot(&mut self) {
        self.a.spot_check = Some(SpotCheck {
            by: "owner".into(),
            pass: true,
            note: String::new(),
            needs_fix: Some(false),
        });
    }
    fn grade(&self, task: &str) -> Vec<Verdict> {
        let s = suite();
        let t = s.task(task).unwrap();
        let data = pm_aero_eval::data_root();
        let cx = Context {
            artifacts: &self.a,
            run_dir: self.dir.path(),
            data_root: &data,
        };
        t.graders
            .iter()
            .map(|g: &GraderSpec| g.grade(&cx))
            .collect()
    }
}

fn passes(c: &Case, task: &str) {
    let v = c.grade(task);
    assert!(
        v.iter().all(Verdict::passed),
        "{task}: the correct result failed: {v:#?}"
    );
}

/// A wrong result fails a programmatic grader (not merely the rubric).
fn fails(c: &Case, task: &str, what: &str) {
    let v = c.grade(task);
    let bad: Vec<&Verdict> = v
        .iter()
        .filter(|v| v.grader != "rubric" && v.status == Status::Fail)
        .collect();
    assert!(!bad.is_empty(), "{task}: {what} passed: {v:#?}");
}

// ---------------------------------------------------------------- cuts

const TAKES: [(&str, [u8; 3]); 5] = [
    ("take-1", [198, 59, 59]),
    ("take-2", [59, 160, 60]),
    ("take-3", [59, 89, 200]),
    ("broll", [210, 180, 40]),
    ("take-4", [150, 58, 169]),
];

/// The colour on screen at `t`: the enabled clip playing then, else grey.
fn shown(doc: &Value, t: f64) -> Rgba {
    let c = doc["proj"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["type"] == "video" && l["on"] != false)
        .find(|l| {
            let f = l["from"].as_f64().unwrap();
            t >= f && t < f + l["dur"].as_f64().unwrap()
        })
        .and_then(|l| TAKES.iter().find(|(a, _)| l["d"]["asset"] == *a))
        .map_or([90, 90, 90], |(_, c)| *c);
    Rgba::solid(W, H, c)
}

fn cut_case(edit: impl Fn(&mut Value)) -> Case {
    let mut c = Case::new(fixture("interview.pmv"));
    edit(c.after());
    let doc = c.a.project_after.clone().unwrap();
    let before = c.a.project_before.clone().unwrap();
    c.frames(
        &[0.5, 1.0, 1.5, 2.5, 3.0, 3.5, 4.5, 5.0, 7.0],
        |t| shown(&before, t),
        |t| shown(&doc, t),
    );
    c
}

fn drop_take_2(d: &mut Value) {
    d["proj"]["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["name"] != "Take 2");
    for (n, f) in [("B-roll", 2), ("Take 3", 4), ("Take 4", 6)] {
        layer(d, n)["from"] = json!(f);
    }
}

#[test]
fn cut_drop_flubbed_take() {
    let t = "cut-drop-flubbed-take";
    passes(&cut_case(drop_take_2), t);
    // Review scenario: the right order with every surviving clip switched off.
    fails(
        &cut_case(|d| {
            drop_take_2(d);
            for n in ["Take 1", "B-roll", "Take 3", "Take 4"] {
                layer(d, n)["on"] = json!(false);
            }
        }),
        t,
        "clips switched off",
    );
    // Take 4 deleted and Take 2 renamed into its place.
    fails(
        &cut_case(|d| {
            d["proj"]["layers"]
                .as_array_mut()
                .unwrap()
                .retain(|l| l["name"] != "Take 4");
            layer(d, "Take 2")["name"] = json!("Take 4");
            for (n, f) in [("B-roll", 2), ("Take 3", 4), ("Take 4", 6)] {
                layer(d, n)["from"] = json!(f);
            }
        }),
        t,
        "the flubbed take renamed",
    );
    // B-roll's media swapped for Take 2's.
    fails(
        &cut_case(|d| {
            drop_take_2(d);
            layer(d, "B-roll")["d"]["asset"] = json!("take-2");
        }),
        t,
        "media swapped",
    );
}

#[test]
fn cut_tighten_takes() {
    let t = "cut-tighten-takes";
    let tighten = |d: &mut Value| {
        for (i, n) in ["Take 1", "Take 2", "Take 3", "B-roll", "Take 4"]
            .iter()
            .enumerate()
        {
            let l = layer(d, n);
            l["from"] = json!(i);
            l["dur"] = json!(1);
            l["d"]["trim"] = json!(0.5);
        }
    };
    passes(&cut_case(tighten), t);
    // Lengths halved by doubling the speed instead of trimming.
    fails(
        &cut_case(|d| {
            tighten(d);
            for n in ["Take 1", "Take 2", "Take 3", "B-roll", "Take 4"] {
                layer(d, n)["d"]["speed"] = json!(2);
            }
        }),
        t,
        "sped up",
    );
    // Right timing, but the takes' media shuffled.
    fails(
        &cut_case(|d| {
            tighten(d);
            layer(d, "Take 1")["d"]["asset"] = json!("take-3");
            layer(d, "Take 3")["d"]["asset"] = json!("take-1");
        }),
        t,
        "media shuffled",
    );
}

// ---------------------------------------------------------------- grades

fn ramp(x: u32) -> f64 {
    20.0 + 210.0 * x as f64 / (W - 1) as f64
}

fn grade_case(f: impl Fn(f64) -> [f64; 3], edit: impl Fn(&mut Value)) -> Case {
    let mut c = Case::new(fixture("chart-plate.pmv"));
    edit(c.after());
    c.spot();
    let px = |v: [f64; 3]| v.map(|x| x.clamp(0.0, 255.0).round() as u8);
    c.frames(
        &[1.0],
        |_| img(|x, _| px([ramp(x); 3])),
        |_| img(|x, _| px(f(ramp(x)))),
    );
    c
}

#[test]
fn grade_late_afternoon() {
    let t = "grade-late-afternoon";
    let warm = |v: f64| {
        let l = v * 0.92 + 20.0;
        [l + 8.0, l + 2.0, l - 6.0]
    };
    let add_fx = |d: &mut Value| {
        layer(d, "Plate")["fx"] =
            json!([{"id": "g", "type": "color-balance", "on": true, "p": {}}]);
    };
    passes(&grade_case(warm, add_fx), t);
    // An orange layer laid over the plate (with a do-nothing effect on it).
    fails(
        &grade_case(
            |v| [0.9 * v + 23.2, 0.9 * v + 11.6, 0.9 * v + 5.9],
            |d| {
                add_fx(d);
                let mut over = layer(d, "Plate").clone();
                over["id"] = json!("tint");
                over["name"] = json!("Tint");
                d["proj"]["layers"].as_array_mut().unwrap().push(over);
            },
        ),
        t,
        "an orange overlay",
    );
    // The grade switched off: the picture does not change.
    fails(&grade_case(|v| [v; 3], add_fx), t, "a switched-off grade");
    // Orange, not late afternoon.
    fails(
        &grade_case(|v| [v + 60.0, v + 10.0, v - 50.0], add_fx),
        t,
        "too orange",
    );
}

#[test]
fn grade_match_reference() {
    let t = "grade-match-reference";
    let reference =
        std::fs::read(pm_aero_eval::data_root().join("references/grade-match/chart.png")).unwrap();
    let chart = Rgba::decode_png(&reference).unwrap();
    let case = |edit: &dyn Fn(&mut Value)| {
        let mut c = Case::new(fixture("grade-match.pmv"));
        edit(c.after());
        let f = c.frame(1.0, &chart);
        c.a.frames.push(f);
        c
    };
    let fx = |d: &mut Value| {
        layer(d, "Plate")["fx"] = json!([{"id": "g", "type": "curves", "on": true, "p": {}}]);
    };
    passes(&case(&fx), t);
    // The plate's picture replaced by the reference still.
    fails(
        &case(&|d| {
            fx(d);
            layer(d, "Plate")["d"]["asset"] = json!("chart");
        }),
        t,
        "the plate swapped for the reference",
    );
    // The reference switched on over the plate.
    fails(
        &case(&|d| layer(d, "Reference")["on"] = json!(true)),
        t,
        "the reference shown",
    );
}

// ---------------------------------------------------------------- effects

/// A dusk-like plate: a sky gradient, a sun, a paved ground of 4-pixel
/// squares, and an irregular white title block (letter strokes).
fn plate(x: u32, y: u32) -> [u8; 3] {
    let horizon = H * 68 / 100;
    if y >= horizon {
        return if (x / 4 + y / 4) % 2 == 1 {
            [96, 90, 82]
        } else {
            [50, 44, 36]
        };
    }
    let t = y as f64 / horizon as f64;
    [
        (40.0 + 200.0 * t) as u8,
        (60.0 + 90.0 * t) as u8,
        (140.0 - 40.0 * t) as u8,
    ]
}

const STROKES: [u32; 14] = [
    82, 86, 95, 101, 108, 117, 120, 131, 139, 144, 152, 163, 170, 176,
];

fn title(x: u32, y: u32) -> bool {
    (82..98).contains(&y) && STROKES.iter().any(|s| (*s..s + 3).contains(&x))
}

fn promo(x: u32, y: u32) -> [u8; 3] {
    if title(x, y) {
        [255, 255, 255]
    } else {
        plate(x, y)
    }
}

fn box_blur(f: &dyn Fn(u32, u32) -> [u8; 3], r: i64, x: u32, y: u32) -> [u8; 3] {
    let mut s = [0.0; 3];
    let mut n = 0.0;
    for dy in -r..=r {
        for dx in -r..=r {
            let (xx, yy) = (
                (x as i64 + dx).clamp(0, W as i64 - 1) as u32,
                (y as i64 + dy).clamp(0, H as i64 - 1) as u32,
            );
            let p = f(xx, yy);
            for i in 0..3 {
                s[i] += p[i] as f64;
            }
            n += 1.0;
        }
    }
    s.map(|v| (v / n).round() as u8)
}

#[test]
fn add_vignette() {
    let t = "add-vignette";
    let vignette = |strength: f64| {
        move |x: u32, y: u32| {
            let (dx, dy) = (x as f64 / W as f64 - 0.5, y as f64 / H as f64 - 0.5);
            let d = (dx * dx + dy * dy).sqrt();
            let k = 1.0 - strength * (d - 0.25).max(0.0) * 2.5;
            plate(x, y).map(|v| (v as f64 * k.max(0.0)) as u8)
        }
    };
    let case = |fx: Value| {
        let mut c = Case::new(fixture("promo.pmv"));
        layer(c.after(), "Background plate")["fx"] = fx;
        c.frames(&[1.0], |_| img(plate), |_| img(vignette(0.5)));
        c
    };
    let v = |amount: u32, on: bool| json!({"type": "vignette", "on": on, "p": {"amount": {"v": amount}}});
    passes(&case(json!([v(50, true)])), t);
    // A half vignette stacked with a full one.
    fails(
        &case(json!([v(50, true), v(100, true)])),
        t,
        "two vignettes",
    );
    // The vignette switched off.
    fails(&case(json!([v(50, false)])), t, "a switched-off vignette");
}

#[test]
fn add_blur_build() {
    let t = "add-blur-build";
    let blur = |on: bool| json!([{"type": "blur", "on": on, "p": {"amount": {"v": 0, "kf": [{"t": 0.0, "v": 0}, {"t": 1.0, "v": 24}]}}}]);
    let case = |fx: Value, after: &dyn Fn(f64) -> Rgba| {
        let mut c = Case::new(fixture("promo.pmv"));
        layer(c.after(), "Background plate")["fx"] = fx;
        c.frames(&[0.0, 1.0, 2.0], |_| img(promo), after);
        c
    };
    let plate_blurred = |t: f64| {
        if t < 0.5 {
            img(promo)
        } else {
            img(|x, y| {
                if title(x, y) {
                    [255, 255, 255]
                } else {
                    box_blur(&plate, 6, x, y)
                }
            })
        }
    };
    passes(&case(blur(true), &plate_blurred), t);
    // Review scenario: a disabled blur with valid keys, the plate unchanged at
    // 0 s and an unrelated red picture at 2 s.
    fails(
        &case(blur(false), &|t| {
            if t < 1.5 {
                img(promo)
            } else {
                Rgba::solid(W, H, [255, 0, 0])
            }
        }),
        t,
        "a disabled blur and a red frame",
    );
    // The whole picture blurred, title included.
    fails(
        &case(blur(true), &|t| {
            if t < 0.5 {
                img(promo)
            } else {
                img(|x, y| box_blur(&promo, 6, x, y))
            }
        }),
        t,
        "the title blurred too",
    );
    // A darkened plate instead of a blur.
    fails(
        &case(blur(true), &|t| {
            if t < 0.5 {
                img(promo)
            } else {
                img(|x, y| promo(x, y).map(|v| v / 2))
            }
        }),
        t,
        "darkened, not blurred",
    );
}

/// A run that wrote and loaded an effect `id`, put on `layer_name`.
fn new_effect_case(
    project: &str,
    layer_name: &str,
    id: &str,
    base: fn(u32, u32) -> [u8; 3],
    after: &dyn Fn(u32, u32) -> [u8; 3],
) -> Case {
    let mut c = Case::new(fixture(project));
    let ext = id.split('.').next().unwrap();
    c.ext(ext, "created", "effect", id, 1);
    layer(c.after(), layer_name)["fx"] =
        json!([{"type": id, "on": true, "p": {"amount": {"v": 1}}}]);
    c.timing(id);
    c.spot();
    c.frames(&[1.0], |_| img(base), |_| img(after));
    c
}

#[test]
fn new_effect_chromatic_split() {
    let t = "new-effect-chromatic-split";
    let in_title =
        |x: u32, y: u32| (W / 4..3 * W / 4).contains(&x) && (H * 38..H * 62).contains(&(y * 100));
    let split = move |x: u32, y: u32| {
        if !in_title(x, y) {
            return promo(x, y);
        }
        let g = promo(x, y);
        [
            promo(x.saturating_sub(4), y)[0],
            g[1],
            promo((x + 4).min(W - 1), y)[2],
        ]
    };
    passes(
        &new_effect_case("promo.pmv", "Title", "split.effect", promo, &split),
        t,
    );
    // Review scenario: any local change passed; here the title is tinted.
    let tint = move |x: u32, y: u32| {
        if title(x, y) {
            [255, 170, 170]
        } else {
            promo(x, y)
        }
    };
    fails(
        &new_effect_case("promo.pmv", "Title", "split.effect", promo, &tint),
        t,
        "a tint",
    );
    // The title moved sideways (both channels together).
    let moved = move |x: u32, y: u32| {
        if in_title(x, y) {
            promo(x.saturating_sub(6), y)
        } else {
            promo(x, y)
        }
    };
    fails(
        &new_effect_case("promo.pmv", "Title", "split.effect", promo, &moved),
        t,
        "a move",
    );
}

#[test]
fn new_effect_halftone() {
    let t = "new-effect-halftone";
    let dots = |x: u32, y: u32| {
        let (cx, cy) = ((x % 6) as i32 - 3, (y % 6) as i32 - 3);
        if cx * cx + cy * cy <= 4 {
            [20, 20, 20]
        } else {
            plate(x, y)
        }
    };
    passes(
        &new_effect_case(
            "promo.pmv",
            "Background plate",
            "halftone.effect",
            plate,
            &dots,
        ),
        t,
    );
    // A darkening "effect" (with the owner's thumbs up): not dots.
    let dark = |x: u32, y: u32| plate(x, y).map(|v| v / 2);
    fails(
        &new_effect_case(
            "promo.pmv",
            "Background plate",
            "halftone.effect",
            plate,
            &dark,
        ),
        t,
        "a darken",
    );
}

// ---------------------------------------------------------------- transitions

const DAWN: [u8; 3] = [232, 116, 59];
const DUSK: [u8; 3] = [31, 138, 138];
const DAWN_BAR: [u8; 3] = [96, 40, 18];
const DUSK_BAR: [u8; 3] = [8, 54, 54];

fn shot_a(x: f64) -> [u8; 3] {
    if (0.80..0.86).contains(&x) {
        DAWN_BAR
    } else {
        DAWN
    }
}
fn shot_b(x: f64) -> [u8; 3] {
    if (0.14..0.20).contains(&x) {
        DUSK_BAR
    } else {
        DUSK
    }
}
/// Progress of a half-second transition centred on the 3 s cut.
fn progress(t: f64, dur: f64) -> f64 {
    ((t - (3.0 - dur / 2.0)) / dur).clamp(0.0, 1.0)
}

fn times() -> Vec<f64> {
    (0..31)
        .map(|i| ((2.5 + i as f64 / 30.0) * 10_000.0).round() / 10_000.0)
        .collect()
}

fn cut_frame(t: f64) -> Rgba {
    img(|x, _| {
        let u = x as f64 / W as f64;
        if t < 3.0 { shot_a(u) } else { shot_b(u) }
    })
}

fn transition_case(id: &str, dur: f64, frame: &dyn Fn(f64) -> Rgba) -> Case {
    let mut c = Case::new(fixture("cards.pmv"));
    c.ext(
        id.split('.').next().unwrap(),
        "created",
        "transition",
        id,
        0,
    );
    layer(c.after(), "Shot B")["transitionIn"] = json!({"type": id, "dur": dur, "p": {}});
    c.timing(id);
    c.frames(&times(), cut_frame, frame);
    c
}

#[test]
fn transition_push() {
    let t = "transition-push";
    let push = |t: f64| {
        let p = progress(t, 0.5);
        img(move |x, _| {
            let u = x as f64 / W as f64;
            if u < 1.0 - p {
                shot_a(u + p)
            } else {
                shot_b(u - (1.0 - p))
            }
        })
    };
    passes(&transition_case("push.transition", 0.5, &push), t);
    // A wipe: the next shot revealed in place, nothing moves.
    let wipe = |t: f64| {
        let p = progress(t, 0.5);
        img(move |x, _| {
            let u = x as f64 / W as f64;
            if u < 1.0 - p { shot_a(u) } else { shot_b(u) }
        })
    };
    fails(&transition_case("push.transition", 0.5, &wipe), t, "a wipe");
    // A slide: the next shot moves in over a still current shot.
    let slide = |t: f64| {
        let p = progress(t, 0.5);
        img(move |x, _| {
            let u = x as f64 / W as f64;
            if u < 1.0 - p {
                shot_a(u)
            } else {
                shot_b(u - (1.0 - p))
            }
        })
    };
    fails(
        &transition_case("push.transition", 0.5, &slide),
        t,
        "a slide",
    );
    // A cross-dissolve.
    let mix = |t: f64| {
        let p = progress(t, 0.5);
        img(move |x, _| {
            let u = x as f64 / W as f64;
            let (a, b) = (shot_a(u), shot_b(u));
            [0, 1, 2].map(|i| (a[i] as f64 * (1.0 - p) + b[i] as f64 * p) as u8)
        })
    };
    fails(
        &transition_case("push.transition", 0.5, &mix),
        t,
        "a dissolve",
    );
    // A hard cut with the transition registered but drawing nothing.
    fails(
        &transition_case("push.transition", 0.5, &cut_frame),
        t,
        "a cut",
    );
}

#[test]
fn transition_iris() {
    let t = "transition-iris";
    let open = |shape: fn(f64, f64, f64) -> bool| {
        move |t: f64| {
            let p = progress(t, 1.0);
            img(move |x, y| {
                let (dx, dy) = (x as f64 - W as f64 / 2.0, y as f64 - H as f64 / 2.0);
                if shape(dx, dy, p) {
                    shot_b(x as f64 / W as f64)
                } else {
                    shot_a(x as f64 / W as f64)
                }
            })
        }
    };
    let iris = open(|dx, dy, p| (dx * dx + dy * dy).sqrt() < p * 184.0);
    passes(&transition_case("iris.transition", 1.0, &iris), t);
    // A barn door opening from the middle.
    let door = open(|dx, _, p| dx.abs() < p * 160.0);
    fails(
        &transition_case("iris.transition", 1.0, &door),
        t,
        "a barn door",
    );
    // A wipe from the left.
    let wipe = open(|dx, _, p| dx + 160.0 < p * 320.0);
    fails(&transition_case("iris.transition", 1.0, &wipe), t, "a wipe");
}

// ---------------------------------------------------------------- panels

fn panel_case(project: &str, panel: PanelObservation, new: bool) -> Case {
    let mut c = Case::new(fixture(project));
    if new {
        c.ext("tool", "created", "panel", &panel.panel.clone(), 0);
    }
    c.spot();
    c.a.panels.push(panel);
    c
}

#[test]
fn panel_marker_list() {
    let t = "panel-marker-list";
    let doc = fixture("markers.pmv");
    let obs = |playhead: f64, text: &[&str]| PanelObservation {
        panel: "tool.panel".into(),
        opened: true,
        text: text.iter().map(|s| s.to_string()).collect(),
        steps: vec!["ok".into(), "ok".into()],
        playhead: Some(playhead),
        project_before: Some(doc.clone()),
        project_after: Some(doc.clone()),
        ..Default::default()
    };
    let list = ["Intro", "0:00.0", "Drop", "0:02.5", "Outro", "0:05.0"];
    passes(&panel_case("markers.pmv", obs(2.5, &list), true), t);
    // Clicking does nothing (the probe first parks the playhead at 0).
    fails(
        &panel_case("markers.pmv", obs(0.0, &list), true),
        t,
        "a dead click",
    );
    // Names without times.
    fails(
        &panel_case("markers.pmv", obs(2.5, &["Intro", "Drop", "Outro"]), true),
        t,
        "no times",
    );
}

#[test]
fn panel_speed_buttons() {
    let t = "panel-speed-buttons";
    let with_speed = |s: f64| {
        let mut d = fixture("speed.pmv");
        layer(&mut d, "Clip")["d"]["speed"] = json!(s);
        d
    };
    let obs = |before: f64, after: f64| PanelObservation {
        panel: "tool.panel".into(),
        opened: true,
        text: vec!["50%".into(), "100%".into(), "200%".into()],
        steps: vec!["ok".into()],
        project_before: Some(with_speed(before)),
        project_after: Some(with_speed(after)),
        ..Default::default()
    };
    passes(&panel_case("speed.pmv", obs(1.0, 0.5), true), t);
    // The run set the speed itself; the button does nothing.
    fails(
        &panel_case("speed.pmv", obs(0.5, 0.5), true),
        t,
        "a dead button",
    );
}

#[test]
fn tool_beat_flash() {
    let t = "tool-beat-flash";
    let normal = || img(promo);
    let white = || Rgba::solid(W, H, [255, 255, 255]);
    let with_markers = |m: Value| {
        let mut d = fixture("promo.pmv");
        d["proj"]["markers"] = m;
        d
    };
    let case = |before: Value, after: Value, flash: bool| {
        let mut c = Case::new(fixture("promo.pmv"));
        c.ext("beat", "created", "panel", "beat.panel", 0);
        c.spot();
        let mut o = PanelObservation {
            panel: "beat.panel".into(),
            opened: true,
            text: vec!["Tap".into()],
            steps: vec!["ok".into(), "ok".into()],
            project_before: Some(before),
            project_after: Some(after),
            ..Default::default()
        };
        for t in [1.95, 2.0167, 2.05, 2.1167] {
            let lit = flash && (2.0..2.0667).contains(&t);
            let b = c.frame(t, &normal());
            let a = c.frame(t, &if lit { white() } else { normal() });
            o.baseline_frames.push(b);
            o.frames.push(a);
        }
        c.a.panels.push(o);
        c
    };
    let tapped = with_markers(json!([{"id": "m1", "t": 2.0, "name": "Beat"}]));
    passes(&case(with_markers(json!([])), tapped, true), t);
    // Review scenario: a no-op Tap, a marker already at 99 s, no flash and a
    // positive spot-check.
    let old = with_markers(json!([{"id": "m0", "t": 99.0}]));
    fails(&case(old.clone(), old, false), t, "a no-op tap");
    // A marker but no flash.
    fails(
        &case(
            with_markers(json!([])),
            with_markers(json!([{"t": 2.0}])),
            false,
        ),
        t,
        "no flash",
    );
}

#[test]
fn tool_lower_third_kit() {
    let t = "tool-lower-third-kit";
    let doc = fixture("promo.pmv");
    let with_third = |from: f64, keys: Value| {
        let mut d = doc.clone();
        d["proj"]["layers"].as_array_mut().unwrap().push(json!({
            "id": "lt1", "type": "text", "name": "Lower third", "from": from, "dur": 4, "on": true,
            "fx": [], "d": {"text": "Ada Park\nColourist"}, "p": {"position.x": {"v": 0, "kf": keys}}}));
        d
    };
    let obs = |after: Value| PanelObservation {
        panel: "kit.panel".into(),
        opened: true,
        text: vec!["Name".into(), "Title".into(), "Add".into()],
        steps: vec!["ok".into(); 4],
        project_before: Some(doc.clone()),
        project_after: Some(after),
        ..Default::default()
    };
    let slide = json!([{"t": 0.0, "v": -400}, {"t": 0.5, "v": 0}]);
    passes(
        &panel_case("promo.pmv", obs(with_third(1.0, slide.clone())), true),
        t,
    );
    // Dropped at the start instead of the playhead.
    fails(
        &panel_case("promo.pmv", obs(with_third(0.0, slide)), true),
        t,
        "at 0",
    );
    // Two identical keys: nothing slides.
    let still = json!([{"t": 0.0, "v": 0}, {"t": 0.5, "v": 0}]);
    fails(
        &panel_case("promo.pmv", obs(with_third(1.0, still)), true),
        t,
        "no slide",
    );
}

// ---------------------------------------------------------------- fixes

#[test]
fn fix_softglow_compile() {
    let t = "fix-softglow-compile";
    let case = |after: &dyn Fn(u32, u32) -> [u8; 3]| {
        let mut c = Case::new(fixture("glow-title.pmv"));
        c.ext("softglow", "updated", "effect", "softglow.effect", 3);
        c.frames(&[1.0], |_| img(promo), |_| img(after));
        c
    };
    let glow = |x: u32, y: u32| {
        let near = (0..5).any(|d| {
            title(x + d, y)
                || title(x.saturating_sub(d), y)
                || title(x, y + d)
                || title(x, y.saturating_sub(d))
        });
        if near && !title(x, y) {
            promo(x, y).map(|v| v.saturating_add(40))
        } else {
            promo(x, y)
        }
    };
    passes(&case(&glow), t);
    // The glow removed so the shader compiles: nothing changes.
    fails(&case(&promo), t, "no glow");
    // A different effect: the title tinted red.
    fails(
        &case(&|x, y| {
            if title(x, y) {
                [255, 90, 90]
            } else {
                promo(x, y)
            }
        }),
        t,
        "a tint",
    );
}

#[test]
fn fix_shot_notes_crash() {
    let t = "fix-shot-notes-crash";
    let case = |text: &[&str], step: &str| {
        let mut c = Case::new(fixture("promo.pmv"));
        c.ext("shot-notes", "updated", "panel", "shot-notes.panel", 0);
        c.a.panels.push(PanelObservation {
            panel: "shot-notes.panel".into(),
            opened: true,
            text: text.iter().map(|s| s.to_string()).collect(),
            steps: vec![step.into()],
            ..Default::default()
        });
        c
    };
    passes(&case(&["Background plate", "Title", "hero shot"], "ok"), t);
    // The crash caught and the list replaced with static text, no notes.
    fails(
        &case(
            &["Background plate", "Title"],
            "no control labelled \"Title\"",
        ),
        t,
        "static text",
    );
}

#[test]
fn fix_mono_negative() {
    let t = "fix-mono-negative";
    let chart = |x: u32, y: u32| {
        let c = [
            [176, 52, 48],
            [64, 148, 70],
            [52, 80, 168],
            [214, 190, 60],
            [24, 24, 24],
            [232, 232, 232],
        ];
        c[((x / 54 + y / 45 * 2) % 6) as usize]
    };
    let luma =
        |p: [u8; 3]| (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) as u8;
    let case = |after: &dyn Fn(u32, u32) -> [u8; 3], amount: f64| {
        let mut c = Case::new(fixture("chart-mono.pmv"));
        c.ext("mono", "updated", "effect", "mono.effect", 1);
        layer(c.after(), "Plate")["fx"][0]["p"] = json!({"amount": {"v": amount}});
        c.frames(
            &[1.0],
            |_| img(|x, y| [255 - luma(chart(x, y)); 3]),
            |_| img(after),
        );
        c
    };
    passes(&case(&|x, y| [luma(chart(x, y)); 3], 1.0), t);
    // A flat grey picture: no chroma, no picture.
    fails(&case(&|_, _| [128, 128, 128], 1.0), t, "flat grey");
    // The effect set to zero: the colour is back.
    fails(&case(&chart, 0.0), t, "amount zero");
}

// ---------------------------------------------------------------- restyles

fn theme_case(tokens: &[(&str, &str)]) -> Case {
    let mut c = Case::new(fixture("promo.pmv"));
    c.ext("look", "created", "theme", "look.theme", 0);
    c.a.theme = Some(ThemeObservation {
        active: "look.theme".into(),
        extension: Some("look".into()),
        tokens: tokens
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    });
    c.spot();
    c
}

#[test]
fn restyle_warm_studio() {
    let t = "restyle-warm-studio";
    let base = [
        ("--pm-accent", "#ffb000"),
        ("--pm-ground", "#1c1814"),
        ("--pm-window", "#221d18"),
        ("--pm-text", "#f4ede4"),
        ("--pm-text-2", "#bfb3a6"),
    ];
    passes(&theme_case(&base), t);
    // Readable-looking text tokens that are nearly transparent.
    let mut faint = base;
    faint[3] = ("--pm-text", "rgba(244, 237, 228, 0.12)");
    fails(&theme_case(&faint), t, "translucent text");
    // A translucent amber accent over an unknown surface.
    let mut ghost = base;
    ghost[0] = ("--pm-accent", "#ffb00020");
    fails(&theme_case(&ghost), t, "a ghost accent");
}

#[test]
fn restyle_projector() {
    let t = "restyle-projector";
    let base = [
        ("--pm-ground", "#ffffff"),
        ("--pm-text", "#111111"),
        ("--pm-text-2", "#333333"),
        ("--pm-hairline", "#888888"),
    ];
    passes(&theme_case(&base), t);
    // Review scenario: fully transparent black text and separators on white
    // reported 21:1.
    fails(
        &theme_case(&[
            ("--pm-ground", "#ffffff"),
            ("--pm-text", "rgba(0, 0, 0, 0)"),
            ("--pm-text-2", "#00000000"),
            ("--pm-hairline", "transparent"),
        ]),
        t,
        "transparent text",
    );
    // A translucent ground: the surface under it is unknown.
    let mut glass = base;
    glass[0] = ("--pm-ground", "rgba(255, 255, 255, 0.4)");
    fails(&theme_case(&glass), t, "a translucent ground");
}

#[test]
fn every_task_has_a_wrong_result_test() {
    let here = include_str!("wrong_results.rs");
    for t in suite().tasks {
        let f = format!("fn {}()", t.id.replace('-', "_"));
        assert!(here.contains(&f), "no wrong-result test for {}", t.id);
    }
}
