//! The benches: one user-facing workload each, through the engine's own
//! viewer path (`pm_node::present`: the view's document, its routing
//! decision per revision, and the composer that draws the viewer's frame)
//! or its export path (`pm_export`).
//!
//! Setup (fixtures, the composer, warm-up frames that build pipelines and
//! caches) happens before the counted span, except where the cold work is
//! the point (`open_large`, `import_3d`).

use std::sync::Arc;
use std::time::Instant;

use pm_gpu::bridge_out::display::WorkingTexture;
use pm_node::present::compose::Composer;
use pm_node::present::document::{Route, ViewDocument};
use pm_perf::Scope;
use pm_protocol::color::ViewerColorSpace;
use pm_protocol::common::{Revision, Seq, Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::frame::{MediaPreference, PreviewResolution, ViewerState};
use serde_json::{Value, json};

use crate::BenchReport;
use crate::fixtures::{self, ROOT, Synthetic};

/// The viewer's device-pixel size in the Phase 2 rows (a 1496 × 904 stage
/// at DPR 2).
pub const VIEW: (u32, u32) = (1496, 904);

/// What the benches share: one GPU device.
pub struct Ctx {
    pub gpu: pm_gpu::Gpu,
}

impl Ctx {
    pub fn new() -> Ctx {
        // The benches report every thread's counts too (the ratchet gates
        // them: work moved to another thread still counts).
        pm_perf::count_globally(true);
        Ctx {
            gpu: pm_gpu::Gpu::new("perfbench").expect("a GPU"),
        }
    }
}

impl Default for Ctx {
    fn default() -> Self {
        Self::new()
    }
}

/// A bench: its name and how to run it.
pub type Bench = (&'static str, fn(&Ctx) -> BenchReport);

/// A configured bench of the correlation set.
pub type BoxedBench = Box<dyn Fn(&Ctx) -> BenchReport>;

/// The correlation set (docs/engine/perfbench.md, "Correlation"): the
/// workloads at several sizes and with the viewer's reuse on and off, so
/// every counter moves across them.
pub fn correlation() -> Vec<(String, BoxedBench)> {
    let mut out: Vec<(String, BoxedBench)> = Vec::new();
    for layers in [125usize, 250, 500, 1000] {
        let n = format!("playback_{layers}");
        out.push((n.clone(), Box::new(move |c| playback(c, &n, layers, 60))));
    }
    for layers in [250usize, 500, 1000] {
        for reuse in [true, false] {
            let n = format!("drag_{layers}_{}", if reuse { "reuse" } else { "noreuse" });
            out.push((n.clone(), Box::new(move |c| drag(c, &n, layers, 40, reuse))));
        }
    }
    for layers in [100usize, 500] {
        let n = format!("export_{layers}");
        out.push((n.clone(), Box::new(move |c| export(c, &n, layers, 30))));
    }
    for vertices in [150_000usize, 600_000] {
        let n = format!("import_3d_{vertices}");
        out.push((n.clone(), Box::new(move |c| import_3d(c, &n, vertices))));
    }
    out.push(("edit_playback_500".into(), Box::new(edit_playback_500)));
    out.push(("scrub_24".into(), Box::new(scrub_24)));
    out.push(("typing_60".into(), Box::new(typing_60)));
    out.push(("open_large_2000".into(), Box::new(open_large_2000)));
    out
}

/// Every bench, in report order.
pub fn all() -> Vec<Bench> {
    vec![
        ("playback_500", playback_500),
        ("drag_500", drag_500),
        ("edit_playback_500", edit_playback_500),
        ("inspector_edit_500", inspector_edit_500),
        ("scrub_24", scrub_24),
        ("export_120", export_120),
        ("typing_60", typing_60),
        ("open_large_2000", open_large_2000),
        ("import_3d_600k", import_3d_600k),
    ]
}

fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).expect("a safe tick")
}

/// The viewer fitting a `cw` × `ch` composition in [`VIEW`] at DPR 2.
pub fn viewer_state(cw: f64, ch: f64) -> ViewerState {
    let (width, height) = VIEW;
    let zoom = (f64::from(width) / 2.0 / cw).min(f64::from(height) / 2.0 / ch);
    ViewerState {
        headless: None,
        composition: ROOT.into(),
        size: Size::new(width, height),
        device_pixel_ratio: 2.0,
        zoom,
        pan: [0.0, 0.0],
        resolution: PreviewResolution::Automatic,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::DisplayP3,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(1).unwrap(),
        vsync: None,
        depth_id: None,
        chrome_selection: None,
        canvas: None,
        surface_epoch: None,
    }
}

