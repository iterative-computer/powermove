//! The viewer's colour policy on real HDR media, offscreen: an HDR project's
//! viewer shows its light, its SDR view equals SDR delivery, and the scope tap
//! delivers counts from the service's own worker.
use pm_engine_service::*;
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use std::{path::PathBuf, time::Duration};

const W: u32 = 320;
const H: u32 = 180;

fn hdr_clip() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/Caches/powermove-engine/media-corpus/formats/hdr/hdr10.mov");
    path.exists().then_some(path)
}

fn project(hdr: bool) -> pm_model::Value {
    let channel = |v: f64| format!(r#"{{"v":{v},"kf":[],"expr":null}}"#);
    let names = [
        ("position.x", 160.),
        ("position.y", 90.),
        ("position.z", 0.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("anchor.z", 0.),
        ("scale.x", 100.),
        ("scale.y", 100.),
        ("scale.z", 100.),
        ("rotation", 0.),
        ("rotation.x", 0.),
        ("rotation.y", 0.),
        ("orientation.x", 0.),
        ("orientation.y", 0.),
        ("orientation.z", 0.),
        ("opacity", 100.),
        ("skew", 0.),
        ("perspective", 50.),
    ];
    let p = names
        .iter()
        .map(|(k, v)| format!(r#""{k}":{}"#, channel(*v)))
        .collect::<Vec<_>>()
        .join(",");
    let color = if hdr {
        r#""color":{"mode":"hdr","masteringPeak":1000},"colorDepth":32,"#
    } else {
        ""
    };
    pm_model::json::parse(&format!(
        r##"{{"id":"P","name":"P","compId":"root","compName":"P","w":{W},"h":{H},"fps":25,"dur":1,
        "bg":"#000000","work":[0,1],{color}
        "layers":[{{"id":"clip","type":"video","name":"clip","from":0,"dur":1,"on":true,
          "blend":"normal","masks":[],"fx":[],"p":{{{p}}},
          "d":{{"asset":"clip","fit":"fill","w":{W},"h":{H},"trim":0,"speed":1}}}}],
        "assets":{{"clip":{{"id":"clip","name":"clip","kind":"video","format":"mov","w":{W},"h":{H},"dur":1,"fps":25}}}},
        "comps":{{}}}}"##
    ))
    .unwrap()
}

fn scene(hdr: bool, content: u64, clip: &PathBuf) -> (Snapshot, Scene, MediaCatalog) {
    let snapshot = pm_kernel::document::DocumentStore::new_untitled(project(hdr)).snapshot();
    let mut media = MediaCatalog::default();
    media.originals.insert("clip".into(), clip.clone());
    let scene = Scene {
        doc: snapshot.document.clone(),
        comp: snapshot.open_comp().unwrap(),
        media: media.export_paths(false),
        content,
    };
    (snapshot, scene, media)
}

const SIZE: Size = Size {
    width: W,
    height: H,
};

#[test]
fn edr_keeps_light_and_the_sdr_view_equals_input_tone_mapping() {
    let Some(clip) = hdr_clip() else {
        eprintln!("hdr/hdr10.mov missing: run the format corpus (skipped)");
        return;
    };
    let time = Ticks::ZERO;
    let (_, hdr_scene, _) = scene(true, 1, &clip);
    let (_, sdr_scene, _) = scene(false, 2, &clip);
    let mut h = Headless::new().unwrap();
    h.mastering(color::Mastering::Edr).unwrap();
    let edr = h
        .frame(&hdr_scene, time, SIZE, &Default::default())
        .unwrap();
    let brightest = edr.iter().flatten().copied().fold(0.0, f64::max);
    assert!(brightest > 4.0, "EDR keeps 1000-nit light: {brightest}");
    h.mastering(color::Mastering::LinearP3).unwrap();
    let sdr_view = h
        .frame(&hdr_scene, time, SIZE, &Default::default())
        .unwrap();
    let mapped = sdr_view.iter().flatten().copied().fold(0.0, f64::max);
    assert!(mapped < 2.0, "the SDR view tone maps: {mapped}");
    let reference = h
        .frame(&sdr_scene, time, SIZE, &Default::default())
        .unwrap();
    let mut errors: Vec<f64> = sdr_view
        .iter()
        .zip(&reference)
        .map(|(a, b)| color::delta(a.map(|v| v.max(0.)), b.map(|v| v.max(0.))))
        .collect();
    errors.sort_by(f64::total_cmp);
    let p99 = errors[errors.len() * 99 / 100];
    assert!(p99 <= 1.0, "SDR view against SDR project: ΔE2000 p99 {p99}");
    // PQ encodes the same light EDR shows.
    h.mastering(color::Mastering::Pq {
        source_peak: 1000.,
        target_peak: 1000.,
    })
    .unwrap();
    let pq = h
        .frame(&hdr_scene, time, SIZE, &Default::default())
        .unwrap();
    use pm_protocol::color::Primaries;
    let to_2020 = pm_color::primaries::conversion(Primaries::DisplayP3, Primaries::Bt2020).unwrap();
    let mut errors: Vec<f64> = edr
        .iter()
        .zip(&pq)
        .flat_map(|(light, signal)| {
            let light = pm_color::primaries::apply(&to_2020, *light);
            light
                .into_iter()
                .zip(*signal)
                .filter(|(light, _)| *light > 0.05)
                .map(|(light, signal)| {
                    (pm_color::tonemap::pq_inverse_eotf(light * 203.) - signal).abs()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    errors.sort_by(f64::total_cmp);
    let p99 = errors[errors.len() * 99 / 100];
    assert!(p99 < 2e-3, "PQ against EDR light: p99 {p99}");
}

#[test]
fn the_scope_tap_reports_a_paused_frame_and_stops_when_cleared() {
    let Some(clip) = hdr_clip() else {
        return;
    };
    let (snapshot, _, media) = scene(true, 3, &clip);
    let service = EngineService::start(pm_gpu::Gpu::new("scope-service").unwrap()).unwrap();
    let viewer = service
        .viewer(ViewerConfig {
            composition: snapshot.open_comp().unwrap(),
            snapshot,
            media,
            output: Output {
                width: W,
                height: H,
                display: 0,
                headroom: 1.,
            },
            layer: None,
            rate: pm_time::Rate::FPS_30,
            end: Some(TICKS_PER_SECOND),
            audio: false,
        })
        .unwrap();
    let (frames, inbox) = async_channel::bounded(1);
    viewer.scopes(Some(frames)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let frame = loop {
        assert!(
            viewer.status().error.is_none(),
            "{:?}",
            viewer.status().error
        );
        match inbox.try_recv() {
            Ok(frame) => break frame,
            Err(_) => {
                assert!(std::time::Instant::now() < deadline, "no scope frame");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    assert_eq!(frame.pixels, u64::from(W * H));
    // The default view is an HDR project's SDR picture: display code values.
    assert_eq!(frame.config.signal, pm_render::scopes::ScopeSignal::Display);
    assert_eq!(
        frame.waveform.iter().map(|v| u64::from(*v)).sum::<u64>(),
        frame.pixels
    );
    let high = frame.config.levels as usize * 3 / 4;
    assert!(
        frame.luma_histogram[high..].iter().any(|v| *v > 0),
        "the frame's highlights reach the scope"
    );
    viewer.scopes(None).unwrap();
    // The service drops its sender with the tap.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !inbox.is_closed() {
        viewer.capture_next().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "the tap outlives its panel"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
