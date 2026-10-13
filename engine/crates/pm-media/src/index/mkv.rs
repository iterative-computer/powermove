//! Matroska and WebM index: Cues for keyframes, plus a header-only block
//! scan for per-frame timestamps (Cues never list other frames). Block
//! payloads are skipped; BlockAdditional (alpha) locations are recorded.

use super::{CodecConfig, FrameIndex, IndexSource, IndexStats, Loc, Sample};
use crate::error::{MediaError, Result};
use pm_time::Rational;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const CODEC_PRIVATE: u32 = 0x63A2;
const DEFAULT_DURATION: u32 = 0x23_E383;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const CONTENT_ENCODING: u32 = 0x6240;
const CONTENT_ENCODING_TYPE: u32 = 0x5033;
const CONTENT_COMPRESSION: u32 = 0x5034;
const CONTENT_COMP_ALGO: u32 = 0x4254;
const CONTENT_COMP_SETTINGS: u32 = 0x4255;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const REFERENCE_BLOCK: u32 = 0xFB;
const BLOCK_ADDITIONS: u32 = 0x75A1;
const BLOCK_MORE: u32 = 0xA6;
const BLOCK_ADD_ID: u32 = 0xEE;
const BLOCK_ADDITIONAL: u32 = 0xA5;
const BLOCK_DURATION: u32 = 0x9B;

/// Top-level Segment children; an unknown-size Cluster ends at the next one.
const TOP_LEVEL: [u32; 8] = [
    CLUSTER,
    CUES,
    INFO,
    TRACKS,
    0x114D_9B74,
    0x1043_A770,
    0x1941_A469,
    0x1254_C367,
];

struct Reader {
    f: File,
    len: u64,
    reads: usize,
}

impl Reader {
    fn read(&mut self, off: u64, n: usize) -> Result<Vec<u8>> {
        let n = n.min(self.len.saturating_sub(off) as usize);
        let mut b = vec![0u8; n];
        self.f.read_exact_at(&mut b, off)?;
        self.reads += 1;
        Ok(b)
    }
    /// Element header at `off`: (id, data offset, data size or None if unknown).
    fn header(&mut self, off: u64) -> Result<(u32, u64, Option<u64>)> {
        let b = self.read(off, 12)?;
        let (id, il) = vint_id(&b)?;
        let (size, sl) = vint_size(&b[il..])?;
        Ok((id, off + (il + sl) as u64, size))
    }
}

fn vint_id(b: &[u8]) -> Result<(u32, usize)> {
    let first = *b
        .first()
        .ok_or_else(|| MediaError::invalid("end of file in an element id"))?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 || b.len() < len {
        return Err(MediaError::invalid("bad EBML element id"));
    }
    Ok((b[..len].iter().fold(0u32, |a, &x| (a << 8) | x as u32), len))
}

pub(crate) fn vint_size(b: &[u8]) -> Result<(Option<u64>, usize)> {
    let first = *b
        .first()
        .ok_or_else(|| MediaError::invalid("end of file in an element size"))?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || b.len() < len {
        return Err(MediaError::invalid("bad EBML element size"));
    }
    let mut v = (first as u64) & (0xFF >> len);
    let mut all_ones = v == (0xFF >> len) as u64;
    for &x in &b[1..len] {
        v = (v << 8) | x as u64;
        all_ones &= x == 0xFF;
    }
    Ok((if all_ones { None } else { Some(v) }, len))
}

fn uint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |a, &x| (a << 8) | x as u64)
}

/// Children of an in-memory master element: (id, payload range within `b`).
fn kids(b: &[u8]) -> Result<Vec<(u32, std::ops::Range<usize>)>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < b.len() {
        let (id, il) = vint_id(&b[p..])?;
        let (size, sl) = vint_size(&b[p + il..])?;
        let size = size
            .ok_or_else(|| MediaError::invalid("unknown size inside a small master element"))?
            as usize;
        let start = p + il + sl;
        if start + size > b.len() {
            return Err(MediaError::invalid("element overruns its parent"));
        }
        out.push((id, start..start + size));
        p = start + size;
    }
    Ok(out)
}

