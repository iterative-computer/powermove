//! OpenEXR through OpenEXRCore (OpenEXR's C library, section 13): scanline
//! and tiled single-part images, every compression the library supports
//! (including DWAA/DWAB, B44, and HTJ2K). RGB(A) or Y(A) channels decode to
//! RGBA16F when every used channel is half, else RGBA32F. EXR is
//! scene-linear with premultiplied alpha; the data window is placed in the
//! display window.

use super::{ImageBackend, ImageColor, ImageFormat, Rgba, StillImage, pool_rgba};
use crate::error::{MediaError, Result};
use crate::pool::FramePool;
use crate::sys::openexr::*;
use pm_protocol::color::{AlphaMode, Transfer};
use std::ffi::{CStr, CString, c_int};
use std::path::Path;

fn check(r: exr_result_t, what: &str) -> Result<()> {
    if r == EXR_ERR_SUCCESS as exr_result_t {
        return Ok(());
    }
    let message = unsafe { CStr::from_ptr(exr_get_default_error_message(r)) }
        .to_string_lossy()
        .into_owned();
    Err(MediaError::OpenExr {
        what: what.into(),
        code: r,
        message,
    })
}

struct Ctx(exr_context_t);

impl Drop for Ctx {
    fn drop(&mut self) {
        unsafe { exr_finish(&mut self.0) };
    }
}

/// Which output component each channel feeds: 0 R, 1 G, 2 B, 3 A; gray
/// (`Y`) feeds R, G, and B.
fn slot(name: &str, gray: bool) -> Option<&'static [usize]> {
    match (name, gray) {
        ("R", false) => Some(&[0]),
        ("G", false) => Some(&[1]),
        ("B", false) => Some(&[2]),
        ("Y", true) => Some(&[0, 1, 2]),
        ("A", _) => Some(&[3]),
        _ => None,
    }
}

