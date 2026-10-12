//! MOV/MP4 sample-table index. Reads only the `moov` box.

use super::{FrameIndex, IndexSource, IndexStats, Loc, Sample, SampleEntry};
use crate::time::Rational;
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::fs::File;
use std::os::unix::fs::FileExt;

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cur { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(self.p + n <= self.b.len(), "truncated box");
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into()?))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into()?))
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    fn left(&self) -> usize {
        self.b.len() - self.p
    }
}

/// Iterates the child boxes of a box payload: (type, payload).
fn children(b: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut c = Cur::new(b);
    while c.left() >= 8 {
        let size32 = c.u32()? as u64;
        let ty: [u8; 4] = c.take(4)?.try_into()?;
        let (hdr, size) = match size32 {
            1 => (16, c.u64()?),
            0 => (8, (c.left() + 8) as u64),
            s => (8, s),
        };
        ensure!(size >= hdr, "bad box size for {}", String::from_utf8_lossy(&ty));
        let body = c.take((size - hdr) as usize)?;
        out.push((ty, body));
    }
    Ok(out)
}

fn child<'a>(b: &'a [u8], ty: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    Ok(children(b)?.into_iter().find(|c| &c.0 == ty).map(|c| c.1))
}

fn need<'a>(b: &'a [u8], ty: &[u8; 4]) -> Result<&'a [u8]> {
    child(b, ty)?.ok_or_else(|| anyhow!("missing {}", String::from_utf8_lossy(ty)))
}

/// Reads the top-level `moov` payload.
fn read_moov(f: &File) -> Result<Vec<u8>> {
    let len = f.metadata()?.len();
    let mut off = 0u64;
    while off + 8 <= len {
        let mut h = [0u8; 16];
        f.read_exact_at(&mut h[..8], off)?;
        let size32 = u32::from_be_bytes(h[0..4].try_into()?) as u64;
        let ty: [u8; 4] = h[4..8].try_into()?;
        let (hdr, size) = match size32 {
            1 => {
                f.read_exact_at(&mut h[8..16], off + 8)?;
                (16, u64::from_be_bytes(h[8..16].try_into()?))
            }
            0 => (8, len - off),
            s => (8, s),
        };
        ensure!(size >= hdr, "bad top-level box size");
        if &ty == b"moov" {
            let mut body = vec![0u8; (size - hdr) as usize];
            f.read_exact_at(&mut body, off + hdr)?;
            return Ok(body);
        }
        off += size;
    }
    bail!("no moov box (fragmented MP4 is not handled by the spike)")
}

struct Edit {
    segment_duration: u64, // movie timescale
    media_time: i64,       // media timescale, -1 = empty edit
    rate_int: i16,
}

