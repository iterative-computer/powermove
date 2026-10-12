//! Frame index (plan section 13, probe item 3).
//!
//! MOV/MP4 come from the sample tables (`stts`, `ctts`, `stss`, `stsz`, `stsc`,
//! `stco`, `elst`); Matroska and WebM from Cues plus a block-header scan (Cues only
//! index keyframes, so per-frame timestamps need the headers but never the
//! payloads); MPEG-TS has no tables and is scanned with FFmpeg's demuxer.

pub mod mkv;
pub mod mp4;

use crate::time::Rational;
use serde::Serialize;
use std::ops::RangeInclusive;

#[derive(Clone, Copy, Debug)]
pub enum Loc {
    /// Bytes at a file offset (MOV/MP4 samples, Matroska block payloads).
    /// `additional` is a Matroska BlockAdditional (VP8/VP9 alpha), id 1.
    File { offset: u64, size: u32, additional: Option<(u64, u32)> },
    /// A packet only the demuxer can reassemble (MPEG-TS PES); `pos` is the byte
    /// position FFmpeg reports for it.
    Demux { pos: i64 },
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub dts: Option<i64>,
    pub pts: i64,
    pub duration: i64,
    pub key: bool,
    pub loc: Loc,
    /// False for samples an edit list decodes but never shows.
    pub presented: bool,
    /// H.264 or HEVC: the sample's first VCL NAL unit type (H.264 IDR = 5;
    /// HEVC CRA = 21, BLA = 16 to 18, RASL = 8 or 9).
    pub nal_type: Option<u8>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum IndexSource {
    /// MOV/MP4 sample tables; nothing scanned.
    Mp4Tables,
    /// Matroska Cues for keyframes plus a header-only block scan for frame timestamps.
    MkvCuesAndBlockScan,
    /// Matroska without Cues: header-only block scan.
    MkvBlockScan,
    /// No container index: every packet demuxed (payloads read, nothing decoded).
    DemuxScan,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct IndexStats {
    pub build_ms: f64,
    pub samples: usize,
    pub presented: usize,
    pub sync_samples: usize,
    /// MOV/MP4: `stss` present (absent means every sample is sync).
    pub has_stss: bool,
    pub has_ctts: bool,
    /// Edit list summary, e.g. "media_time 2002, 1 edit".
    pub edit_list: Option<String>,
    /// Matroska: cue points for this track and how many matched a scanned keyframe.
    pub cue_points: Option<usize>,
    pub cue_points_matched: Option<usize>,
    /// Header reads (Matroska) or packets (demux scan) needed to build the index.
    pub scan_reads: usize,
    /// Open GOP, from each sample's NAL headers (a few bytes per sample, no
    /// payload): sync samples that are not IDR (HEVC CRA or BLA; H.264 non-IDR
    /// I pictures with a recovery point) and their skippable leading pictures.
    pub open_gop: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
}

pub struct FrameIndex {
    pub source: IndexSource,
    pub timebase: Rational,
    /// Decode order.
    pub samples: Vec<Sample>,
    /// Presented samples in presentation order (indices into `samples`).
    pub order: Vec<usize>,
    /// MOV/MP4: sample entry 4CC and its child atoms (avcC, hvcC, av1C, colr, ...).
    pub sample_entry: Option<SampleEntry>,
    /// Matroska CodecPrivate and CodecID.
    pub codec_private: Option<Vec<u8>>,
    pub codec_id: Option<String>,
    /// Set when NAL types were read (H.264, HEVC).
    pub nal_codec: Option<NalCodec>,
    pub stats: IndexStats,
}

#[derive(Clone, Debug)]
pub struct SampleEntry {
    pub fourcc: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub atoms: Vec<([u8; 4], Vec<u8>)>,
}

impl SampleEntry {
    pub fn atom(&self, name: &[u8; 4]) -> Option<&[u8]> {
        self.atoms.iter().find(|a| &a.0 == name).map(|a| a.1.as_slice())
    }
}

impl FrameIndex {
    pub(crate) fn finish(&mut self) {
        let mut order: Vec<usize> = (0..self.samples.len()).filter(|&i| self.samples[i].presented).collect();
        order.sort_by_key(|&i| (self.samples[i].pts, i));
        self.order = order;
        self.stats.samples = self.samples.len();
        self.stats.presented = self.order.len();
        self.stats.sync_samples = self.samples.iter().filter(|s| s.key).count();
    }

    pub fn presented_pts(&self) -> Vec<i64> {
        self.order.iter().map(|&i| self.samples[i].pts).collect()
    }

    /// Decode-order samples to feed for presented frame `n`: from the last sync
    /// sample at or before it in decode order whose own PTS is not after frame n's
    /// (so frame n is never a leading picture of the chosen sync sample: RASL after
    /// an open-GOP CRA or open-GOP H.264 I frame), through frame n itself.
    pub fn decode_range(&self, n: usize) -> RangeInclusive<usize> {
        let s = self.order[n];
        let p = self.samples[s].pts;
        let start = (0..=s).rev().find(|&k| self.samples[k].key && self.samples[k].pts <= p).unwrap_or(0);
        start..=s
    }

    /// Whether decoding that starts at sync sample `k` cannot reconstruct its
    /// leading pictures (open GOP).
    fn open_sync(&self, k: usize) -> bool {
        match (self.nal_codec, self.samples[k].nal_type) {
            (Some(NalCodec::Hevc), Some(t)) => matches!(t, 16..=18 | 21),
            (Some(NalCodec::H264), Some(t)) => t != 5,
            _ => false,
        }
    }

    /// Whether sample `k`, a leading picture of an open sync sample decoding
    /// starts at, is skipped: HEVC RASL pictures (never referenced by trailing
    /// pictures); every H.264 leading picture (x264 and Apple open GOPs never
    /// reference them from trailing pictures; the random-access comparison
    /// would expose a stream that does).
    fn skippable_leading(&self, k: usize) -> bool {
        match self.nal_codec {
            Some(NalCodec::Hevc) => matches!(self.samples[k].nal_type, Some(8 | 9)),
            Some(NalCodec::H264) => true,
            None => false,
        }
    }

    /// The samples of `decode_range(n)` that are actually fed: when the range
    /// starts at an open-GOP sync sample, its leading pictures (decode order
    /// after it, PTS before it) reference pictures before the random-access
    /// point and are skipped.
    pub fn decode_plan(&self, n: usize) -> Vec<usize> {
        let r = self.decode_range(n);
        let start = *r.start();
        let sp = self.samples[start].pts;
        let open = self.open_sync(start);
        r.filter(|&k| k == start || !(open && self.samples[k].pts < sp && self.skippable_leading(k))).collect()
    }

    /// Reads the NAL unit headers of every sample (length-prefixed H.264 or
    /// HEVC in MOV/MP4 or Matroska) and records the first VCL NAL type.
    pub fn mark_nal_types(&mut self, path: &std::path::Path, codec: NalCodec, length_size: usize) -> anyhow::Result<()> {
        use std::os::unix::fs::FileExt;
        let f = std::fs::File::open(path)?;
        self.nal_codec = Some(codec);
        for s in &mut self.samples {
            let Loc::File { offset, size, .. } = s.loc else { anyhow::bail!("NAL scan needs file locations") };
            let mut p = 0u64;
            while p + (length_size as u64) < size as u64 {
                let mut h = [0u8; 5];
                f.read_exact_at(&mut h[..length_size + 1], offset + p)?;
                let len = h[..length_size].iter().fold(0u64, |a, &b| (a << 8) | b as u64);
                if let Some(t) = codec.first_vcl([&h[length_size..length_size + 1]]) {
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
        let open: Vec<usize> = (0..self.samples.len()).filter(|&k| self.samples[k].key && self.open_sync(k)).collect();
        let mut leading = 0;
        for &k in &open {
            let sp = self.samples[k].pts;
            let next = (k + 1..self.samples.len()).find(|&j| self.samples[j].key).unwrap_or(self.samples.len());
            leading += (k + 1..next).filter(|&j| self.samples[j].pts < sp && self.skippable_leading(j)).count();
        }
        let sync = self.samples.iter().filter(|s| s.key).count();
        self.stats.open_gop = Some(format!(
            "{codec:?}: {} of {sync} sync samples open ({}), {leading} skippable leading pictures",
            open.len(),
            if codec == NalCodec::Hevc { "CRA/BLA" } else { "non-IDR" }
        ));
    }

    /// Duplicate PTS among presented frames make "frame N" ambiguous.
    pub fn duplicate_pts(&self) -> usize {
        let p = self.presented_pts();
        p.windows(2).filter(|w| w[0] == w[1]).count()
    }
}
