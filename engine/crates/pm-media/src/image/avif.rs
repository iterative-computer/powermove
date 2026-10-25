//! AVIF stills (AV1 Image File Format over HEIF, ISO/IEC 23008-12, and
//! MIAF, ISO/IEC 23000-22): the primary item, a single AV1 image or a
//! `grid` of them, decoded by dav1d at its native depth. The samples stay
//! Y′CbCr (with an auxiliary alpha plane); the renderer converts them with
//! the item's color description as it converts video (H.273).
//!
//! Color: a `colr` `nclx` box gives primaries, transfer and matrix, else the
//! AV1 sequence header's color description, else MIAF's default (BT.709
//! primaries, sRGB transfer, BT.601 matrix). The range is the bitstream's.
//! A `colr` ICC profile overrides the primaries and transfer.
//!
//! Geometry is drawn as Chromium shows it (docs/engine/media.md): `irot` and
//! `imir` become the EXIF orientation of Blink's `kAxisAngleToOrientation`,
//! applied by the consumer after conversion; a valid `clap` anchored at
//! (0, 0) crops the planes; any other `clap` shows the whole image; EXIF is
//! ignored. Gain maps are not applied: the primary item is the base image.

use super::{ImageBackend, ImageColor, ImageFormat, StillImage};
use crate::dav1d::{Av1Image, decode_image};
use crate::error::{MediaError, Result};
use crate::frame::{
    DecodedFrame, Layout, PixelFormat, Pixels, PlaneDesc, PlaneFormat, PoolFrame, PoolPlane,
};
use crate::pool::FramePool;
use pm_protocol::color::{AlphaMode, Range, SampleAlignment, Transfer};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn invalid(what: impl Into<String>) -> MediaError {
    MediaError::invalid(format!("AVIF: {}", what.into()))
}

/// A big-endian reader over a box body.
struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Cursor<'a> {
        Cursor { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len());
        let end = end.ok_or_else(|| invalid("truncated box"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn uint(&mut self, bytes: usize) -> Result<u64> {
        Ok(self
            .take(bytes)?
            .iter()
            .fold(0u64, |v, &b| (v << 8) | u64::from(b)))
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(self.uint(2)? as u16)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(self.uint(4)? as u32)
    }
    /// A full box's version and flags.
    fn full(&mut self) -> Result<(u8, u32)> {
        let v = self.u32()?;
        Ok(((v >> 24) as u8, v & 0xFF_FFFF))
    }
    fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.at..];
        self.at = self.b.len();
        s
    }
}

/// The boxes of `b`: (type, body).
fn boxes(b: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut c = Cursor::new(b);
    while c.at + 8 <= b.len() {
        let start = c.at;
        let size = c.u32()? as u64;
        let ty: [u8; 4] = c.take(4)?.try_into().unwrap();
        let size = match size {
            0 => (b.len() - start) as u64,
            1 => c.uint(8)?,
            n => n,
        };
        let header = c.at - start;
        let size = usize::try_from(size).map_err(|_| invalid("box size"))?;
        if size < header || start + size > b.len() {
            return Err(invalid(format!(
                "box {} overruns its container",
                String::from_utf8_lossy(&ty)
            )));
        }
        out.push((ty, &b[c.at..start + size]));
        c.at = start + size;
    }
    Ok(out)
}

fn find<'a>(list: &[([u8; 4], &'a [u8])], ty: &[u8; 4]) -> Option<&'a [u8]> {
    list.iter().find(|(t, _)| t == ty).map(|(_, b)| *b)
}

#[derive(Clone, Debug, Default)]
struct Location {
    method: u8,
    base: u64,
    extents: Vec<(u64, u64)>,
}

#[derive(Clone, Debug)]
enum Property {
    Av1c(Vec<u8>),
    Nclx([u8; 4]),
    Icc(Vec<u8>),
    Irot(u8),
    Imir(u8),
    /// width N/D, height N/D, horizontal offset N/D, vertical offset N/D.
    Clap([i64; 8]),
    AuxAlpha,
    Other,
}

