//! MOV/MP4 (and M4V, 3GP, 3G2) frame index from the sample tables, and
//! fragmented MP4 from `moof` headers. Only box headers and table payloads
//! are read; media data is skipped.

use super::{CodecConfig, Cur, Frame, FrameIndex, IndexSource, IndexStats, Loc, Sample};
use crate::error::{MediaError, Result};
use pm_time::Rational;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

/// A top-level box: type, file offset of its header, header length, total size.
#[derive(Clone, Copy, Debug)]
pub struct TopBox {
    pub ty: [u8; 4],
    pub offset: u64,
    pub header: u64,
    pub size: u64,
}

/// Walks the top-level box headers of a file.
pub fn top_level(f: &File) -> Result<Vec<TopBox>> {
    let len = f.metadata()?.len();
    let mut out = Vec::new();
    let mut off = 0u64;
    while off + 8 <= len {
        let mut h = [0u8; 16];
        f.read_exact_at(&mut h[..8], off)?;
        let size32 = u32::from_be_bytes(h[0..4].try_into().unwrap()) as u64;
        let ty: [u8; 4] = h[4..8].try_into().unwrap();
        let (header, size) = match size32 {
            1 => {
                f.read_exact_at(&mut h[8..16], off + 8)?;
                (16, u64::from_be_bytes(h[8..16].try_into().unwrap()))
            }
            0 => (8, len - off),
            s => (8, s),
        };
        if size < header {
            return Err(MediaError::invalid(format!(
                "bad top-level box size at {off}"
            )));
        }
        out.push(TopBox {
            ty,
            offset: off,
            header,
            size: size.min(len - off),
        });
        off += size;
    }
    Ok(out)
}

/// Child boxes of a box payload: (type, payload).
pub fn children(b: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut c = Cur::new(b);
    while c.left() >= 8 {
        let size32 = c.u32()? as u64;
        let ty: [u8; 4] = c.take(4)?.try_into().unwrap();
        let (hdr, size) = match size32 {
            1 => (16, c.u64()?),
            0 => (8, (c.left() + 8) as u64),
            s => (8, s),
        };
        if size < hdr || (size - hdr) as usize > c.left() {
            return Err(MediaError::invalid(format!(
                "bad size for box {}",
                String::from_utf8_lossy(&ty)
            )));
        }
        out.push((ty, c.take((size - hdr) as usize)?));
    }
    Ok(out)
}

pub fn child<'a>(b: &'a [u8], ty: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    Ok(children(b)?.into_iter().find(|c| &c.0 == ty).map(|c| c.1))
}

pub fn need<'a>(b: &'a [u8], ty: &[u8; 4]) -> Result<&'a [u8]> {
    child(b, ty)?
        .ok_or_else(|| MediaError::invalid(format!("missing {} box", String::from_utf8_lossy(ty))))
}

/// The `moov` payload and the file's top-level layout.
pub struct Movie {
    pub moov: Vec<u8>,
    pub top: Vec<TopBox>,
    pub timescale: u32,
}

impl Movie {
    pub fn read(f: &File) -> Result<Movie> {
        let top = top_level(f)?;
        let m = top
            .iter()
            .find(|b| &b.ty == b"moov")
            .ok_or_else(|| MediaError::invalid("no moov box"))?;
        let mut moov = vec![0u8; (m.size - m.header) as usize];
        f.read_exact_at(&mut moov, m.offset + m.header)?;
        let mvhd = need(&moov, b"mvhd")?;
        let mut c = Cur::new(mvhd);
        let v = c.u8()?;
        c.skip(3)?;
        c.skip(if v == 1 { 16 } else { 8 })?;
        let timescale = c.u32()?;
        Ok(Movie {
            moov,
            top,
            timescale,
        })
    }

