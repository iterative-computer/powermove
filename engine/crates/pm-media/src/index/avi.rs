//! AVI frame index from OpenDML `indx`/`ix##` standard indexes or the legacy
//! `idx1`, for codecs without frame reordering (MJPEG, DV, CineForm, ...).
//! AVI stores no composition offsets, so reordering codecs go through the
//! demux scan.

use super::{CodecConfig, Frame, FrameIndex, IndexSource, IndexStats, Loc, Sample};
use crate::error::{MediaError, Result};
use pm_time::Rational;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().unwrap())
}
fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

#[derive(Debug, Default, Clone)]
struct StreamHeader {
    kind: [u8; 4],
    scale: u32,
    rate: u32,
    start: u32,
    super_index: Option<Vec<u8>>,
}

struct Chunk {
    id: [u8; 4],
    data: u64,
    size: u64,
}

fn read_chunk(f: &File, off: u64) -> Result<Chunk> {
    let mut h = [0u8; 8];
    f.read_exact_at(&mut h, off)?;
    Ok(Chunk { id: h[..4].try_into().unwrap(), data: off + 8, size: le32(&h[4..]) as u64 })
}

/// Children of a LIST payload region [start, end).
fn chunks(f: &File, mut at: u64, end: u64) -> Result<Vec<Chunk>> {
    let mut out = Vec::new();
    while at + 8 <= end {
        let c = read_chunk(f, at)?;
        at = c.data + c.size + (c.size & 1);
        out.push(c);
    }
    Ok(out)
}