/// The parsed `meta` box.
struct Meta<'a> {
    file: &'a [u8],
    primary: u32,
    types: HashMap<u32, [u8; 4]>,
    locations: HashMap<u32, Location>,
    idat: &'a [u8],
    /// (type, from, to)
    refs: Vec<([u8; 4], u32, Vec<u32>)>,
    properties: Vec<Property>,
    /// Property indices (1-based) of each item.
    associations: HashMap<u32, Vec<usize>>,
}

impl<'a> Meta<'a> {
    fn parse(file: &'a [u8]) -> Result<Meta<'a>> {
        let top = boxes(file)?;
        let meta = find(&top, b"meta").ok_or_else(|| invalid("no meta box"))?;
        let mut c = Cursor::new(meta);
        c.full()?;
        let children = boxes(c.rest())?;
        let mut m = Meta {
            file,
            primary: 0,
            types: HashMap::new(),
            locations: HashMap::new(),
            idat: find(&children, b"idat").unwrap_or(&[]),
            refs: Vec::new(),
            properties: Vec::new(),
            associations: HashMap::new(),
        };
        let pitm = find(&children, b"pitm").ok_or_else(|| invalid("no primary item"))?;
        let mut c = Cursor::new(pitm);
        let (v, _) = c.full()?;
        m.primary = if v == 0 {
            u32::from(c.u16()?)
        } else {
            c.u32()?
        };

        let iinf = find(&children, b"iinf").ok_or_else(|| invalid("no item information"))?;
        let mut c = Cursor::new(iinf);
        let (v, _) = c.full()?;
        let _count = if v == 0 {
            u32::from(c.u16()?)
        } else {
            c.u32()?
        };
        for (ty, body) in boxes(c.rest())? {
            if &ty != b"infe" {
                continue;
            }
            let mut c = Cursor::new(body);
            let (v, _) = c.full()?;
            if v < 2 {
                continue;
            }
            let id = if v == 2 {
                u32::from(c.u16()?)
            } else {
                c.u32()?
            };
            c.u16()?; // protection index
            m.types.insert(id, c.take(4)?.try_into().unwrap());
        }

        let iloc = find(&children, b"iloc").ok_or_else(|| invalid("no item locations"))?;
        let mut c = Cursor::new(iloc);
        let (v, _) = c.full()?;
        let sizes = c.u16()?;
        let (offset_size, length_size, base_size) = (
            usize::from(sizes >> 12),
            usize::from((sizes >> 8) & 15),
            usize::from((sizes >> 4) & 15),
        );
        let index_size = if v >= 1 { usize::from(sizes & 15) } else { 0 };
        let count = if v < 2 { u32::from(c.u16()?) } else { c.u32()? };
        for _ in 0..count {
            let id = if v < 2 { u32::from(c.u16()?) } else { c.u32()? };
            let method = if v >= 1 { (c.u16()? & 15) as u8 } else { 0 };
            c.u16()?; // data reference index
            let base = c.uint(base_size)?;
            let n = c.u16()?;
            let mut extents = Vec::with_capacity(usize::from(n));
            for _ in 0..n {
                c.uint(index_size)?;
                extents.push((c.uint(offset_size)?, c.uint(length_size)?));
            }
            m.locations.insert(
                id,
                Location {
                    method,
                    base,
                    extents,
                },
            );
        }

        if let Some(iref) = find(&children, b"iref") {
            let mut c = Cursor::new(iref);
            let (v, _) = c.full()?;
            for (ty, body) in boxes(c.rest())? {
                let mut c = Cursor::new(body);
                let id = |c: &mut Cursor| -> Result<u32> {
                    Ok(if v == 0 {
                        u32::from(c.u16()?)
                    } else {
                        c.u32()?
                    })
                };
                let from = id(&mut c)?;
                let n = c.u16()?;
                let to = (0..n).map(|_| id(&mut c)).collect::<Result<Vec<_>>>()?;
                m.refs.push((ty, from, to));
            }
        }

        let iprp = find(&children, b"iprp").ok_or_else(|| invalid("no item properties"))?;
        let iprp = boxes(iprp)?;
        let ipco = find(&iprp, b"ipco").ok_or_else(|| invalid("no property container"))?;
        for (ty, body) in boxes(ipco)? {
            m.properties.push(property(ty, body)?);
        }
        for (ty, body) in &iprp {
            if ty != b"ipma" {
                continue;
            }
            let mut c = Cursor::new(body);
            let (v, flags) = c.full()?;
            let n = c.u32()?;
            for _ in 0..n {
                let id = if v < 1 { u32::from(c.u16()?) } else { c.u32()? };
                let k = c.u8()?;
                let list = m.associations.entry(id).or_default();
                for _ in 0..k {
                    let index = if flags & 1 != 0 {
                        usize::from(c.u16()? & 0x7FFF)
                    } else {
                        usize::from(c.u8()? & 0x7F)
                    };
                    if index > 0 {
                        list.push(index);
                    }
                }
            }
        }
        Ok(m)
    }

    fn properties_of(&self, id: u32) -> impl Iterator<Item = &Property> {
        self.associations
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|&i| self.properties.get(i - 1))
    }

