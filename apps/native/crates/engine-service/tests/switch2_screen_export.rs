//! Headless delivery regression using a locally supplied, read-only project.

use pm_engine_service::studio::{self, Preview, SceneRequest};
use pm_export::render::{
    ExportJob, MediaPaths, Unrestricted, plan_export_for, run_export_into, sink_for,
};
use pm_export::settings::Overwrite;
use pm_kernel::project_io::open_project;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag, WorkingEncoding};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{AlphaOutput, ExportFormat, ExportSettings, QualityTier};
use pm_render::text::host::RenderFonts;
use pm_render::{RenderRequest, Renderer};
use pm_text::host::{FontBroker, FontService, serve};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

const SIZE: Size = Size::new(960, 540);
const FPS: i64 = 60;
const FRAMES: i64 = 3;
const RGB_BYTES: usize = (SIZE.width * SIZE.height * 3) as usize;

fn main() {
    let Some(project_path) = std::env::var_os("PM_SWITCH2_PROJECT") else {
        println!("ignored: supply PM_SWITCH2_PROJECT and PM_SWITCH2_SCREEN_CAPTURES");
        return;
    };
    let output = PathBuf::from(
        std::env::var_os("PM_SWITCH2_SCREEN_CAPTURES").expect("PM_SWITCH2_SCREEN_CAPTURES"),
    );
    let mut service = FontService::new().unwrap();
    let broker = FontBroker::new();
    broker.set_wake(Arc::new(|| {}));
    broker.set_generation(service.generation());
    let fonts = RenderFonts::new(broker.clone());
    let scenario =
        std::thread::spawn(move || compare_delivery(Path::new(&project_path), &output, fonts));
    while !scenario.is_finished() {
        pm_text::system_fonts::pump();
        serve(&mut service, broker.take_asks());
        std::thread::sleep(Duration::from_millis(1));
    }
    scenario.join().unwrap();
    println!("test switch2_delivery_retains_screen ... ok");
    println!("test result: ok. 1 passed; 0 failed; 0 ignored");
}

fn compare_delivery(project_path: &Path, output: &Path, fonts: RenderFonts) {
    std::fs::create_dir_all(output).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let project = open_project(project_path, Default::default()).unwrap();
    let comp = project.open_comp.unwrap();
    let doc = Arc::new(project.document);
    let assets = pm_eval::node::member(doc.root(), "assets");
    let mut media = MediaPaths::default();
    for (index, entry) in project.file.media.iter().enumerate() {
        let id = entry.id.to_string_lossy();
        let asset = pm_eval::node::member(assets, &id);
        let name = pm_eval::node::string(pm_eval::node::member(asset, "name"))
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let mut path = scratch.path().join(index.to_string());
        if let Some(extension) = Path::new(&name).extension() {
            path.set_extension(extension);
        }
        entry.copy_to(&path).unwrap();
        media.insert(id, path);
    }
    let media = Arc::new(media);
    // The black opening cover fades out at 2.73 s, so also check a range with
    // the artwork exposed. The first frame in each range uses a fresh export.
    let mut scores = Vec::new();
    for (first, name) in [(0, "opening"), (64, "reported-shot"), (268, "artwork")] {
        let output = output.join(name);
        std::fs::create_dir_all(&output).unwrap();
        scores.extend(compare_range(&doc, &comp, &media, &fonts, first, &output));
    }
    for (index, score) in &scores {
        eprintln!("frame {index}: {score:?}");
    }
    for (index, score) in scores {
        if index >= 268 {
            assert!(
                score.viewer_artwork > 1000 && score.export_artwork > 1000,
                "screen artwork missing in frame {index}: {score:?}"
            );
            assert!(
                score.artwork_error < 0.14,
                "encoded artwork differs in frame {index}: {score:?}"
            );
        }
        if index < FRAMES {
            assert!(
                score.viewer_dark > 0.6 && score.export_dark > 0.6,
                "black opening cover missing in frame {index}: {score:?}"
            );
        }
        // The screen crop is defined for the two front views. The rotating
        // shot supplies an encoded capture at the Fast regression's time.
        if index < FRAMES || index >= 268 {
            assert!(
                score.rgb_error < 0.09,
                "export screen differs in frame {index}: {score:?}"
            );
        }
    }
}

