//! Raster overrides asked a frame ahead (`CompatRasters::ask_ahead`) ask the
//! producer exactly what serial frames ask, in serial order, and deliver the
//! serial frames. The producer here is stateful: each answer draws how many
//! layers it was asked for before, so any change of order, any ask skipped
//! or added, changes the pixels and the ask trace. Review w90 (Sol) found
//! the cases below on the first look-ahead (d2997f70f).

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, ExportFormat, ExportSettings, ProResProfile as WireProRes,
    QualityTier,
};
use pm_render::Renderer;
use pm_render::compat::{
    AheadNeed, CompatAnswer, CompatFormat, CompatRaster, CompatRasters, CompatSource, Lookahead,
    Step,
};
use serde_json::{Value, json};
use support::doc::*;

const W: u32 = 128;
const H: u32 = 64;

/// A stateful producer answered in this process, through the same serial
/// cache and look-ahead the export job's host uses (`Lookahead`).
#[derive(Default)]
struct Counting {
    /// Every ask the producer ran, in order: layer and time.
    asked: Mutex<Vec<(String, u64)>>,
    ahead: Mutex<Lookahead>,
    /// Asks made ahead, answered when asked (in order).
    pending: Mutex<Vec<Vec<AheadNeed>>>,
    /// How many layers were asked ahead.
    ahead_asks: Mutex<usize>,
}

impl Counting {
    fn run(&self, need: &AheadNeed) -> CompatAnswer {
        let mut asked = self.asked.lock().unwrap();
        asked.push((need.layer.clone(), need.seconds.to_bits()));
        let level = (asked.len() * 29 % 256) as u8;
        CompatAnswer::Raster(Arc::new(CompatRaster {
            width: 4,
            height: 4,
            format: CompatFormat::Rgba8Premultiplied,
            anchor: [2.0, 2.0],
            size: [24.0, 24.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            data: Arc::new([level, level, level, 255].repeat(16)),
            window: None,
        }))
    }
    fn trace(&self) -> Vec<(String, u64)> {
        self.asked.lock().unwrap().clone()
    }
}

fn need(composition: &str, layer: &str, seconds: f64, scale: f64) -> AheadNeed {
    AheadNeed {
        composition: composition.into(),
        layer: layer.into(),
        seconds,
        scale,
    }
}

impl CompatRasters for Counting {
    fn producing(&self, source: CompatSource) -> bool {
        source == CompatSource::RasterOverride
    }
    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        _: CompatSource,
        _: Option<pm_protocol::common::PixelRect>,
    ) -> CompatAnswer {
        let need = need(composition, layer, time, scale);
        let step = self.ahead.lock().unwrap().answer(&need);
        match step {
            Step::Answer(answer) => answer,
            Step::Diverged(why) => panic!("{why}"),
            Step::Ask => {
                let answer = self.run(&need);
                self.ahead.lock().unwrap().store(&need, answer.clone());
                answer
            }
            Step::Planned => {
                if !self.ahead.lock().unwrap().has_ready() {
                    // The producer answers a request ahead when it is first
                    // needed: in order, after every earlier one.
                    let next = self.pending.lock().unwrap().remove(0);
                    let answers: Vec<CompatAnswer> = next.iter().map(|n| self.run(n)).collect();
                    self.ahead.lock().unwrap().arrive(answers);
                }
                self.ahead.lock().unwrap().take_planned(&need)
            }
        }
    }
    fn answer_probed(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        _: CompatSource,
        _: Option<pm_protocol::common::PixelRect>,
    ) -> CompatAnswer {
        self.ahead
            .lock()
            .unwrap()
            .probe(&need(composition, layer, time, scale));
        CompatAnswer::Missing
    }
    fn probe_unsafe(&self) {
        self.ahead.lock().unwrap().probe_unsafe();
    }
    fn ask_ahead(&self) {
        let needs = self
            .ahead
            .lock()
            .unwrap()
            .ahead()
            .unwrap_or_else(|why| panic!("{why}"));
        if !needs.is_empty() {
            *self.ahead_asks.lock().unwrap() += needs.len();
            self.pending.lock().unwrap().push(needs);
        }
    }
}

/// Admits whole frames, overlapping or not, and every `.1`th frame in one
/// band (0: never).
struct Gate(bool, u32, std::sync::atomic::AtomicU32);

impl GpuGate for Gate {
    fn admit(&self, _: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        let i = self.2.fetch_add(1, Ordering::Relaxed) + 1;
        let banded = self.1 > 0 && i.is_multiple_of(self.1);
        (!cancel.load(Ordering::Relaxed)).then_some(if banded { 1 } else { units })
    }
    fn finished(&self, _: u64, _: u32, _: f64) {}
    fn abandoned(&self, _: u64) {}
    fn measures(&self) -> bool {
        false
    }
    fn overlaps(&self) -> bool {
        self.0
    }
}

