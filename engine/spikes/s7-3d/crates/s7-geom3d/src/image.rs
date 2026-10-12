//! Image decode for glTF textures: PNG and JPEG (pure Rust), WebP (`image-webp`, pure
//! Rust, also in wasm), AVIF (`avif-parse` + `rav1d`, native only: rav1d does not compile
//! for wasm32-unknown-unknown, so the editor's wasm build leaves AVIF to the browser).
//!
//! Output is straight-alpha RGBA8 in the file's encoding, as `createImageBitmap(blob,
//! { colorSpaceConversion: 'none' })` hands it to WebGL; color management is the
//! renderer's job (section 9: color maps are sRGB, data maps linear).

use crate::model::{Error, Image, Result, err};

pub fn decode(bytes: &[u8], mime: &str) -> Result<Option<Image>> {
    let kind = sniff(bytes).unwrap_or(mime);
    let img = match kind {
        "image/png" => Some(png(bytes)?),
        "image/jpeg" => Some(jpeg(bytes)?),
        "image/webp" => Some(webp(bytes)?),
        "image/avif" => avif(bytes)?,
        other => return err(format!("unsupported image type {other}")),
    };
    Ok(img)
}

fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis") {
        Some("image/avif")
    } else {
        None
    }
}

fn png(bytes: &[u8]) -> Result<Image> {
    let mut decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::EXPAND | ::png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| Error(format!("PNG: {e}")))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| Error("PNG size".into()))?];
    let info = reader.next_frame(&mut buf).map_err(|e| Error(format!("PNG: {e}")))?;
    let (w, h) = (info.width, info.height);
    let n = (w * h) as usize;
    let rgba = match info.color_type {
        ::png::ColorType::Rgba => buf[..n * 4].to_vec(),
        ::png::ColorType::Rgb => buf[..n * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        ::png::ColorType::GrayscaleAlpha => buf[..n * 2].chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        ::png::ColorType::Grayscale => buf[..n].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ::png::ColorType::Indexed => return err("PNG palette was not expanded"),
    };
    Ok(Image { width: w, height: h, rgba, mime: "image/png".into() })
}

fn jpeg(bytes: &[u8]) -> Result<Image> {
    use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(bytes, options);
    let rgba = decoder.decode().map_err(|e| Error(format!("JPEG: {e:?}")))?;
    let (w, h) = decoder.dimensions().ok_or_else(|| Error("JPEG size".into()))?;
    Ok(Image { width: w as u32, height: h as u32, rgba, mime: "image/jpeg".into() })
}

fn webp(bytes: &[u8]) -> Result<Image> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| Error(format!("WebP: {e}")))?;
    let (w, h) = decoder.dimensions();
    let has_alpha = decoder.has_alpha();
    let mut buf = vec![0; decoder.output_buffer_size().ok_or_else(|| Error("WebP size".into()))?];
    decoder.read_image(&mut buf).map_err(|e| Error(format!("WebP: {e}")))?;
    let rgba = if has_alpha { buf } else { buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
    Ok(Image { width: w, height: h, rgba, mime: "image/webp".into() })
}

#[cfg(target_arch = "wasm32")]
fn avif(_bytes: &[u8]) -> Result<Option<Image>> {
    // The host (Chromium's AVIF decoder) supplies texels in the editor; geometry
    // queries never need them.
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
fn avif(bytes: &[u8]) -> Result<Option<Image>> {
    let data = avif_parse::read_avif(&mut std::io::Cursor::new(bytes)).map_err(|e| Error(format!("AVIF: {e:?}")))?;
    let color = av1::decode(&data.primary_item)?;
    let (w, h) = (color.width, color.height);
    let mut rgba = color.to_rgba8()?;
    if let Some(alpha) = data.alpha_item.as_deref() {
        let a = av1::decode(alpha)?;
        if (a.width, a.height) != (w, h) {
            return err("AVIF alpha size differs from color");
        }
        let plane = a.luma_8bit();
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            px[3] = plane[i];
            if data.premultiplied_alpha && px[3] > 0 {
                for c in 0..3 {
                    px[c] = ((px[c] as u32 * 255 + px[3] as u32 / 2) / px[3] as u32).min(255) as u8;
                }
            }
        }
    }
    Ok(Some(Image { width: w, height: h, rgba, mime: "image/avif".into() }))
}