/// A view as the presenter keeps one: its document (replaced, then
/// patched revision by revision), its media answers, its composer.
struct Viewer<'a> {
    ctx: &'a Ctx,
    view: ViewDocument,
    composer: Composer,
    working: WorkingTexture,
    state: ViewerState,
    revision: u64,
}

impl<'a> Viewer<'a> {
    fn new(
        ctx: &'a Ctx,
        doc: &Value,
        media: &std::collections::BTreeMap<String, std::path::PathBuf>,
    ) -> Viewer<'a> {
        let mut view = ViewDocument::default();
        for (asset, path) in media {
            view.set_media(asset, Some(path.clone()));
        }
        let cw = doc["w"].as_f64().unwrap_or(1920.0);
        let ch = doc["h"].as_f64().unwrap_or(1080.0);
        let mut v = Viewer {
            ctx,
            view,
            composer: Composer::new(&ctx.gpu.device, &ctx.gpu.queue),
            working: WorkingTexture::new(&ctx.gpu.device, Size::new(VIEW.0, VIEW.1)),
            state: viewer_state(cw, ch),
            revision: 1,
        };
        v.replace(doc);
        v
    }

    /// A resync: the whole document at the next revision.
    fn replace(&mut self, doc: &Value) {
        let root = pm_model::json::parse(&doc.to_string()).expect("a fixture parses");
        self.replace_parsed(root);
    }

    fn replace_parsed(&mut self, root: pm_model::Value) {
        self.revision += 1;
        self.view
            .update(&pm_engine::document::DocumentUpdate::Replace {
                root: Some(Arc::new(root)),
                revision: Revision::new(self.revision).unwrap(),
            })
            .expect("a replace applies");
    }

    /// One edit batch at the next revision.
    fn patch(&mut self, patches: Vec<pm_model::Patch>) {
        self.revision += 1;
        self.view
            .update(&pm_engine::document::DocumentUpdate::Patches {
                patches: patches.into(),
                revision: Revision::new(self.revision).unwrap(),
            })
            .expect("an edit applies");
    }

    /// The presenter's document step and draw for one frame: the route at
    /// this revision, the compose, and the wait for the GPU (wall-clock only).
    fn frame(&mut self, time: Ticks) -> f64 {
        let start = Instant::now();
        let route = self.view.route(ROOT);
        assert_eq!(
            route,
            Route::Engine,
            "the bench fixture routes to the engine"
        );
        let composed = self
            .composer
            .compose(
                self.view.document(),
                ROOT,
                self.view.media(),
                time,
                &self.state,
                &self.working,
            )
            .expect("compose");
        assert!(composed.failures.is_empty(), "{:?}", composed.failures);
        for m in &composed.scene3d_meta {
            assert!(
                m.error.is_none() && m.warnings.is_empty(),
                "the bench's 3D scene draws: {m:?}"
            );
        }
        let index = self.ctx.gpu.queue.submit([]);
        self.ctx
            .gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("the GPU finishes");
        start.elapsed().as_secs_f64() * 1e3
    }
}

/// The counts of a measured span, taken as soon as it ends (before the
/// report's own allocations).
struct Measured(pm_perf::Counters, pm_perf::Counters);

fn measured(scope: Scope) -> Measured {
    Measured(scope.finish(), scope.finish_global())
}

fn report(
    name: &str,
    unit: &'static str,
    m: Measured,
    wall_ms: Vec<f64>,
    notes: Value,
) -> BenchReport {
    BenchReport {
        name: name.to_owned(),
        units: wall_ms.len() as u32,
        unit,
        counters: m.0,
        global: m.1,
        wall_ms,
        notes,
    }
}

/// The index of the n-th solid among the root's layers (top first, as
/// `compose_bench`'s `PM_BENCH_LAYER` counts them).
fn nth_solid(doc: &Value, n: usize) -> usize {
    let solids: Vec<usize> = doc["layers"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, l)| l["type"] == "solid")
        .map(|(i, _)| i)
        .collect();
    solids[n.min(solids.len() - 1)]
}

fn leaf_set(index: usize, path: &[&str], value: pm_model::Value) -> pm_model::Patch {
    use pm_model::patch::PathPart;
    let mut p = vec![
        PathPart::Key("layers".into()),
        PathPart::Index(index as f64),
    ];
    p.extend(path.iter().map(|k| PathPart::Key((*k).into())));
    pm_model::Patch::set(p, value)
}

