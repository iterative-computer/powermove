//! Cross-product export isolation: viewer representations, resolutions,
//! Blender preview surfaces and sustained lease pressure share one renderer.
#![cfg(target_os = "macos")]
#[path = "../../pm-render/tests/support/doc.rs"]
mod doc;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};
use pm_blender::{DecodedSurface, snapshot::Snapshot};
use pm_gpu::bridge_out::display::WorkingTexture;
use pm_node::present::{compose::Composer, document::MediaMap};
use pm_protocol::color::{Precision, ViewerColorSpace};
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::frame::{MediaPreference, PreviewResolution, ViewerState};
use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_render::blender::{BlenderRender, PreviewSurfaces, SnapshotOptions};
use pm_render::RenderRequest;
use serde_json::json;

const W: u32 = 192;
const H: u32 = 108;

struct Preview(Arc<DecodedSurface>);
impl PreviewSurfaces for Preview {
    fn options(&self) -> SnapshotOptions { SnapshotOptions { force_eevee: true, ..Default::default() } }
    fn lookup(&self, _: &Snapshot, _: &str) -> Option<Arc<DecodedSurface>> { Some(self.0.clone()) }
    fn wanted(&self, _: Snapshot) { panic!("the preview test surface must already be available") }
}

fn surface(color: [u16; 4]) -> Arc<DecodedSurface> {
    let pixels: Vec<u16> = (0..W * H).flat_map(|_| color).collect();
    Arc::new(pm_blender::decode_surface(&pm_blender::surface::encode_rgba16(W, H, &pixels, &[])).unwrap())
}

