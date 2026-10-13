//! MXF frame index from index table segments (SMPTE ST 377-1 section 11).
//!
//! Partitions are located through the Random Index Pack (or the footer's
//! previous-partition chain). Index table segments give each edit unit's
//! stream offset, flags, and temporal offset (or a constant edit-unit byte
//! count); partition packs map stream offsets to file positions. Within each
//! content package only KLV headers are read, to find the picture element.

use super::{CodecConfig, Frame, FrameIndex, IndexSource, IndexStats, Loc, Sample};
use crate::error::{MediaError, Result};
use pm_time::Rational;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

const UL_PREFIX: [u8; 4] = [0x06, 0x0E, 0x2B, 0x34];

/// A KLV header: key, value offset, value length.
#[derive(Clone, Copy, Debug)]
pub struct Klv {
    pub key: [u8; 16],
    pub offset: u64,
    pub value: u64,
    pub len: u64,
}

impl Klv {
    pub fn end(&self) -> u64 {
        self.value + self.len
    }
    fn is(&self, pattern: &[u8]) -> bool {
        self.key[..pattern.len()] == *pattern
    }
    /// Partition pack: 06.0E.2B.34.02.05.01.01.0D.01.02.01.01.{02,03,04}.xx.
    pub fn is_partition(&self) -> bool {
        self.is(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01])
            && matches!(self.key[13], 0x02..=0x04)
    }
    pub fn is_index_segment(&self) -> bool {
        self.is(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01])
    }
    pub fn is_fill(&self) -> bool {
        self.key[..4] == UL_PREFIX && self.key[4..7] == [0x01, 0x01, 0x01] && self.key[8..13] == [0x03, 0x01, 0x02, 0x10, 0x01]
    }
    /// Generic container content-package items (system, picture, sound,
    /// data, compound), or Avid's essence keys.
    pub fn is_essence(&self) -> bool {
        self.key[..4] == UL_PREFIX
            && (self.key[8..12] == [0x0D, 0x01, 0x03, 0x01] || self.key[8..12] == [0x0E, 0x04, 0x03, 0x01])
    }
    /// A picture element (generic container 0x15, content package 0x05).
    pub fn is_picture(&self) -> bool {
        self.is_essence() && self.key[4] == 0x01 && matches!(self.key[12], 0x15 | 0x05)
    }
}

/// Reads a KLV header at `off` from a buffer that starts at file offset `base`.
pub fn parse_klv(buf: &[u8], base: u64, off: u64) -> Option<Klv> {
    let p = (off - base) as usize;
    let key: [u8; 16] = buf.get(p..p + 16)?.try_into().ok()?;
    let b = *buf.get(p + 16)?;
    let (len, hl) = if b & 0x80 == 0 {
        (b as u64, 1)
    } else {
        let n = (b & 0x7F) as usize;
        if n == 0 || n > 8 {
            return None;
        }
        (buf.get(p + 17..p + 17 + n)?.iter().fold(0u64, |a, &x| (a << 8) | x as u64), 1 + n)
    };
    Some(Klv { key, offset: off, value: off + 16 + hl as u64, len })
}

pub fn read_klv(f: &File, off: u64, file_len: u64) -> Result<Klv> {
    let mut b = [0u8; 25];
    let n = (file_len.saturating_sub(off) as usize).min(25);
    f.read_exact_at(&mut b[..n], off)?;
    let k = parse_klv(&b[..n], off, off).ok_or_else(|| MediaError::invalid(format!("bad KLV at {off}")))?;
    if k.key[..4] != UL_PREFIX {
        return Err(MediaError::invalid(format!("no SMPTE key at {off}")));
    }
    Ok(k)
}

#[derive(Clone, Debug)]
pub struct Partition {
    pub offset: u64,
    pub pack_end: u64,
    pub previous: u64,
    pub footer: u64,
    pub header_bytes: u64,
    pub index_bytes: u64,
    pub index_sid: u32,
    pub body_offset: u64,
    pub body_sid: u32,
}

