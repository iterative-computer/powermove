//! Audio priming (section 13 probe item 4): encoder delay from the MP4 edit
//! list, the iTunes `iTunSMPB` tag, or the LAME/Xing header (+529 decoder
//! delay), assuming 2112 only for Apple-encoded AAC without metadata.

use crate::index::mp4::{Movie, TrackSelect, child, children};
use serde::Serialize;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PrimingSource {
    /// MP4/MOV `elst` media time of the first media edit.
    EditList,
    /// iTunes `iTunSMPB` (gapless) tag.
    ITunSmpb,
    /// LAME/Xing header encoder delay plus the 529-sample decoder delay.
    LameHeader,
    /// Opus pre-skip or another codec-level delay FFmpeg reports.
    Codec,
    /// Apple-encoded AAC without priming metadata: 2112 samples.
    AppleDefault,
}

/// Samples at the start of an audio stream that are encoder priming, not
/// program audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Priming {
    /// Priming in samples at the stream's sample rate.
    pub samples: i64,
    /// Trailing padding in samples, when the metadata says.
    pub padding: Option<i64>,
    pub source: PrimingSource,
}

/// Apple's AAC encoder delay.
pub const APPLE_AAC_PRIMING: i64 = 2112;
/// The MP3 decoder delay LAME's encoder delay field excludes.
pub const MP3_DECODER_DELAY: i64 = 529;

/// Priming of an audio stream. `track_id` is the MP4 track (FFmpeg stream id),
/// `codec` FFmpeg's codec name, `initial_padding` FFmpeg's codec-level delay.
pub fn priming(path: &Path, format: &str, track_id: u64, codec: &str, sample_rate: i64, initial_padding: i64) -> Option<Priming> {
    if format.contains("mov") || format.contains("mp4") {
        if let Some(p) = mp4_priming(path, track_id as u32, codec, sample_rate) {
            return Some(p);
        }
    }
    if codec == "mp3" {
        if let Some(p) = lame_priming(path) {
            return Some(p);
        }
    }
    if format == "mp3"
        && let Some(p) = id3_itunsmpb(path)
    {
        return Some(p);
    }
    (initial_padding > 0).then_some(Priming { samples: initial_padding, padding: None, source: PrimingSource::Codec })
}

fn mp4_priming(path: &Path, track_id: u32, codec: &str, sample_rate: i64) -> Option<Priming> {
    let f = File::open(path).ok()?;
    let movie = Movie::read(&f).ok()?;
    let track = movie.track(TrackSelect::Id(track_id)).ok()?;
    let to_samples = |media: i64| {
        if sample_rate > 0 && track.timescale as i64 != sample_rate {
            (media as i128 * sample_rate as i128 / track.timescale as i128) as i64
        } else {
            media
        }
    };
    let smpb = itunsmpb(&movie.moov);
    if let Some(e) = track.edits.iter().find(|e| e.media_time != -1)
        && e.media_time > 0
    {
        return Some(Priming {
            samples: to_samples(e.media_time),
            padding: smpb.and_then(|s| s.padding),
            source: PrimingSource::EditList,
        });
    }
    if let Some(s) = smpb {
        return Some(s);
    }
    if codec == "aac" && apple_encoded(&movie, track.trak) {
        return Some(Priming { samples: APPLE_AAC_PRIMING, padding: None, source: PrimingSource::AppleDefault });
    }
    None
}

/// `moov/udta/meta/ilst/----` with name `iTunSMPB`.
fn itunsmpb(moov: &[u8]) -> Option<Priming> {
    let udta = child(moov, b"udta").ok()??;
    let meta = child(udta, b"meta").ok()??;
    // `meta` is a full box: version and flags before its children.
    let ilst = child(meta.get(4..)?, b"ilst").ok()??;
    for (ty, item) in children(ilst).ok()? {
        if &ty != b"----" {
            continue;
        }
        let kids = children(item).ok()?;
        let name = kids.iter().find(|k| &k.0 == b"name").and_then(|k| k.1.get(4..))?;
        if name != b"iTunSMPB" {
            continue;
        }
        let data = kids.iter().find(|k| &k.0 == b"data").and_then(|k| k.1.get(8..))?;
        return parse_itunsmpb(&String::from_utf8_lossy(data));
    }
    None
}

/// " 00000000 00000840 000001C4 0000000000003A3C ...": priming, padding,
/// sample count in hex.
pub fn parse_itunsmpb(s: &str) -> Option<Priming> {
    let f: Vec<&str> = s.split_whitespace().collect();
    let priming = i64::from_str_radix(f.get(1)?, 16).ok()?;
    let padding = f.get(2).and_then(|p| i64::from_str_radix(p, 16).ok());
    Some(Priming { samples: priming, padding, source: PrimingSource::ITunSmpb })
}

