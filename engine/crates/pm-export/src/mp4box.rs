//! A QuickTime / ISO BMFF reader for verification: the facts section 17's probe checks
//! that live in the container (sample entries, `colr`, timescales, sample counts, edit
//! lists, the `tmcd` track, the H.264 SPS). Bounds-checked; malformed input is an
//! error, never a panic. One writer: [`add_channel_layout`].

use std::path::Path;

use crate::Error;

/// A box: type and payload range in the file.
#[derive(Clone, Copy, Debug)]
struct BoxRef {
    kind: [u8; 4],
    start: usize,
    end: usize,
}

fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn be64(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

/// Child boxes of `d[start..end]`.
fn children(d: &[u8], start: usize, end: usize) -> Vec<BoxRef> {
    let mut out = vec![];
    let mut o = start;
    while o + 8 <= end {
        let Some(size32) = be32(d, o) else { break };
        let kind: [u8; 4] = d[o + 4..o + 8].try_into().unwrap();
        let (size, header) = match size32 {
            1 => match be64(d, o + 8) {
                Some(s) => (s as usize, 16),
                None => break,
            },
            0 => (end - o, 8),
            s => (s as usize, 8),
        };
        if size < header || o.checked_add(size).is_none_or(|e| e > end) {
            break;
        }
        out.push(BoxRef {
            kind,
            start: o + header,
            end: o + size,
        });
        o += size;
    }
    out
}

fn find(v: &[BoxRef], kind: &[u8; 4]) -> Option<BoxRef> {
    v.iter().copied().find(|b| &b.kind == kind)
}

/// `colr` contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colr {
    /// `nclc` (QuickTime) or `nclx` (ISO, with a range flag).
    pub kind: [u8; 4],
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: Option<bool>,
}

/// H.264 SPS facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sps {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub chroma_format_idc: u32,
    pub bit_depth_luma: u32,
    pub full_range: Option<bool>,
    pub colour_description: Option<(u8, u8, u8)>,
}

/// HEVC decoder configuration (`hvcC`) facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hevc {
    /// 1 Main, 2 Main10, 4 range extensions.
    pub profile_idc: u8,
    pub level_idc: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
}

/// A video sample entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visual {
    pub width: u16,
    pub height: u16,
    /// QuickTime depth: 24 opaque, 32 with alpha.
    pub depth: u16,
    pub colr: Option<Colr>,
    /// `gama` box value.
    pub gamma: Option<f64>,
    pub sps: Option<Sps>,
    pub hevc: Option<Hevc>,
    /// An `lhvC` box: a layered HEVC configuration, which HEVC with alpha carries
    /// for its alpha layer.
    pub layered_hevc: bool,
    /// The `almo` box's payload (HEVC with alpha: the alpha mode).
    pub alpha_mode: Option<Vec<u8>>,
    /// The sample entry's child boxes, in order (diagnostics).
    pub boxes: Vec<[u8; 4]>,
}

/// An audio sample entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Audio {
    pub channels: u32,
    pub sample_rate: f64,
    pub bits_per_channel: u32,
    /// LPCM samples are floating point (SoundDescriptionV2 format flags).
    pub float: bool,
}

/// A `tmcd` sample entry and its first sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tmcd {
    pub flags: u32,
    pub timescale: u32,
    pub frame_duration: u32,
    pub frames_per_second: u8,
    /// The frame number of the first sample (the start timecode).
    pub start_frame: Option<u32>,
    pub reel: Option<String>,
}

impl Tmcd {
    pub fn drop_frame(&self) -> bool {
        self.flags & 1 != 0
    }
}