pub fn build(path: &std::path::Path) -> Result<FrameIndex> {
    let t0 = std::time::Instant::now();
    let f = File::open(path)?;
    let moov = read_moov(&f)?;
    let mvhd = need(&moov, b"mvhd")?;
    let movie_ts = {
        let mut c = Cur::new(mvhd);
        let v = c.u8()?;
        c.skip(3)?;
        c.skip(if v == 1 { 16 } else { 8 })?;
        c.u32()?
    };
    // First video track.
    let trak = children(&moov)?
        .into_iter()
        .filter(|c| &c.0 == b"trak")
        .find(|t| {
            (|| -> Result<bool> {
                let hdlr = need(need(t.1, b"mdia")?, b"hdlr")?;
                Ok(&hdlr[8..12] == b"vide")
            })()
            .unwrap_or(false)
        })
        .map(|t| t.1)
        .context("no video track")?;
    let mdia = need(trak, b"mdia")?;
    let media_ts = {
        let mut c = Cur::new(need(mdia, b"mdhd")?);
        let v = c.u8()?;
        c.skip(3)?;
        c.skip(if v == 1 { 16 } else { 8 })?;
        c.u32()?
    };
    let stbl = need(need(mdia, b"minf")?, b"stbl")?;

    let sample_entry = parse_stsd(need(stbl, b"stsd")?)?;

    // stts
    let mut durations: Vec<u32> = Vec::new();
    {
        let mut c = Cur::new(need(stbl, b"stts")?);
        c.skip(4)?;
        for _ in 0..c.u32()? {
            let (count, delta) = (c.u32()?, c.u32()?);
            durations.extend(std::iter::repeat_n(delta, count as usize));
        }
    }
    let n = durations.len();
    // ctts: offsets are signed in practice for both versions (as FFmpeg reads them).
    let ctts = child(stbl, b"ctts")?;
    let mut cts_off = vec![0i64; n];
    if let Some(b) = ctts {
        let mut c = Cur::new(b);
        c.skip(4)?;
        let mut i = 0;
        for _ in 0..c.u32()? {
            let (count, off) = (c.u32()? as usize, c.i32()? as i64);
            for _ in 0..count {
                ensure!(i < n, "ctts longer than stts");
                cts_off[i] = off;
                i += 1;
            }
        }
    }
    // stss
    let stss = child(stbl, b"stss")?;
    let mut key = vec![stss.is_none(); n];
    if let Some(b) = stss {
        let mut c = Cur::new(b);
        c.skip(4)?;
        for _ in 0..c.u32()? {
            let s = c.u32()? as usize;
            ensure!(s >= 1 && s <= n, "stss sample out of range");
            key[s - 1] = true;
        }
    }
    // stsz / stz2
    let sizes: Vec<u32> = if let Some(b) = child(stbl, b"stsz")? {
        let mut c = Cur::new(b);
        c.skip(4)?;
        let (fixed, count) = (c.u32()?, c.u32()? as usize);
        if fixed != 0 { vec![fixed; count] } else { (0..count).map(|_| c.u32()).collect::<Result<_>>()? }
    } else {
        let mut c = Cur::new(need(stbl, b"stz2")?);
        c.skip(4 + 3)?;
        let field = c.u8()?;
        let count = c.u32()? as usize;
        let mut v = Vec::with_capacity(count);
        match field {
            4 => {
                for i in 0..count {
                    if i % 2 == 0 {
                        let b = c.u8()?;
                        v.push((b >> 4) as u32);
                        if i + 1 < count {
                            v.push((b & 15) as u32);
                        }
                    }
                }
            }
            8 => (0..count).try_for_each(|_| -> Result<()> { v.push(c.u8()? as u32); Ok(()) })?,
            16 => (0..count).try_for_each(|_| -> Result<()> { v.push(c.u16()? as u32); Ok(()) })?,
            _ => bail!("bad stz2 field size"),
        }
        v
    };
    ensure!(sizes.len() == n, "stsz count {} != stts count {}", sizes.len(), n);
    // chunk offsets
    let chunk_off: Vec<u64> = if let Some(b) = child(stbl, b"stco")? {
        let mut c = Cur::new(b);
        c.skip(4)?;
        (0..c.u32()?).map(|_| c.u32().map(|v| v as u64)).collect::<Result<_>>()?
    } else {
        let mut c = Cur::new(need(stbl, b"co64")?);
        c.skip(4)?;
        (0..c.u32()?).map(|_| c.u64()).collect::<Result<_>>()?
    };
    // stsc → per-sample offsets
    let mut stsc: Vec<(u32, u32)> = Vec::new();
    {
        let mut c = Cur::new(need(stbl, b"stsc")?);
        c.skip(4)?;
        for _ in 0..c.u32()? {
            let (first, per) = (c.u32()?, c.u32()?);
            c.skip(4)?;
            stsc.push((first, per));
        }
    }
    let mut offsets = Vec::with_capacity(n);
    let mut s = 0usize;
    for (ci, &base) in chunk_off.iter().enumerate() {
        let chunk = ci as u32 + 1;
        let per = stsc.iter().rev().find(|e| e.0 <= chunk).map(|e| e.1).context("stsc")?;
        let mut o = base;
        for _ in 0..per {
            if s >= n {
                break;
            }
            offsets.push(o);
            o += sizes[s] as u64;
            s += 1;
        }
    }
    ensure!(offsets.len() == n, "stsc/stco cover {} of {} samples", offsets.len(), n);

    // Edit list.
    let mut edits = Vec::new();
    if let Some(edts) = child(trak, b"edts")?
        && let Some(elst) = child(edts, b"elst")?
    {
        let mut c = Cur::new(elst);
        let v = c.u8()?;
        c.skip(3)?;
        for _ in 0..c.u32()? {
            let (segment_duration, media_time) =
                if v == 1 { (c.u64()?, c.i64()?) } else { (c.u32()? as u64, c.i32()? as i64) };
            let rate_int = c.u16()? as i16;
            c.skip(2)?;
            edits.push(Edit { segment_duration, media_time, rate_int });
        }
    }

    // DTS, CTS.
    let mut dts = 0i64;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let cts = dts + cts_off[i];
        samples.push(Sample {
            dts: Some(dts),
            pts: cts,
            duration: durations[i] as i64,
            key: key[i],
            loc: Loc::File { offset: offsets[i], size: sizes[i], additional: None },
            presented: true,
            nal_type: None,
        });
        dts += durations[i] as i64;
    }

    // Apply the edit list: leading empty edits shift presentation; one media edit
    // at rate 1 selects [media_time, media_time + duration). Anything else is
    // reported as unsupported rather than guessed.
    let mut edit_desc = None;
    if !edits.is_empty() {
        let mut empty_media = 0i64;
        let mut media: Vec<&Edit> = Vec::new();
        for e in &edits {
            if e.media_time == -1 {
                ensure!(media.is_empty(), "empty edit after a media edit is not handled by the spike");
                empty_media += rescale(e.segment_duration as i64, movie_ts as i64, media_ts as i64);
            } else {
                media.push(e);
            }
        }
        ensure!(media.len() <= 1, "{} media edits: multi-segment edit lists are not handled by the spike", media.len());
        if let Some(e) = media.first() {
            ensure!(e.rate_int == 1, "edit rate {} not handled", e.rate_int);
            let start = e.media_time;
            let dur = rescale(e.segment_duration as i64, movie_ts as i64, media_ts as i64);
            // segment_duration 0 means "to the end of the media" in some writers.
            let end = if e.segment_duration == 0 { i64::MAX } else { start + dur };
            for s in samples.iter_mut() {
                // A frame is shown if any part of it falls inside the edit window.
                let shown = s.pts + s.duration > start && s.pts < end;
                s.presented = shown;
                s.pts = s.pts - start + empty_media;
            }
            // A frame straddling the start is shown from the edit start, as
            // FFmpeg and AVFoundation do; its PTS stays its own (negative).
            edit_desc = Some(format!(
                "{} edit(s): empty {} + media_time {} for {} (media units)",
                edits.len(),
                empty_media,
                start,
                if end == i64::MAX { "rest".into() } else { dur.to_string() }
            ));
        }
    }

    let stats = IndexStats {
        has_stss: stss.is_some(),
        has_ctts: ctts.is_some(),
        edit_list: edit_desc,
        ..Default::default()
    };
    let mut idx = FrameIndex {
        source: IndexSource::Mp4Tables,
        timebase: Rational::new(1, media_ts as i64),
        samples,
        order: vec![],
        sample_entry: Some(sample_entry),
        codec_private: None,
        codec_id: None,
        nal_codec: None,
        stats,
    };
    idx.finish();
    idx.stats.build_ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok(idx)
}

fn rescale(v: i64, from: i64, to: i64) -> i64 {
    ((v as i128 * to as i128 + from as i128 / 2) / from as i128) as i64
}

fn parse_stsd(b: &[u8]) -> Result<SampleEntry> {
    let mut c = Cur::new(b);
    c.skip(4)?;
    let count = c.u32()?;
    ensure!(count >= 1, "empty stsd");
    let entries = children(&b[8..])?;
    let (fourcc, body) = entries.first().copied().context("stsd entry")?;
    // VisualSampleEntry: 6 reserved + 2 data_reference_index + 70 bytes of fields.
    ensure!(body.len() >= 78, "short visual sample entry");
    let width = u16::from_be_bytes([body[24], body[25]]);
    let height = u16::from_be_bytes([body[26], body[27]]);
    let atoms = children(&body[78..])?.into_iter().map(|(t, p)| (t, p.to_vec())).collect();
    Ok(SampleEntry { fourcc, width, height, atoms })
}