fn read_partition(f: &File, off: u64, len: u64) -> Result<Partition> {
    let k = read_klv(f, off, len)?;
    if !k.is_partition() || k.len < 64 {
        return Err(MediaError::invalid(format!("no partition pack at {off}")));
    }
    let mut v = vec![0u8; 64];
    f.read_exact_at(&mut v, k.value)?;
    let u64_at = |p: usize| u64::from_be_bytes(v[p..p + 8].try_into().unwrap());
    let u32_at = |p: usize| u32::from_be_bytes(v[p..p + 4].try_into().unwrap());
    Ok(Partition {
        offset: off,
        pack_end: k.end(),
        // 2 + 2 + 4 (versions, KAG), then:
        previous: u64_at(16),
        footer: u64_at(24),
        header_bytes: u64_at(32),
        index_bytes: u64_at(40),
        index_sid: u32_at(48),
        body_offset: u64_at(52),
        body_sid: u32_at(60),
    })
}

/// Every partition, from the Random Index Pack or the footer chain.
pub fn partitions(f: &File) -> Result<Vec<Partition>> {
    let len = f.metadata()?.len();
    let header = read_partition(f, 0, len)?;
    let mut offsets: Vec<u64> = Vec::new();
    // Random Index Pack: the last 4 bytes are its total length.
    if len > 20 {
        let mut t = [0u8; 4];
        f.read_exact_at(&mut t, len - 4)?;
        let rip_len = u32::from_be_bytes(t) as u64;
        if rip_len >= 20 && rip_len < len
            && let Ok(k) = read_klv(f, len - rip_len, len)
            && k.is(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x11, 0x01])
        {
            let mut v = vec![0u8; k.len as usize];
            f.read_exact_at(&mut v, k.value)?;
            // (BodySID u32, ByteOffset u64) pairs, then the 4-byte length.
            for e in v[..v.len().saturating_sub(4)].chunks_exact(12) {
                offsets.push(u64::from_be_bytes(e[4..12].try_into().unwrap()));
            }
        }
    }
    if offsets.is_empty() {
        // Footer chain.
        let mut at = header.footer;
        if at == 0 {
            return Err(MediaError::unsupported("MXF without a Random Index Pack or footer partition"));
        }
        let mut seen = std::collections::HashSet::new();
        while seen.insert(at) {
            offsets.push(at);
            let p = read_partition(f, at, len)?;
            if p.previous == at {
                break;
            }
            at = p.previous;
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    offsets.into_iter().map(|o| read_partition(f, o, len)).collect()
}

#[derive(Clone, Debug, Default)]
pub struct IndexSegment {
    pub edit_rate: (i32, i32),
    pub start: i64,
    pub duration: i64,
    pub edit_unit_bytes: u32,
    pub index_sid: u32,
    pub body_sid: u32,
    pub slice_count: u8,
    pub pos_table_count: u8,
    /// (temporal offset, key frame offset, flags, stream offset).
    pub entries: Vec<(i8, i8, u8, u64)>,
}

fn parse_segment(v: &[u8]) -> Result<IndexSegment> {
    let mut s = IndexSegment::default();
    let mut p = 0;
    let mut raw_entries: Option<&[u8]> = None;
    while p + 4 <= v.len() {
        let tag = u16::from_be_bytes([v[p], v[p + 1]]);
        let n = u16::from_be_bytes([v[p + 2], v[p + 3]]) as usize;
        let d = v.get(p + 4..p + 4 + n).ok_or_else(|| MediaError::invalid("index segment local set overruns"))?;
        let be32 = |b: &[u8]| u32::from_be_bytes(b[..4].try_into().unwrap());
        match tag {
            0x3F0B if n >= 8 => s.edit_rate = (be32(d) as i32, be32(&d[4..]) as i32),
            0x3F0C if n >= 8 => s.start = i64::from_be_bytes(d[..8].try_into().unwrap()),
            0x3F0D if n >= 8 => s.duration = i64::from_be_bytes(d[..8].try_into().unwrap()),
            0x3F05 if n >= 4 => s.edit_unit_bytes = be32(d),
            0x3F06 if n >= 4 => s.index_sid = be32(d),
            0x3F07 if n >= 4 => s.body_sid = be32(d),
            0x3F08 if n >= 1 => s.slice_count = d[0],
            0x3F0E if n >= 1 => s.pos_table_count = d[0],
            0x3F0A => raw_entries = Some(d),
            _ => {}
        }
        p += 4 + n;
    }
    if let Some(d) = raw_entries {
        if d.len() < 8 {
            return Err(MediaError::invalid("short IndexEntryArray"));
        }
        let count = u32::from_be_bytes(d[..4].try_into().unwrap()) as usize;
        let item = u32::from_be_bytes(d[4..8].try_into().unwrap()) as usize;
        let min = 11 + 4 * s.slice_count as usize + 8 * s.pos_table_count as usize;
        if item < 11 || item < min || d.len() < 8 + count * item {
            return Err(MediaError::invalid("bad IndexEntryArray"));
        }
        s.entries = (0..count)
            .map(|i| {
                let e = &d[8 + i * item..];
                (e[0] as i8, e[1] as i8, e[2], u64::from_be_bytes(e[3..11].try_into().unwrap()))
            })
            .collect();
    }
    Ok(s)
}

/// Index table segments in a partition's index region.
fn segments_in(f: &File, p: &Partition, len: u64) -> Result<Vec<IndexSegment>> {
    let mut out = Vec::new();
    if p.index_bytes == 0 {
        return Ok(out);
    }
    // Skip fill, then header metadata, to the index region; walk its KLVs.
    let mut at = p.pack_end;
    let end_limit = len;
    // Header metadata (if any) is header_bytes long, starting after the pack
    // and any fill; walk KLVs and collect index segments until index_bytes
    // are covered past the header metadata.
    let mut header_left = p.header_bytes;
    let mut index_left = p.index_bytes;
    while at < end_limit && (header_left > 0 || index_left > 0) {
        let k = read_klv(f, at, len)?;
        let size = k.end() - at;
        if k.is_essence() || k.is_partition() {
            break;
        }
        if header_left > 0 && !k.is_index_segment() {
            header_left = header_left.saturating_sub(size);
        } else {
            if k.is_index_segment() {
                let mut v = vec![0u8; k.len as usize];
                f.read_exact_at(&mut v, k.value)?;
                out.push(parse_segment(&v)?);
            }
            index_left = index_left.saturating_sub(size);
            header_left = 0;
        }
        at = k.end();
    }
    Ok(out)
}

/// The first essence KLV of a partition (its essence container start), or
/// None when the partition holds no essence.
fn essence_start(f: &File, p: &Partition, next: u64, len: u64) -> Result<Option<u64>> {
    let mut at = p.pack_end;
    while at < next {
        let k = read_klv(f, at, len)?;
        if k.is_partition() {
            return Ok(None);
        }
        if k.is_essence() {
            return Ok(Some(at));
        }
        at = k.end();
    }
    Ok(None)
}

pub fn build(path: &Path, input: &crate::ff::Input, stream: usize) -> Result<FrameIndex> {
    let f = File::open(path)?;
    let len = f.metadata()?.len();
    let parts = partitions(&f)?;
    let mut segments: Vec<IndexSegment> = Vec::new();
    for p in &parts {
        for s in segments_in(&f, p, len)? {
            // Header and footer often repeat a segment; keep the one with entries.
            match segments.iter_mut().find(|e| e.index_sid == s.index_sid && e.start == s.start) {
                Some(e) if e.entries.len() < s.entries.len() => *e = s,
                Some(_) => {}
                None => segments.push(s),
            }
        }
    }
    if segments.is_empty() {
        return Err(MediaError::unsupported("MXF without index table segments"));
    }
    let body_sid = segments[0].body_sid;
    if segments.iter().any(|s| s.body_sid != body_sid) {
        return Err(MediaError::unsupported("MXF with several indexed essence containers"));
    }
    segments.sort_by_key(|s| s.start);
    let edit_rate = segments[0].edit_rate;
    if edit_rate.0 <= 0 || edit_rate.1 <= 0 {
        return Err(MediaError::invalid("MXF index edit rate is not positive"));
    }
    // Essence regions of this body: (body offset, file start, file end).
    let mut regions: Vec<(u64, u64, u64)> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if p.body_sid != body_sid {
            continue;
        }
        let next = parts.get(i + 1).map(|n| n.offset).unwrap_or(len);
        if let Some(start) = essence_start(&f, p, next, len)? {
            regions.push((p.body_offset, start, next));
        }
    }
    if regions.is_empty() {
        return Err(MediaError::invalid("MXF index refers to a body with no essence"));
    }
    let locate = |so: u64| -> Option<(u64, u64)> {
        regions
            .iter()
            .rev()
            .find(|r| r.0 <= so)
            .map(|r| (r.1 + (so - r.0), r.2))
            .filter(|(at, end)| at < end)
    };

    // Edit units in stored order: (stream offset, flags, temporal offset).
    let mut units: Vec<(u64, u8, i64)> = Vec::new();
    for s in &segments {
        if s.edit_unit_bytes != 0 {
            let total: u64 = regions.iter().map(|r| r.2 - r.1).sum();
            let n = if s.duration > 0 { s.duration as u64 } else { total / s.edit_unit_bytes as u64 };
            for k in 0..n {
                units.push(((s.start as u64 + k) * s.edit_unit_bytes as u64, 0x80, 0));
            }
        } else {
            // Avid writes 2 × duration + 1 entries (one per field and a final one).
            let delta = if s.duration > 0 && s.entries.len() as i64 == 2 * s.duration + 1 { 2 } else { 1 };
            let n = if delta == 2 { s.entries.len() - 1 } else { s.entries.len() };
            for j in (0..n).step_by(delta) {
                let (to, _kfo, flags, so) = s.entries[j];
                units.push((so, flags, to as i64 / delta as i64));
            }
        }
    }
    if units.is_empty() {
        return Err(MediaError::invalid("MXF index has no edit units"));
    }
    // ptses[x + temporal_offset[x]] = x: the offset at display position x
    // names the stored position of that picture (as FFmpeg's mxfdec reads it).
    let n = units.len();
    let mut pts_of: Vec<Option<i64>> = vec![None; n];
    for (x, u) in units.iter().enumerate() {
        let k = x as i64 + u.2;
        if k >= 0 && (k as usize) < n {
            pts_of[k as usize] = Some(x as i64);
        }
    }

    // Clip wrapping: one picture KLV holds every edit unit; offsets are
    // relative to its value.
    let first = read_klv(&f, regions[0].1, len)?;
    let clip_wrapped = first.is_picture() && n > 1 && units[1].0 > 0 && first.len > units[1].0 + 16;
    let mut samples = Vec::with_capacity(n);
    let mut reads = 0usize;
    for (k, &(so, flags, _)) in units.iter().enumerate() {
        let (offset, size) = if clip_wrapped {
            let next = units.get(k + 1).map(|u| u.0).unwrap_or(first.len);
            if next > first.len || next < so {
                return Err(MediaError::invalid("clip-wrapped MXF index exceeds its essence"));
            }
            (first.value + so, next - so)
        } else {
            let (at, region_end) = locate(so).ok_or_else(|| MediaError::invalid(format!("stream offset {so} outside the essence")))?;
            let cp_end = units
                .get(k + 1)
                .and_then(|u| locate(u.0))
                .map(|(a, _)| a)
                .filter(|&a| a > at)
                .unwrap_or(region_end);
            let mut p = at;
            let mut found = None;
            // A content package's items are read header by header.
            let mut buf = vec![0u8; (cp_end - at).min(4096) as usize];
            f.read_exact_at(&mut buf, at)?;
            reads += 1;
            while p < cp_end {
                let kv = match parse_klv(&buf, at, p) {
                    Some(kv) => kv,
                    None => {
                        reads += 1;
                        read_klv(&f, p, len)?
                    }
                };
                if kv.key[..4] != UL_PREFIX {
                    return Err(MediaError::invalid(format!("no KLV at content package offset {p}")));
                }
                if kv.is_picture() {
                    found = Some((kv.value, kv.len));
                    break;
                }
                p = kv.end();
            }
            found.ok_or_else(|| MediaError::invalid(format!("edit unit {k} has no picture element")))?
        };
        let size = u32::try_from(size).map_err(|_| MediaError::invalid("MXF picture element over 4 GiB"))?;
        samples.push(Sample {
            dts: Some(k as i64),
            cts: pts_of[k].unwrap_or(k as i64),
            duration: 1,
            key: flags & 0x80 != 0,
            loc: Loc::File { offset, size, additional: None },
            nal_type: None,
        });
    }
    let frames = samples.iter().enumerate().map(|(i, s)| Frame { sample: i, pts: s.cts }).collect();
    let mut idx = FrameIndex {
        source: IndexSource::MxfIndexTables,
        track: input.stream_id(stream),
        timebase: Rational::new(edit_rate.1 as i64, edit_rate.0 as i64)?,
        samples,
        frames,
        config: CodecConfig::default(),
        nal_codec: None,
        stats: IndexStats {
            scan_reads: reads,
            detail: Some(format!(
                "{} index segment(s), {}, {} partition(s) with essence{}",
                segments.len(),
                if segments.iter().all(|s| s.edit_unit_bytes != 0) { "constant edit-unit size" } else { "per-unit entries" },
                regions.len(),
                if clip_wrapped { ", clip-wrapped" } else { ", frame-wrapped" }
            )),
            ..Default::default()
        },
    };
    idx.finish();
    Ok(idx)
}
