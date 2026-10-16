//! Interpret Footage's media facts (`pm_protocol::media::MediaFacts`): what
//! a file is, read-only, and each setting the sheet can override with its
//! value, its automatic value, and where that came from (the file, a
//! section 9 default rule, or the override).

use crate::error::{MediaError, Result};
use crate::image::{self, ImageFormat};
use crate::interpret::{Field, Interpretation, Origin, resolve_still, resolve_video};
use crate::pool::FramePool;
use crate::probe::color::HdrFormat;
use crate::probe::{FieldOrder, MediaProbe, StreamKind, VideoProbe};
use pm_protocol::color::{AlphaMode, Primaries, Subsampling, Transfer};
use pm_protocol::media::{FactLine, FactOrigin, InterpretSetting, MediaFacts, MediaKind};
use serde::Serialize;
use std::path::Path;

fn origin(o: &Origin) -> FactOrigin {
    match o {
        Origin::File { source } => FactOrigin::File {
            source: source.clone(),
        },
        Origin::Default { rule } => FactOrigin::Default { rule: rule.clone() },
        Origin::Override => FactOrigin::Override,
    }
}

/// The member's stored form of a contract 4 enum (its serde name).
fn name<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn alpha_name(a: &AlphaMode) -> String {
    match a {
        AlphaMode::Opaque => "ignore",
        AlphaMode::Straight => "straight",
        AlphaMode::Premultiplied => "premultiplied",
        AlphaMode::PremultipliedWithColor { .. } => "premultipliedWithColor",
    }
    .into()
}

fn field_name(f: FieldOrder) -> String {
    match f {
        FieldOrder::TopFirst => "topFirst",
        FieldOrder::BottomFirst => "bottomFirst",
        FieldOrder::Progressive | FieldOrder::Unknown => "progressive",
    }
    .into()
}

fn setting<T>(member: &str, f: &Field<T>, show: impl Fn(&T) -> String) -> InterpretSetting {
    InterpretSetting {
        member: member.into(),
        value: show(&f.value),
        automatic: show(&f.automatic),
        origin: origin(&f.origin),
    }
}

fn line(label: &str, value: impl Into<String>) -> FactLine {
    FactLine {
        label: label.into(),
        value: value.into(),
    }
}

/// Seconds as `h:mm:ss.fff` (or `m:ss.fff`).
fn clock(s: f64) -> String {
    let ms = (s * 1000.0).round() as i64;
    let (h, m, sec, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}.{f:03}")
    } else {
        format!("{m}:{sec:02}.{f:03}")
    }
}

fn rate_text(num: i64, den: i64) -> String {
    let v = num as f64 / den as f64;
    if den == 1 {
        format!("{num} fps")
    } else {
        format!("{} fps", (v * 1000.0).round() / 1000.0)
    }
}

fn parse_interpret(text: Option<&str>) -> (Interpretation, Vec<String>) {
    let Some(text) = text else {
        return (Interpretation::default(), vec![]);
    };
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => {
            let (i, issues) = Interpretation::from_json(&v);
            (i, issues.iter().map(ToString::to_string).collect())
        }
        Err(e) => (
            Interpretation::default(),
            vec![format!("interpret is not JSON: {e}")],
        ),
    }
}

fn is_still(path: &Path) -> Result<bool> {
    let mut head = [0u8; 512];
    let n = {
        use std::io::Read;
        std::fs::File::open(path)?.read(&mut head)?
    };
    Ok(!matches!(image::sniff(&head[..n]), ImageFormat::Other))
}

/// The facts of a file, read with `interpret` (an asset's `interpret`
/// member as JSON text).
pub fn media_facts(path: &Path, interpret: Option<&str>) -> Result<MediaFacts> {
    let (interp, issues) = parse_interpret(interpret);
    if is_still(path)? && image::animation(path).ok().flatten().is_none() {
        return still_facts(path, &interp, issues);
    }
    let probed = crate::probe::probe(path)?;
    let p = &probed.probe;
    match &p.video {
        Some(v) => Ok(video_facts(p, v, &interp, issues)),
        None if p.audio_stream.is_some() => Ok(audio_facts(p, issues)),
        None => Err(MediaError::invalid("no video, image, or audio stream")),
    }
}

fn audio_lines(p: &MediaProbe, out: &mut Vec<FactLine>) {
    let Some(a) = p
        .audio_stream
        .and_then(|i| p.streams.get(i))
        .and_then(|s| s.audio.as_ref())
    else {
        return;
    };
    let layout = a
        .channel_layout
        .clone()
        .unwrap_or_else(|| format!("{} channels", a.channels));
    out.push(line(
        "Audio",
        format!(
            "{} · {} kHz · {layout}",
            a.codec,
            a.sample_rate as f64 / 1000.0
        ),
    ));
    if let Some(pr) = &a.priming {
        out.push(line("Audio priming", format!("{} samples", pr.samples)));
    }
}