#[derive(Default)]
struct Track {
    number: u64,
    ty: u64,
    codec_id: String,
    codec_private: Option<Vec<u8>>,
    default_duration_ns: Option<u64>,
    strip_prefix: Option<Vec<u8>>,
    /// A content encoding the index cannot express (compression other than
    /// header stripping, or encryption).
    unsupported_encoding: Option<String>,
}

fn parse_track(e: &[u8]) -> Result<Track> {
    let mut t = Track::default();
    for (k, kr) in kids(e)? {
        match k {
            TRACK_NUMBER => t.number = uint(&e[kr]),
            TRACK_TYPE => t.ty = uint(&e[kr]),
            CODEC_ID => {
                t.codec_id = String::from_utf8_lossy(&e[kr])
                    .trim_end_matches('\0')
                    .to_string()
            }
            CODEC_PRIVATE => t.codec_private = Some(e[kr].to_vec()),
            DEFAULT_DURATION => t.default_duration_ns = Some(uint(&e[kr])),
            CONTENT_ENCODINGS => {
                let encs = &e[kr];
                for (ek, er) in kids(encs)? {
                    if ek != CONTENT_ENCODING {
                        continue;
                    }
                    let enc = &encs[er];
                    let mut ty = 0;
                    let mut algo = None;
                    let mut settings = None;
                    for (ck, cr) in kids(enc)? {
                        match ck {
                            CONTENT_ENCODING_TYPE => ty = uint(&enc[cr]),
                            CONTENT_COMPRESSION => {
                                let comp = &enc[cr];
                                algo = Some(0);
                                for (ak, ar) in kids(comp)? {
                                    match ak {
                                        CONTENT_COMP_ALGO => algo = Some(uint(&comp[ar])),
                                        CONTENT_COMP_SETTINGS => settings = Some(comp[ar].to_vec()),
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    match (ty, algo) {
                        (0, Some(3)) => t.strip_prefix = settings,
                        (0, Some(a)) => {
                            t.unsupported_encoding =
                                Some(format!("Matroska content compression {a}"))
                        }
                        (1, _) => {
                            t.unsupported_encoding = Some("Matroska content encryption".into())
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(t)
}

/// Builds the index of the track with TrackNumber `track`, or of the first
/// video track when `track` is 0.
pub fn build(path: &Path, track: u64) -> Result<FrameIndex> {
    let f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut r = Reader { f, len, reads: 0 };

    let (id, data, size) = r.header(0)?;
    if id != EBML {
        return Err(MediaError::invalid("not EBML"));
    }
    let mut off = data + size.ok_or_else(|| MediaError::invalid("EBML header of unknown size"))?;
    let (id, seg_data, seg_size) = r.header(off)?;
    if id != SEGMENT {
        return Err(MediaError::invalid("no Segment"));
    }
    let seg_end = seg_size.map(|s| seg_data + s).unwrap_or(len).min(len);

    let mut ts_scale = 1_000_000u64;
    let mut chosen: Option<Track> = None;
    let mut cues_raw: Option<Vec<u8>> = None;
    let mut samples: Vec<Sample> = Vec::new();

    off = seg_data;
    while off < seg_end {
        let (id, data, size) = r.header(off)?;
        match id {
            INFO => {
                let b = r.read(
                    data,
                    size.ok_or_else(|| MediaError::invalid("Info of unknown size"))? as usize,
                )?;
                for (cid, rg) in kids(&b)? {
                    if cid == TIMESTAMP_SCALE {
                        ts_scale = uint(&b[rg]);
                    }
                }
            }
            TRACKS => {
                let b = r.read(
                    data,
                    size.ok_or_else(|| MediaError::invalid("Tracks of unknown size"))? as usize,
                )?;
                for (cid, rg) in kids(&b)? {
                    if cid != TRACK_ENTRY {
                        continue;
                    }
                    let t = parse_track(&b[rg])?;
                    let wanted = if track == 0 {
                        t.ty == 1
                    } else {
                        t.number == track
                    };
                    if wanted && chosen.is_none() {
                        chosen = Some(t);
                    }
                }
                if let Some(t) = &chosen
                    && let Some(why) = &t.unsupported_encoding
                {
                    return Err(MediaError::unsupported(why));
                }
            }
            CUES => {
                cues_raw = Some(r.read(
                    data,
                    size.ok_or_else(|| MediaError::invalid("Cues of unknown size"))? as usize,
                )?);
            }
            CLUSTER => {
                let number = chosen
                    .as_ref()
                    .ok_or_else(|| MediaError::invalid("Cluster before Tracks, or no such track"))?
                    .number;
                let end = match size {
                    Some(s) => (data + s).min(seg_end),
                    None => unknown_cluster_end(&mut r, data, seg_end)?,
                };
                scan_cluster(&mut r, data, end, number, &mut samples)?;
                off = end;
                continue;
            }
            _ => {}
        }
        match size {
            Some(s) => off = data + s,
            None => {
                return Err(MediaError::invalid(format!(
                    "unknown-size element 0x{id:X} outside a Cluster"
                )));
            }
        }
    }
    let t = chosen.ok_or_else(|| MediaError::invalid("no such track"))?;
    if samples.is_empty() {
        return Err(MediaError::invalid("track has no blocks"));
    }
    let mut cue_points = None;
    let mut cue_matched = None;
    if let Some(b) = &cues_raw {
        let mut cues: Vec<u64> = Vec::new();
        for (cid, rg) in kids(b)? {
            if cid != CUE_POINT {
                continue;
            }
            let p = &b[rg];
            let mut time = None;
            for (k, kr) in kids(p)? {
                match k {
                    CUE_TIME => time = Some(uint(&p[kr])),
                    CUE_TRACK_POSITIONS => {
                        let tp = &p[kr];
                        let mut tr = 0;
                        for (kk, kkr) in kids(tp)? {
                            if kk == CUE_TRACK {
                                tr = uint(&tp[kkr]);
                            }
                        }
                        if tr == t.number
                            && let Some(tm) = time
                        {
                            cues.push(tm);
                        }
                    }
                    _ => {}
                }
            }
        }
        let keys: std::collections::HashSet<i64> =
            samples.iter().filter(|s| s.key).map(|s| s.cts).collect();
        cue_points = Some(cues.len());
        cue_matched = Some(
            cues.iter()
                .filter(|&&tm| keys.contains(&(tm as i64)))
                .count(),
        );
    }

    // Durations from the next PTS in presentation order (video blocks rarely
    // carry BlockDuration); the last uses DefaultDuration.
    let mut by_pts: Vec<usize> = (0..samples.len()).collect();
    by_pts.sort_by_key(|&i| (samples[i].cts, i));
    let default_dur = t
        .default_duration_ns
        .map(|ns| ((ns as i128 + ts_scale as i128 / 2) / ts_scale as i128) as i64)
        .unwrap_or(0);
    for w in 0..by_pts.len() {
        if samples[by_pts[w]].duration == 0 {
            samples[by_pts[w]].duration = if w + 1 < by_pts.len() {
                samples[by_pts[w + 1]].cts - samples[by_pts[w]].cts
            } else {
                default_dur
            };
        }
    }

    let stats = IndexStats {
        cue_points,
        cue_points_matched: cue_matched,
        scan_reads: r.reads,
        ..Default::default()
    };
    let mut idx = FrameIndex {
        source: if cues_raw.is_some() {
            IndexSource::MkvCuesAndBlockScan
        } else {
            IndexSource::MkvBlockScan
        },
        track: t.number,
        timebase: Rational::new(ts_scale as i64, 1_000_000_000)?,
        samples,
        frames: vec![],
        config: CodecConfig {
            codec_id: Some(t.codec_id),
            codec_private: t.codec_private,
            strip_prefix: t.strip_prefix,
            ..Default::default()
        },
        nal_codec: None,
        stats,
    };
    idx.present_all();
    idx.finish();
    Ok(idx)
}

fn unknown_cluster_end(r: &mut Reader, mut off: u64, seg_end: u64) -> Result<u64> {
    while off < seg_end {
        let (id, data, size) = r.header(off)?;
        if TOP_LEVEL.contains(&id) {
            return Ok(off);
        }
        off = data
            + size.ok_or_else(|| {
                MediaError::invalid("unknown size inside an unknown-size Cluster")
            })?;
    }
    Ok(seg_end)
}

fn scan_cluster(
    r: &mut Reader,
    mut off: u64,
    end: u64,
    track: u64,
    out: &mut Vec<Sample>,
) -> Result<()> {
    let mut cluster_ts: Option<i64> = None;
    while off < end {
        let (id, data, size) = r.header(off)?;
        let size = size.ok_or_else(|| MediaError::invalid("unknown-size Cluster child"))?;
        match id {
            CLUSTER_TIMESTAMP => cluster_ts = Some(uint(&r.read(data, size as usize)?) as i64),
            SIMPLE_BLOCK => {
                if let Some(s) = block(r, data, size, cluster_ts, track, None)? {
                    out.push(s);
                }
            }
            BLOCK_GROUP => {
                let mut p = data;
                let gend = data + size;
                let mut blk = None;
                let mut has_ref = false;
                let mut additional = None;
                let mut duration = None;
                while p < gend {
                    let (cid, cdata, csize) = r.header(p)?;
                    let csize =
                        csize.ok_or_else(|| MediaError::invalid("unknown size in a BlockGroup"))?;
                    match cid {
                        BLOCK => blk = Some((cdata, csize)),
                        REFERENCE_BLOCK => has_ref = true,
                        BLOCK_DURATION => {
                            duration = Some(uint(&r.read(cdata, csize as usize)?) as i64)
                        }
                        BLOCK_ADDITIONS => {
                            let b = r.read(cdata, csize as usize)?;
                            for (k, kr) in kids(&b)? {
                                if k != BLOCK_MORE {
                                    continue;
                                }
                                let m = &b[kr.clone()];
                                let mut add_id = 1;
                                let mut payload = None;
                                for (kk, kkr) in kids(m)? {
                                    match kk {
                                        BLOCK_ADD_ID => add_id = uint(&m[kkr]),
                                        BLOCK_ADDITIONAL => {
                                            payload = Some((
                                                cdata + (kr.start + kkr.start) as u64,
                                                kkr.len() as u32,
                                            ))
                                        }
                                        _ => {}
                                    }
                                }
                                if add_id == 1 {
                                    additional = payload;
                                }
                            }
                        }
                        _ => {}
                    }
                    p = cdata + csize;
                }
                if let Some((bd, bs)) = blk
                    && let Some(mut s) = block(r, bd, bs, cluster_ts, track, Some(!has_ref))?
                {
                    if let Loc::File { additional: a, .. } = &mut s.loc {
                        *a = additional;
                    }
                    if let Some(d) = duration {
                        s.duration = d;
                    }
                    out.push(s);
                }
            }
            _ => {}
        }
        off = data + size;
    }
    Ok(())
}

/// `group_key`: None for a SimpleBlock (keyframe flag in its header); for a
/// Block inside a BlockGroup, Some(no ReferenceBlock present).
fn block(
    r: &mut Reader,
    data: u64,
    size: u64,
    cluster_ts: Option<i64>,
    track: u64,
    group_key: Option<bool>,
) -> Result<Option<Sample>> {
    let h = r.read(data, 12.min(size as usize))?;
    let (tn, tl) = vint_size(&h)?;
    if tn != Some(track) {
        return Ok(None);
    }
    if h.len() < tl + 3 {
        return Err(MediaError::invalid("short block header"));
    }
    let rel = i16::from_be_bytes([h[tl], h[tl + 1]]) as i64;
    let flags = h[tl + 2];
    if flags & 0x06 != 0 {
        return Err(MediaError::unsupported("laced blocks on this track"));
    }
    let hdr = tl as u64 + 3;
    let ct = cluster_ts.ok_or_else(|| MediaError::invalid("block before ClusterTimestamp"))?;
    Ok(Some(Sample {
        dts: None,
        cts: ct + rel,
        duration: 0,
        key: group_key.unwrap_or(flags & 0x80 != 0),
        loc: Loc::File {
            offset: data + hdr,
            size: (size - hdr) as u32,
            additional: None,
        },
        nal_type: None,
    }))
}