    /// Every track.
    pub fn tracks(&self) -> Result<Vec<Track<'_>>> {
        children(&self.moov)?
            .into_iter()
            .filter(|c| &c.0 == b"trak")
            .map(|(_, trak)| Track::parse(trak))
            .collect()
    }

    pub fn track(&self, select: TrackSelect) -> Result<Track<'_>> {
        let tracks = self.tracks()?;
        let found = match select {
            TrackSelect::Id(id) => tracks.into_iter().find(|t| t.id == id),
            TrackSelect::FirstVideo => tracks.into_iter().find(|t| &t.handler == b"vide"),
            TrackSelect::FirstAudio => tracks.into_iter().find(|t| &t.handler == b"soun"),
        };
        found.ok_or_else(|| MediaError::invalid(format!("no track {select:?}")))
    }

    /// `mvex/trex` defaults for a track (fragmented files).
    fn trex(&self, track: u32) -> Result<Option<Trex>> {
        let Some(mvex) = child(&self.moov, b"mvex")? else {
            return Ok(None);
        };
        for (ty, b) in children(mvex)? {
            if &ty != b"trex" {
                continue;
            }
            let mut c = Cur::new(b);
            c.skip(4)?;
            if c.u32()? != track {
                continue;
            }
            c.skip(4)?; // default_sample_description_index
            return Ok(Some(Trex {
                duration: c.u32()?,
                size: c.u32()?,
                flags: c.u32()?,
            }));
        }
        Ok(Some(Trex::default()))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TrackSelect {
    Id(u32),
    FirstVideo,
    FirstAudio,
}

/// One `trak`.
pub struct Track<'a> {
    pub id: u32,
    pub handler: [u8; 4],
    pub timescale: u32,
    pub trak: &'a [u8],
    pub stbl: &'a [u8],
    pub edits: Vec<Edit>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edit {
    /// Movie timescale.
    pub segment_duration: u64,
    /// Media timescale; −1 is an empty edit.
    pub media_time: i64,
    pub rate: i32,
    pub rate_fraction: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct Trex {
    duration: u32,
    size: u32,
    flags: u32,
}

impl<'a> Track<'a> {
    fn parse(trak: &'a [u8]) -> Result<Track<'a>> {
        let tkhd = need(trak, b"tkhd")?;
        let mut c = Cur::new(tkhd);
        let v = c.u8()?;
        c.skip(3)?;
        c.skip(if v == 1 { 16 } else { 8 })?;
        let id = c.u32()?;
        let mdia = need(trak, b"mdia")?;
        let hdlr = need(mdia, b"hdlr")?;
        let handler: [u8; 4] = hdlr
            .get(8..12)
            .ok_or_else(|| MediaError::invalid("short hdlr"))?
            .try_into()
            .unwrap();
        let mut c = Cur::new(need(mdia, b"mdhd")?);
        let v = c.u8()?;
        c.skip(3)?;
        c.skip(if v == 1 { 16 } else { 8 })?;
        let timescale = c.u32()?;
        if timescale == 0 {
            return Err(MediaError::invalid("media timescale 0"));
        }
        let stbl = need(need(mdia, b"minf")?, b"stbl")?;
        let mut edits = Vec::new();
        if let Some(edts) = child(trak, b"edts")?
            && let Some(elst) = child(edts, b"elst")?
        {
            let mut c = Cur::new(elst);
            let v = c.u8()?;
            c.skip(3)?;
            for _ in 0..c.u32()? {
                let (segment_duration, media_time) = if v == 1 {
                    (c.u64()?, c.i64()?)
                } else {
                    (c.u32()? as u64, c.i32()? as i64)
                };
                let rate = c.u16()? as i16 as i32;
                let rate_fraction = c.u16()?;
                edits.push(Edit {
                    segment_duration,
                    media_time,
                    rate,
                    rate_fraction,
                });
            }
        }
        Ok(Track {
            id,
            handler,
            timescale,
            trak,
            stbl,
            edits,
        })
    }

    /// The first sample entry: four-character code, coded size (visual
    /// entries), and child atoms.
    pub fn sample_entry(&self) -> Result<CodecConfig> {
        let stsd = need(self.stbl, b"stsd")?;
        if stsd.len() < 8 {
            return Err(MediaError::invalid("short stsd"));
        }
        let entries = children(&stsd[8..])?;
        let (fourcc, body) = entries
            .first()
            .copied()
            .ok_or_else(|| MediaError::invalid("empty stsd"))?;
        let mut cfg = CodecConfig {
            fourcc: Some(fourcc),
            ..Default::default()
        };
        if &self.handler == b"vide" {
            // VisualSampleEntry: 6 reserved + 2 data_reference_index + 70 bytes of fields.
            if body.len() < 78 {
                return Err(MediaError::invalid("short visual sample entry"));
            }
            cfg.width = u16::from_be_bytes([body[24], body[25]]);
            cfg.height = u16::from_be_bytes([body[26], body[27]]);
            cfg.atoms = children(&body[78..])
                .unwrap_or_default()
                .into_iter()
                .map(|(t, p)| (t, p.to_vec()))
                .collect();
        } else if &self.handler == b"soun" && body.len() >= 28 {
            // AudioSampleEntry v0/v1/v2: child atoms after the version-dependent fields.
            let version = u16::from_be_bytes([body[8], body[9]]);
            let start = match version {
                1 => 28 + 16,
                2 => 28 + 36,
                _ => 28,
            };
            if body.len() >= start {
                cfg.atoms = children(&body[start..])
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(t, p)| (t, p.to_vec()))
                    .collect();
            }
        }
        Ok(cfg)
    }
}

/// Builds the frame index of a track.
pub fn build(path: &Path, select: TrackSelect) -> Result<FrameIndex> {
    let f = File::open(path)?;
    let movie = Movie::read(&f)?;
    let track = movie.track(select)?;
    let config = track.sample_entry()?;
    let mut stats = IndexStats::default();
    let mut samples = table_samples(track.stbl, &mut stats)?;
    let fragmented = child(&movie.moov, b"mvex")?.is_some();
    if fragmented {
        let trex = movie.trex(track.id)?.unwrap_or_default();
        let start_dts = samples
            .last()
            .map(|s: &Sample| s.dts.unwrap_or(0) + s.duration)
            .unwrap_or(0);
        fragment_samples(
            &f,
            &movie.top,
            track.id,
            trex,
            start_dts,
            &mut samples,
            &mut stats,
        )?;
    }
    if samples.is_empty() {
        return Err(MediaError::invalid("track has no samples"));
    }
    let (frames, edit_desc) =
        apply_edits(&samples, &track.edits, movie.timescale, track.timescale)?;
    stats.edit_list = edit_desc;
    let mut idx = FrameIndex {
        source: if fragmented {
            IndexSource::Mp4Fragments
        } else {
            IndexSource::Mp4Tables
        },
        track: track.id as u64,
        timebase: Rational::new(1, track.timescale as i64)?,
        samples,
        frames,
        config,
        nal_codec: None,
        stats,
    };
    idx.finish();
    Ok(idx)
}

/// Samples from `stbl` (empty tables in fragmented files yield none).
fn table_samples(stbl: &[u8], stats: &mut IndexStats) -> Result<Vec<Sample>> {
    let mut durations: Vec<u32> = Vec::new();
    {
        let mut c = Cur::new(need(stbl, b"stts")?);
        c.skip(4)?;
        for _ in 0..c.u32()? {
            let (count, delta) = (c.u32()?, c.u32()?);
            if durations.len() + count as usize > 1 << 28 {
                return Err(MediaError::invalid("stts sample count is implausible"));
            }
            durations.extend(std::iter::repeat_n(delta, count as usize));
        }
    }
    let n = durations.len();
    // ctts: offsets are signed in practice for both versions (as FFmpeg reads them).
    let ctts = child(stbl, b"ctts")?;
    stats.has_ctts = ctts.is_some();
    let mut cts_off = vec![0i64; n];
    if let Some(b) = ctts {
        let mut c = Cur::new(b);
        c.skip(4)?;
        let mut i = 0;
        for _ in 0..c.u32()? {
            let (count, off) = (c.u32()? as usize, c.i32()? as i64);
            for _ in 0..count {
                if i >= n {
                    return Err(MediaError::invalid("ctts longer than stts"));
                }
                cts_off[i] = off;
                i += 1;
            }
        }
    }
    let stss = child(stbl, b"stss")?;
    stats.has_stss = stss.is_some();
    let mut key = vec![stss.is_none(); n];
    if let Some(b) = stss {
        let mut c = Cur::new(b);
        c.skip(4)?;
        for _ in 0..c.u32()? {
            let s = c.u32()? as usize;
            if s == 0 || s > n {
                return Err(MediaError::invalid("stss sample out of range"));
            }
            key[s - 1] = true;
        }
    }
    let sizes: Vec<u32> = if let Some(b) = child(stbl, b"stsz")? {
        let mut c = Cur::new(b);
        c.skip(4)?;
        let (fixed, count) = (c.u32()?, c.u32()? as usize);
        if fixed != 0 {
            vec![fixed; count]
        } else {
            (0..count).map(|_| c.u32()).collect::<Result<_>>()?
        }
    } else if let Some(b) = child(stbl, b"stz2")? {
        let mut c = Cur::new(b);
        c.skip(4 + 3)?;
        let field = c.u8()?;
        let count = c.u32()? as usize;
        let mut v = Vec::with_capacity(count);
        match field {
            4 => {
                while v.len() < count {
                    let b = c.u8()?;
                    v.push((b >> 4) as u32);
                    if v.len() < count {
                        v.push((b & 15) as u32);
                    }
                }
            }
            8 => (0..count).try_for_each(|_| -> Result<()> {
                v.push(c.u8()? as u32);
                Ok(())
            })?,
            16 => (0..count).try_for_each(|_| -> Result<()> {
                v.push(c.u16()? as u32);
                Ok(())
            })?,
            _ => return Err(MediaError::invalid("bad stz2 field size")),
        }
        v
    } else {
        return Err(MediaError::invalid("no stsz or stz2"));
    };
    if sizes.len() != n {
        return Err(MediaError::invalid(format!(
            "stsz count {} != stts count {n}",
            sizes.len()
        )));
    }
    let chunk_off: Vec<u64> = if let Some(b) = child(stbl, b"stco")? {
        let mut c = Cur::new(b);
        c.skip(4)?;
        (0..c.u32()?)
            .map(|_| c.u32().map(|v| v as u64))
            .collect::<Result<_>>()?
    } else {
        let mut c = Cur::new(need(stbl, b"co64")?);
        c.skip(4)?;
        (0..c.u32()?).map(|_| c.u64()).collect::<Result<_>>()?
    };
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
    let mut entry = 0usize;
    for (ci, &base) in chunk_off.iter().enumerate() {
        let chunk = ci as u32 + 1;
        while entry + 1 < stsc.len() && stsc[entry + 1].0 <= chunk {
            entry += 1;
        }
        let per = stsc
            .get(entry)
            .filter(|e| e.0 <= chunk)
            .map(|e| e.1)
            .unwrap_or(0);
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
    if offsets.len() != n {
        return Err(MediaError::invalid(format!(
            "stsc/stco cover {} of {n} samples",
            offsets.len()
        )));
    }
    let mut dts = 0i64;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        samples.push(Sample {
            dts: Some(dts),
            cts: dts + cts_off[i],
            duration: durations[i] as i64,
            key: key[i],
            loc: Loc::File {
                offset: offsets[i],
                size: sizes[i],
                additional: None,
            },
            nal_type: None,
        });
        dts += durations[i] as i64;
    }
    Ok(samples)
}

const TFHD_BASE_DATA_OFFSET: u32 = 0x1;
const TFHD_DESCRIPTION: u32 = 0x2;
const TFHD_DURATION: u32 = 0x8;
const TFHD_SIZE: u32 = 0x10;
const TFHD_FLAGS: u32 = 0x20;
const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x20000;
const TRUN_DATA_OFFSET: u32 = 0x1;
const TRUN_FIRST_FLAGS: u32 = 0x4;
const TRUN_DURATION: u32 = 0x100;
const TRUN_SIZE: u32 = 0x200;
const TRUN_FLAGS: u32 = 0x400;
const TRUN_CTO: u32 = 0x800;
/// `sample_is_non_sync_sample` in ISO BMFF sample flags.
const NON_SYNC: u32 = 0x1_0000;

/// Appends the track's samples from every `moof` (headers only).
fn fragment_samples(
    f: &File,
    top: &[TopBox],
    track: u32,
    trex: Trex,
    mut next_dts: i64,
    out: &mut Vec<Sample>,
    stats: &mut IndexStats,
) -> Result<()> {
    for m in top.iter().filter(|b| &b.ty == b"moof") {
        let mut moof = vec![0u8; (m.size - m.header) as usize];
        f.read_exact_at(&mut moof, m.offset + m.header)?;
        stats.scan_reads += 1;
        // End of the previous track fragment's data, for trafs without a base.
        let mut implicit_base = m.offset;
        for (ty, traf) in children(&moof)? {
            if &ty != b"traf" {
                continue;
            }
            let mut c = Cur::new(need(traf, b"tfhd")?);
            let flags = c.u32()? & 0x00FF_FFFF;
            let id = c.u32()?;
            let base_data_offset = if flags & TFHD_BASE_DATA_OFFSET != 0 {
                Some(c.u64()?)
            } else {
                None
            };
            if flags & TFHD_DESCRIPTION != 0 {
                c.skip(4)?;
            }
            let def_duration = if flags & TFHD_DURATION != 0 {
                c.u32()?
            } else {
                trex.duration
            };
            let def_size = if flags & TFHD_SIZE != 0 {
                c.u32()?
            } else {
                trex.size
            };
            let def_flags = if flags & TFHD_FLAGS != 0 {
                c.u32()?
            } else {
                trex.flags
            };
            let base = base_data_offset.unwrap_or(if flags & TFHD_DEFAULT_BASE_IS_MOOF != 0 {
                m.offset
            } else {
                implicit_base
            });
            if let Some(tfdt) = child(traf, b"tfdt")? {
                let mut c = Cur::new(tfdt);
                let v = c.u8()?;
                c.skip(3)?;
                next_dts = if v == 1 {
                    c.u64()? as i64
                } else {
                    c.u32()? as i64
                };
            }
            let mut data = base;
            for (ty, trun) in children(traf)? {
                if &ty != b"trun" {
                    continue;
                }
                let mut c = Cur::new(trun);
                let vf = c.u32()?;
                let (version, tflags) = (vf >> 24, vf & 0x00FF_FFFF);
                let count = c.u32()?;
                if tflags & TRUN_DATA_OFFSET != 0 {
                    data = (base as i64 + c.i32()? as i64) as u64;
                }
                let first_flags = if tflags & TRUN_FIRST_FLAGS != 0 {
                    Some(c.u32()?)
                } else {
                    None
                };
                for i in 0..count {
                    let duration = if tflags & TRUN_DURATION != 0 {
                        c.u32()?
                    } else {
                        def_duration
                    };
                    let size = if tflags & TRUN_SIZE != 0 {
                        c.u32()?
                    } else {
                        def_size
                    };
                    let sflags = if tflags & TRUN_FLAGS != 0 {
                        c.u32()?
                    } else {
                        def_flags
                    };
                    let sflags = if i == 0 {
                        first_flags.unwrap_or(sflags)
                    } else {
                        sflags
                    };
                    let cto = if tflags & TRUN_CTO != 0 {
                        if version == 0 {
                            c.u32()? as i64
                        } else {
                            c.i32()? as i64
                        }
                    } else {
                        0
                    };
                    if id == track {
                        out.push(Sample {
                            dts: Some(next_dts),
                            cts: next_dts + cto,
                            duration: duration as i64,
                            key: sflags & NON_SYNC == 0,
                            loc: Loc::File {
                                offset: data,
                                size,
                                additional: None,
                            },
                            nal_type: None,
                        });
                        next_dts += duration as i64;
                    }
                    data += size as u64;
                }
            }
            implicit_base = data;
        }
    }
    Ok(())
}

fn rescale(v: i64, from: i64, to: i64) -> i64 {
    let n = v as i128 * to as i128;
    let d = from as i128;
    ((2 * n + d).div_euclid(2 * d)) as i64
}

/// Applies an edit list with QuickTime semantics. Empty edits shift the
/// presentation; media edits at rate 1 present every sample overlapping
/// their window (a sample straddling the window start is shown from the edit
/// start); dwell edits (rate 0) hold one sample. Samples outside every edit
/// are decoded but not presented.
pub fn apply_edits(
    samples: &[Sample],
    edits: &[Edit],
    movie_ts: u32,
    media_ts: u32,
) -> Result<(Vec<Frame>, Option<String>)> {
    let all = || {
        samples
            .iter()
            .enumerate()
            .map(|(i, s)| Frame {
                sample: i,
                pts: s.cts,
            })
            .collect()
    };
    if edits.is_empty() {
        return Ok((all(), None));
    }
    if movie_ts == 0 {
        return Err(MediaError::invalid("movie timescale 0 with an edit list"));
    }
    let mut frames = Vec::new();
    let mut pres = 0i64; // presentation position, media units
    let mut desc = Vec::new();
    let mut first_media = true;
    let media_end = samples
        .iter()
        .map(|s| s.cts + s.duration)
        .max()
        .unwrap_or(0);
    for (k, e) in edits.iter().enumerate() {
        let dur = rescale(e.segment_duration as i64, movie_ts as i64, media_ts as i64);
        if e.media_time == -1 {
            pres += dur;
            desc.push(format!("empty {dur}"));
            continue;
        }
        let last = k + 1 == edits.len();
        match (e.rate, e.rate_fraction) {
            (1, 0) => {
                let start = e.media_time;
                // A zero duration on the last edit means "to the end of the media"
                // (fragmented files and some writers).
                let end = if dur == 0 && last {
                    media_end
                } else {
                    start + dur
                };
                for (i, s) in samples.iter().enumerate() {
                    if s.cts + s.duration.max(1) > start && s.cts < end {
                        let mut pts = pres + (s.cts - start);
                        // A frame straddling the edit's start shows from the
                        // edit's start, not during what precedes it (an
                        // earlier edit or an empty edit). Only at the very
                        // start of the presentation does it keep its
                        // negative time (it is the first frame shown).
                        if !first_media || pres > 0 {
                            pts = pts.max(pres);
                        }
                        frames.push(Frame { sample: i, pts });
                    }
                }
                desc.push(format!(
                    "media_time {start} for {}",
                    if dur == 0 && last {
                        "rest".into()
                    } else {
                        dur.to_string()
                    }
                ));
                pres += end - start;
                first_media = false;
            }
            (0, 0) => {
                let at = samples
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.cts <= e.media_time)
                    .max_by_key(|(_, s)| s.cts)
                    .map(|(i, _)| i);
                if let Some(i) = at {
                    frames.push(Frame {
                        sample: i,
                        pts: pres,
                    });
                }
                desc.push(format!("dwell at {} for {dur}", e.media_time));
                pres += dur;
                first_media = false;
            }
            (r, frac) => {
                return Err(MediaError::unsupported(format!(
                    "edit list rate {}: only rate 1 and dwell edits are indexed",
                    r as f64 + frac as f64 / 65536.0
                )));
            }
        }
    }
    if frames.is_empty() {
        return Err(MediaError::invalid("edit list presents no samples"));
    }
    Ok((
        frames,
        Some(format!("{} edit(s): {}", edits.len(), desc.join(", "))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(cts: &[i64]) -> Vec<Sample> {
        cts.iter()
            .map(|&c| Sample {
                dts: None,
                cts: c,
                duration: 10,
                key: false,
                loc: Loc::Demux {
                    pos: 0,
                    nth: 0,
                    size: 1,
                },
                nal_type: None,
            })
            .collect()
    }

    fn edit(dur: u64, media_time: i64, rate: i32) -> Edit {
        Edit {
            segment_duration: dur,
            media_time,
            rate,
            rate_fraction: 0,
        }
    }

    #[test]
    fn single_edit_skips_and_keeps_a_straddling_frame() {
        let s = samples(&[0, 10, 20, 30, 40]);
        let (f, _) = apply_edits(&s, &[edit(25, 15, 1)], 1, 1).unwrap();
        // Window [15, 40): samples at 10 (straddles), 20, 30.
        assert_eq!(
            f.iter().map(|f| (f.sample, f.pts)).collect::<Vec<_>>(),
            vec![(1, -5), (2, 5), (3, 15)]
        );
    }

    #[test]
    fn empty_edit_shifts_and_second_segment_repeats() {
        let s = samples(&[0, 10, 20, 30]);
        // 5 empty, then [0,20), then [0,10) again.
        let (f, d) =
            apply_edits(&s, &[edit(5, -1, 1), edit(20, 0, 1), edit(10, 0, 1)], 1, 1).unwrap();
        assert_eq!(
            f.iter().map(|f| (f.sample, f.pts)).collect::<Vec<_>>(),
            vec![(0, 5), (1, 15), (0, 25)]
        );
        assert!(d.unwrap().contains("3 edit(s)"));
    }

    #[test]
    fn a_frame_straddling_the_first_media_edit_waits_for_an_empty_edit() {
        let s = samples(&[0, 10, 20, 30]);
        // 5 empty, then [15, 35): the sample at 10 straddles 15 and shows
        // from 5 (the end of the empty edit), not during it.
        let (f, _) = apply_edits(&s, &[edit(5, -1, 1), edit(20, 15, 1)], 1, 1).unwrap();
        assert_eq!(
            f.iter().map(|f| (f.sample, f.pts)).collect::<Vec<_>>(),
            vec![(1, 5), (2, 10), (3, 20)]
        );
    }

    #[test]
    fn movie_timescale_is_rescaled_and_dwell_edits_hold() {
        let s = samples(&[0, 10, 20]);
        // Movie timescale 1, media 10: 2 movie units = 20 media units.
        let (f, _) = apply_edits(&s, &[edit(2, 0, 1), edit(1, 15, 0)], 1, 10).unwrap();
        assert_eq!(
            f.iter().map(|f| (f.sample, f.pts)).collect::<Vec<_>>(),
            vec![(0, 0), (1, 10), (1, 20)]
        );
    }

    #[test]
    fn unusual_rates_are_unsupported() {
        let s = samples(&[0, 10]);
        let e = Edit {
            segment_duration: 10,
            media_time: 0,
            rate: 0,
            rate_fraction: 0x8000,
        };
        assert!(apply_edits(&s, &[e], 1, 1).unwrap_err().is_unsupported());
    }
}
