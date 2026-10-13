//! Frame index (section 13, probe item 3).
//!
//! Built from container tables wherever they exist, without decoding and
//! without reading payloads:
//!
//! | Container | Source |
//! |---|---|
//! | MOV, MP4, M4V, 3GP, 3G2 | `stts`, `ctts`, `stss`, `stsz`/`stz2`, `stsc`, `stco`/`co64`, `elst`; fragmented files from `trex`, `tfhd`, `tfdt`, `trun` |
//! | Matroska, WebM | Cues for keyframes plus a header-only block scan (Cues never list other frames) |
//! | MXF | Index table segments and partition packs |
//! | AVI | `idx1` or OpenDML `indx`/`ix##` (intra-only codecs) |
//! | Raw DV | Constant frame size from the first DIF block |
//! | Everything else (MPEG-TS, M2TS, MPEG-PS, VOB, FLV, ASF, Ogg, ...) | FFmpeg's demuxer reads every packet, decoding nothing |
//!
//! Timestamps stay in the stream's own timebase. Presented frames are listed
//! in presentation order with their presentation timestamp after edit lists;
//! samples keep their media timestamps.

pub mod avi;
pub mod dv;
pub mod mkv;
pub mod mp4;
pub mod mxf;
pub mod scan;

use crate::error::{MediaError, Result};
use pm_time::Rational;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

/// Where a sample's bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Loc {
    /// Bytes at a file offset. `additional` is a Matroska BlockAdditional
    /// with id 1 (VP8 and VP9 alpha).
    File {
        offset: u64,
        size: u32,
        additional: Option<(u64, u32)>,
    },
    /// A packet only the demuxer can reassemble (MPEG-TS and PS, FLV, ASF,
    /// Ogg, elementary streams); `pos` is the byte position FFmpeg reports
    /// for it, `nth` its ordinal among this stream's packets reported at the
    /// same position (Ogg reports the page position for every packet that
    /// starts on the page), and `size` its size (checked when the packet is
    /// read again).
    Demux {
        pos: i64,
        #[serde(default)]
        nth: u16,
        size: u32,
    },
}

/// One coded sample (access unit) in decode order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Decode timestamp in the media timebase, when the container has one.
    pub dts: Option<i64>,
    /// Composition timestamp in the media timebase (before edit lists).
    pub cts: i64,
    pub duration: i64,
    /// A sync sample (random-access point).
    pub key: bool,
    pub loc: Loc,
    /// H.264 or HEVC: the sample's first VCL NAL unit type.
    pub nal_type: Option<u8>,
}

/// A presented frame: a sample at a presentation time (after edit lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// Index into [`FrameIndex::samples`].
    pub sample: usize,
    /// Presentation timestamp in the index timebase, on the presentation
    /// timeline (edit lists applied; PTS 0 is the start of the timeline).
    pub pts: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IndexSource {
    /// MOV/MP4 sample tables; nothing scanned.
    Mp4Tables,
    /// Fragmented MP4: `moof` headers walked (payloads skipped).
    Mp4Fragments,
    /// Matroska Cues for keyframes plus a header-only block scan.
    MkvCuesAndBlockScan,
    /// Matroska without Cues: header-only block scan.
    MkvBlockScan,
    /// MXF index table segments.
    MxfIndexTables,
    /// AVI `idx1` or OpenDML indexes.
    AviIndex,
    /// Raw DV: constant frame size.
    DvConstantFrames,
    /// No container index: every packet demuxed (payloads read, nothing decoded).
    DemuxScan,
    /// No container index and no presentation timestamps on a codec that
    /// reorders (AVI and elementary streams with B frames): every packet
    /// demuxed, and presentation order taken from one decode pass with
    /// reconstruction skipped (no IDCT, no loop filter).
    DemuxScanDecodeOrder,
}

/// Facts about how the index was built.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub build_ms: f64,
    pub samples: usize,
    pub presented: usize,
    pub sync_samples: usize,
    /// MOV/MP4: `stss` present (absent means every sample is sync).
    pub has_stss: bool,
    pub has_ctts: bool,
    /// Edit list summary.
    pub edit_list: Option<String>,
    /// Matroska: cue points for this track and how many matched a scanned keyframe.
    pub cue_points: Option<usize>,
    pub cue_points_matched: Option<usize>,
    /// Header reads (Matroska, fragments) or packets (demux scan).
    pub scan_reads: usize,
    /// Open GOP summary from NAL headers.
    pub open_gop: Option<String>,
    /// Why the container tables were not used (demux-scan fallback).
    pub fallback_reason: Option<String>,
    /// Container-specific notes (which AVI index, MXF segment layout, DV frame size).
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NalCodec {
    H264,
    Hevc,
}

