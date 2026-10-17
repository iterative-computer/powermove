//! Exports with Blender layers end to end (plan section 17's Blender stage,
//! contract 11) on the installed Blender: the job plans each frame, builds
//! its snapshots, renders the surfaces once through the pinned worker
//! (16-bit RGBA PNG through AgX, stamps off), decodes them, and composites
//! them; the exported pixels equal the f64 composite of the stored surfaces
//! (section 25's Blender surfaces class); a second export finds every
//! surface in the cache; batching a frame's layers gives the same pixels
//! as one invocation per layer; cancel stops Blender; a missing Blender or
//! a missing surface fails the export.
//!
//! Without a Blender 4.5 or newer (`POWERMOVE_BLENDER_PATH`, the
//! Applications folders, `PATH`), the Blender runs are skipped with a note.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_blender::job::{BlenderBackend, BlenderStage, LocalBackend, RenderJob, RenderedFrame};
use pm_blender::{BlenderError, SurfaceCache, decode_surface};
use pm_export::blender::ExportBlender;
use pm_export::render::*;
use pm_protocol::blender::ResolvedDevice;
use pm_protocol::color::{OutputColor, Precision};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{AlphaOutput as WireAlpha, ExportFormat, ExportSettings, QualityTier};
use pm_render::blender::{BlenderRender, SnapshotOptions};
use pm_render::plan::{Content, Source};
use pm_render::{RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;

const W: u32 = 160;
const H: u32 = 90;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-blender-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seconds(s: f64) -> Ticks {
    Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

fn object3d(id: &str, primitive: &str, color: &str, x: f64, dur: f64) -> Value {
    let mut l = layer(
        id,
        "extension",
        json!({
            "definition": "powermove.3d.object", "version": 1, "w": W, "h": H, "params": {},
            "data": { "object": {
                "source": { "primitive": primitive, "parameters": {} },
                "material": { "p": {
                    "color": ch(json!(color)), "roughness": ch(json!(0.4)), "metalness": ch(json!(0.1)),
                    "emissive": ch(json!("#000000")), "emissiveIntensity": ch(json!(1)), "opacity": ch(json!(1))
                }, "maps": {}, "doubleSided": true },
                "useSourceMaterials": false, "slots": [], "castShadow": true, "receiveShadow": true
            } }
        }),
        x,
        0.0,
        dur,
    );
    l["threeD"] = json!(true);
    for k in ["scale.x", "scale.y", "scale.z"] {
        set(&mut l, k, json!(100));
    }
    l
}

fn sun(dur: f64) -> Value {
    let mut l = layer(
        "sun",
        "extension",
        json!({
            "definition": "powermove.3d.light", "version": 1, "w": 0, "h": 0, "params": {},
            "data": { "light": { "type": "sun", "castShadow": true, "p": {
                "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
                "color": ch(json!("#FFFFFF")), "intensity": ch(json!(3)), "distance": ch(json!(0)),
                "decay": ch(json!(2)), "angle": ch(json!(45)), "penumbra": ch(json!(0.4)),
                "width": ch(json!(1)), "height": ch(json!(1))
            } } }
        }),
        -3.0,
        5.0,
        dur,
    );
    l["threeD"] = json!(true);
    set(&mut l, "position.z", json!(4));
    l
}

fn camera(dur: f64) -> Value {
    let mut l = layer(
        "cam",
        "extension",
        json!({
            "definition": "powermove.3d.camera", "version": 1, "w": 0, "h": 0, "params": {},
            "data": {
                "camera": { "projection": "perspective", "p": {
                    "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
                    "fov": ch(json!(45)), "zoom": ch(json!(1)), "near": ch(json!(0.01)), "far": ch(json!(1000))
                } },
                "environment": { "p": {
                    "ambient": ch(json!(0.5)), "ambientColor": ch(json!("#FFFFFF")), "exposure": ch(json!(1))
                }, "background": null, "shadows": true }
            }
        }),
        0.0,
        0.0,
        dur,
    );
    l["threeD"] = json!(true);
    set(&mut l, "position.z", json!(4));
    l
}

/// Two models, one turning, Blender (EEVEE) on.
fn scene(dur: f64, engine: &str, samples: u32) -> Value {
    let mut cube = object3d("cube", "box", "#FF8040", -0.7, dur);
    set(&mut cube, "rotation.y", keys(json!(10), &[(0.0, json!(10)), (dur, json!(70))]));
    let ball = object3d("ball", "sphere", "#4080FF", 0.7, dur);
    let mut doc = project(W, H, dur, solid_fill("#182028"), vec![camera(dur), sun(dur), cube, ball]);
    doc["render3d"] = json!({ "enabled": true, "engine": engine, "samples": samples, "device": "cpu" });
    doc
}

fn settings(frames: u32) -> ExportSettings {
    ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: true },
        alpha: WireAlpha::None,
        quality: QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: seconds(f64::from(frames) / 30.0),
        },
        audio: false,
        motion_blur: false,
        color: OutputColor::SrgbImage,
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

fn job(doc: &Value, destination: PathBuf, frames: u32) -> ExportJob {
    ExportJob {
        document: Arc::new(load(doc)),
        composition: "Ptest-root".into(),
        settings: settings(frames),
        media: Arc::new(MediaPaths::default()),
        destination,
        overwrite: pm_export::settings::Overwrite::Replace,
        mode: Some(0o600),
        seed: 1,
    }
}

fn local(scratch: &Path) -> Option<LocalBackend> {
    match LocalBackend::discover(scratch) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("skipped: no Blender ({e})");
            None
        }
    }
}