pub fn read(path: &Path, pool: &FramePool) -> Result<StillImage> {
    let cpath = CString::new(
        path.to_str()
            .ok_or_else(|| MediaError::invalid("path is not UTF-8"))?,
    )
    .map_err(|_| MediaError::invalid("path contains NUL"))?;
    let init = exr_context_initializer_t {
        size: std::mem::size_of::<exr_context_initializer_t>(),
        flags: EXR_CONTEXT_FLAG_SILENT_HEADER_PARSE as c_int,
        ..Default::default()
    };
    let mut raw: exr_context_t = std::ptr::null_mut();
    check(
        unsafe { exr_start_read(&mut raw, cpath.as_ptr(), &init) },
        "open",
    )?;
    let ctx = Ctx(raw);
    let c = ctx.0 as exr_const_context_t;
    let part = 0;
    let mut storage: exr_storage_t = 0;
    check(unsafe { exr_get_storage(c, part, &mut storage) }, "storage")?;
    if storage != EXR_STORAGE_SCANLINE && storage != EXR_STORAGE_TILED {
        return Err(MediaError::unsupported("deep OpenEXR images"));
    }
    let mut dw = exr_attr_box2i_t::default();
    let mut disp = exr_attr_box2i_t::default();
    check(
        unsafe { exr_get_data_window(c, part, &mut dw) },
        "data window",
    )?;
    check(
        unsafe { exr_get_display_window(c, part, &mut disp) },
        "display window",
    )?;
    let (dx0, dy0, dx1, dy1) = (dw.min.x, dw.min.y, dw.max.x, dw.max.y);
    let w = (dx1 as i64 - dx0 as i64 + 1).max(0) as usize;
    let h = (dy1 as i64 - dy0 as i64 + 1).max(0) as usize;
    let mut chl: *const exr_attr_chlist_t = std::ptr::null();
    check(unsafe { exr_get_channels(c, part, &mut chl) }, "channels")?;
    let entries =
        unsafe { std::slice::from_raw_parts((*chl).entries, (*chl).num_channels as usize) };
    let names: Vec<(String, exr_pixel_type_t, i32, i32)> = entries
        .iter()
        .map(|e| {
            let n = unsafe { CStr::from_ptr(e.name.str_) }
                .to_string_lossy()
                .into_owned();
            (n, e.pixel_type, e.x_sampling, e.y_sampling)
        })
        .collect();
    let has = |n: &str| names.iter().any(|e| e.0 == n);
    let gray = !has("R") && has("Y");
    let rgb = has("R") && has("G") && has("B");
    if !gray && !rgb {
        return Err(MediaError::unsupported(format!(
            "OpenEXR channels {:?} (RGB, RGBA, Y, or YA)",
            names.iter().map(|n| n.0.as_str()).collect::<Vec<_>>()
        )));
    }
    let used: Vec<&(String, exr_pixel_type_t, i32, i32)> = names
        .iter()
        .filter(|e| slot(&e.0, gray).is_some())
        .collect();
    if used.iter().any(|e| e.2 != 1 || e.3 != 1) {
        return Err(MediaError::unsupported("subsampled OpenEXR channels"));
    }
    let half = used.iter().all(|e| e.1 == EXR_PIXEL_HALF);
    let alpha = has("A");
    let elem = if half { 2usize } else { 4 };
    let mut buf = vec![0u8; w * h * 4 * elem];
    // Opaque alpha when the file has none.
    if !alpha {
        for px in buf.chunks_exact_mut(4 * elem) {
            if half {
                px[6..8].copy_from_slice(&0x3C00u16.to_ne_bytes());
            } else {
                px[12..16].copy_from_slice(&1.0f32.to_ne_bytes());
            }
        }
    }
    // `origin` is the chunk's top-left pixel relative to the data window:
    // scanline chunks carry absolute coordinates, tile chunks carry tile
    // indices (OpenEXRCore `chunk.c`).
    let decode_chunk =
        |cinfo: &exr_chunk_info_t, origin: (usize, usize), buf: &mut Vec<u8>| -> Result<()> {
            let mut dec: exr_decode_pipeline_t = unsafe { std::mem::zeroed() };
            check(
                unsafe { exr_decoding_initialize(c, part, cinfo, &mut dec) },
                "decode init",
            )?;
            let (col0, row0) = origin;
            for k in 0..dec.channel_count as usize {
                let ch = unsafe { &mut *dec.channels.add(k) };
                let name = unsafe { CStr::from_ptr(ch.channel_name) }.to_string_lossy();
                match slot(&name, gray) {
                    Some(targets) => {
                        let off = ((row0 * w + col0) * 4 + targets[0]) * elem;
                        ch.__bindgen_anon_1.decode_to_ptr = unsafe { buf.as_mut_ptr().add(off) };
                        ch.user_pixel_stride = (4 * elem) as i32;
                        ch.user_line_stride = (w * 4 * elem) as i32;
                        ch.user_data_type = if half {
                            EXR_PIXEL_HALF as u16
                        } else {
                            EXR_PIXEL_FLOAT as u16
                        };
                        ch.user_bytes_per_element = elem as i16;
                    }
                    None => ch.__bindgen_anon_1.decode_to_ptr = std::ptr::null_mut(),
                }
            }
            let r = (|| {
                check(
                    unsafe { exr_decoding_choose_default_routines(c, part, &mut dec) },
                    "decode routines",
                )?;
                check(unsafe { exr_decoding_run(c, part, &mut dec) }, "decode")
            })();
            unsafe { exr_decoding_destroy(c, &mut dec) };
            r?;
            // Gray: copy Y into G and B for this chunk.
            if gray {
                for y in row0..(row0 + cinfo.height as usize).min(h) {
                    for x in col0..(col0 + cinfo.width as usize).min(w) {
                        let p = (y * w + x) * 4 * elem;
                        let (r, rest) = buf[p..p + 3 * elem].split_at_mut(elem);
                        rest[..elem].copy_from_slice(r);
                        rest[elem..2 * elem].copy_from_slice(r);
                    }
                }
            }
            Ok(())
        };
    if storage == EXR_STORAGE_SCANLINE {
        let mut y = dy0;
        while y <= dy1 {
            let mut ci = exr_chunk_info_t::default();
            check(
                unsafe { exr_read_scanline_chunk_info(c, part, y, &mut ci) },
                "scanline chunk",
            )?;
            decode_chunk(
                &ci,
                ((ci.start_x - dx0) as usize, (ci.start_y - dy0) as usize),
                &mut buf,
            )?;
            y = ci.start_y + ci.height.max(1);
        }
    } else {
        let (mut tx, mut ty): (i32, i32) = (0, 0);
        check(
            unsafe { exr_get_tile_counts(c, part, 0, 0, &mut tx, &mut ty) },
            "tile counts",
        )?;
        let (mut tw, mut th) = (0u32, 0u32);
        let (mut lm, mut rm) = (0, 0);
        check(
            unsafe { exr_get_tile_descriptor(c, part, &mut tw, &mut th, &mut lm, &mut rm) },
            "tile descriptor",
        )?;
        for j in 0..ty {
            for i in 0..tx {
                let mut ci = exr_chunk_info_t::default();
                check(
                    unsafe { exr_read_tile_chunk_info(c, part, i, j, 0, 0, &mut ci) },
                    "tile chunk",
                )?;
                decode_chunk(
                    &ci,
                    (i as usize * tw as usize, j as usize * th as usize),
                    &mut buf,
                )?;
            }
        }
    }
    // Place the data window in the display window.
    let (vx0, vy0) = (disp.min.x, disp.min.y);
    let vw = (disp.max.x as i64 - vx0 as i64 + 1).max(0) as usize;
    let vh = (disp.max.y as i64 - vy0 as i64 + 1).max(0) as usize;
    let (out, ow, oh) = if (vx0, vy0, vw, vh) == (dx0, dy0, w, h) {
        (buf, w, h)
    } else {
        let mut o = vec![0u8; vw * vh * 4 * elem];
        for y in 0..h {
            let vy = dy0 as i64 + y as i64 - vy0 as i64;
            if vy < 0 || vy >= vh as i64 {
                continue;
            }
            for x in 0..w {
                let vx = dx0 as i64 + x as i64 - vx0 as i64;
                if vx < 0 || vx >= vw as i64 {
                    continue;
                }
                let s = (y * w + x) * 4 * elem;
                let d = (vy as usize * vw + vx as usize) * 4 * elem;
                o[d..d + 4 * elem].copy_from_slice(&buf[s..s + 4 * elem]);
            }
        }
        (o, vw, vh)
    };
    let label = format!(
        "OpenEXR {} {}",
        if half { "half" } else { "float" },
        if storage == EXR_STORAGE_TILED {
            "tiled"
        } else {
            "scanline"
        }
    );
    let frame = if half {
        let v: Vec<u16> = out
            .chunks_exact(2)
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .collect();
        pool_rgba(pool, ow, oh, Rgba::F16(&v), alpha, 16, label)?
    } else {
        let v: Vec<f32> = out
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        pool_rgba(pool, ow, oh, Rgba::F32(&v), alpha, 32, label)?
    };
    let mut chroma = exr_attr_chromaticities_t::default();
    let name = CString::new("chromaticities").unwrap();
    let has_chroma = unsafe { exr_attr_get_chromaticities(c, part, name.as_ptr(), &mut chroma) }
        == EXR_ERR_SUCCESS as exr_result_t;
    let chromaticities = has_chroma.then(|| {
        let ch = chroma;
        [
            ch.red_x, ch.red_y, ch.green_x, ch.green_y, ch.blue_x, ch.blue_y, ch.white_x,
            ch.white_y,
        ]
    });
    Ok(StillImage {
        frame,
        format: ImageFormat::Exr,
        backend: ImageBackend::OpenExr,
        bit_depth: if half { 16 } else { 32 },
        float: true,
        color: ImageColor {
            icc: None,
            icc_bytes: None,
            icc_class: None,
            transfer: Transfer::Linear,
            alpha: if alpha {
                AlphaMode::Premultiplied
            } else {
                AlphaMode::Opaque
            },
            chromaticities,
        },
        orientation: 1,
    })
}