impl NalCodec {
    /// Type of the first VCL NAL unit, given NAL unit payloads in order.
    pub fn first_vcl<'a>(self, nals: impl IntoIterator<Item = &'a [u8]>) -> Option<u8> {
        for n in nals {
            let Some(&b) = n.first() else { continue };
            match self {
                NalCodec::H264 => {
                    let t = b & 0x1F;
                    if (1..=5).contains(&t) {
                        return Some(t);
                    }
                }
                NalCodec::Hevc => {
                    let t = (b >> 1) & 0x3F;
                    if t < 32 {
                        return Some(t);
                    }
                }
            }
        }
        None
    }

    /// Whether a sync sample with this first VCL NAL type is an open-GOP
    /// random-access point (its leading pictures may reference pictures
    /// before it): HEVC CRA and BLA, H.264 non-IDR I pictures.
    pub fn open_sync(self, nal_type: u8) -> bool {
        match self {
            NalCodec::Hevc => matches!(nal_type, 16..=18 | 21),
            NalCodec::H264 => nal_type != 5,
        }
    }
}

/// Codec configuration the container carries.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodecConfig {
    /// MOV/MP4 sample entry four-character code (`avc1`, `apch`, ...).
    pub fourcc: Option<[u8; 4]>,
    /// Coded size from the sample entry.
    pub width: u16,
    pub height: u16,
    /// MOV/MP4: the sample entry's child atoms (`avcC`, `hvcC`, `colr`, `fiel`, ...).
    pub atoms: Vec<([u8; 4], Vec<u8>)>,
    /// Matroska CodecID and CodecPrivate.
    pub codec_id: Option<String>,
    pub codec_private: Option<Vec<u8>>,
    /// Matroska ContentEncoding header stripping: bytes to prepend to every frame.
    pub strip_prefix: Option<Vec<u8>>,
}

impl CodecConfig {
    pub fn atom(&self, name: &[u8; 4]) -> Option<&[u8]> {
        self.atoms
            .iter()
            .find(|a| &a.0 == name)
            .map(|a| a.1.as_slice())
    }
}

/// The frame index of one stream.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameIndex {
    pub source: IndexSource,
    /// Container track id (MP4 `track_ID`, Matroska TrackNumber, MXF track
    /// id, AVI stream number, or FFmpeg stream index for scans).
    pub track: u64,
    /// Seconds per timestamp unit.
    #[serde(with = "rational_serde")]
    pub timebase: Rational,
    /// Decode order.
    pub samples: Vec<Sample>,
    /// Presented frames in presentation order.
    pub frames: Vec<Frame>,
    pub config: CodecConfig,
    /// Set when NAL types were read (H.264, HEVC).
    pub nal_codec: Option<NalCodec>,
    pub stats: IndexStats,
}

pub(crate) mod rational_serde {
    use pm_time::Rational;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(r: &Rational, s: S) -> Result<S::Ok, S::Error> {
        [r.num(), r.den()].serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Rational, D::Error> {
        let [n, den] = <[i64; 2]>::deserialize(d)?;
        Rational::new(n, den).map_err(serde::de::Error::custom)
    }
}

impl FrameIndex {
    /// Sorts frames into presentation order and fills the counts.
    pub(crate) fn finish(&mut self) {
        self.frames.sort_by_key(|f| (f.pts, f.sample));
        self.stats.samples = self.samples.len();
        self.stats.presented = self.frames.len();
        self.stats.sync_samples = self.samples.iter().filter(|s| s.key).count();
    }

    /// An index presenting every sample once at its composition time.
    pub(crate) fn present_all(&mut self) {
        self.frames = self
            .samples
            .iter()
            .enumerate()
            .map(|(i, s)| Frame {
                sample: i,
                pts: s.cts,
            })
            .collect();
    }

    /// Presented PTS in presentation order.
    pub fn presented_pts(&self) -> Vec<i64> {
        self.frames.iter().map(|f| f.pts).collect()
    }

    /// Samples decoded but never presented (edit lists).
    pub fn hidden_samples(&self) -> usize {
        let mut shown = vec![false; self.samples.len()];
        for f in &self.frames {
            shown[f.sample] = true;
        }
        shown.iter().filter(|s| !**s).count()
    }

    /// Duplicate presentation timestamps make "frame N" ambiguous.
    pub fn duplicate_pts(&self) -> usize {
        self.frames
            .windows(2)
            .filter(|w| w[0].pts == w[1].pts)
            .count()
    }

    /// The sync sample decoding starts at for sample `s`: the last sync
    /// sample at or before it in decode order whose own composition time is
    /// not after `s`'s (so `s` is never a leading picture of the chosen sync
    /// sample).
    pub fn sync_for(&self, s: usize) -> usize {
        let cts = self.samples[s].cts;
        (0..=s)
            .rev()
            .find(|&k| self.samples[k].key && self.samples[k].cts <= cts)
            .unwrap_or(0)
    }