fn settings(format: ExportFormat, fps: u32, first: u64, end: u64) -> ExportSettings {
    let at = |n: u64| Ticks::new(n as i64 * TICKS_PER_SECOND / i64::from(fps)).unwrap();
    let mut s = ExportSettings {
        format,
        alpha: WireAlpha::None,
        quality: QualityTier::Max,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(fps, 1),
        range: TickRange {
            start: at(first),
            end: at(end),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    s.delivery.compat.push(CompatSource::RasterOverride);
    s
}

fn prores_4444() -> ExportFormat {
    ExportFormat::ProRes {
        profile: WireProRes::P4444Xq,
    }
}

type Run = (Vec<Vec<u8>>, Vec<(String, u64)>, usize);

/// Every frame's encoder input, the producer's ask trace and how many of
/// its asks were made ahead, exported serially, with overlapping frames, and
/// with overlapping frames every third of which is admitted in bands.
fn runs(doc: &Value, media: MediaPaths, settings: ExportSettings) -> [Run; 3] {
    [(false, 0), (true, 0), (true, 3)].map(|(overlaps, bands)| {
        let producer = Arc::new(Counting::default());
        let mut renderer = Renderer::headless().unwrap();
        renderer.set_compat_rasters(Some(producer.clone()));
        let job = ExportJob {
            document: Arc::new(load(doc)),
            composition: "Ptest-root".into(),
            settings: settings.clone(),
            media: Arc::new(media.clone()),
            destination: PathBuf::from("/unused.mov"),
            overwrite: Overwrite::Fail,
            mode: None,
            seed: 5,
        };
        let plan = plan_export_for(&job, &renderer).unwrap();
        let out = run_export_into(
            &mut renderer,
            &job,
            plan,
            MemorySink::default(),
            &Gate(overlaps, bands, Default::default()),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let frames = out
            .frames
            .iter()
            .map(|f| format!("{f:?}").into_bytes())
            .collect();
        let ahead = *producer.ahead_asks.lock().unwrap();
        (frames, producer.trace(), ahead)
    })
}

fn rect(id: &str, x: f64, w: f64, dur: f64) -> Value {
    layer(
        id,
        "shape",
        json!({ "shape": "rect", "w": w, "h": 24, "color": "#FFFFFF", "stroke": 0 }),
        x,
        32.0,
        dur,
    )
}

/// The serial asks and frames in every run; `asked_ahead`: whether the
/// overlapping runs asked layers ahead at all (or ran serially).
fn assert_serial(label: &str, runs: [Run; 3], asked_ahead: bool) {
    let [serial, rest @ ..] = runs;
    assert!(serial.0.len() > 2, "{label}: frames");
    assert_eq!(serial.2, 0, "{label}: a serial gate asks nothing ahead");
    for run in rest {
        assert_eq!(serial.1, run.1, "{label}: the producer was asked otherwise");
        assert!(serial.0 == run.0, "{label}: frames differ");
        assert_eq!(run.2 > 0, asked_ahead, "{label}: asked ahead {}", run.2);
    }
}

/// A track matte's consumer C under its matte M under a layer A: serial
/// planning asks C, then M (visited from C), then A. A probe that skips the
/// matte asks M after A.
#[test]
fn a_track_matte_is_asked_where_serial_frames_ask_it() {
    let dur = 0.2;
    let mut consumer = rect("C", 64.0, 100.0, dur);
    consumer["matteSource"] = json!("M");
    let doc = project(
        W,
        H,
        dur,
        no_fill(),
        vec![
            rect("A", 30.0, 20.0, dur),
            rect("M", 64.0, 60.0, dur),
            consumer,
        ],
    );
    let runs = runs(
        &doc,
        MediaPaths::default(),
        settings(prores_4444(), 30, 0, 6),
    );
    assert!(
        runs[0].1.iter().any(|(l, _)| l == "M"),
        "the matte is drawn by the producer"
    );
    assert_serial("matte", runs, false);
}

/// A solid matte asks the producer nothing: its consumer's frames are asked
/// ahead, as serial frames ask them.
#[test]
fn a_solid_matte_keeps_its_frames_asked_ahead() {
    let dur = 0.2;
    let mut consumer = rect("C", 64.0, 100.0, dur);
    consumer["matteSource"] = json!("M");
    let solid = layer(
        "M",
        "solid",
        json!({ "w": 60, "h": 30, "color": "#FFFFFF" }),
        64.0,
        32.0,
        dur,
    );
    let doc = project(
        W,
        H,
        dur,
        no_fill(),
        vec![rect("A", 30.0, 20.0, dur), solid, consumer],
    );
    let runs = runs(
        &doc,
        MediaPaths::default(),
        settings(prores_4444(), 30, 0, 6),
    );
    assert_serial("solid matte", runs, true);
}

/// A precomp held on its own frame rate (15 fps child in a 30 fps parent,
/// keepFrameRate) under a root layer A drawn first: each frame asks A at its
/// time, which clears the serial cache, so the held child layer is asked
/// again every frame though its time stays. Answers kept across frames skip
/// those asks.
#[test]
fn a_held_precomp_is_asked_again_as_serial_frames_ask_it() {
    let dur = 0.2;
    let mut doc = project(W, H, dur, no_fill(), vec![]);
    composition(
        &mut doc,
        "inner",
        W,
        H,
        dur,
        vec![rect("B", 90.0, 30.0, dur)],
    );
    doc["comps"]["inner"]["fps"] = json!(15);
    doc["layers"] = json!([
        layer(
            "pre",
            "precomp",
            json!({ "comp": "inner", "w": W, "h": H, "keepFrameRate": true }),
            64.0,
            32.0,
            dur,
        ),
        rect("A", 30.0, 20.0, dur),
    ]);
    let runs = runs(
        &doc,
        MediaPaths::default(),
        settings(prores_4444(), 30, 0, 6),
    );
    let held = runs[0].1.iter().filter(|(l, _)| l == "B").count();
    assert!(
        held >= 6,
        "the held layer is asked every frame: {:?}",
        runs[0].1
    );
    assert_serial("held precomp", runs, true);
}

/// A source clip written by the exporter: one flat picture in every frame.
fn still_clip(path: &std::path::Path) {
    let frames = 8u64;
    let spec = ExportSpec {
        width: W,
        height: H,
        rate: pm_time::Rate::FPS_24,
        frames,
        codec: Codec::ProRes(ProResProfile::Hq),
        container: Container::Mov,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: path.to_path_buf(),
        overwrite: Overwrite::Replace,
        mode: None,
    };
    let rgba: Vec<f32> = (0..W * H)
        .flat_map(|i| [0.2 + (i % 7) as f32 * 0.05, 0.4, 0.6, 1.0])
        .collect();
    let mut writer = pm_export::export(spec).unwrap();
    for _ in 0..frames {
        writer
            .push_frame(&WorkingFrame {
                width: W,
                height: H,
                rgba: &rgba,
            })
            .unwrap();
    }
    writer.finish().unwrap();
}

/// A subsampled Rec.709 export whose first frame is untouched footage looks
/// along the footage's run before and after the range (the chroma policy),
/// asking the producer for the layers of those frames as it plans them,
/// during the first frame's delivery. A frame drawn ahead of that delivery
/// asks its own layers first.
#[test]
fn a_chroma_scan_asks_where_serial_frames_ask_it() {
    let dir = std::env::temp_dir().join(format!("pm-lookahead-chroma-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("clip.mov");
    still_clip(&clip);
    let dur = 8.0 / 24.0;
    let mut video = layer(
        "V",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        dur,
    );
    video["from"] = json!(0);
    // One counted layer only on the frame before the range, one from its
    // third frame on.
    let mut before = rect("S1", 30.0, 20.0, 1.0 / 24.0);
    before["from"] = json!(0);
    let mut later = rect("S2", 90.0, 20.0, 5.0 / 24.0);
    later["from"] = json!(3.0 / 24.0);
    let mut doc = project(W, H, dur, no_fill(), vec![later, before, video]);
    doc["fps"] = json!(24);
    doc["colorDepth"] = json!(32);
    asset(&mut doc, "clip", "video", "mov", W, H, dur);
    let mut media = MediaPaths::default();
    media.insert("clip", clip);
    let format = ExportFormat::ProRes {
        profile: WireProRes::Hq,
    };
    let runs = runs(&doc, media, settings(format, 24, 1, 8));
    assert!(
        runs[0].1.iter().any(|(l, _)| l == "S1"),
        "the scan before the range asks its layer: {:?}",
        runs[0].1
    );
    assert_serial("chroma scan", runs, false);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Two stateful layers a frame, asked ahead with overlapping frames and
/// around frames admitted in bands: the serial asks and frames.
#[test]
fn layers_asked_ahead_keep_the_serial_order_and_frames() {
    let dur = 0.2;
    let doc = project(
        W,
        H,
        dur,
        no_fill(),
        vec![rect("a", 30.0, 20.0, dur), rect("b", 90.0, 30.0, dur)],
    );
    let runs = runs(
        &doc,
        MediaPaths::default(),
        settings(prores_4444(), 30, 0, 6),
    );
    assert_eq!(runs[0].1.len(), 12, "two layers a frame");
    assert_serial("two layers", runs, true);
}