    fn data(&self, id: u32) -> Result<Vec<u8>> {
        let loc = self
            .locations
            .get(&id)
            .ok_or_else(|| invalid(format!("item {id} has no location")))?;
        let source = match loc.method {
            0 => self.file,
            1 => self.idat,
            m => return Err(invalid(format!("construction method {m}"))),
        };
        let mut out = Vec::new();
        for &(offset, length) in &loc.extents {
            let start = usize::try_from(loc.base + offset).map_err(|_| invalid("offset"))?;
            let len = if length == 0 && loc.extents.len() == 1 {
                source.len().saturating_sub(start)
            } else {
                usize::try_from(length).map_err(|_| invalid("length"))?
            };
            let end = start
                .checked_add(len)
                .filter(|&e| e <= source.len())
                .ok_or_else(|| invalid(format!("item {id} lies outside the file")))?;
            out.extend_from_slice(&source[start..end]);
        }
        Ok(out)
    }

    /// The items `from` references with `ty`.
    fn referenced(&self, ty: &[u8; 4], from: u32) -> Vec<u32> {
        self.refs
            .iter()
            .filter(|(t, f, _)| t == ty && *f == from)
            .flat_map(|(_, _, to)| to.iter().copied())
            .collect()
    }
}

fn property(ty: [u8; 4], body: &[u8]) -> Result<Property> {
    let mut c = Cursor::new(body);
    Ok(match &ty {
        b"av1C" => Property::Av1c(body.get(4..).unwrap_or(&[]).to_vec()),
        b"colr" => match c.take(4)? {
            b"nclx" => {
                let p = c.u16()?;
                let t = c.u16()?;
                let m = c.u16()?;
                let full = c.u8()? >> 7;
                let clamp = |v: u16| v.min(255) as u8;
                Property::Nclx([clamp(p), clamp(t), clamp(m), full])
            }
            b"rICC" | b"prof" => Property::Icc(c.rest().to_vec()),
            _ => Property::Other,
        },
        b"irot" => Property::Irot(c.u8()? & 3),
        b"imir" => Property::Imir(c.u8()? & 1),
        b"clap" => {
            let mut v = [0i64; 8];
            for (i, slot) in v.iter_mut().enumerate() {
                let raw = c.u32()?;
                // Offsets' numerators are signed; everything else unsigned.
                *slot = if i == 4 || i == 6 {
                    i64::from(raw as i32)
                } else {
                    i64::from(raw)
                };
            }
            Property::Clap(v)
        }
        b"auxC" => {
            c.full()?;
            let urn = c.rest();
            let urn = &urn[..urn.iter().position(|&b| b == 0).unwrap_or(urn.len())];
            if urn == b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha"
                || urn == b"urn:mpeg:hevc:2015:auxid:1"
            {
                Property::AuxAlpha
            } else {
                Property::Other
            }
        }
        _ => Property::Other,
    })
}