    /// Decode-order samples to feed to show sample `s`: from [`sync_for`]
    /// through `s`, without the sync sample's leading pictures (decode order
    /// after it, composition time before it). Leading pictures may reference
    /// pictures before the random-access point (open GOP: HEVC RASL, H.264
    /// and MPEG-2 open-GOP B pictures) and no trailing picture references
    /// them, so they are never needed for a frame at or after the sync
    /// sample.
    ///
    /// [`sync_for`]: FrameIndex::sync_for
    pub fn decode_plan(&self, s: usize) -> Vec<usize> {
        let start = self.sync_for(s);
        let sp = self.samples[start].cts;
        (start..=s)
            .filter(|&k| k == start || k == s || self.samples[k].cts >= sp)
            .collect()
    }

    /// [`decode_plan`] for presented frame `n` (presentation order).
    ///
    /// [`decode_plan`]: FrameIndex::decode_plan
    pub fn decode_plan_for_frame(&self, n: usize) -> Vec<usize> {
        self.decode_plan(self.frames[n].sample)
    }

    /// Reads the NAL unit headers of every sample (length-prefixed H.264 or
    /// HEVC in MOV/MP4 or Matroska) and records the first VCL NAL type.
    /// Reads a few bytes per NAL unit, never a payload.
    pub fn mark_nal_types(
        &mut self,
        path: &Path,
        codec: NalCodec,
        length_size: usize,
    ) -> Result<()> {
        if !(1..=4).contains(&length_size) {
            return Err(MediaError::invalid(format!(
                "NAL length size {length_size}"
            )));
        }
        let f = File::open(path)?;
        self.nal_codec = Some(codec);
        let strip = self.config.strip_prefix.clone().unwrap_or_default();
        for s in &mut self.samples {
            let Loc::File { offset, size, .. } = s.loc else {
                return Err(MediaError::invalid("NAL scan needs file locations"));
            };
            // With header stripping the stored bytes start mid-sample; the
            // stripped prefix comes first.
            let mut p = 0u64;
            let total = size as u64 + strip.len() as u64;
            let mut hdr = [0u8; 5];
            while p + (length_size as u64) < total {
                for (i, b) in hdr[..length_size + 1].iter_mut().enumerate() {
                    let at = p + i as u64;
                    *b = if at < strip.len() as u64 {
                        strip[at as usize]
                    } else {
                        let mut one = [0u8; 1];
                        f.read_exact_at(&mut one, offset + at - strip.len() as u64)?;
                        one[0]
                    };
                }
                let len = hdr[..length_size]
                    .iter()
                    .fold(0u64, |a, &b| (a << 8) | b as u64);
                if let Some(t) = codec.first_vcl([&hdr[length_size..length_size + 1]]) {
                    s.nal_type = Some(t);
                    break;
                }
                p += length_size as u64 + len;
            }
        }
        self.summarize_open_gop();
        Ok(())
    }

