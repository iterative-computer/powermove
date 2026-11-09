//! Encoding render-cache and compatibility rasters to RGBA8, and placing a
//! painted crop into a canvas window.
//!
//! The viewer and the background jobs read back premultiplied float or unorm
//! pixels; these turn them into the RGBA8 the snapshot transport carries, with
//! the alpha treatment the request asks for, and crop/place them into the
//! window the caller wants.

use pm_protocol::common::PixelRect;
use pm_protocol::snapshot::SnapshotPixels;

fn push_rgba8(out: &mut Vec<u8>, [r, g, b, a]: [f32; 4], pixels: SnapshotPixels) {
    // Final renderer frames are premultiplied E, including LinearLight
    // projects (pm-render's final color pass encodes those before readback).
    let alpha = match pixels {
        SnapshotPixels::Opaque => crate::AlphaOutput::None,
        SnapshotPixels::Premultiplied => crate::AlphaOutput::Premultiplied,
        SnapshotPixels::Straight => crate::AlphaOutput::Straight,
    };
    out.extend_from_slice(&crate::convert::rgba8_pixel(
        [r, g, b, a],
        crate::WorkingEncoding::Standard,
        alpha,
        crate::ColorTags::SrgbImage,
        [0.0; 3],
    ));
}

fn push_rgba8_unorm(out: &mut Vec<u8>, [r, g, b, a]: [u8; 4], pixels: SnapshotPixels) {
    match pixels {
        SnapshotPixels::Opaque => out.extend_from_slice(&[r, g, b, 255]),
        SnapshotPixels::Premultiplied => out.extend_from_slice(&[r, g, b, a]),
        SnapshotPixels::Straight if a == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
        SnapshotPixels::Straight => {
            let un =
                |v: u8| ((u32::from(v) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            out.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
}

/// Premultiplied float RGBA to RGBA8, with the request's alpha treatment.
pub fn encode_rgba8_f32(rgba: &[f32], pixels: SnapshotPixels) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        push_rgba8(&mut out, [px[0], px[1], px[2], px[3]], pixels);
    }
    out
}

/// Unorm RGBA8 to RGBA8, with the request's alpha treatment.
pub fn encode_rgba8_unorm(rgba: &[u8], pixels: SnapshotPixels) -> Vec<u8> {
    if pixels == SnapshotPixels::Premultiplied {
        return rgba.to_vec();
    }
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        push_rgba8_unorm(&mut out, [px[0], px[1], px[2], px[3]], pixels);
    }
    out
}

/// Places a painted `crop` (`[x, y, w, h]` in canvas pixels) into the canvas
/// `window` (or the whole canvas), returning the window's RGBA8 and its size.
/// `None` when the window is empty.
pub fn place_raster(
    painted: &[u8],
    crop: [u32; 4],
    canvas: (u32, u32),
    window: Option<PixelRect>,
) -> Option<(Vec<u8>, u32, u32)> {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    let (wx, wy, ww, wh) = match window {
        Some(w) => {
            let x = w.x.min(cw);
            let y = w.y.min(ch);
            (x, y, w.width.min(cw - x), w.height.min(ch - y))
        }
        None => (0, 0, cw, ch),
    };
    if ww == 0 || wh == 0 {
        return None;
    }
    let mut out = vec![0u8; (ww as usize) * (wh as usize) * 4];
    let [px, py, pw, ph] = crop;
    for row in 0..ph {
        let y = py + row;
        if y < wy || y >= wy + wh {
            continue;
        }
        for col in 0..pw {
            let x = px + col;
            if x < wx || x >= wx + ww {
                continue;
            }
            let src = ((row * pw + col) * 4) as usize;
            let dst = (((y - wy) * ww + (x - wx)) * 4) as usize;
            if src + 4 <= painted.len() {
                out[dst..dst + 4].copy_from_slice(&painted[src..src + 4]);
            }
        }
    }
    Some((out, ww, wh))
}