fn identity_lines(p: &MediaProbe, out: &mut Vec<FactLine>) {
    if let Some(tc) = &p.timecode {
        out.push(line(
            "Timecode",
            format!(
                "{}{}",
                tc.text,
                if tc.drop_frame { " (drop frame)" } else { "" }
            ),
        ));
    }
    if let Some(r) = &p.reel {
        out.push(line("Reel", r.clone()));
    }
    if let Some(u) = &p.umid {
        out.push(line("UMID", u.clone()));
    }
    if let Some(c) = &p.content_identifier {
        out.push(line("Content identifier", c.clone()));
    }
}

fn video_facts(
    p: &MediaProbe,
    v: &VideoProbe,
    interp: &Interpretation,
    issues: Vec<String>,
) -> MediaFacts {
    let r = resolve_video(v, interp);
    let c = &v.color;
    let mut facts = vec![
        line("Format", p.container.clone()),
        line(
            "Codec",
            match &v.codec.profile_name {
                Some(profile) => format!("{} ({profile})", v.codec.codec),
                None => v.codec.codec.clone(),
            },
        ),
        line("Encoded size", format!("{} × {}", v.width, v.height)),
        line(
            "Samples",
            format!(
                "{}-bit {}{}",
                c.interpreted.bit_depth,
                match c.interpreted.subsampling {
                    Subsampling::S444 => "4:4:4",
                    Subsampling::S422 if v.codec.log2_chroma_w == 2 => "4:1:1",
                    Subsampling::S422 => "4:2:2",
                    Subsampling::S420 => "4:2:0",
                },
                if v.alpha.is_some() { " with alpha" } else { "" }
            ),
        ),
    ];
    if let Some(n) = &v.nominal_rate {
        facts.push(line(
            "Frame rate",
            format!(
                "{}{}",
                rate_text(n.rate.num(), n.rate.den()),
                if v.variable_rate {
                    " · variable, played by timestamp"
                } else {
                    ""
                }
            ),
        ));
    }
    facts.push(line("Frames", v.frames.to_string()));
    if let Some(d) = p.duration_us {
        facts.push(line("Duration", clock(d as f64 / 1e6)));
    }
    identity_lines(p, &mut facts);
    if let Some(h) = &c.hdr {
        facts.push(line(
            "HDR",
            match h {
                HdrFormat::Hdr10 => "HDR10 (PQ)".to_owned(),
                HdrFormat::Hlg => "HLG".to_owned(),
                HdrFormat::DolbyVision {
                    profile,
                    level,
                    base_compatibility,
                } => format!(
                    "Dolby Vision profile {profile}.{base_compatibility} level {level} (base layer shown)"
                ),
            },
        ));
    }
    if let Some(m) = &c.mastering
        && let (Some(max), Some(min)) = (m.max_luminance, m.min_luminance)
    {
        facts.push(line("Mastering display", format!("{min}–{max} cd/m²")));
    }
    if let Some(l) = &c.content_light {
        facts.push(line(
            "Content light",
            format!("MaxCLL {} · MaxFALL {} cd/m²", l.max_cll, l.max_fall),
        ));
    }
    if let Some(t) = &r.tone_map {
        facts.push(line(
            "Tone mapping",
            format!("BT.2446 Method A from {} cd/m²", t.peak_nits),
        ));
    }
    if let Some(icc) = &c.icc {
        facts.push(line("ICC profile", format!("{} bytes", icc.bytes)));
    }
    if r.rotation.abs() > 0.01 {
        facts.push(line("Rotation", format!("{}°", r.rotation)));
    }
    facts.push(line(
        "Decoder",
        format!(
            "{:?}{}",
            v.decode.backend,
            if v.decode.hardware { " (hardware)" } else { "" }
        ),
    ));
    audio_lines(p, &mut facts);

    let mut settings = vec![
        setting("primaries", &r.primaries, name),
        setting("transfer", &r.transfer, name),
        setting("matrix", &r.matrix, name),
        setting("range", &r.range, name),
    ];
    if c.interpreted.subsampling != Subsampling::S444 {
        settings.push(setting("siting", &r.siting, name));
    }
    if v.alpha.is_some() {
        settings.push(setting("alpha", &r.alpha, alpha_name));
    }
    settings.push(setting("frameRate", &r.frame_rate, |rate| {
        format!("{}/{}", rate.num(), rate.den())
    }));
    settings.push(setting("fieldOrder", &r.field_order, |f| field_name(*f)));
    settings.push(setting("pixelAspect", &r.pixel_aspect, |(n, d)| {
        format!("{n}:{d}")
    }));
    if v.clean_aperture.is_some() {
        settings.push(setting("cleanAperture", &r.clean_aperture, |c| {
            c.is_some().to_string()
        }));
    }
    let (w, h) = r.display_size((v.width, v.height));
    let duration = match r.conformed() {
        Some(rate) => Some(v.frames as f64 * rate.den() as f64 / rate.num() as f64),
        None => p.duration_us.map(|d| d as f64 / 1e6),
    };
    MediaFacts {
        kind: MediaKind::Video,
        facts,
        settings,
        issues,
        display_size: Some([w, h]),
        duration,
    }
}