    pub(crate) fn summarize_open_gop(&mut self) {
        let Some(codec) = self.nal_codec else { return };
        let open: Vec<usize> = (0..self.samples.len())
            .filter(|&k| {
                self.samples[k].key && self.samples[k].nal_type.is_some_and(|t| codec.open_sync(t))
            })
            .collect();
        let mut leading = 0;
        for &k in &open {
            let sp = self.samples[k].cts;
            let next = (k + 1..self.samples.len())
                .find(|&j| self.samples[j].key)
                .unwrap_or(self.samples.len());
            leading += (k + 1..next).filter(|&j| self.samples[j].cts < sp).count();
        }
        let sync = self.samples.iter().filter(|s| s.key).count();
        self.stats.open_gop = Some(format!(
            "{codec:?}: {} of {sync} sync samples open ({}), {leading} leading pictures skipped on random access",
            open.len(),
            if codec == NalCodec::Hevc {
                "CRA/BLA"
            } else {
                "non-IDR"
            }
        ));
    }
}

/// Builds the index of stream `stream` (an FFmpeg stream index) of the file,
/// choosing the container tables when they exist.
pub fn build(path: &Path, input: &crate::ff::Input, stream: usize) -> Result<FrameIndex> {
    let t0 = std::time::Instant::now();
    let format = input.format_name();
    let track_id = input.stream_id(stream);
    let reorders = input.codec_reorders(stream);
    let attempt: Option<Result<FrameIndex>> = if format.contains("mov") || format.contains("mp4") {
        Some(mp4::build(path, mp4::TrackSelect::Id(track_id as u32)))
    } else if format.contains("matroska") || format.contains("webm") {
        Some(mkv::build(path, track_id))
    } else if format == "mxf" {
        Some(mxf::build(path, input, stream))
    } else if format == "avi" && !reorders {
        Some(avi::build(path, input, stream))
    } else if format == "dv" {
        Some(dv::build(path, input, stream))
    } else {
        None
    };
    let mut idx = match attempt {
        Some(Ok(idx)) => idx,
        Some(Err(e)) if e.is_unsupported() => {
            let mut idx = scan::build(path, stream)?;
            idx.stats.fallback_reason = Some(e.to_string());
            idx
        }
        Some(Err(e)) => return Err(e),
        None => {
            let mut idx = scan::build(path, stream)?;
            idx.stats.fallback_reason = Some(if format == "avi" {
                "AVI with a reordering codec (AVI has no composition times): timestamps come from the demuxer".into()
            } else {
                format!("{format} has no frame table")
            });
            idx
        }
    };
    // Open GOP: NAL types from the length-prefixed samples (MPEG-TS packets
    // are marked during the scan).
    let config = match (idx.config.fourcc.as_ref(), idx.config.codec_id.as_deref()) {
        (Some(b"hvc1" | b"hev1" | b"dvh1" | b"dvhe"), _) => idx
            .config
            .atom(b"hvcC")
            .map(|a| (NalCodec::Hevc, a.to_vec())),
        (Some(b"avc1" | b"avc3"), _) => idx
            .config
            .atom(b"avcC")
            .map(|a| (NalCodec::H264, a.to_vec())),
        (None, Some("V_MPEGH/ISO/HEVC")) => idx
            .config
            .codec_private
            .clone()
            .map(|c| (NalCodec::Hevc, c)),
        (None, Some("V_MPEG4/ISO/AVC")) => idx
            .config
            .codec_private
            .clone()
            .map(|c| (NalCodec::H264, c)),
        _ => None,
    };
    if let Some((codec, c)) = config
        && let Some(ls) = nal_length_size(codec, &c)
        && idx
            .samples
            .iter()
            .all(|s| matches!(s.loc, Loc::File { .. }))
    {
        idx.mark_nal_types(path, codec, ls)?;
    }
    idx.stats.build_ms = t0.elapsed().as_secs_f64() * 1e3;
    if idx.frames.is_empty() {
        return Err(MediaError::invalid("the stream has no presented frames"));
    }
    Ok(idx)
}

/// NAL unit length size from an `avcC` or `hvcC` record.
pub fn nal_length_size(codec: NalCodec, config: &[u8]) -> Option<usize> {
    match codec {
        NalCodec::Hevc if config.len() > 21 => Some((config[21] & 3) as usize + 1),
        NalCodec::H264 if config.len() > 4 => Some((config[4] & 3) as usize + 1),
        _ => None,
    }
}

/// Big-endian cursor over a box or element payload.
pub(crate) struct Cur<'a> {
    pub b: &'a [u8],
    pub p: usize,
}

impl<'a> Cur<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Cur { b, p: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.p.checked_add(n).is_none_or(|e| e > self.b.len()) {
            return Err(MediaError::invalid("truncated structure"));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    pub fn left(&self) -> usize {
        self.b.len() - self.p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cts: i64, key: bool) -> Sample {
        Sample {
            dts: None,
            cts,
            duration: 1,
            key,
            loc: Loc::Demux {
                pos: 0,
                nth: 0,
                size: 1,
            },
            nal_type: None,
        }
    }

    fn index(samples: Vec<Sample>) -> FrameIndex {
        let mut idx = FrameIndex {
            source: IndexSource::DemuxScan,
            track: 0,
            timebase: Rational::new(1, 25).unwrap(),
            samples,
            frames: vec![],
            config: CodecConfig::default(),
            nal_codec: None,
            stats: IndexStats::default(),
        };
        idx.present_all();
        idx.finish();
        idx
    }

    #[test]
    fn decode_plans_skip_leading_pictures_of_an_open_gop() {
        // Decode order: I0 P3 B1 B2 | I6 B4 B5 P9 B7 B8 (open GOP: B4, B5 lead I6).
        let cts = [0, 3, 1, 2, 6, 4, 5, 9, 7, 8];
        let idx = index(cts.iter().map(|&c| sample(c, c % 6 == 0)).collect());
        // B4 (sample 5) leads I6: decoding starts at I0.
        assert_eq!(idx.decode_plan(5), vec![0, 1, 2, 3, 4, 5]);
        // B7 (sample 8): start at I6, skip B4 and B5.
        assert_eq!(idx.decode_plan(8), vec![4, 7, 8]);
        // I6 itself.
        assert_eq!(idx.decode_plan(4), vec![4]);
        // Presentation order and plans by frame.
        assert_eq!(idx.presented_pts(), (0..10).collect::<Vec<i64>>());
        assert_eq!(idx.decode_plan_for_frame(7), vec![4, 7, 8]);
    }
}