/// Playback: 120 consecutive frames of the 500-layer synthetic project.
pub fn playback_500(ctx: &Ctx) -> BenchReport {
    playback(ctx, "playback_500", 500, 120)
}

/// Playback of `frames` consecutive frames of the `layers`-layer synthetic
/// project (after two warm-up frames).
pub fn playback(ctx: &Ctx, name: &str, layers: usize, frames: u32) -> BenchReport {
    let f = fixtures::synthetic(Synthetic::new(layers));
    let mut v = Viewer::new(ctx, &f.doc, &f.media);
    v.composer.set_playing(true);
    let fps = 30.0;
    for k in 0..2 {
        v.frame(ticks(f64::from(k) / fps));
    }
    let scope = Scope::begin();
    let wall = (2..2 + frames)
        .map(|k| v.frame(ticks(f64::from(k) / fps)))
        .collect();
    let m = measured(scope);
    report(name, "frames", m, wall, f.summary.clone())
}

/// A layer drag, paused: 60 moves of one solid, each a new revision drawn
/// at the same time (the D6 row).
pub fn drag_500(ctx: &Ctx) -> BenchReport {
    drag(ctx, "drag_500", 500, 60, true)
}

/// `moves` moves of the 150th solid of the `layers`-layer synthetic
/// project; `reuse`: the viewer's draw reuse and frame prefix (on in the
/// app).
pub fn drag(ctx: &Ctx, name: &str, layers: usize, moves: u32, reuse: bool) -> BenchReport {
    let f = fixtures::synthetic(Synthetic::new(layers));
    let layer = nth_solid(&f.doc, 150);
    let mut v = Viewer::new(ctx, &f.doc, &f.media);
    v.composer.set_draw_reuse(reuse);
    let at = ticks(1.0);
    v.frame(at);
    v.frame(at);
    let scope = Scope::begin();
    let wall = (0..moves)
        .map(|k| {
            v.patch(vec![
                leaf_set(
                    layer,
                    &["p", "position.x", "v"],
                    pm_model::Value::Number(100.0 + f64::from(k)),
                ),
                leaf_set(
                    layer,
                    &["p", "position.y", "v"],
                    pm_model::Value::Number(300.0 + f64::from(k) / 2.0),
                ),
            ]);
            v.frame(at)
        })
        .collect();
    let m = measured(scope);
    report(
        name,
        "moves",
        m,
        wall,
        json!({ "fixture": f.summary, "layer": layer, "reuse": reuse }),
    )
}

/// Inspector edits while paused: 60 edits of one layer's position, each
/// as the editor applies it (`PM.Edit.apply`): the layer's channel, the
/// project's `revision`, and an entry appended to its `edits` provenance log
/// (`legacy/core/editing.ts`), one batch per edit.
pub fn inspector_edit_500(ctx: &Ctx) -> BenchReport {
    use pm_model::patch::PathPart;
    let f = fixtures::synthetic(Synthetic::new(500));
    let layer = nth_solid(&f.doc, 150);
    let mut v = Viewer::new(ctx, &f.doc, &f.media);
    let at = ticks(1.0);
    v.frame(at);
    v.frame(at);
    let entry = |k: u32| {
        pm_model::json::parse(&format!(
            r#"{{"id":"e{k}","label":"Position","origin":"inspector","revision":{},"at":{k}}}"#,
            k + 1
        ))
        .unwrap()
    };
    let scope = Scope::begin();
    let wall = (0..60)
        .map(|k| {
            v.patch(vec![
                leaf_set(
                    layer,
                    &["p", "position.x", "v"],
                    pm_model::Value::Number(100.0 + f64::from(k)),
                ),
                pm_model::Patch::set(
                    vec![PathPart::Key("revision".into())],
                    pm_model::Value::Number(f64::from(k + 1)),
                ),
                pm_model::Patch::set(
                    vec![PathPart::Key("edits".into()), PathPart::Index(f64::from(k))],
                    entry(k),
                ),
            ]);
            v.frame(at)
        })
        .collect();
    let m = measured(scope);
    report(
        "inspector_edit_500",
        "edits",
        m,
        wall,
        json!({ "fixture": f.summary, "layer": layer }),
    )
}