/// Blink's orientation for `irot`/`imir` (`kAxisAngleToOrientation`) as an
/// EXIF orientation: rows no mirror, top-bottom mirror (axis 0), left-right
/// mirror (axis 1); columns anticlockwise quarter turns.
fn orientation(angle: Option<u8>, axis: Option<u8>) -> u8 {
    const TABLE: [[u8; 4]; 3] = [[1, 8, 3, 6], [4, 5, 2, 7], [2, 7, 4, 5]];
    let row = axis.map_or(0, |a| 1 + usize::from(a & 1));
    TABLE[row][usize::from(angle.unwrap_or(0) & 3)]
}

/// The crop of a valid `clap` anchored at (0, 0) (Blink's rule), as the
/// clean aperture's size; None shows the whole image.
fn clean_aperture(clap: &[i64; 8], w: usize, h: usize) -> Option<(usize, usize)> {
    let [wn, wd, hn, hd, xn, xd, yn, yd] = *clap;
    if wd <= 0 || hd <= 0 || xd <= 0 || yd <= 0 || wn % wd != 0 || hn % hd != 0 {
        return None;
    }
    let (cw, ch) = (wn / wd, hn / hd);
    if cw <= 0 || ch <= 0 || cw as usize > w || ch as usize > h {
        return None;
    }
    // The left edge, horizOff + (W − cleanW) / 2, must be 0 exactly:
    // 2·xn + (W − cleanW)·xd = 0.
    let left = 2 * xn + (w as i64 - cw) * xd;
    let top = 2 * yn + (h as i64 - ch) * yd;
    (left == 0 && top == 0).then_some((cw as usize, ch as usize))
}

/// The AV1 temporal unit of an `av01` item, with the configuration's
/// sequence header in front when the item data has none.
fn av1_item(meta: &Meta, id: u32) -> Result<Av1Image> {
    if meta.types.get(&id) != Some(b"av01") {
        return Err(invalid(format!("item {id} is not an AV1 image")));
    }
    let mut data = meta.data(id)?;
    let has_sequence_header = {
        let mut i = 0;
        let mut found = false;
        while i < data.len() {
            let header = data[i];
            let kind = (header >> 3) & 15;
            let extension = header & 4 != 0;
            let has_size = header & 2 != 0;
            if kind == 1 {
                found = true;
                break;
            }
            if !has_size {
                break;
            }
            let mut j = i + 1 + usize::from(extension);
            let (mut size, mut shift) = (0usize, 0);
            while j < data.len() {
                let b = data[j];
                size |= usize::from(b & 0x7F) << shift;
                j += 1;
                shift += 7;
                if b & 0x80 == 0 || shift > 56 {
                    break;
                }
            }
            i = j.saturating_add(size);
        }
        found
    };
    if !has_sequence_header
        && let Some(Property::Av1c(config)) = meta
            .properties_of(id)
            .find(|p| matches!(p, Property::Av1c(_)))
    {
        let mut with = config.clone();
        with.extend_from_slice(&data);
        data = with;
    }
    decode_image(&data, 4)
}