/// One edit list entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// In the movie timescale.
    pub segment_duration: u64,
    /// In the media timescale; −1 is an empty edit.
    pub media_time: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: u32,
    pub handler: [u8; 4],
    pub media_timescale: u32,
    pub media_duration: u64,
    pub sample_entry: [u8; 4],
    pub sample_count: u64,
    /// `stts` entries (count, delta).
    pub stts: Vec<(u32, u32)>,
    pub edits: Vec<Edit>,
    /// `tref` references: (type, track ids).
    pub references: Vec<([u8; 4], Vec<u32>)>,
    pub visual: Option<Visual>,
    pub audio: Option<Audio>,
    pub tmcd: Option<Tmcd>,
}

impl Track {
    /// Sum of `stts` deltas: the media duration in the media timescale.
    pub fn stts_duration(&self) -> u64 {
        self.stts.iter().map(|&(n, d)| n as u64 * d as u64).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Movie {
    pub brand: Option<[u8; 4]>,
    pub movie_timescale: u32,
    pub movie_duration: u64,
    pub tracks: Vec<Track>,
}

impl Movie {
    pub fn track(&self, handler: &[u8; 4]) -> Option<&Track> {
        self.tracks.iter().find(|t| &t.handler == handler)
    }

    /// Reads the container facts of a file: only its `ftyp` and `moov` boxes are
    /// loaded (exports run to many gigabytes); the `tmcd` sample is read where its
    /// chunk offset points.
    pub fn read(path: &Path) -> Result<Movie, Error> {
        use std::io::{Read, Seek, SeekFrom};
        let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
        let mut file = std::fs::File::open(path).map_err(io)?;
        let len = file.metadata().map_err(io)?.len();
        let mut buf = vec![];
        let mut at = 0u64;
        while at + 8 <= len {
            file.seek(SeekFrom::Start(at)).map_err(io)?;
            let mut head = [0u8; 16];
            let n = file.read(&mut head).map_err(io)?;
            if n < 8 {
                break;
            }
            let size32 = u32::from_be_bytes(head[0..4].try_into().unwrap()) as u64;
            let size = match size32 {
                1 if n >= 16 => u64::from_be_bytes(head[8..16].try_into().unwrap()),
                1 => break,
                0 => len - at,
                s => s,
            };
            if size < 8 || at + size > len {
                break;
            }
            if &head[4..8] == b"ftyp" || &head[4..8] == b"moov" {
                if size > 1 << 30 {
                    return Err(Error::Verify("container: moov is larger than 1 GiB".into()));
                }
                let mut b = vec![0u8; size as usize];
                file.seek(SeekFrom::Start(at)).map_err(io)?;
                file.read_exact(&mut b).map_err(io)?;
                buf.extend(b);
            }
            at += size;
        }
        let read_at = |offset: u64| -> Option<u32> {
            let mut f = std::fs::File::open(path).ok()?;
            f.seek(SeekFrom::Start(offset)).ok()?;
            let mut b = [0u8; 4];
            f.read_exact(&mut b).ok()?;
            Some(u32::from_be_bytes(b))
        };
        Movie::parse_with(&buf, &read_at)
    }

    /// Parses a whole file held in memory.
    pub fn parse(d: &[u8]) -> Result<Movie, Error> {
        Movie::parse_with(d, &|o| be32(d, o as usize))
    }

    fn parse_with(d: &[u8], read_at: &dyn Fn(u64) -> Option<u32>) -> Result<Movie, Error> {
        let bad = |m: &str| Error::Verify(format!("container: {m}"));
        let top = children(d, 0, d.len());
        let brand = find(&top, b"ftyp")
            .and_then(|b| d.get(b.start..b.start + 4))
            .map(|s| s.try_into().unwrap());
        let moov = find(&top, b"moov").ok_or_else(|| bad("no moov box"))?;
        let mv = children(d, moov.start, moov.end);
        let mvhd = find(&mv, b"mvhd").ok_or_else(|| bad("no mvhd"))?;
        let (movie_timescale, movie_duration) =
            full_header_times(d, mvhd.start).ok_or_else(|| bad("short mvhd"))?;
        let mut tracks = vec![];
        for trak in mv.iter().filter(|b| &b.kind == b"trak") {
            tracks.push(parse_track(d, *trak, read_at).ok_or_else(|| bad("malformed trak"))?);
        }
        Ok(Movie {
            brand,
            movie_timescale,
            movie_duration,
            tracks,
        })
    }
}

/// Gives a QuickTime movie's sound track a channel layout (`chan`, Mono or
/// Stereo by tag) when its sample entry has none: AVAssetWriter writes LPCM
/// without one, and players then read an unknown layout. The movie's `moov`
/// must be its last box, so no chunk offset moves.
pub fn add_channel_layout(path: &Path, channels: u16) -> Result<(), Error> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let bad = |m: &str| Error::Verify(format!("container: {m}"));
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    // kAudioChannelLayoutTag_Mono (100 << 16) | 1, _Stereo (101 << 16) | 2.
    let tag: u32 = match channels {
        1 => (100 << 16) | 1,
        2 => (101 << 16) | 2,
        _ => return Ok(()),
    };
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    let len = file.metadata().map_err(io)?.len();
    let (mut at, mut moov) = (0u64, None);
    while at + 8 <= len {
        let mut head = [0u8; 8];
        file.seek(SeekFrom::Start(at)).map_err(io)?;
        file.read_exact(&mut head).map_err(io)?;
        let size = u32::from_be_bytes(head[0..4].try_into().unwrap()) as u64;
        if size < 8 {
            break;
        }
        if &head[4..8] == b"moov" {
            moov = Some((at, size));
        }
        at += size;
    }
    let (at, size) = moov.ok_or_else(|| bad("no moov box"))?;
    if at + size != len || size > 1 << 30 {
        return Err(bad("moov is not the movie's last box"));
    }
    let mut d = vec![0u8; size as usize];
    file.seek(SeekFrom::Start(at)).map_err(io)?;
    file.read_exact(&mut d).map_err(io)?;
    // The boxes from `moov` down to the sound sample entry, each grown by `chan`.
    let mut chain = vec![BoxRef {
        kind: *b"moov",
        start: 8,
        end: d.len(),
    }];
    let trak = children(&d, 8, d.len())
        .into_iter()
        .filter(|b| &b.kind == b"trak")
        .find(|t| {
            find(&children(&d, t.start, t.end), b"mdia")
                .and_then(|m| find(&children(&d, m.start, m.end), b"hdlr"))
                .and_then(|h| d.get(h.start + 8..h.start + 12))
                == Some(b"soun".as_slice())
        })
        .ok_or_else(|| bad("no sound track"))?;
    chain.push(trak);
    for kind in [b"mdia", b"minf", b"stbl", b"stsd"] {
        let parent = chain[chain.len() - 1];
        chain.push(
            find(&children(&d, parent.start, parent.end), kind)
                .ok_or_else(|| bad("malformed sound track"))?,
        );
    }
    let stsd = chain[chain.len() - 1];
    let entry = *children(&d, stsd.start + 8, stsd.end)
        .first()
        .ok_or_else(|| bad("no sound sample entry"))?;
    if d[entry.start..entry.end].windows(4).any(|w| w == b"chan") {
        return Ok(());
    }
    chain.push(entry);
    let mut chan = Vec::with_capacity(24);
    chan.extend_from_slice(&24u32.to_be_bytes());
    chan.extend_from_slice(b"chan");
    chan.extend_from_slice(&0u32.to_be_bytes()); // version and flags
    chan.extend_from_slice(&tag.to_be_bytes());
    chan.extend_from_slice(&[0u8; 8]); // bitmap, no channel descriptions
    for b in &chain {
        let header = b.start - 8;
        if be32(&d, header) != Some((b.end - header) as u32) {
            return Err(bad("unexpected 64-bit box size in the sound track"));
        }
        let grown = (b.end - header + chan.len()) as u32;
        d[header..header + 4].copy_from_slice(&grown.to_be_bytes());
    }
    d.splice(entry.end..entry.end, chan);
    file.seek(SeekFrom::Start(at)).map_err(io)?;
    file.write_all(&d).map_err(io)?;
    file.sync_all().map_err(io)
}

/// (timescale, duration) of an `mvhd` or `mdhd` payload.
fn full_header_times(d: &[u8], s: usize) -> Option<(u32, u64)> {
    if *d.get(s)? == 1 {
        Some((be32(d, s + 20)?, be64(d, s + 24)?))
    } else {
        Some((be32(d, s + 12)?, be32(d, s + 16)? as u64))
    }
}

fn parse_track(d: &[u8], trak: BoxRef, read_at: &dyn Fn(u64) -> Option<u32>) -> Option<Track> {
    let t = children(d, trak.start, trak.end);
    let tkhd = find(&t, b"tkhd")?;
    let id = if *d.get(tkhd.start)? == 1 {
        be32(d, tkhd.start + 20)?
    } else {
        be32(d, tkhd.start + 12)?
    };
    let mut edits = vec![];
    if let Some(edts) = find(&t, b"edts")
        && let Some(elst) = find(&children(d, edts.start, edts.end), b"elst")
    {
        let version = *d.get(elst.start)?;
        let n = be32(d, elst.start + 4)? as usize;
        let mut o = elst.start + 8;
        for _ in 0..n.min(1024) {
            if version == 1 {
                edits.push(Edit {
                    segment_duration: be64(d, o)?,
                    media_time: be64(d, o + 8)? as i64,
                });
                o += 20;
            } else {
                edits.push(Edit {
                    segment_duration: be32(d, o)? as u64,
                    media_time: be32(d, o + 4)? as i32 as i64,
                });
                o += 12;
            }
        }
    }
    let mut references = vec![];
    if let Some(tref) = find(&t, b"tref") {
        for r in children(d, tref.start, tref.end) {
            let ids = (r.start..r.end)
                .step_by(4)
                .filter_map(|o| be32(d, o))
                .collect();
            references.push((r.kind, ids));
        }
    }
    let mdia = find(&t, b"mdia")?;
    let m = children(d, mdia.start, mdia.end);
    let (media_timescale, media_duration) = full_header_times(d, find(&m, b"mdhd")?.start)?;
    let hdlr = find(&m, b"hdlr")?;
    let handler: [u8; 4] = d.get(hdlr.start + 8..hdlr.start + 12)?.try_into().ok()?;
    let minf = find(&m, b"minf")?;
    let stbl = find(&children(d, minf.start, minf.end), b"stbl")?;
    let st = children(d, stbl.start, stbl.end);
    let stts = match find(&st, b"stts") {
        Some(b) => {
            let n = be32(d, b.start + 4)? as usize;
            (0..n.min(1 << 20))
                .map(|i| {
                    Some((
                        be32(d, b.start + 8 + i * 8)?,
                        be32(d, b.start + 12 + i * 8)?,
                    ))
                })
                .collect::<Option<Vec<_>>>()?
        }
        None => vec![],
    };
    let stsz = find(&st, b"stsz");
    let sample_count = stsz.and_then(|b| be32(d, b.start + 8)).unwrap_or(0) as u64;
    let stsd = find(&st, b"stsd")?;
    let entry = *children(d, stsd.start + 8, stsd.end).first()?;
    let mut track = Track {
        id,
        handler,
        media_timescale,
        media_duration,
        sample_entry: entry.kind,
        sample_count,
        stts,
        edits,
        references,
        visual: None,
        audio: None,
        tmcd: None,
    };
    match &handler {
        b"vide" => track.visual = Some(parse_visual(d, entry)?),
        b"soun" => track.audio = parse_audio(d, entry),
        b"tmcd" => {
            let mut tc = parse_tmcd(d, entry)?;
            // First sample: a big-endian frame number at the first chunk offset.
            let first_chunk = find(&st, b"stco")
                .and_then(|b| be32(d, b.start + 8).map(|v| v as u64))
                .or_else(|| find(&st, b"co64").and_then(|b| be64(d, b.start + 8)));
            tc.start_frame = first_chunk.and_then(read_at);
            track.tmcd = Some(tc);
        }
        _ => {}
    }
    Some(track)
}

fn parse_visual(d: &[u8], e: BoxRef) -> Option<Visual> {
    // VisualSampleEntry: 8 bytes SampleEntry, 16 bytes predefined/reserved, then
    // width, height, resolutions, reserved, frame count, compressor name, depth.
    let s = e.start;
    let mut v = Visual {
        width: be16(d, s + 24)?,
        height: be16(d, s + 26)?,
        depth: be16(d, s + 74)?,
        ..Visual::default()
    };
    for k in children(d, s + 78, e.end) {
        v.boxes.push(k.kind);
        match &k.kind {
            b"colr" => {
                let kind: [u8; 4] = d.get(k.start..k.start + 4)?.try_into().ok()?;
                if &kind == b"nclc" || &kind == b"nclx" {
                    v.colr = Some(Colr {
                        kind,
                        primaries: be16(d, k.start + 4)?,
                        transfer: be16(d, k.start + 6)?,
                        matrix: be16(d, k.start + 8)?,
                        full_range: if &kind == b"nclx" {
                            d.get(k.start + 10).map(|b| b & 0x80 != 0)
                        } else {
                            None
                        },
                    });
                }
            }
            b"gama" => v.gamma = be32(d, k.start).map(|g| g as f64 / 65536.0),
            b"hvcC" => {
                let b = |o: usize| d.get(k.start + o).copied();
                v.hevc = Some(Hevc {
                    profile_idc: b(1)? & 0x1f,
                    level_idc: b(12)?,
                    chroma_format_idc: b(16)? & 0x03,
                    bit_depth_luma: (b(17)? & 0x07) + 8,
                    bit_depth_chroma: (b(18)? & 0x07) + 8,
                });
            }
            b"lhvC" => v.layered_hevc = true,
            b"almo" => v.alpha_mode = d.get(k.start..k.end).map(<[u8]>::to_vec),
            b"avcC" => {
                let nsps = (*d.get(k.start + 5)? & 0x1f) as usize;
                if nsps > 0 {
                    let len = be16(d, k.start + 6)? as usize;
                    v.sps = parse_sps(d.get(k.start + 8..k.start + 8 + len)?);
                }
            }
            _ => {}
        }
    }
    Some(v)
}

fn parse_audio(d: &[u8], e: BoxRef) -> Option<Audio> {
    let s = e.start;
    let version = be16(d, s + 8)?;
    if version == 2 {
        // SoundDescriptionV2: sample rate as f64, channels, bits per channel.
        let rate = f64::from_bits(be64(d, s + 32)?);
        Some(Audio {
            channels: be32(d, s + 40)?,
            sample_rate: rate,
            bits_per_channel: be32(d, s + 48)?,
            // kAudioFormatFlagIsFloat.
            float: be32(d, s + 52)? & 1 != 0,
        })
    } else {
        Some(Audio {
            channels: be16(d, s + 16)? as u32,
            sample_rate: be32(d, s + 24)? as f64 / 65536.0,
            // FFmpeg's QuickTime `in24` entry retains the generic 16-bit
            // SoundDescriptionV0 field; the tag identifies 24-bit samples.
            bits_per_channel: if e.kind == *b"in24" {
                24
            } else {
                be16(d, s + 18)? as u32
            },
            float: false,
        })
    }
}

fn parse_tmcd(d: &[u8], e: BoxRef) -> Option<Tmcd> {
    // 8 bytes SampleEntry, reserved (4), flags (4), timescale (4), frame duration (4),
    // frames per second (1), reserved (1), then an optional `name` box.
    let s = e.start;
    let mut tc = Tmcd {
        flags: be32(d, s + 12)?,
        timescale: be32(d, s + 16)?,
        frame_duration: be32(d, s + 20)?,
        frames_per_second: *d.get(s + 24)?,
        start_frame: None,
        reel: None,
    };
    for k in children(d, s + 26, e.end) {
        if &k.kind == b"name" {
            let len = be16(d, k.start)? as usize;
            tc.reel = d
                .get(k.start + 4..k.start + 4 + len)
                .map(|b| String::from_utf8_lossy(b).into_owned());
        }
    }
    Some(tc)
}

/// RBSP bit reader with emulation-prevention bytes removed.
struct Bits {
    d: Vec<u8>,
    pos: usize,
}

impl Bits {
    fn new(nal: &[u8]) -> Self {
        let mut d = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &b in nal {
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            d.push(b);
        }
        Self { d, pos: 0 }
    }