pub fn build(path: &Path, input: &crate::ff::Input, stream: usize) -> Result<FrameIndex> {
    let f = File::open(path)?;
    let len = f.metadata()?.len();
    let riff = read_chunk(&f, 0)?;
    let mut form = [0u8; 4];
    f.read_exact_at(&mut form, 8)?;
    if &riff.id != b"RIFF" || &form != b"AVI " {
        return Err(MediaError::invalid("not an AVI file"));
    }
    let riff_end = (riff.data + riff.size).min(len);
    let mut streams: Vec<StreamHeader> = Vec::new();
    let mut movi: Option<u64> = None; // position of the 'movi' fourcc
    let mut idx1: Option<Chunk> = None;
    for c in chunks(&f, 12, riff_end)? {
        let mut ty = [0u8; 4];
        if &c.id == b"LIST" {
            f.read_exact_at(&mut ty, c.data)?;
        }
        match (&c.id, &ty) {
            (b"LIST", b"hdrl") => {
                for h in chunks(&f, c.data + 4, c.data + c.size)? {
                    let mut lt = [0u8; 4];
                    if &h.id != b"LIST" {
                        continue;
                    }
                    f.read_exact_at(&mut lt, h.data)?;
                    if &lt != b"strl" {
                        continue;
                    }
                    let mut sh = StreamHeader::default();
                    for s in chunks(&f, h.data + 4, h.data + h.size)? {
                        match &s.id {
                            b"strh" if s.size >= 36 => {
                                let mut b = vec![0u8; 36];
                                f.read_exact_at(&mut b, s.data)?;
                                sh.kind = b[..4].try_into().unwrap();
                                sh.scale = le32(&b[20..]);
                                sh.rate = le32(&b[24..]);
                                sh.start = le32(&b[28..]);
                            }
                            b"indx" => {
                                let mut b = vec![0u8; s.size as usize];
                                f.read_exact_at(&mut b, s.data)?;
                                sh.super_index = Some(b);
                            }
                            _ => {}
                        }
                    }
                    streams.push(sh);
                }
            }
            (b"LIST", b"movi") => movi = Some(c.data),
            (b"idx1", _) => idx1 = Some(c),
            _ => {}
        }
    }
    // The n-th video stream in FFmpeg's order is the n-th 'vids' strl.
    let ordinal = input.video_ordinal(stream).ok_or_else(|| MediaError::invalid("not a video stream"))?;
    let (number, sh) = streams
        .iter()
        .enumerate()
        .filter(|(_, s)| &s.kind == b"vids")
        .nth(ordinal)
        .ok_or_else(|| MediaError::unsupported("AVI video stream without a 'vids' header"))?;
    if sh.scale == 0 || sh.rate == 0 {
        return Err(MediaError::invalid("AVI stream rate is zero"));
    }
    let tag = format!("{number:02}");
    let is_ours = |id: &[u8]| id[..2] == *tag.as_bytes() && matches!(&id[2..4], b"dc" | b"db");

    // (data offset, size, keyframe) per frame in order; size 0 is a dropped frame.
    let mut entries: Vec<(u64, u32, bool)> = Vec::new();
    let mut source = "idx1";
    if let Some(si) = &sh.super_index
        && si.len() >= 24
        && si[3] == 0
    {
        // Super index (type AVI_INDEX_OF_INDEXES): qwOffset, dwSize, dwDuration.
        let used = le32(&si[4..]) as usize;
        for e in 0..used {
            let p = 24 + e * 16;
            let Some(entry) = si.get(p..p + 16) else { break };
            let ix_off = le64(entry);
            let ix = read_chunk(&f, ix_off)?;
            let mut b = vec![0u8; ix.size as usize];
            f.read_exact_at(&mut b, ix.data)?;
            if b.len() < 24 || b[3] != 1 {
                return Err(MediaError::invalid("bad OpenDML standard index"));
            }
            let n = le32(&b[4..]) as usize;
            let base = le64(&b[12..]);
            for i in 0..n {
                let q = 24 + i * 8;
                let Some(e) = b.get(q..q + 8) else { break };
                entries.push((base + le32(e) as u64, le32(&e[4..]) & 0x7FFF_FFFF, le32(&e[4..]) & 0x8000_0000 == 0));
            }
        }
        source = "OpenDML";
    } else if let Some(ix) = idx1 {
        let movi = movi.ok_or_else(|| MediaError::invalid("AVI without a movi list"))?;
        let mut b = vec![0u8; ix.size as usize];
        f.read_exact_at(&mut b, ix.data)?;
        let raw: Vec<(&[u8], u32, u64, u32)> =
            b.chunks_exact(16).map(|e| (&e[..4], le32(&e[4..]), le32(&e[8..]) as u64, le32(&e[12..]))).collect();
        // Offsets are relative to the 'movi' fourcc, or (some writers) absolute.
        let relative = match raw.iter().find(|e| is_ours(e.0)) {
            Some(&(id, _, off, _)) => {
                let mut probe = [0u8; 4];
                f.read_exact_at(&mut probe, movi + off).is_ok() && probe == id[..4]
            }
            None => true,
        };
        for (id, flags, off, size) in raw {
            if is_ours(id) {
                let header = if relative { movi + off } else { off };
                entries.push((header + 8, size, flags & 0x10 != 0)); // AVIIF_KEYFRAME
            }
        }
    } else {
        return Err(MediaError::unsupported("AVI without idx1 or OpenDML index"));
    }
    if entries.is_empty() {
        return Err(MediaError::invalid("AVI index lists no frames for the video stream"));
    }
    let mut samples = Vec::new();
    let intra_only = input.codec_is_intra_only(stream);
    for (k, &(data, size, key)) in entries.iter().enumerate() {
        if size == 0 {
            continue; // a dropped frame: the previous one holds
        }
        if data + size as u64 > len {
            return Err(MediaError::invalid("AVI index entry past the end of the file"));
        }
        let t = sh.start as i64 + k as i64;
        samples.push(Sample {
            dts: Some(t),
            cts: t,
            duration: 1,
            key: key || intra_only,
            loc: Loc::File { offset: data, size, additional: None },
            nal_type: None,
        });
    }
    let frames = samples.iter().enumerate().map(|(i, s)| Frame { sample: i, pts: s.cts }).collect();
    let mut idx = FrameIndex {
        source: IndexSource::AviIndex,
        track: number as u64,
        timebase: Rational::new(sh.scale as i64, sh.rate as i64)?,
        samples,
        frames,
        config: CodecConfig::default(),
        nal_codec: None,
        stats: IndexStats { detail: Some(format!("{source} index, stream {number}")), ..Default::default() },
    };
    idx.finish();
    Ok(idx)
}