/// Apple encoders name themselves in the handler or the tool tag.
fn apple_encoded(movie: &Movie, trak: &[u8]) -> bool {
    let apple = |b: &[u8]| {
        let s = String::from_utf8_lossy(b);
        s.contains("Apple") || s.contains("Core Media") || s.contains("iTunes") || s.contains("QuickTime")
    };
    let hdlr = child(trak, b"mdia").ok().flatten().and_then(|m| child(m, b"hdlr").ok().flatten());
    if hdlr.is_some_and(|h| h.len() > 24 && apple(&h[24..])) {
        return true;
    }
    let tool = child(&movie.moov, b"udta").ok().flatten().and_then(|u| {
        child(u, b"\xa9too").ok().flatten().or_else(|| {
            let meta = child(u, b"meta").ok().flatten()?;
            let ilst = child(meta.get(4..)?, b"ilst").ok().flatten()?;
            child(ilst, b"\xa9too").ok().flatten()
        })
    });
    tool.is_some_and(apple)
}

/// Priming from the LAME (or FFmpeg's LAME-compatible) tag in the Xing/Info
/// frame of an MP3 file.
pub fn lame_priming(path: &Path) -> Option<Priming> {
    let f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut start = 0u64;
    let mut h = [0u8; 10];
    f.read_exact_at(&mut h, 0).ok()?;
    if &h[..3] == b"ID3" {
        let size = h[6..10].iter().fold(0u64, |a, &b| (a << 7) | (b & 0x7F) as u64);
        start = 10 + size + if h[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let n = (len.saturating_sub(start)).min(1 << 16) as usize;
    let mut buf = vec![0u8; n];
    f.read_exact_at(&mut buf, start).ok()?;
    let p = buf.windows(2).position(|w| w[0] == 0xFF && w[1] & 0xE0 == 0xE0)?;
    let hdr = buf.get(p..p + 4)?;
    let version = (hdr[1] >> 3) & 3; // 3 = MPEG-1
    let mono = (hdr[3] >> 6) & 3 == 3;
    let side = match (version == 3, mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let x = p + 4 + side;
    let tag = buf.get(x..x + 4)?;
    if tag != b"Xing" && tag != b"Info" {
        return None;
    }
    let flags = u32::from_be_bytes(buf.get(x + 4..x + 8)?.try_into().ok()?);
    let mut q = x + 8;
    if flags & 1 != 0 {
        q += 4;
    }
    if flags & 2 != 0 {
        q += 4;
    }
    if flags & 4 != 0 {
        q += 100;
    }
    if flags & 8 != 0 {
        q += 4;
    }
    // LAME extension: encoder string (9), revision (1), lowpass (1), peak (4),
    // two replay gains (4), flags (1), bitrate (1), then 12-bit delay and padding.
    let enc = buf.get(q..q + 4)?;
    if enc != b"LAME" && enc != b"Lavc" && enc != b"Lavf" && enc != b"L3.9" {
        return None;
    }
    let d = buf.get(q + 21..q + 24)?;
    let delay = ((d[0] as i64) << 4) | (d[1] as i64 >> 4);
    let padding = (((d[1] & 0x0F) as i64) << 8) | d[2] as i64;
    Some(Priming { samples: delay + MP3_DECODER_DELAY, padding: Some(padding), source: PrimingSource::LameHeader })
}

/// iTunSMPB in an MP3's ID3v2 COMM frame.
fn id3_itunsmpb(path: &Path) -> Option<Priming> {
    let f = File::open(path).ok()?;
    let mut h = [0u8; 10];
    f.read_exact_at(&mut h, 0).ok()?;
    if &h[..3] != b"ID3" {
        return None;
    }
    let size = h[6..10].iter().fold(0usize, |a, &b| (a << 7) | (b & 0x7F) as usize);
    let mut tag = vec![0u8; size.min(1 << 20)];
    f.read_exact_at(&mut tag, 10).ok()?;
    let at = tag.windows(8).position(|w| w == b"iTunSMPB")?;
    let rest = &tag[at + 8..];
    let s: String = rest.iter().take(200).map(|&b| if b.is_ascii_hexdigit() || b == b' ' { b as char } else { ' ' }).collect();
    parse_itunsmpb(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itunsmpb_fields() {
        let p = parse_itunsmpb(" 00000000 00000840 000001C4 0000000000003A3C 00000000").unwrap();
        assert_eq!((p.samples, p.padding), (2112, Some(452)));
    }
}
