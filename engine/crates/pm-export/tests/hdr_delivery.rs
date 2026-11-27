//! HDR delivery end to end on real HDR10 media (plan section 9): an HDR
//! project keeps BT.2100 light, the file carries it with the source's
//! mastering and light levels, and SDR delivery tone maps only at output.

#![cfg(target_os = "macos")]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::render::*;
use pm_export::*;
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, Container as VideoContainer, ExportFormat, ExportSettings,
    ProResProfile as WireProRes, QualityTier,
};
use pm_render::Renderer;
use serde_json::{Value, json};
use support::doc::*;

const W: u32 = 320;
const H: u32 = 180;
const DUR: f64 = 0.4;

fn source() -> Option<PathBuf> {
    let path = std::env::var_os("PM_MEDIA_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/media-corpus")
        })
        .join("formats/hdr/hdr10.mov");
    path.exists().then_some(path)
}

fn project_doc(hdr: bool) -> Value {
    let clip = layer(
        "clip",
        "video",
        json!({ "asset": "clip", "fit": "fill", "w": W, "h": H, "trim": 0, "speed": 1 }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        DUR,
    );
    let mut doc = project(W, H, DUR, solid_fill("#000000"), vec![clip]);
    asset(&mut doc, "clip", "video", "mov", W, H, 1.0);
    if hdr {
        doc["color"] = json!({ "mode": "hdr", "masteringPeak": 1000 });
        doc["colorDepth"] = json!(32);
    }
    doc
}

fn export_job(
    doc: &Value,
    media: &MediaPaths,
    format: ExportFormat,
    color: OutputColor,
    destination: PathBuf,
) -> ExportJob {
    ExportJob {
        document: Arc::new(load(doc)),
        composition: "Ptest-root".into(),
        settings: ExportSettings {
            format,
            alpha: WireAlpha::None,
            quality: QualityTier::High,
            size: Size::new(W, H),
            frame_rate: FrameRate::new(25, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: Ticks::new((DUR * TICKS_PER_SECOND as f64) as i64).unwrap(),
            },
            audio: false,
            audio_index_playhead: None,
            motion_blur: false,
            color,
            precision: Precision::Float32,
            dither: false,
            draft_from_proxies: false,
            delivery: Default::default(),
        },
        media: Arc::new(media.clone()),
        destination,
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 7,
    }
}

/// One decoded frame: normalized Y′, centred Cb and Cr per luma pixel.
struct Decoded {
    width: usize,
    ycbcr: Vec<[f64; 3]>,
    transfer: Option<u8>,
    primaries: Option<u8>,
    mastering: Option<pm_media::probe::color::MasteringDisplay>,
    content_light: Option<pm_media::probe::color::ContentLight>,
}

fn first_frame(path: &std::path::Path) -> Decoded {
    let source = MediaSource::open(path).unwrap();
    let service = DecodeService::new(ServiceConfig::default());
    let mut session = service
        .session(&source, 1, pm_media::vt::DecodePurpose::Export)
        .unwrap();
    let frame = session.frame(0).unwrap();
    let (luma, chroma) = frame.reference().unwrap();
    let (width, sx, sy) = (
        frame.format.width,
        1usize << frame.format.log2_chroma_w,
        1usize << frame.format.log2_chroma_h,
    );
    let ycbcr = (0..luma.len())
        .map(|i| {
            let c = chroma[(i / width) / sy * (width / sx) + (i % width) / sx];
            [f64::from(luma[i][0]), f64::from(c[0]), f64::from(c[1])]
        })
        .collect();
    let v = source.probe.video.as_ref().unwrap();
    Decoded {
        width,
        ycbcr,
        transfer: v.color.tagged.transfer,
        primaries: v.color.tagged.primaries,
        mastering: v.color.mastering.clone(),
        content_light: v.color.content_light,
    }
}

/// A BT.2020 pixel's largest of R′, G′ and B′: the channel the master holds.
fn brightest_channel([y, cb, cr]: [f64; 3]) -> f64 {
    let (kr, kb) = (0.2627, 0.0593);
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    r.max(g).max(b)
}

/// PQ's signal for `nits` (ST 2084): normalized Y′ of a white of that light.
fn pq(nits: f64) -> f64 {
    let y = (nits / 10_000.0).powf(0.159_301_757_812_5);
    ((0.835_937_5 + 18.851_562_5 * y) / (1.0 + 18.687_5 * y)).powf(78.843_75)
}

/// Y′ of a BT.2020 pixel once R′G′B′ is held to PQ's domain, as BT.2100
/// defines it. Saturated fixture bars leave that domain on purpose.
fn clipped_luma([y, cb, cr]: [f64; 3]) -> f64 {
    let (kr, kb) = (0.2627, 0.0593);
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    kr * r.clamp(0.0, 1.0) + (1.0 - kr - kb) * g.clamp(0.0, 1.0) + kb * b.clamp(0.0, 1.0)
}

#[test]
fn hdr_projects_deliver_their_source_light_and_metadata() {
    let Some(path) = source() else {
        eprintln!("hdr/hdr10.mov missing: run the format corpus (skipped)");
        return;
    };
    let doc = project_doc(true);
    let mut media = MediaPaths::default();
    media.insert("clip", path.clone());
    let dir = std::env::temp_dir().join(format!("pm-hdr-delivery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let cancel = AtomicBool::new(false);
    let source_frame = first_frame(&path);
    for (name, format, color, transfer) in [
        (
            "pq.mp4",
            ExportFormat::Hevc {
                ten_bit: true,
                container: VideoContainer::Mp4,
            },
            OutputColor::Rec2100Pq,
            16,
        ),
        (
            "hlg.mov",
            ExportFormat::Hevc {
                ten_bit: true,
                container: VideoContainer::Mov,
            },
            OutputColor::Rec2100Hlg,
            18,
        ),
        (
            "prores.mov",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            OutputColor::Rec2100Pq,
            16,
        ),
    ] {
        let destination = dir.join(name);
        let job = export_job(&doc, &media, format, color, destination.clone());
        let plan = plan_export(&job).unwrap();
        // The source's light is the master's: held there, never rolled off.
        assert_eq!(
            plan.request.output_tone,
            Some(pm_render::OutputTone {
                source_peak: 1000.0,
                target_peak: 1000.0
            }),
            "{name}: the target holds it"
        );
        let delivered =
            run_export(&mut renderer, &job, &Unrestricted, &cancel, &mut |_| {}).unwrap();
        let facts = first_frame(&destination);
        assert_eq!(facts.transfer, Some(transfer), "{name}");
        assert_eq!(facts.primaries, Some(9), "{name}");
        let mastering = facts.mastering.as_ref().expect("mdcv");
        assert_eq!(mastering.max_luminance, Some(1000.0), "{name}");
        assert_eq!(mastering.green, Some((0.265, 0.69)), "{name}");
        // The light levels of the frames delivered, not the source's
        // statement (1,000 and 400 cd/m²).
        let light = facts.content_light.expect("clli");
        let measured = delivered.report.content_light.expect("measured");
        assert_eq!(
            (light.max_cll, light.max_fall),
            (u32::from(measured.peak), u32::from(measured.average)),
            "{name}"
        );
        assert!(measured.peak <= 1000 && measured.average < measured.peak, "{name}: {measured:?}");
        // PQ to PQ through the working space keeps the signal within the
        // 1,000 cd/m² master, flat fields judged at the lossy codec's reach.
        // The fixture's bars are brighter than it states (3,536 cd/m²):
        // they are held at the master.
        if transfer == 16 {
            let master = pq(1000.0);
            let mut errors: Vec<f64> = source_frame
                .ycbcr
                .iter()
                .zip(&facts.ycbcr)
                .filter(|(a, _)| brightest_channel(**a) < master - 4.0 / 876.0)
                .map(|(a, b)| (clipped_luma(*a) - b[0]).abs())
                .collect();
            errors.sort_by(f64::total_cmp);
            let median = errors[errors.len() / 2];
            println!("{name}: median luma error within the master {median}");
            assert!(median < 3.0 / 876.0, "{name}: {median}");
            let brightest = facts.ycbcr.iter().map(|p| p[0]).fold(0.0, f64::max);
            println!("{name}: brightest Y′ {brightest:.4}, the master is {master:.4}");
            assert!(brightest > master - 4.0 / 876.0, "{name}: highlights lost, {brightest}");
            assert!(brightest < master + 8.0 / 876.0, "{name}: past the master, {brightest}");
        }
        assert_eq!(facts.width, W as usize);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A grade that lifts light past the project's master (a white solid's
/// exposure raised 2.4 stops in a 300 cd/m² master): the file holds the
/// master, says the light it holds (MaxCLL and MaxFALL measured from its
/// frames) and names the project's master as its mastering display.
#[test]
fn a_graded_picture_sets_the_files_light_and_its_master_holds_it() {
    let mut lifted = layer(
        "lifted",
        "solid",
        json!({ "color": "#FFFFFF", "w": W / 2, "h": H }),
        f64::from(W) / 4.0,
        f64::from(H) / 2.0,
        DUR,
    );
    lifted["fx"] = json!([{ "id": "e", "type": "color", "on": true,
        "p": { "exposure": ch(json!(2.4)) } }]);
    let mut doc = project(W, H, DUR, solid_fill("#000000"), vec![lifted]);
    doc["color"] = json!({ "mode": "hdr", "masteringPeak": 300 });
    doc["colorDepth"] = json!(32);
    let dir = std::env::temp_dir().join(format!("pm-hdr-graded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        renderer
            .set_definitions(&pm_render::builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let cancel = AtomicBool::new(false);
    for (name, format) in [
        (
            "graded.mp4",
            ExportFormat::Hevc {
                ten_bit: true,
                container: VideoContainer::Mp4,
            },
        ),
        (
            "graded.mov",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
        ),
    ] {
        let destination = dir.join(name);
        let job = export_job(
            &doc,
            &MediaPaths::default(),
            format,
            OutputColor::Rec2100Pq,
            destination.clone(),
        );
        // Planned with the renderer's effects.
        let plan = plan_export_for(&job, &renderer).unwrap();
        let sink = sink_for(&plan).unwrap();
        let delivered = run_export_into(
            &mut renderer,
            &job,
            plan,
            sink,
            &Unrestricted,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        let facts = first_frame(&destination);
        // The lit half is held at the master; only the codec's ringing at
        // its edge reaches past it.
        let mut lit: Vec<f64> = facts
            .ycbcr
            .iter()
            .map(|p| p[0])
            .filter(|y| *y > 0.3)
            .collect();
        lit.sort_by(f64::total_cmp);
        let (median, brightest) = (lit[lit.len() / 2], lit[lit.len() - 1]);
        println!("{name}: lit Y′ median {median:.4}, brightest {brightest:.4}; 300 cd/m² is {:.4}", pq(300.0));
        assert!((median - pq(300.0)).abs() <= 2.0 / 876.0, "{name}: {median} is not the master");
        assert!(brightest <= pq(300.0) + 8.0 / 876.0, "{name}: {brightest} past the master");
        let measured = delivered.report.content_light.expect("measured");
        println!("{name}: measured {measured:?}");
        assert!((295..=300).contains(&measured.peak), "{name}: {measured:?}");
        // MaxFALL: the frame's average, its lit part at the master.
        let lit_part = lit.len() as f64 / facts.ycbcr.len() as f64;
        println!("{name}: lit part {lit_part:.3}");
        assert!(
            (f64::from(measured.average) - f64::from(measured.peak) * lit_part).abs() <= 3.0,
            "{name}: {measured:?}, lit part {lit_part:.3}"
        );
        let light = facts.content_light.expect("clli");
        assert_eq!(
            (light.max_cll, light.max_fall),
            (u32::from(measured.peak), u32::from(measured.average)),
            "{name}"
        );
        let mastering = facts.mastering.as_ref().expect("mdcv");
        assert_eq!(mastering.max_luminance, Some(300.0), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sdr_delivery_of_an_hdr_project_equals_input_tone_mapping_in_an_sdr_project() {
    let Some(path) = source() else {
        return;
    };
    let mut media = MediaPaths::default();
    media.insert("clip", path);
    let sdr_color = OutputColor::Rec709QuickTime {
        candidate: QuickTimeTag::Cicp1_13_1,
    };
    let format = ExportFormat::ProRes {
        profile: WireProRes::Hq,
    };
    let mut renderer = Renderer::headless().unwrap();
    let mut frame = |doc: &Value| {
        let job = export_job(doc, &media, format, sdr_color, PathBuf::from("unused.png"));
        let plan = plan_export(&job).unwrap();
        let mut request = plan.request.clone();
        request.precision = Precision::Float32;
        let rendered = render_frame(
            &mut renderer,
            &job.document,
            &job.composition,
            Ticks::ZERO,
            &request,
            &media,
            &Unrestricted,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(rendered.failures.is_empty(), "{:?}", rendered.failures);
        (plan.request.output_tone, rendered.rgba)
    };
    let (tone, tone_mapped) = frame(&project_doc(true));
    let tone = tone.expect("SDR delivery of HDR light needs BT.2390");
    assert_eq!((tone.source_peak, tone.target_peak), (1000.0, 203.0));
    let (none, reference) = frame(&project_doc(false));
    assert_eq!(none, None, "SDR projects tone map at input");
    let (index, worst) = tone_mapped
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a.clamp(0.0, 1.0) - b.clamp(0.0, 1.0)).abs())
        .enumerate()
        .fold(
            (0, 0.0f32),
            |best, (i, e)| if e > best.1 { (i, e) } else { best },
        );
    let p = index / 4 * 4;
    assert!(
        worst < 1.0 / 255.0,
        "delivery differs by {worst}: {:?} {:?}",
        &tone_mapped[p..p + 4],
        &reference[p..p + 4]
    );
}

#[test]
fn only_what_the_composition_shows_sets_the_tone_map() {
    let Some(path) = source() else {
        eprintln!("hdr/hdr10.mov missing: run the format corpus (skipped)");
        return;
    };
    let mut media = MediaPaths::default();
    media.insert("clip", path.clone());
    media.insert("log", path);
    let plan_of = |doc: &Value| {
        let job = export_job(
            doc,
            &media,
            ExportFormat::H264,
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            },
            std::env::temp_dir().join("pm-unused-log.mp4"),
        );
        plan_export(&job).unwrap().request.output_tone.unwrap()
    };
    let base = project_doc(true);
    let shown = plan_of(&base);
    assert_eq!(shown.source_peak, 1000.0);
    let log = |on: bool| {
        let mut clip = base["layers"][0].clone();
        clip["id"] = json!("log");
        clip["on"] = json!(on);
        clip["d"]["asset"] = json!("log");
        clip
    };
    let mut doc = base.clone();
    doc["assets"]["log"] = doc["assets"]["clip"].clone();
    doc["assets"]["log"]["id"] = json!("log");
    doc["assets"]["log"]["interpret"] = json!({ "transfer": "sLog3", "primaries": "sGamut3Cine" });
    // A clip in a composition nothing shows, and one switched off.
    let mut unused = doc.clone();
    unused["comps"]["unused"] = json!({ "id": "unused", "layers": [log(true)] });
    assert_eq!(plan_of(&unused), shown);
    let mut off = doc.clone();
    off["layers"].as_array_mut().unwrap().push(log(false));
    assert_eq!(plan_of(&off), shown);
    // A log clip the delivery shows states what the curve can hold.
    let mut on = doc;
    on["layers"].as_array_mut().unwrap().push(log(true));
    assert!(plan_of(&on).source_peak > 7000.0);
}

#[test]
fn hdr_delivery_needs_an_hdr_project_and_a_deep_codec() {
    let Some(path) = source() else {
        return;
    };
    let mut media = MediaPaths::default();
    media.insert("clip", path);
    let hevc = ExportFormat::Hevc {
        ten_bit: true,
        container: VideoContainer::Mp4,
    };
    let sdr_project = export_job(
        &project_doc(false),
        &media,
        hevc,
        OutputColor::Rec2100Pq,
        PathBuf::from("a.mp4"),
    );
    assert!(plan_export(&sdr_project).is_err());
    let h264 = export_job(
        &project_doc(true),
        &media,
        ExportFormat::H264,
        OutputColor::Rec2100Pq,
        PathBuf::from("a.mp4"),
    );
    assert!(plan_export(&h264).is_err());
}
