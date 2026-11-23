//! Opt-in, headless regression for the private Switch 2 Rig fixture.

use pm_engine_service::studio::{self, Preview, SceneRequest};
use pm_export::render::{MediaPaths, asset_ids};
use pm_kernel::project_io::{OpenOptions, open_project};
use pm_protocol::{
    color::{Precision, WorkingEncoding},
    common::{Size, Ticks},
};
use pm_render::{OutputKind, RenderRequest, Renderer, text::host::RenderFonts};
use pm_text::host::{FontBroker, FontService, serve};
use std::{path::Path, sync::Arc};

const FULL_SIZE: Size = Size::new(1920, 1080);
const FAST_SIZE: Size = Size::new(FULL_SIZE.width / 2, FULL_SIZE.height / 2);

fn main() {
    let Some(project) = std::env::var_os("PM_SWITCH2_PROJECT") else {
        println!("ignored: supply PM_SWITCH2_PROJECT and PM_SWITCH2_CAPTURES");
        return;
    };
    let captures = std::env::var_os("PM_SWITCH2_CAPTURES").expect("PM_SWITCH2_CAPTURES");
    let mut service = FontService::new().unwrap();
    let broker = FontBroker::new();
    broker.set_wake(Arc::new(|| {}));
    broker.set_generation(service.generation());
    let fonts = RenderFonts::new(broker.clone());
    let comparison = std::thread::spawn(move || {
        switch2_fast_preview_matches_full(Path::new(&project), Path::new(&captures), &fonts);
    });
    while !comparison.is_finished() {
        pm_text::system_fonts::pump();
        serve(&mut service, broker.take_asks());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    comparison.join().unwrap();
    println!("test switch2_fast_preview_matches_full ... ok");
    println!("test result: ok. 1 passed; 0 failed; 0 ignored");
}

fn switch2_fast_preview_matches_full(file: &Path, output: &Path, fonts: &RenderFonts) {
    std::fs::create_dir_all(output).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let project = open_project(file, OpenOptions { verify_media: true }).unwrap();
    let comp = project.open_comp.unwrap();
    let document = Arc::new(project.document);
    let working = pm_render::composite::project_color(document.root().unwrap())
        .unwrap()
        .working();
    let mut media = MediaPaths::default();
    for (index, asset) in asset_ids(&document).into_iter().enumerate() {
        let Some(entry) = project.file.media.get(&asset) else {
            continue;
        };
        let record = document
            .root()
            .and_then(|root| root.get("assets"))
            .and_then(|assets| assets.get(&asset));
        let value = record.map(pm_model::Node::to_value);
        let name = value
            .as_ref()
            .and_then(|asset| asset.get("name"))
            .and_then(pm_model::Value::as_str)
            .unwrap_or("");
        let mut path = temp.path().join(index.to_string());
        if let Some(extension) = Path::new(name).extension() {
            path.set_extension(extension);
        }
        entry.copy_to(&path).unwrap();
        media.insert(asset, path);
    }
    let frame: i64 = std::env::var("PM_SWITCH2_FRAME")
        .unwrap_or_else(|_| "64".into())
        .parse()
        .unwrap();
    let ticks = Ticks::new(frame * (pm_protocol::common::TICKS_PER_SECOND / 60)).unwrap();
    let render = |size: Size, preview: Preview, export: bool, name: &str| {
        let mut renderer = Renderer::headless().unwrap();
        let refused = renderer.set_document_effects(&document, &[]).unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        let scene = SceneRequest {
            composition: comp.clone(),
            revision: document.revision(),
            ticks: ticks.get(),
            width: size.width,
            height: size.height,
        };
        fonts.sync(&mut renderer, &mut None);
        studio::attach_with_camera_fonts(
            &mut renderer,
            document.clone(),
            &media,
            &scene,
            preview,
            studio::ViewCamera::Active,
            Some(fonts),
        )
        .unwrap();
        let mut request = RenderRequest::sized(size, Precision::Float16);
        request.viewer = !export;
        request.export = export;
        request.output_kind = if export {
            OutputKind::Final
        } else {
            OutputKind::Preview
        };
        let start = std::time::Instant::now();
        let frame = renderer
            .render(&document, &comp, ticks, &request, &media)
            .unwrap();
        assert!(frame.failures.is_empty(), "{name}: {:?}", frame.failures);
        let rgba = renderer.read_rgba(&frame).unwrap();
        let counters = renderer.studio_counters();
        assert!(counters.traces > 0, "{name}: the models did not render");
        eprintln!(
            "{name}: {} ms, {} Studio samples",
            start.elapsed().as_millis(),
            counters.samples
        );
        let bytes: Vec<_> = rgba
            .iter()
            .enumerate()
            .map(|(i, &channel)| {
                assert!(channel.is_finite(), "{name}: non-finite channel {i}");
                let linear = channel.clamp(0., 1.);
                let encoded = if i % 4 == 3 {
                    linear
                } else {
                    display(working, linear)
                };
                (encoded * 255.).round() as u8
            })
            .collect();
        let path = output.join(format!("{name}.png"));
        pm_studio3d_engine::gpu::write_png(&path, size.width, size.height, &bytes).unwrap();
        (rgba, counters.samples)
    };
    let (fast, fast_samples) = render(FAST_SIZE, Preview::Fast, false, "fast");
    let (full, full_samples) = render(FULL_SIZE, Preview::Final, false, "full");
    render(FULL_SIZE, Preview::Final, true, "export");
    assert!(
        fast_samples < full_samples,
        "Fast uses fewer Studio samples"
    );
    let (mut error, mut noise, mut clipped_fast, mut clipped_full) = (0., 0., 0usize, 0usize);
    let luma = |v: [f32; 3]| {
        0.2126 * display(working, v[0])
            + 0.7152 * display(working, v[1])
            + 0.0722 * display(working, v[2])
    };
    let fast_width = FAST_SIZE.width as usize;
    let full_row = FULL_SIZE.width as usize * 4;
    for y in 0..FAST_SIZE.height as usize {
        let mut last: Option<(f32, f32)> = None;
        for x in 0..fast_width {
            let preview = std::array::from_fn(|c| fast[(y * fast_width + x) * 4 + c]);
            let full = std::array::from_fn(|c| {
                let i = y * 2 * full_row + x * 8 + c;
                (full[i] + full[i + 4] + full[i + full_row] + full[i + full_row + 4]) / 4.
            });
            let (p, q) = (luma(preview), luma(full));
            error += (0..3)
                .map(|c| (display(working, preview[c]) - display(working, full[c])).abs())
                .sum::<f32>()
                / 3.;
            if let Some((prev_p, prev_q)) = last {
                noise += ((p - prev_p).abs() - (q - prev_q).abs()).max(0.);
            }
            clipped_fast += usize::from(p > 0.98);
            clipped_full += usize::from(q > 0.98);
            last = Some((p, q));
        }
    }
    let denominator = (FAST_SIZE.width * FAST_SIZE.height) as f32;
    let (error, noise) = (error / denominator, noise / denominator);
    let clipping = clipped_fast.saturating_sub(clipped_full) as f32 / denominator;
    eprintln!(
        "frame={frame} mean display RGB error={error:.5} excess horizontal luma variation={noise:.5} excess clipping={clipping:.5}"
    );
    assert!(
        error < 0.045 && noise < 0.006 && clipping < 0.003,
        "Fast deviates from Full: color {error:.5}, noise {noise:.5}, clipping {clipping:.5}"
    );
}

fn display(working: WorkingEncoding, value: f32) -> f32 {
    pm_color::transfer::working_to_display(working, f64::from(value)).clamp(0., 1.) as f32
}