    fn u(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.d.get(self.pos / 8)?;
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Some(v)
    }

    fn flag(&mut self) -> Option<bool> {
        Some(self.u(1)? == 1)
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.u(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.u(zeros)?)
    }

    fn se(&mut self) -> Option<i32> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 {
            ((k + 1) / 2) as i32
        } else {
            -(k / 2) as i32
        })
    }
}

/// Parses an H.264 SPS (with its NAL header) through the VUI video signal type.
pub fn parse_sps(nal: &[u8]) -> Option<Sps> {
    let mut v = Sps::default();
    let mut b = Bits::new(nal.get(1..)?);
    v.profile_idc = b.u(8)? as u8;
    b.u(8)?;
    v.level_idc = b.u(8)? as u8;
    b.ue()?;
    v.chroma_format_idc = 1;
    v.bit_depth_luma = 8;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&v.profile_idc) {
        v.chroma_format_idc = b.ue()?;
        if v.chroma_format_idc == 3 {
            b.flag()?;
        }
        v.bit_depth_luma = b.ue()? + 8;
        b.ue()?;
        b.flag()?;
        if b.flag()? {
            let lists = if v.chroma_format_idc == 3 { 12 } else { 8 };
            for i in 0..lists {
                if b.flag()? {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i32, 8i32);
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + b.se()? + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    b.ue()?;
    match b.ue()? {
        0 => {
            b.ue()?;
        }
        1 => {
            b.flag()?;
            b.se()?;
            b.se()?;
            for _ in 0..b.ue()? {
                b.se()?;
            }
        }
        _ => {}
    }
    b.ue()?;
    b.flag()?;
    b.ue()?;
    b.ue()?;
    if !b.flag()? {
        b.flag()?;
    }
    b.flag()?;
    if b.flag()? {
        for _ in 0..4 {
            b.ue()?;
        }
    }
    if !b.flag()? {
        return Some(v);
    }
    if b.flag()? && b.u(8)? == 255 {
        b.u(32)?;
    }
    if b.flag()? {
        b.flag()?;
    }
    if b.flag()? {
        b.u(3)?;
        v.full_range = Some(b.flag()?);
        if b.flag()? {
            v.colour_description = Some((b.u(8)? as u8, b.u(8)? as u8, b.u(8)? as u8));
        }
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    fn ue(v: u32) -> String {
        let x = v + 1;
        let n = 32 - x.leading_zeros();
        format!(
            "{}{:0width$b}",
            "0".repeat((n - 1) as usize),
            x,
            width = n as usize
        )
    }

    /// A High-profile SPS with VUI colour description 1-13-1, limited range.
    fn sps() -> Vec<u8> {
        let pieces: Vec<String> = vec![
            format!("{:08b}{:08b}{:08b}", 100, 0, 40),
            ue(0),
            ue(1),
            ue(0),
            ue(0),
            "0".into(),
            "0".into(),
            ue(0),
            ue(0),
            ue(0),
            ue(4),
            "0".into(),
            ue(119),
            ue(67),
            "1".into(),
            "1".into(),
            "1".into(),
            ue(0),
            ue(0),
            ue(0),
            ue(4),
            "1".into(),
            "0".into(),
            "0".into(),
            "1".into(),
            "101".into(),
            "0".into(),
            "1".into(),
            format!("{:08b}{:08b}{:08b}", 1, 13, 1),
            "0".into(),
            "1".into(),
        ];
        let mut bits = pieces.concat();
        while bits.len() % 8 != 0 {
            bits.push('0');
        }
        let mut nal = vec![0x67u8];
        nal.extend(
            bits.as_bytes()
                .chunks(8)
                .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 2).unwrap()),
        );
        nal
    }

    #[test]
    fn parses_sps_vui() {
        let v = parse_sps(&sps()).unwrap();
        assert_eq!(
            (v.profile_idc, v.chroma_format_idc, v.bit_depth_luma),
            (100, 1, 8)
        );
        assert_eq!(v.full_range, Some(false));
        assert_eq!(v.colour_description, Some((1, 13, 1)));
        assert!(parse_sps(&[0x67, 100]).is_none(), "truncated SPS");
    }

    /// A synthetic movie: one video track (avcC, colr nclx, 3 samples at 1001/24000)
    /// and a tmcd track at 29.97 DF starting at frame 107892 with reel "A001".
    fn movie() -> Vec<u8> {
        let mvhd = {
            let mut p = vec![0u8; 100];
            p[12..16].copy_from_slice(&24000u32.to_be_bytes());
            p[16..20].copy_from_slice(&3003u32.to_be_bytes());
            p
        };
        let mdhd = |ts: u32, dur: u32| {
            let mut p = vec![0u8; 24];
            p[12..16].copy_from_slice(&ts.to_be_bytes());
            p[16..20].copy_from_slice(&dur.to_be_bytes());
            p
        };
        let hdlr = |h: &[u8; 4]| {
            let mut p = vec![0u8; 24];
            p[8..12].copy_from_slice(h);
            p
        };
        let tkhd = |id: u32| {
            let mut p = vec![0u8; 84];
            p[12..16].copy_from_slice(&id.to_be_bytes());
            p
        };
        let stsd = |entry: Vec<u8>| {
            let mut p = vec![0, 0, 0, 0, 0, 0, 0, 1];
            p.extend(entry);
            bx(b"stsd", &p)
        };
        let stts = |n: u32, delta: u32| {
            let mut p = vec![0, 0, 0, 0, 0, 0, 0, 1];
            p.extend(n.to_be_bytes());
            p.extend(delta.to_be_bytes());
            bx(b"stts", &p)
        };
        let stsz = |n: u32| {
            let mut p = vec![0u8; 8];
            p.extend(n.to_be_bytes());
            bx(b"stsz", &p)
        };
        // Video sample entry.
        let mut visual = vec![0u8; 78];
        visual[24..26].copy_from_slice(&64u16.to_be_bytes());
        visual[26..28].copy_from_slice(&32u16.to_be_bytes());
        visual[74..76].copy_from_slice(&24u16.to_be_bytes());
        visual.extend(bx(b"colr", &[b'n', b'c', b'l', b'x', 0, 1, 0, 13, 0, 1, 0]));
        let sps = sps();
        let mut avcc = vec![1, 100, 0, 40, 0xff, 0xe1];
        avcc.extend((sps.len() as u16).to_be_bytes());
        avcc.extend(&sps);
        avcc.push(0);
        visual.extend(bx(b"avcC", &avcc));
        let video_stbl = [stsd(bx(b"avc1", &visual)), stts(3, 1001), stsz(3)].concat();
        let video = [
            bx(b"tkhd", &tkhd(1)),
            bx(b"tref", &bx(b"tmcd", &2u32.to_be_bytes())),
            bx(
                b"mdia",
                &[
                    bx(b"mdhd", &mdhd(24000, 3003)),
                    bx(b"hdlr", &hdlr(b"vide")),
                    bx(b"minf", &bx(b"stbl", &video_stbl)),
                ]
                .concat(),
            ),
        ]
        .concat();
        // Timecode sample entry, name box, and a chunk offset to patch.
        let mut tmcd = vec![0u8; 26];
        tmcd[12..16].copy_from_slice(&3u32.to_be_bytes());
        tmcd[16..20].copy_from_slice(&30000u32.to_be_bytes());
        tmcd[20..24].copy_from_slice(&1001u32.to_be_bytes());
        tmcd[24] = 30;
        let mut name = (4u16).to_be_bytes().to_vec();
        name.extend([0, 0]);
        name.extend(b"A001");
        tmcd.extend(bx(b"name", &name));
        let stco = bx(b"stco", &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        let tc_stbl = [stsd(bx(b"tmcd", &tmcd)), stts(1, 3003), stsz(1), stco].concat();
        let timecode = [
            bx(b"tkhd", &tkhd(2)),
            bx(
                b"mdia",
                &[
                    bx(b"mdhd", &mdhd(30000, 3003)),
                    bx(b"hdlr", &hdlr(b"tmcd")),
                    bx(b"minf", &bx(b"stbl", &tc_stbl)),
                ]
                .concat(),
            ),
        ]
        .concat();
        let moov = bx(
            b"moov",
            &[
                bx(b"mvhd", &mvhd),
                bx(b"trak", &video),
                bx(b"trak", &timecode),
            ]
            .concat(),
        );
        let ftyp = bx(b"ftyp", b"qt  \0\0\0\0");
        let mdat_at = ftyp.len() + moov.len() + 8;
        let mut file = [ftyp, moov, bx(b"mdat", &107_892u32.to_be_bytes())].concat();
        // Patch the stco offset (the last 4 bytes of the stco payload).
        let at = file.windows(4).position(|w| w == b"stco").unwrap() + 4 + 8;
        file[at..at + 4].copy_from_slice(&(mdat_at as u32).to_be_bytes());
        file
    }

    #[test]
    fn parses_tracks_tags_timescales_and_timecode() {
        let m = Movie::parse(&movie()).unwrap();
        assert_eq!(&m.brand.unwrap(), b"qt  ");
        assert_eq!((m.movie_timescale, m.movie_duration), (24000, 3003));
        let v = m.track(b"vide").unwrap();
        assert_eq!(
            (v.media_timescale, v.sample_count, v.stts_duration()),
            (24000, 3, 3003)
        );
        assert_eq!(&v.sample_entry, b"avc1");
        assert_eq!(v.references, vec![(*b"tmcd", vec![2])]);
        let vis = v.visual.as_ref().unwrap();
        assert_eq!((vis.width, vis.height, vis.depth), (64, 32, 24));
        assert_eq!(
            vis.colr,
            Some(Colr {
                kind: *b"nclx",
                primaries: 1,
                transfer: 13,
                matrix: 1,
                full_range: Some(false)
            })
        );
        assert_eq!(
            vis.sps.as_ref().unwrap().colour_description,
            Some((1, 13, 1))
        );
        let tc = m.track(b"tmcd").unwrap().tmcd.clone().unwrap();
        assert!(tc.drop_frame());
        assert_eq!(
            (tc.timescale, tc.frame_duration, tc.frames_per_second),
            (30000, 1001, 30)
        );
        assert_eq!(tc.start_frame, Some(107_892));
        assert_eq!(tc.reel.as_deref(), Some("A001"));
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        let full = movie();
        for cut in [0, 7, 8, 20, 64, full.len() / 2, full.len() - 5] {
            let _ = Movie::parse(&full[..cut]);
        }
        let mut broken = full.clone();
        // A box claiming to be larger than the file.
        broken[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Movie::parse(&broken).is_err());
        assert!(Movie::parse(b"").is_err());
    }
}