/// The image of `id`: an AV1 item, or a `grid` of them.
fn image(meta: &Meta, id: u32) -> Result<Av1Image> {
    match meta.types.get(&id) {
        Some(b"av01") => av1_item(meta, id),
        Some(b"grid") => {
            let desc = meta.data(id)?;
            let mut c = Cursor::new(&desc);
            c.u8()?; // version
            let wide = c.u8()? & 1 != 0;
            let rows = usize::from(c.u8()?) + 1;
            let columns = usize::from(c.u8()?) + 1;
            let (w, h) = if wide {
                (c.u32()? as usize, c.u32()? as usize)
            } else {
                (usize::from(c.u16()?), usize::from(c.u16()?))
            };
            let tiles = meta.referenced(b"dimg", id);
            if tiles.len() != rows * columns {
                return Err(invalid(format!(
                    "grid of {rows}×{columns} with {} tiles",
                    tiles.len()
                )));
            }
            let tiles = tiles
                .iter()
                .map(|&t| av1_item(meta, t))
                .collect::<Result<Vec<_>>>()?;
            let first = &tiles[0];
            if tiles.iter().any(|t| {
                (t.width, t.height, t.bits, t.log2_chroma, t.full_range)
                    != (
                        first.width,
                        first.height,
                        first.bits,
                        first.log2_chroma,
                        first.full_range,
                    )
            }) {
                return Err(invalid("grid tiles differ in size or format"));
            }
            if w == 0 || h == 0 || w > first.width * columns || h > first.height * rows {
                return Err(invalid("grid output size"));
            }
            let mut out = Av1Image {
                width: w,
                height: h,
                planes: Vec::new(),
                ..*first
            };
            let bps = first.bytes_per_sample();
            for i in 0..first.planes.len() {
                let (tw, th) = first.plane_size(i);
                let (pw, ph) = out.plane_size(i);
                let mut plane = vec![0u8; pw * ph * bps];
                for (k, tile) in tiles.iter().enumerate() {
                    let (x0, y0) = ((k % columns) * tw, (k / columns) * th);
                    for y in 0..th.min(ph.saturating_sub(y0)) {
                        let n = tw.min(pw.saturating_sub(x0)) * bps;
                        let src = &tile.planes[i][y * tw * bps..][..n];
                        plane[((y0 + y) * pw + x0) * bps..][..n].copy_from_slice(src);
                    }
                }
                out.planes.push(plane);
            }
            Ok(out)
        }
        Some(t) => Err(invalid(format!(
            "primary item of type {}",
            String::from_utf8_lossy(t)
        ))),
        None => Err(invalid(format!("item {id} is not described"))),
    }
}

/// Crops every plane to the top-left `w` × `h` of the image.
fn crop(image: &mut Av1Image, w: usize, h: usize) {
    let bps = image.bytes_per_sample();
    let old: Vec<(usize, usize)> = (0..image.planes.len())
        .map(|i| image.plane_size(i))
        .collect();
    image.width = w;
    image.height = h;
    for (i, &(ow, _)) in old.iter().enumerate() {
        let (pw, ph) = image.plane_size(i);
        let plane = &image.planes[i];
        image.planes[i] = (0..ph)
            .flat_map(|y| plane[y * ow * bps..][..pw * bps].iter().copied())
            .collect();
    }
}