#[test]
fn full_export_is_identical_after_every_viewer_cache_contamination_input() {
    let temp = tempfile::tempdir().unwrap();
    let ffmpeg = std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let source = temp.path().join("original.mov");
    assert!(std::process::Command::new(ffmpeg.join("bin/ffmpeg")).args([
        "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=24", "-frames:v", "24",
        "-c:v", "prores_videotoolbox", "-profile:v", "2", "-pix_fmt", "yuv422p10le", "-y"
    ]).arg(&source).status().unwrap().success());
    let cancel = AtomicBool::new(false);
    let hash = pm_media::hash::hash_file(&source, pm_media::hash::Settle { quiet: Duration::ZERO, poll: Duration::from_millis(1) }, &cancel, &|_| {}).unwrap().hex();
    let store = pm_proxy::ProxyStore::open(temp.path().join("generated")).unwrap();
    // A deterministic host quota isolates pixel-cache validation from other
    // lanes' build space. The real-volume guard is exercised independently.
    let proxy = pm_proxy::generate_with_disk_probe(pm_proxy::GenerationRequest {
        source: &source, source_hash: &hash, format: ProxyFormat::ProResProxy, size: ProxySize::Half,
        ffmpeg: &ffmpeg.join("bin/ffmpeg"), ffprobe: &ffmpeg.join("bin/ffprobe"), store: &store,
        control: pm_proxy::GenerationControl { cancel: &cancel, pause: &|| false, on_pause: &|_| {} },
    }, &|_| Ok((40u64 << 30, 100u64 << 30))).unwrap();
    assert!(proxy.stored.path.exists());
    let locations = temp.path().join("locations.json");
    std::fs::write(&locations, serde_json::to_vec(&json!({"roots":[store.root()]})).unwrap()).unwrap();
    let mut value = doc::project(W, H, 1.0, doc::solid_fill("#102030"), vec![
        doc::camera("camera", [0.0, 0.0, 4.0], false, 1.0),
        doc::object3d("object", "box", json!({}), "#40FF80", W, H, 1.0),
        doc::layer("video", "video", json!({"asset":"clip","w":W,"h":H,"fit":"stretch","trim":0,"speed":1}), W as f64 / 2.0, H as f64 / 2.0, 1.0),
    ]);
    value["render3d"] = json!({"enabled":true,"engine":"eevee","samples":16});
    doc::asset(&mut value, "clip", "video", "mov", 160, 90, 1.0);
    value["assets"]["clip"]["contentHash"] = json!(hash);
    value["media"] = json!({"proxies":{"size":"half"}});
    let document = doc::load(&value);
    let media = MediaMap(Arc::new(BTreeMap::from([("clip".into(), Some(source))])));
    let gpu = pm_gpu::Gpu::new("export contamination matrix").unwrap();
    let mut composer = Composer::new(&gpu.device, &gpu.queue);
    composer.set_proxy_locations(locations);
    composer.set_cache_budget(32 << 20);
    composer.set_preview_limit(16 << 20);
    let working = WorkingTexture::new(&gpu.device, Size::new(W, H));
    let at = Ticks::new(705_600_000 / 4).unwrap();
    let mut export = RenderRequest::sized(Size::new(W, H), Precision::Float32);
    export.export = true;
    let snapshots = composer.renderer().blender_snapshots(&document, "Ptest-root", at, &export, &media, false, SnapshotOptions::default()).unwrap();
    assert!(snapshots.errors.is_empty(), "{:?}", snapshots.errors);
    assert!(!snapshots.snapshots.is_empty());
    let mut final_surfaces = pm_blender::job::Surfaces::new();
    for snapshot in &snapshots.snapshots {
        final_surfaces.insert(snapshot.frame.lookup(), snapshot.layers().into_iter().map(|layer| (layer.to_string(), surface([0, 65535, 0, 32768]))).collect::<HashMap<_, _>>());
    }
    export.blender = BlenderRender::Final(Arc::new(final_surfaces));
    let read_export = |composer: &mut Composer| {
        let renderer = composer.renderer();
        let frame = renderer.render(&document, "Ptest-root", at, &export, &media).unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        renderer.read_rgba(&frame).unwrap().into_iter().flat_map(f32::to_le_bytes).collect::<Vec<u8>>()
    };
    let clean = read_export(&mut composer);
    let mut cells = 0;
    let mut proxy_frames = 0;
    for resolution in [PreviewResolution::Automatic, PreviewResolution::Half, PreviewResolution::Quarter] {
        for preference in [MediaPreference::Automatic, MediaPreference::Originals, MediaPreference::Proxies] {
            for blender_preview in [false, true] {
                for pressure in [false, true] {
                    composer.set_cache_budget(if pressure { 1 << 20 } else { 32 << 20 });
                    composer.set_blender(if blender_preview { BlenderRender::Preview(Arc::new(Preview(surface([65535, 0, 0, 65535])))) } else { BlenderRender::Native });
                    composer.set_playing(true);
                    let state = ViewerState {
                        headless: None, composition: "Ptest-root".into(), size: Size::new(W, H), device_pixel_ratio: 1.0,
                        zoom: 1.0, pan: [0.0, 0.0], resolution, media: preference, color_space: ViewerColorSpace::Srgb,
                        view: None, play_every_frame: false, edr: false, view_seq: Seq::new(1).unwrap(), vsync: None, depth_id: None, canvas: None,
                    };
                    composer.compose(&document, "Ptest-root", &media, at, &state, &working).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while composer.has_pending_proxy_lookups() && Instant::now() < deadline {
                        composer.poll_proxy_updates(); std::thread::sleep(Duration::from_millis(2));
                    }
                    for frame in 0..12 {
                        let time = Ticks::new(at.get() + frame * 705_600_000 / 24).unwrap();
                        let drawn = composer.compose(&document, "Ptest-root", &media, time, &state, &working).unwrap();
                        assert!(drawn.failures.is_empty(), "{:?}", drawn.failures);
                        proxy_frames += usize::from(drawn.representation == pm_protocol::frame::Representation::Proxy);
                    }
                    assert!(composer.preview_cache_stats().bytes <= composer.preview_cache_stats().budget);
                    assert_eq!(read_export(&mut composer), clean, "export contaminated by {resolution:?}, {preference:?}, Blender preview {blender_preview}, pressure {pressure}");
                    cells += 1;
                }
            }
        }
    }
    assert_eq!(cells, 36);
    assert!(proxy_frames > 0, "the matrix must actually draw a generated proxy");
    println!("export contamination: {cells} cells, {proxy_frames} proxy frames; byte-identical original/final exports");
}