fn export(renderer: &mut Renderer, blender: &mut ExportBlender, job: &ExportJob) -> Result<(), JobError> {
    let plan = plan_export_with(job, true)?;
    let sink = sink_for(&plan)?;
    run_export_with_blender(renderer, job, plan, sink, &Unrestricted, &AtomicBool::new(false), &mut |_| {}, Some(blender))
        .map(|_| ())
}

/// 16-bit RGBA PNG samples, top-down.
fn read16(path: &Path) -> (u32, u32, Vec<u16>) {
    let mut d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    d.set_transformations(png::Transformations::IDENTITY);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!(info.bit_depth, png::BitDepth::Sixteen);
    let ch = info.color_type.samples();
    let mut out = Vec::new();
    for px in buf[..info.buffer_size()].chunks_exact(2 * ch) {
        for c in 0..4 {
            out.push(if c < ch { u16::from_be_bytes([px[2 * c], px[2 * c + 1]]) } else { 65535 });
        }
    }
    (info.width, info.height, out)
}

fn png_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    v.sort();
    v
}

#[test]
fn an_export_with_blender_layers_composites_its_surfaces_exactly() {
    let dir = scratch("e2e");
    let Some(backend) = local(&dir) else { return };
    let mut renderer = Renderer::headless().unwrap();
    let mut blender = ExportBlender::new(
        BlenderStage::new(SurfaceCache::new(256 << 20, Some(dir.join("cache")))),
        Box::new(backend),
    );
    let doc = scene(2.0, "eevee", 4);
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let started = Instant::now();
    export(&mut renderer, &mut blender, &job(&doc, out.join("seq"), 2)).unwrap();
    let first = started.elapsed();
    let stats = blender.stage.stats;
    eprintln!("first export: {first:?}, {stats:?}, devices {:?}", blender.stage.devices);
    assert_eq!(stats.jobs, 2, "one Blender request per frame (layers batched)");
    assert_eq!(stats.rendered, 4);
    assert!(blender
        .stage
        .devices
        .iter()
        .all(|d| matches!(d, ResolvedDevice::Metal { .. })), "{:?}", blender.stage.devices);
    // The same export again: every surface from the cache, Blender not run.
    let again = Instant::now();
    export(&mut renderer, &mut blender, &job(&doc, out.join("seq"), 2)).unwrap();
    eprintln!("second export: {:?}, {:?}", again.elapsed(), blender.stage.stats);
    assert_eq!(blender.stage.stats.jobs, 2);
    assert_eq!(blender.stage.stats.cached, 4);
    // Exactness: each frame equals the f64 composite of its stored surfaces.
    let files = png_files(&out.join("seq"));
    assert_eq!(files.len(), 2, "{files:?}");
    let document = load(&doc);
    let request = {
        let mut r = RenderRequest::sized(Size::new(W, H), Precision::Float32);
        r.export = true;
        r
    };
    for (n, file) in files.iter().enumerate() {
        let t = seconds(n as f64 / 30.0);
        let surfaces = blender
            .prepare(&mut renderer, &document, "Ptest-root", t, &request, &MediaPaths::default(), &AtomicBool::new(false), &mut |_, _| {})
            .unwrap()
            .unwrap();
        let mut req = request.clone();
        req.blender = BlenderRender::Final(surfaces.clone());
        let (plan, _) = renderer.plan(&document, "Ptest-root", t, &req, &MediaPaths::default()).unwrap();
        let frame = surfaces.values().next().unwrap();
        let order: Vec<String> = plan
            .draws
            .iter()
            .filter(|d| matches!(&d.content, Content::Texture { source: Source::Blender(_), .. }))
            .map(|d| d.layer.clone())
            .collect();
        assert_eq!(order.len(), 2);
        let (w, h, got) = read16(file);
        assert_eq!((w, h), (W, H));
        let bg = [24.0 / 255.0, 32.0 / 255.0, 40.0 / 255.0, 1.0];
        let mut max = 0u16;
        let mut covered = 0usize;
        for y in 0..H {
            for x in 0..W {
                let mut c: [f64; 4] = bg;
                for id in &order {
                    let p = frame[id].pixel(x, y);
                    if p[3] > 0.0 {
                        covered += 1;
                    }
                    for k in 0..4 {
                        c[k] = f64::from(p[k]) + c[k] * (1.0 - f64::from(p[3]));
                    }
                }
                let i = ((y * W + x) * 4) as usize;
                for k in 0..4 {
                    let want = (c[k].clamp(0.0, 1.0) * 65535.0).round() as i64;
                    max = max.max((i64::from(got[i + k]) - want).unsigned_abs() as u16);
                }
            }
        }
        eprintln!("frame {n}: max |export − composite of stored surfaces| = {max} of 65535; {covered} covered samples");
        assert!(covered > 500, "Blender drew the models");
        assert!(max <= 1, "{max}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batched_and_per_layer_surfaces_are_equal_and_byte_stable() {
    let dir = scratch("batch");
    let Some(mut backend) = local(&dir) else { return };
    let mut renderer = Renderer::headless().unwrap();
    let doc = load(&scene(1.0, "eevee", 4));
    let request = RenderRequest::sized(Size::new(W, H), Precision::Float32);
    let snaps = renderer
        .blender_snapshots(&doc, "Ptest-root", Ticks::ZERO, &request, &MediaPaths::default(), false, SnapshotOptions::default())
        .unwrap();
    let snapshot = &snaps.snapshots[0];
    let settings = pm_blender::job::snapshot_settings(snapshot).unwrap();
    let render = |backend: &mut LocalBackend, batch: bool, id: &str| -> RenderedFrame {
        backend
            .render(
                &RenderJob {
                    id: id.into(),
                    snapshot,
                    layers: snapshot.layers(),
                    settings: settings.clone(),
                    batch,
                    keys: vec![],
                },
                &AtomicBool::new(false),
            )
            .unwrap()
    };
    let bytes = |f: &RenderedFrame, layer: &str| match &f.surfaces.iter().find(|s| s.layer == layer).unwrap().png {
        pm_blender::job::SurfacePng::Bytes(b) => b.clone(),
        pm_blender::job::SurfacePng::File(p) => std::fs::read(p).unwrap(),
    };
    let batched = render(&mut backend, true, "batched");
    let again = render(&mut backend, true, "again");
    let single = render(&mut backend, false, "single");
    for layer in snapshot.layers() {
        let a = decode_surface(&bytes(&batched, &layer)).unwrap();
        let b = decode_surface(&bytes(&single, &layer)).unwrap();
        assert_eq!(a.bit_depth, 16);
        assert_eq!(a, b, "{layer}: batched and per-layer surfaces differ");
        assert_eq!(bytes(&batched, &layer), bytes(&again, &layer), "{layer}: not byte-stable");
        let chunks: Vec<String> = pm_blender_chunks(&bytes(&batched, &layer));
        assert!(!chunks.iter().any(|c| c == "tEXt" || c == "iTXt" || c == "zTXt"), "{layer}: {chunks:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn pm_blender_chunks(png: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 8;
    while i + 8 <= png.len() {
        let n = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&png[i + 4..i + 8]).into_owned());
        i += 12 + n;
    }
    out
}

#[test]
fn cancel_stops_blender() {
    let dir = scratch("cancel");
    let Some(backend) = local(&dir) else { return };
    let mut renderer = Renderer::headless().unwrap();
    let mut blender = ExportBlender::new(BlenderStage::new(SurfaceCache::new(64 << 20, None)), Box::new(backend));
    // A slow render: Cycles on the CPU with many samples at a larger size.
    let doc = scene(1.0, "cycles", 4096);
    let document = load(&doc);
    let request = RenderRequest::sized(Size::new(W * 12, H * 12), Precision::Float32);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        flag.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let r = blender.prepare(&mut renderer, &document, "Ptest-root", Ticks::ZERO, &request, &MediaPaths::default(), &cancel, &mut |_, _| {});
    canceller.join().unwrap();
    let took = started.elapsed();
    assert!(matches!(r, Err(JobError::Cancelled)), "{:?}", r.err());
    assert!(took < Duration::from_secs(10), "cancel took {took:?}");
    // The worker's process group is gone.
    let pids = std::process::Command::new("/bin/ps").args(["-axo", "pid,command"]).output().unwrap();
    let running = String::from_utf8_lossy(&pids.stdout)
        .lines()
        .filter(|l| l.contains(&dir.to_string_lossy().to_string()) && l.contains("worker.py"))
        .count();
    assert_eq!(running, 0, "Blender still runs after cancel");
    std::fs::remove_dir_all(dir).unwrap();
}

struct NoBlender;

impl BlenderBackend for NoBlender {
    fn version(&mut self) -> Result<String, BlenderError> {
        Err(BlenderError::Missing(pm_blender::BLENDER_MISSING_MESSAGE.into()))
    }
    fn render(&mut self, _: &RenderJob<'_>, _: &AtomicBool) -> Result<RenderedFrame, BlenderError> {
        unreachable!()
    }
}

/// Renders every layer but the last.
struct Dropping(Mutex<Vec<String>>);

impl BlenderBackend for Dropping {
    fn version(&mut self) -> Result<String, BlenderError> {
        Ok("4.5.14".into())
    }
    fn render(&mut self, job: &RenderJob<'_>, _: &AtomicBool) -> Result<RenderedFrame, BlenderError> {
        self.0.lock().unwrap().extend(job.layers.iter().cloned());
        let (w, h) = job.snapshot.size();
        Ok(RenderedFrame {
            surfaces: job.layers[..job.layers.len() - 1]
                .iter()
                .map(|l| pm_blender::job::RenderedSurface {
                    layer: l.clone(),
                    png: pm_blender::job::SurfacePng::Bytes(pm_blender::surface::encode_rgba16(w, h, &vec![30000; (w * h * 4) as usize], &[])),
                })
                .collect(),
            device: ResolvedDevice::Cpu,
        })
    }
}

#[test]
fn a_missing_blender_or_surface_fails_the_export_and_nothing_is_routed() {
    let dir = scratch("missing");
    let mut renderer = Renderer::headless().unwrap();
    let doc = scene(1.0, "eevee", 4);
    let mut none = ExportBlender::new(BlenderStage::new(SurfaceCache::new(1 << 20, None)), Box::new(NoBlender));
    let e = export(&mut renderer, &mut none, &job(&doc, dir.join("a"), 1)).unwrap_err();
    assert!(matches!(&e, JobError::Blender(BlenderError::Missing(m)) if m == pm_blender::BLENDER_MISSING_MESSAGE), "{e:?}");
    let mut dropping = ExportBlender::new(BlenderStage::new(SurfaceCache::new(1 << 20, None)), Box::new(Dropping(Mutex::new(vec![]))));
    let e = export(&mut renderer, &mut dropping, &job(&doc, dir.join("b"), 1)).unwrap_err();
    assert!(matches!(&e, JobError::Blender(BlenderError::Failed(m)) if m.contains("did not render the surface")), "{e:?}");
    let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    assert!(left.is_empty(), "a failed export leaves nothing: {left:?}");
    // Without a Blender stage the job is routed, as before.
    assert!(matches!(plan_export(&job(&doc, dir.join("c.png"), 1)), Err(JobError::Unsupported(_))));
    std::fs::remove_dir_all(dir).unwrap();
}