fn still_facts(path: &Path, interp: &Interpretation, issues: Vec<String>) -> Result<MediaFacts> {
    let pool = FramePool::new(0);
    let s = image::read(path, &pool)?;
    let (w, h) = (s.frame.format.width as u32, s.frame.format.height as u32);
    let icc = s.color.icc_class;
    use crate::image::icc::IccClass;
    let (primaries, p_origin) = match icc {
        None => (
            Primaries::Bt709,
            Origin::Default {
                rule: "Stills without a profile read as sRGB".into(),
            },
        ),
        Some(IccClass::Srgb | IccClass::GraySrgb) => (
            Primaries::Bt709,
            Origin::File {
                source: "ICC profile equivalent to sRGB (passed through)".into(),
            },
        ),
        Some(IccClass::DisplayP3) => (
            Primaries::DisplayP3,
            Origin::File {
                source: "ICC profile (Display P3)".into(),
            },
        ),
        Some(IccClass::Other) => (
            Primaries::Bt709,
            Origin::File {
                source: "ICC profile (converted by ColorSync)".into(),
            },
        ),
    };
    let transfer_origin = if s.float {
        Origin::Default {
            rule: "Floating-point stills are linear".into(),
        }
    } else {
        Origin::Default {
            rule: "Integer stills read with the display encoding (sRGB curve)".into(),
        }
    };
    let has_alpha = s.color.alpha != AlphaMode::Opaque;
    let alpha_origin = if has_alpha {
        Origin::File {
            source: "alpha channel".into(),
        }
    } else {
        Origin::Default {
            rule: "No alpha channel".into(),
        }
    };
    let transfer = if icc == Some(IccClass::Other) {
        Transfer::Linear
    } else {
        s.color.transfer
    };
    let r = resolve_still(
        (primaries, p_origin),
        (transfer, transfer_origin),
        (s.color.alpha, alpha_origin),
        has_alpha,
        interp,
    );
    let mut facts = vec![
        line("Format", format!("{:?}", s.format)),
        line("Size", format!("{w} × {h}")),
        line(
            "Samples",
            format!(
                "{}-bit{}{}",
                s.bit_depth,
                if s.float { " float" } else { "" },
                if has_alpha { " with alpha" } else { "" }
            ),
        ),
        line("Decoder", format!("{:?}", s.backend)),
    ];
    if let Some(b) = s.color.icc_bytes {
        facts.push(line("ICC profile", format!("{b} bytes")));
    }
    if s.orientation != 1 {
        facts.push(line(
            "Orientation",
            format!("EXIF {} (applied)", s.orientation),
        ));
    }
    if let Ok(Some(seq)) = image::sequence::detect(path) {
        facts.push(line(
            "Image sequence",
            format!(
                "frames {}–{} ({} files)",
                seq.first(),
                seq.last(),
                seq.frames.len()
            ),
        ));
        if !seq.gaps.is_empty() {
            let gaps: Vec<String> = seq
                .gaps
                .iter()
                .map(|(a, b)| {
                    if a == b {
                        a.to_string()
                    } else {
                        format!("{a}–{b}")
                    }
                })
                .collect();
            facts.push(line("Missing frames", gaps.join(", ")));
        }
    }
    let mut settings = vec![
        setting("primaries", &r.primaries, name),
        setting("transfer", &r.transfer, name),
    ];
    if has_alpha {
        settings.push(setting("alpha", &r.alpha, alpha_name));
    }
    Ok(MediaFacts {
        kind: MediaKind::Still,
        facts,
        settings,
        issues,
        display_size: Some([w, h]),
        duration: None,
    })
}

fn audio_facts(p: &MediaProbe, issues: Vec<String>) -> MediaFacts {
    let mut facts = vec![line("Format", p.container.clone())];
    audio_lines(p, &mut facts);
    if let Some(d) = p.duration_us {
        facts.push(line("Duration", clock(d as f64 / 1e6)));
    }
    let streams = p
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Audio)
        .count();
    if streams > 1 {
        facts.push(line("Audio streams", streams.to_string()));
    }
    identity_lines(p, &mut facts);
    MediaFacts {
        kind: MediaKind::Audio,
        facts,
        settings: vec![],
        issues,
        display_size: None,
        duration: p.duration_us.map(|d| d as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_and_rates_read_well() {
        assert_eq!(clock(83.5), "1:23.500");
        assert_eq!(clock(3725.25), "1:02:05.250");
        assert_eq!(rate_text(24000, 1001), "23.976 fps");
        assert_eq!(rate_text(25, 1), "25 fps");
    }

    #[test]
    fn bad_interpret_text_is_an_issue() {
        let (i, issues) = parse_interpret(Some("{"));
        assert!(i.is_empty());
        assert_eq!(issues.len(), 1);
        let (i, issues) = parse_interpret(Some(r#"{"range":"full","x":1}"#));
        assert!(i.range.is_some());
        assert_eq!(issues, ["interpret.x: is not an interpretation setting"]);
    }
}
