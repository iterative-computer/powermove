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
        assert_eq!(
            plan.request.output_tone, None,
            "{name}: the target holds it"
        );
        run_export(&mut renderer, &job, &Unrestricted, &cancel, &mut |_| {}).unwrap();
        let facts = first_frame(&destination);
        assert_eq!(facts.transfer, Some(transfer), "{name}");
        assert_eq!(facts.primaries, Some(9), "{name}");
        let mastering = facts.mastering.as_ref().expect("mdcv");
        assert_eq!(mastering.max_luminance, Some(1000.0), "{name}");
        assert_eq!(mastering.green, Some((0.265, 0.69)), "{name}");
        let light = facts.content_light.expect("clli");
        assert_eq!((light.max_cll, light.max_fall), (1000, 400), "{name}");
        // PQ to PQ through the working space keeps the signal, flat fields
        // judged at the lossy codec's reach.
        if transfer == 16 {
            let mut errors: Vec<f64> = source_frame
                .ycbcr
                .iter()
                .zip(&facts.ycbcr)
                .map(|(a, b)| (clipped_luma(*a) - b[0]).abs())
                .collect();
            errors.sort_by(f64::total_cmp);
            let median = errors[errors.len() / 2];
            println!("{name}: median luma error {median}");
            assert!(median < 3.0 / 876.0, "{name}: {median}");
            let brightest = facts.ycbcr.iter().map(|p| p[0]).fold(0.0, f64::max);
            assert!(brightest > 0.7, "{name}: highlights lost, {brightest}");
        }
        assert_eq!(facts.width, W as usize);
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