/// Inspector edits during playback: an opacity edit every frame while 60
/// frames play (the worst rate of the Phase 2 row).
pub fn edit_playback_500(ctx: &Ctx) -> BenchReport {
    let f = fixtures::synthetic(Synthetic::new(500));
    let layer = nth_solid(&f.doc, 40);
    let mut v = Viewer::new(ctx, &f.doc, &f.media);
    v.composer.set_playing(true);
    let fps = 30.0;
    v.frame(ticks(0.0));
    v.frame(ticks(1.0 / fps));
    let scope = Scope::begin();
    let wall = (2..62)
        .map(|k| {
            v.patch(vec![leaf_set(
                layer,
                &["p", "opacity", "v"],
                pm_model::Value::Number(30.0 + f64::from(k % 60)),
            )]);
            v.frame(ticks(f64::from(k) / fps))
        })
        .collect();
    let m = measured(scope);
    report(
        "edit_playback_500",
        "frames",
        m,
        wall,
        json!({ "fixture": f.summary, "layer": layer }),
    )
}

/// Scrubbing: 24 paused seeks across a video (exact frames).
pub fn scrub_24(ctx: &Ctx) -> BenchReport {
    let doc = fixtures::video_doc();
    let media = std::collections::BTreeMap::from([("clip".to_owned(), fixtures::video_fixture())]);
    let mut v = Viewer::new(ctx, &doc, &media);
    v.frame(ticks(0.0));
    let scope = Scope::begin();
    // Every frame of 0..1.9 s visited in a scattered order (7 is coprime
    // with 24): forward and backward seeks of every length.
    let wall = (0..24)
        .map(|k| v.frame(ticks(f64::from((k * 7) % 24) * 1.9 / 24.0 + 1.0 / 60.0)))
        .collect();
    let m = measured(scope);
    report(
        "scrub_24",
        "seeks",
        m,
        wall,
        json!({ "media": "h264-aac.mp4 (320x180, 30 fps)" }),
    )
}

/// Export: 120 frames of the 500-layer synthetic project to H.264 at
/// 1920 × 1080 (VideoToolbox; the file is written and removed).
pub fn export_120(ctx: &Ctx) -> BenchReport {
    export(ctx, "export_120", 500, 120)
}

/// Export of `frames` frames of the `layers`-layer synthetic project.
pub fn export(ctx: &Ctx, name: &str, layers: usize, frames: u32) -> BenchReport {
    use pm_export::render::*;
    use pm_export::*;
    use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
    use pm_protocol::common::{FrameRate, TickRange};
    use pm_protocol::priority::{
        AlphaOutput as WireAlpha, ExportFormat, ExportSettings, QualityTier,
    };
    let f = fixtures::synthetic(Synthetic::new(layers));
    let dir = fixtures::TempDir::new("export");
    let mut media = MediaPaths::default();
    for (k, p) in &f.media {
        media.insert(k.clone(), p.clone());
    }
    let job = ExportJob {
        document: Arc::new(fixtures::load(&f.doc)),
        composition: ROOT.into(),
        settings: ExportSettings {
            format: ExportFormat::H264,
            alpha: WireAlpha::None,
            quality: QualityTier::High,
            size: Size::new(1920, 1080),
            frame_rate: FrameRate::new(30, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: ticks(f64::from(frames) / 30.0),
            },
            audio: false,
            audio_index_playhead: None,
            motion_blur: false,
            color: OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            },
            precision: Precision::Float16,
            dither: true,
            draft_from_proxies: false,
            delivery: Default::default(),
        },
        media: Arc::new(media),
        destination: dir.path().join("bench.mp4"),
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 7,
    };
    let mut renderer = pm_render::Renderer::new(ctx.gpu.device.clone(), ctx.gpu.queue.clone());
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut stamps = Vec::new();
    let start = Instant::now();
    let scope = Scope::begin();
    let mut last = 0.0;
    let delivered = run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &cancel,
        &mut |p: Progress| {
            let now = start.elapsed().as_secs_f64() * 1e3;
            let _ = p;
            stamps.push(now - last);
            last = now;
        },
    )
    .expect("the export finishes");
    let counters = scope.finish();
    let global = scope.finish_global();
    drop(delivered);
    let total = start.elapsed().as_secs_f64() * 1e3;
    BenchReport {
        name: name.into(),
        units: frames,
        unit: "frames",
        counters,
        global,
        // Per frame on average: progress reports are not one per frame.
        wall_ms: vec![total / f64::from(frames); frames as usize],
        notes: json!({ "fixture": f.summary, "size": "1920x1080", "format": "h264", "progress_reports": stamps.len() }),
    }
}

/// The CoreText-backed fonts the engine's text layers measure and draw with
/// (the font service gives the app's renderer the same).
#[derive(Default)]
struct Fonts(Option<pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>>);

// SAFETY: the benches run on one thread (the main thread: the CoreText
// registry uses AppKit's font manager); the renderer never sends its fonts
// to another thread.
unsafe impl Send for Fonts {}