fn compare_range(
    doc: &Arc<pm_model::Document>,
    comp: &str,
    media: &Arc<MediaPaths>,
    fonts: &RenderFonts,
    first: i64,
    output: &Path,
) -> Vec<(i64, ScreenScores)> {
    let at = |index| Ticks::new(index * (TICKS_PER_SECOND / FPS)).unwrap();
    let scene = SceneRequest {
        composition: comp.to_owned(),
        revision: doc.revision(),
        ticks: at(first).get(),
        width: SIZE.width,
        height: SIZE.height,
    };
    let mut viewer = Renderer::headless().unwrap();
    viewer.set_document_effects(doc, &[]).unwrap();
    fonts.sync(&mut viewer, &mut None);
    studio::attach_with_camera_fonts(
        &mut viewer,
        doc.clone(),
        media,
        &scene,
        Preview::Live,
        None::<studio::FreeView>.into(),
        Some(fonts),
    )
    .unwrap();
    let mut request = RenderRequest::sized(SIZE, Precision::Float16);
    request.viewer = true;
    let working = pm_render::composite::project_color(doc.root().unwrap())
        .unwrap()
        .working();
    let mut reference = Vec::new();
    for index in first..first + FRAMES {
        let frame = viewer
            .render(doc, comp, at(index), &request, media.as_ref())
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        reference.push(encode_display(&viewer.read_rgba(&frame).unwrap(), working));
    }
    drop(viewer);

    let settings = ExportSettings {
        format: ExportFormat::H264,
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: SIZE,
        frame_rate: FrameRate::new(FPS as u32, 1),
        range: TickRange {
            start: at(first),
            end: at(first + FRAMES),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: true,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let destination = output.join("delivery.mp4");
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_document_effects(doc, &[]).unwrap();
    fonts.sync(&mut renderer, &mut None);
    studio::attach_with_export_options_fonts(
        &mut renderer,
        doc.clone(),
        media,
        comp,
        None,
        1.,
        Some(fonts),
    )
    .unwrap();
    let job = ExportJob {
        document: doc.clone(),
        composition: comp.to_owned(),
        settings,
        media: media.clone(),
        destination: destination.clone(),
        overwrite: Overwrite::Replace,
        mode: Some(0o600),
        seed: 1,
    };
    let plan = plan_export_for(&job, &renderer).unwrap();
    let sink = sink_for(&plan).unwrap();
    run_export_into(
        &mut renderer,
        &job,
        plan,
        sink,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let decoded = Command::new("ffmpeg")
        .env_remove("DYLD_LIBRARY_PATH")
        .args(["-v", "error", "-i"])
        .arg(&destination)
        .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(decoded.stdout.len(), RGB_BYTES * FRAMES as usize);

    reference
        .iter()
        .zip(decoded.stdout.chunks_exact(RGB_BYTES))
        .enumerate()
        .map(|(offset, (viewer, exported))| {
            let index = first + offset as i64;
            save_rgb(&output.join(format!("viewer-{index:03}.png")), viewer);
            save_rgb(&output.join(format!("export-{index:03}.png")), exported);
            (index, screen_scores(viewer, exported))
        })
        .collect()
}

fn encode_display(rgba: &[f32], working: WorkingEncoding) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|pixel| {
            std::array::from_fn::<_, 3, _>(|channel| {
                let value = pixel[channel];
                assert!(value.is_finite(), "nonfinite viewer channel");
                let encoded = pm_color::transfer::working_to_display(working, f64::from(value));
                (encoded.clamp(0., 1.) * 255.).round() as u8
            })
        })
        .collect()
}

fn save_rgb(path: &Path, bytes: &[u8]) {
    let mut png = png::Encoder::new(
        std::fs::File::create(path).unwrap(),
        SIZE.width,
        SIZE.height,
    );
    png.set_color(png::ColorType::Rgb);
    png.write_header().unwrap().write_image_data(bytes).unwrap();
}

#[derive(Debug)]
struct ScreenScores {
    viewer_artwork: usize,
    export_artwork: usize,
    viewer_dark: f64,
    export_dark: f64,
    rgb_error: f64,
    artwork_error: f64,
}

fn screen_scores(viewer: &[u8], exported: &[u8]) -> ScreenScores {
    // The display interior in the two front views. Exclude the bezel and
    // controllers so their colors cannot satisfy the artwork check.
    let (left, right, top, bottom) = (250, 690, 160, 390);
    let pixels = (right - left) * (bottom - top);
    let classify = |pixel: &[u8]| {
        let max = *pixel.iter().max().unwrap();
        let min = *pixel.iter().min().unwrap();
        (max > 70 && max - min > 40, max < 24)
    };
    let mut viewer_artwork = 0;
    let mut export_artwork = 0;
    let mut viewer_dark = 0;
    let mut export_dark = 0;
    let mut error = 0u64;
    let mut artwork_error = 0u64;
    for y in top..bottom {
        for x in left..right {
            let i = (y * SIZE.width as usize + x) * 3;
            let preview = &viewer[i..i + 3];
            let delivery = &exported[i..i + 3];
            let (preview_art, preview_dark) = classify(preview);
            let (delivery_art, delivery_dark) = classify(delivery);
            viewer_artwork += usize::from(preview_art);
            export_artwork += usize::from(delivery_art);
            viewer_dark += usize::from(preview_dark);
            export_dark += usize::from(delivery_dark);
            let delta = preview
                .iter()
                .zip(delivery)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum::<u64>();
            error += delta;
            if preview_art {
                artwork_error += delta;
            }
        }
    }
    ScreenScores {
        viewer_artwork,
        export_artwork,
        viewer_dark: viewer_dark as f64 / pixels as f64,
        export_dark: export_dark as f64 / pixels as f64,
        rgb_error: error as f64 / (255 * 3 * pixels) as f64,
        artwork_error: artwork_error as f64 / (255 * 3 * viewer_artwork.max(1)) as f64,
    }
}