#[cfg(not(target_arch = "wasm32"))]
mod av1 {
    //! AV1 still-picture decode through rav1d's dav1d-compatible API, then Y′CbCr to
    //! R′G′B′ with the sequence header's matrix and range (AVIF: an unspecified matrix
    //! means BT.601, as libavif treats it).
    use super::{Error, Result, err};
    use rav1d::include::dav1d::data::Dav1dData;
    use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
    use rav1d::include::dav1d::headers::*;
    use rav1d::include::dav1d::picture::Dav1dPicture;
    use rav1d::src::lib::*;
    use std::ptr::NonNull;

    pub struct Planes {
        pub width: u32,
        pub height: u32,
        pub bpc: u32,
        pub layout: u32,
        pub matrix: u32,
        pub full_range: bool,
        /// Planes widened to u16 (8-bit content keeps 0..255).
        pub y: Vec<u16>,
        pub u: Vec<u16>,
        pub v: Vec<u16>,
        pub chroma_w: usize,
    }

    pub fn decode(obus: &[u8]) -> Result<Planes> {
        unsafe {
            let mut settings: Dav1dSettings = std::mem::zeroed();
            dav1d_default_settings(NonNull::from(&mut settings));
            settings.n_threads = 1;
            settings.max_frame_delay = 1;
            let mut ctx: Option<Dav1dContext> = None;
            if dav1d_open(Some(NonNull::from(&mut ctx)), Some(NonNull::from(&mut settings))).0 != 0 {
                return err("rav1d: open failed");
            }
            let mut data: Dav1dData = std::mem::zeroed();
            let buf = dav1d_data_create(Some(NonNull::from(&mut data)), obus.len());
            if buf.is_null() {
                dav1d_close(Some(NonNull::from(&mut ctx)));
                return err("rav1d: data_create failed");
            }
            std::ptr::copy_nonoverlapping(obus.as_ptr(), buf, obus.len());
            let mut picture: Dav1dPicture = std::mem::zeroed();
            let mut got = false;
            for _ in 0..64 {
                if data.sz > 0 {
                    let r = dav1d_send_data(ctx.clone(), Some(NonNull::from(&mut data))).0;
                    if r != 0 && r != -11 {
                        dav1d_data_unref(Some(NonNull::from(&mut data)));
                        dav1d_close(Some(NonNull::from(&mut ctx)));
                        return err(format!("rav1d: send_data {r}"));
                    }
                }
                let r = dav1d_get_picture(ctx.clone(), Some(NonNull::from(&mut picture))).0;
                if r == 0 {
                    got = true;
                    break;
                }
                if r != -11 {
                    break;
                }
                if data.sz == 0 {
                    // Drain.
                    let r = dav1d_get_picture(ctx.clone(), Some(NonNull::from(&mut picture))).0;
                    got = r == 0;
                    break;
                }
            }
            dav1d_data_unref(Some(NonNull::from(&mut data)));
            if !got {
                dav1d_close(Some(NonNull::from(&mut ctx)));
                return err("rav1d: no picture");
            }
            let w = picture.p.w as usize;
            let h = picture.p.h as usize;
            let bpc = picture.p.bpc as u32;
            let layout = picture.p.layout;
            let seq = picture.seq_hdr.ok_or_else(|| Error("rav1d: no sequence header".into()))?.as_ref();
            let (matrix, full_range) = (seq.mtrx, seq.color_range != 0);
            let (cw, ch) = match layout {
                DAV1D_PIXEL_LAYOUT_I420 => (w.div_ceil(2), h.div_ceil(2)),
                DAV1D_PIXEL_LAYOUT_I422 => (w.div_ceil(2), h),
                DAV1D_PIXEL_LAYOUT_I444 => (w, h),
                _ => (0, 0),
            };
            let read = |plane: usize, pw: usize, ph: usize, stride: isize| -> Vec<u16> {
                let base = picture.data[plane].map(|p| p.as_ptr() as *const u8).unwrap_or(std::ptr::null());
                let mut out = Vec::with_capacity(pw * ph);
                for y in 0..ph {
                    let row = base.offset(stride * y as isize);
                    for x in 0..pw {
                        out.push(if bpc > 8 { *(row as *const u16).add(x) } else { *row.add(x) as u16 });
                    }
                }
                out
            };
            let y = read(0, w, h, picture.stride[0]);
            let (u, v) = if cw > 0 { (read(1, cw, ch, picture.stride[1]), read(2, cw, ch, picture.stride[1])) } else { (Vec::new(), Vec::new()) };
            dav1d_picture_unref(Some(NonNull::from(&mut picture)));
            dav1d_close(Some(NonNull::from(&mut ctx)));
            Ok(Planes { width: w as u32, height: h as u32, bpc, layout, matrix, full_range, y, u, v, chroma_w: cw })
        }
    }