impl Fonts {
    fn new() -> Fonts {
        Fonts(Some(pm_eval::text_measure::CompatMeasure::new(
            pm_text::registry::CoreTextRegistry::new(),
        )))
    }

    fn m(
        &mut self,
    ) -> &mut pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry> {
        self.0.as_mut().expect("fonts")
    }
}

impl pm_render::text::TextFonts for Fonts {
    fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
        self.m()
    }
    fn line_runs(&mut self, d: &pm_text::compat::TextContent) -> Vec<pm_text::compat::LineRuns> {
        self.m().compat().line_runs(d)
    }
    fn text_runs(
        &mut self,
        d: &pm_text::compat::TextContent,
        t: &str,
        tracking: f64,
    ) -> Vec<pm_text::compat::GlyphRun> {
        self.m().compat().text_runs(d, t, tracking).0
    }
    fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
        use pm_text::font::FontResolver;
        self.m().compat().resolver().face(id)
    }
}

/// Typing: 60 keystrokes into a 96 px title, each a new revision of the
/// layer's text drawn paused (the canvas text typing row; the engine's
/// share).
pub fn typing_60(ctx: &Ctx) -> BenchReport {
    let sentence = "The quick brown fox jumps over the lazy dog while editors type captions";
    let doc = fixtures::text_doc("T");
    let mut v = Viewer::new(ctx, &doc, &Default::default());
    v.view.set_fonts(true);
    v.composer
        .renderer()
        .set_text_fonts(Some(Box::new(Fonts::new())));
    let at = ticks(0.0);
    let mut typed = String::from("T");
    let key = |v: &mut Viewer<'_>, typed: &mut String, i: usize| {
        typed.push(sentence.as_bytes()[i % sentence.len()] as char);
        v.patch(vec![leaf_set(
            0,
            &["d", "text"],
            pm_model::Value::String(typed.as_str().into()),
        )]);
        v.frame(at)
    };
    v.frame(at);
    for i in 1..3 {
        key(&mut v, &mut typed, i);
    }
    let scope = Scope::begin();
    let wall = (3..63).map(|i| key(&mut v, &mut typed, i)).collect();
    let m = measured(scope);
    report(
        "typing_60",
        "keys",
        m,
        wall,
        json!({ "font": "SF Pro Display 600, 96 px" }),
    )
}

/// Opening a large project: the 2,000-layer synthetic document's text
/// parsed, given to a new view, routed, and its first frame drawn by a new
/// composer (cold: pipelines and caches are built).
pub fn open_large_2000(ctx: &Ctx) -> BenchReport {
    let f = fixtures::synthetic(Synthetic::new(2000));
    let text = f.doc.to_string();
    let mut v = Viewer::new(
        ctx,
        &fixtures::project(
            1920,
            1080,
            30.0,
            10.0,
            fixtures::solid_fill("#000000"),
            vec![],
        ),
        &f.media,
    );
    let start = Instant::now();
    let scope = Scope::begin();
    let root = pm_model::json::parse(&text).expect("parse");
    v.replace_parsed(root);
    let draw = v.frame(ticks(0.0));
    let m = measured(scope);
    let total = start.elapsed().as_secs_f64() * 1e3;
    let mut r = report(
        "open_large_2000",
        "opens",
        m,
        vec![total],
        json!({
            "fixture": f.summary, "bytes": text.len(), "first_frame_ms": draw
        }),
    );
    r.units = 1;
    r
}

/// Importing a 600,000-vertex model: the first frame of a 3D scene that
/// places it (parse, upload, shadow pass), then 10 frames turning it.
pub fn import_3d_600k(ctx: &Ctx) -> BenchReport {
    import_3d(ctx, "import_3d_600k", 600_000)
}

/// The first frame of a scene placing a `vertices`-vertex model, then 10
/// frames turning it.
pub fn import_3d(ctx: &Ctx, name: &str, vertices: usize) -> BenchReport {
    let dir = fixtures::TempDir::new("model");
    let path = dir.path().join("model.glb");
    fixtures::write_glb(&path, vertices);
    let doc = fixtures::model_doc();
    let media = std::collections::BTreeMap::from([("model".to_owned(), path)]);
    let mut v = Viewer::new(ctx, &doc, &media);
    let scope = Scope::begin();
    let wall = (0..11)
        .map(|k| v.frame(ticks(f64::from(k) / 30.0)))
        .collect();
    let m = measured(scope);
    report(
        name,
        "frames",
        m,
        wall,
        json!({ "vertices": vertices, "first": "import" }),
    )
}