/// Reads the primary image of an AVIF.
pub fn read(path: &Path, pool: &FramePool) -> Result<StillImage> {
    let file = std::fs::read(path)?;
    let meta = Meta::parse(&file)?;
    let id = meta.primary;
    let mut color = image(&meta, id)?;
    let alpha_id = meta
        .refs
        .iter()
        .filter(|(t, _, to)| t == b"auxl" && to.contains(&id))
        .map(|(_, from, _)| *from)
        .find(|a| {
            meta.properties_of(*a)
                .any(|p| matches!(p, Property::AuxAlpha))
        });
    let mut alpha = alpha_id.map(|a| image(&meta, a)).transpose()?;
    let premultiplied = alpha_id.is_some_and(|a| {
        meta.referenced(b"prem", id).contains(&a) || meta.referenced(b"prem", a).contains(&id)
    });
    let (mut nclx, mut icc, mut angle, mut axis, mut clap) = (None, None, None, None, None);
    for p in meta.properties_of(id) {
        match p {
            Property::Nclx(v) if nclx.is_none() => nclx = Some(*v),
            Property::Icc(v) if icc.is_none() => icc = Some(v.clone()),
            Property::Irot(a) => angle = Some(*a),
            Property::Imir(a) => axis = Some(*a),
            Property::Clap(c) => clap = Some(*c),
            _ => {}
        }
    }
    if let Some(a) = &alpha
        && (a.width, a.height) != (color.width, color.height)
    {
        return Err(invalid("the alpha plane's size differs from the image's"));
    }
    if let Some((w, h)) = clap.and_then(|c| clean_aperture(&c, color.width, color.height)) {
        crop(&mut color, w, h);
        if let Some(a) = alpha.as_mut() {
            crop(a, w, h);
        }
    }
    let orientation = orientation(angle, axis);
    // MIAF's default color information; the bitstream's range.
    let cicp = nclx
        .map(|n| [n[0], n[1], n[2]])
        .or(color.cicp)
        .unwrap_or([1, 13, 6]);
    let image_color = ImageColor {
        cicp: Some([cicp[0], cicp[1], cicp[2], u8::from(color.full_range)]),
        chroma_location: color.chroma_location,
        ..ImageColor::plain(
            Transfer::DisplayE,
            match (&alpha, premultiplied) {
                (None, _) => AlphaMode::Opaque,
                (Some(_), true) => AlphaMode::Premultiplied,
                (Some(_), false) => AlphaMode::Straight,
            },
        )
        .with_icc(icc)
    };
    // GBR samples (matrix 0, 4:4:4) are RGB planes.
    let rgb = cicp[2] == 0 && color.log2_chroma == Some((0, 0));
    let frame = planar(pool, &color, alpha.as_ref(), rgb)?;
    Ok(StillImage {
        frame,
        format: ImageFormat::Avif,
        backend: ImageBackend::Dav1d,
        bit_depth: color.bits,
        float: false,
        color: image_color,
        orientation,
        oriented: orientation == 1,
    })
}