    impl Planes {
        pub fn luma_8bit(&self) -> Vec<u8> {
            let max = ((1u32 << self.bpc) - 1) as f64;
            self.y.iter().map(|&v| ((v as f64) * 255.0 / max).round() as u8).collect()
        }

        pub fn to_rgba8(&self) -> Result<Vec<u8>> {
            let (w, h) = (self.width as usize, self.height as usize);
            let max = ((1u32 << self.bpc) - 1) as f64;
            let scale = (1u32 << (self.bpc - 8)) as f64;
            // Kr, Kb per matrix; unspecified (2) is BT.601 for AVIF.
            let (kr, kb) = match self.matrix {
                DAV1D_MC_BT709 => (0.2126, 0.0722),
                DAV1D_MC_BT470BG | DAV1D_MC_BT601 | DAV1D_MC_UNKNOWN => (0.299, 0.114),
                DAV1D_MC_SMPTE240 => (0.212, 0.087),
                DAV1D_MC_BT2020_NCL => (0.2627, 0.0593),
                DAV1D_MC_IDENTITY => (0.0, 0.0),
                m => return err(format!("AVIF matrix coefficients {m} not supported")),
            };
            let mut out = vec![0u8; w * h * 4];
            let (sx, sy) = match self.layout {
                DAV1D_PIXEL_LAYOUT_I420 => (2, 2),
                DAV1D_PIXEL_LAYOUT_I422 => (2, 1),
                _ => (1, 1),
            };
            let monochrome = self.layout == DAV1D_PIXEL_LAYOUT_I400;
            for y in 0..h {
                for x in 0..w {
                    let yy = self.y[y * w + x] as f64;
                    let (cb, cr) = if monochrome {
                        (128.0 * scale, 128.0 * scale)
                    } else {
                        let ci = (y / sy) * self.chroma_w + x / sx;
                        (self.u[ci] as f64, self.v[ci] as f64)
                    };
                    let (e_y, e_cb, e_cr) = if self.full_range {
                        (yy / max, (cb - 128.0 * scale) / max, (cr - 128.0 * scale) / max)
                    } else {
                        ((yy - 16.0 * scale) / (219.0 * scale), (cb - 128.0 * scale) / (224.0 * scale), (cr - 128.0 * scale) / (224.0 * scale))
                    };
                    let (r, g, b) = if self.matrix == DAV1D_MC_IDENTITY {
                        // GBR: Y = G, Cb = B, Cr = R.
                        (e_cr + if self.full_range { 128.0 * scale / max } else { 0.5 }, e_y, e_cb + if self.full_range { 128.0 * scale / max } else { 0.5 })
                    } else {
                        let kg = 1.0 - kr - kb;
                        let r = e_y + 2.0 * (1.0 - kr) * e_cr;
                        let b = e_y + 2.0 * (1.0 - kb) * e_cb;
                        let g = (e_y - kr * r - kb * b) / kg;
                        (r, g, b)
                    };
                    let o = (y * w + x) * 4;
                    out[o] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[o + 1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[o + 2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[o + 3] = 255;
                }
            }
            Ok(out)
        }
    }
}