/// The planes in pool memory: Y′, Cb, Cr (neutral for monochrome, at
/// 4:2:0), or G, B, R when `rgb`, and the alpha plane at full range.
fn planar(
    pool: &FramePool,
    color: &Av1Image,
    alpha: Option<&Av1Image>,
    rgb: bool,
) -> Result<DecodedFrame> {
    const ALIGN: usize = 256;
    let bps = color.bytes_per_sample();
    let (w, h) = (color.width, color.height);
    if w == 0 || h == 0 || w.checked_mul(h).is_none_or(|n| n > 1 << 28) {
        return Err(invalid(format!("a {w}×{h} image")));
    }
    let (lw, lh) = color.log2_chroma.unwrap_or((1, 1));
    let (cw, ch) = (w.div_ceil(1 << lw), h.div_ceil(1 << lh));
    let neutral = |n: usize| -> Vec<u8> {
        let v = 1u16 << (color.bits - 1);
        if bps == 1 {
            vec![v as u8; n]
        } else {
            v.to_le_bytes().repeat(n)
        }
    };
    let mut planes: Vec<(Vec<u8>, usize, usize, usize)> =
        vec![(color.planes[0].clone(), w, h, bps)];
    if color.log2_chroma.is_some() {
        planes.push((color.planes[1].clone(), cw, ch, bps));
        planes.push((color.planes[2].clone(), cw, ch, bps));
    } else {
        planes.push((neutral(cw * ch), cw, ch, bps));
        planes.push((neutral(cw * ch), cw, ch, bps));
    }
    if let Some(a) = alpha {
        let abps = a.bytes_per_sample();
        let mut samples = a.planes[0].clone();
        if !a.full_range {
            // Limited-range alpha to full range (H.273 equation 20).
            let max = f64::from((1u32 << a.bits) - 1);
            let scale = f64::from(1u32 << (a.bits - 8));
            let full = |v: f64| ((v / scale - 16.0) / 219.0 * max).round().clamp(0.0, max);
            if abps == 1 {
                for v in &mut samples {
                    *v = full(f64::from(*v)) as u8;
                }
            } else {
                for c in samples.chunks_exact_mut(2) {
                    let v = full(f64::from(u16::from_le_bytes([c[0], c[1]]))) as u16;
                    c.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        planes.push((samples, w, h, abps));
    }
    let strides: Vec<usize> = planes
        .iter()
        .map(|(_, pw, _, b)| (pw * b).div_ceil(ALIGN) * ALIGN)
        .collect();
    let total: usize = planes
        .iter()
        .zip(&strides)
        .map(|((_, _, ph, _), s)| s * ph)
        .sum();
    let slot = pool.acquire(total)?;
    let mut offset = 0;
    let mut pool_planes = Vec::new();
    let mut descs = Vec::new();
    for ((data, pw, ph, b), &stride) in planes.iter().zip(&strides) {
        for y in 0..*ph {
            // SAFETY: the slot holds `total` bytes; each row lies inside its
            // plane's `stride × height` region.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr().add(y * pw * b),
                    slot.ptr().add(offset + y * stride),
                    pw * b,
                )
            };
        }
        pool_planes.push(PoolPlane {
            decoder_ptr: slot.ptr() as usize + offset,
            slot: slot.clone(),
            offset,
            stride,
        });
        descs.push(PlaneDesc {
            format: if *b == 1 {
                PlaneFormat::R8
            } else {
                PlaneFormat::R16
            },
            width: *pw,
            height: *ph,
        });
        offset += stride * ph;
    }
    let sub = match (lw, lh) {
        (1, 1) => "420",
        (1, 0) => "422",
        _ => "444",
    };
    Ok(DecodedFrame {
        pts: 0,
        format: Arc::new(PixelFormat {
            layout: if rgb {
                Layout::PlanarRgb
            } else {
                Layout::Planar
            },
            bits: color.bits,
            alignment: SampleAlignment::Lsb,
            range: if color.full_range {
                Range::Full
            } else {
                Range::Limited
            },
            log2_chroma_w: lw,
            log2_chroma_h: lh,
            alpha_bits: alpha.map_or(0, |a| a.bits),
            planes: descs,
            width: w,
            height: h,
            label: format!(
                "AVIF yuv{}{}p{}",
                if color.log2_chroma.is_some() {
                    sub
                } else {
                    "400"
                },
                if alpha.is_some() { "a" } else { "" },
                color.bits
            ),
            rgba_premultiplied: false,
        }),
        pixels: Pixels::Pool(Arc::new(PoolFrame {
            planes: pool_planes,
            owner: None,
            copied: true,
        })),
        color: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_orientation_table() {
        assert_eq!(orientation(None, None), 1);
        assert_eq!(orientation(Some(1), None), 8);
        assert_eq!(orientation(Some(2), None), 3);
        assert_eq!(orientation(Some(3), None), 6);
        assert_eq!(orientation(None, Some(0)), 4);
        assert_eq!(orientation(None, Some(1)), 2);
        assert_eq!(orientation(Some(1), Some(0)), 5);
        assert_eq!(orientation(Some(3), Some(1)), 5);
        assert_eq!(orientation(Some(1), Some(1)), 7);
    }

    #[test]
    fn only_a_valid_origin_anchored_clean_aperture_crops() {
        // 95×63 of 96×64, centred so that its left/top edge is at 0:
        // horizOff = (95 − 96) / 2 = −1/2.
        assert_eq!(
            clean_aperture(&[95, 1, 63, 1, -1, 2, -1, 2], 96, 64),
            Some((95, 63))
        );
        // Shifted by one pixel: shows the whole image.
        assert_eq!(clean_aperture(&[95, 1, 63, 1, 1, 2, -1, 2], 96, 64), None);
        // A fractional size is invalid.
        assert_eq!(clean_aperture(&[191, 2, 63, 1, -1, 2, -1, 2], 96, 64), None);
        assert_eq!(clean_aperture(&[97, 1, 63, 1, 0, 1, 0, 1], 96, 64), None);
    }
}
