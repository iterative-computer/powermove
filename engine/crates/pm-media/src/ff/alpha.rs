//! VP8 and VP9 with alpha through two FFmpeg native decoders (section 26
//! amendment 6): one for the color stream, one for the alpha stream carried
//! in Matroska BlockAdditional id 1 (an independent VP8 or VP9 stream whose
//! luma is the alpha plane). This is the structure of FFmpeg's libvpx
//! wrapper, built from FFmpeg's native decoders, which render into pool
//! frames: neither stream is copied, although libvpx's VP8 decoder has no
//! external frame buffers. VP8 and VP9 decoding is exact (the reference
//! decoder defines the output), so the native decoders produce libvpx's
//! pixels; the regression suite checks that against FFmpeg's libvpx decode.

use super::Packet;
use super::decoder::FfDecoder;
use crate::error::{MediaError, Result};
use crate::frame::{DecodedFrame, Pixels, PlaneDesc, PoolFrame, PoolPlane};
use crate::pool::FramePool;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct AlphaPairDecoder {
    color: FfDecoder,
    alpha: FfDecoder,
    pool: FramePool,
    colors: BTreeMap<i64, DecodedFrame>,
    masks: BTreeMap<i64, DecodedFrame>,
    /// PTS of frames whose packet carried no alpha (opaque).
    opaque: std::collections::BTreeSet<i64>,
}

impl AlphaPairDecoder {
    pub fn new(input: &super::Input, stream: usize, pool: FramePool, threads: i32) -> Result<Self> {
        let color = FfDecoder::new(input, stream, None, pool.clone(), threads)?;
        let alpha = FfDecoder::new(input, stream, None, pool.clone(), threads)?;
        if color.name() != "vp8" && color.name() != "vp9" {
            return Err(MediaError::unsupported(format!(
                "alpha pairing needs FFmpeg's vp8 or vp9, got {}",
                color.name()
            )));
        }
        if !color.direct() || !alpha.direct() {
            return Err(MediaError::unsupported(format!(
                "{} does not render into pool frames",
                color.name()
            )));
        }
        Ok(AlphaPairDecoder {
            color,
            alpha,
            pool,
            colors: BTreeMap::new(),
            masks: BTreeMap::new(),
            opaque: Default::default(),
        })
    }

    pub fn name(&self) -> String {
        format!("{} + {} alpha", self.color.name(), self.alpha.name())
    }

    pub fn copies(&self) -> usize {
        self.color.copies + self.alpha.copies
    }

    pub fn send(&mut self, p: Option<&Packet>, out: &mut Vec<DecodedFrame>) -> Result<()> {
        let mut got = Vec::new();
        match p {
            Some(p) => {
                self.color.send(Some(p), &mut got)?;
                self.colors.extend(got.drain(..).map(|f| (f.pts, f)));
                match &p.additional {
                    Some(add) => {
                        let ap = Packet {
                            data: add.clone(),
                            additional: None,
                            ..p.clone()
                        };
                        self.alpha.send(Some(&ap), &mut got)?;
                        self.masks.extend(got.drain(..).map(|f| (f.pts, f)));
                    }
                    None => {
                        self.opaque.insert(p.pts);
                    }
                }
            }
            None => {
                self.color.send(None, &mut got)?;
                self.colors.extend(got.drain(..).map(|f| (f.pts, f)));
                self.alpha.send(None, &mut got)?;
                self.masks.extend(got.drain(..).map(|f| (f.pts, f)));
            }
        }
        self.pair(out)?;
        if p.is_none() && !self.colors.is_empty() {
            return Err(MediaError::decode(format!(
                "color pictures without alpha after drain: {:?}",
                self.colors.keys().collect::<Vec<_>>()
            )));
        }
        Ok(())
    }

    /// Emits every frame whose color and alpha pictures have both arrived.
    fn pair(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        let ready: Vec<i64> = self
            .colors
            .keys()
            .copied()
            .filter(|k| self.masks.contains_key(k) || self.opaque.contains(k))
            .collect();
        for pts in ready {
            let c = self.colors.remove(&pts).unwrap();
            let Pixels::Pool(cp) = &c.pixels else {
                unreachable!("FFmpeg frames live in the pool")
            };
            let mut format = (*c.format).clone();
            let alpha_plane = if self.opaque.remove(&pts) {
                // No alpha for this frame: an opaque plane at the color depth.
                let (w, h) = (format.width, format.height);
                let bps = if format.bits > 8 { 2 } else { 1 };
                let stride = (w * bps).div_ceil(crate::pool::ROW_ALIGN) * crate::pool::ROW_ALIGN;
                let slot = self.pool.acquire(stride * h)?;
                let max = (1u32 << format.bits) - 1;
                for y in 0..h {
                    let row = unsafe { slot.ptr().add(y * stride) };
                    for x in 0..w {
                        unsafe {
                            if bps == 2 {
                                *(row as *mut u16).add(x) = max as u16;
                            } else {
                                *row.add(x) = max as u8;
                            }
                        }
                    }
                }
                PoolPlane {
                    decoder_ptr: slot.ptr() as usize,
                    slot,
                    offset: 0,
                    stride,
                }
            } else {
                let a = self.masks.remove(&pts).unwrap();
                let Pixels::Pool(ap) = &a.pixels else {
                    unreachable!("FFmpeg frames live in the pool")
                };
                if a.format.width != format.width
                    || a.format.height != format.height
                    || a.format.bits != format.bits
                {
                    return Err(MediaError::decode(format!(
                        "alpha picture {}x{} {}-bit differs from color {}x{} {}-bit",
                        a.format.width,
                        a.format.height,
                        a.format.bits,
                        format.width,
                        format.height,
                        format.bits
                    )));
                }
                let plane = ap.planes[0].clone();
                // The color frame's owner keeps the color AVFrame; the alpha
                // AVFrame stays alive through the frame kept below.
                let owner = (c.clone(), a.clone());
                let mut planes = cp.planes.clone();
                planes.push(plane);
                format.planes.push(PlaneDesc {
                    format: a.format.planes[0].format,
                    width: format.width,
                    height: format.height,
                });
                format.alpha_bits = format.bits;
                format.label = format.label.replacen("yuv", "yuva", 1);
                out.push(DecodedFrame {
                    pts,
                    format: Arc::new(format),
                    pixels: Pixels::Pool(Arc::new(PoolFrame {
                        planes,
                        owner: Some(Box::new(KeepFrames(owner))),
                        copied: cp.copied || ap.copied,
                    })),
                    color: None,
                });
                continue;
            };
            let mut planes = cp.planes.clone();
            planes.push(alpha_plane);
            format.planes.push(PlaneDesc {
                format: format.planes[0].format,
                width: format.width,
                height: format.height,
            });
            format.alpha_bits = format.bits;
            format.label = format.label.replacen("yuv", "yuva", 1);
            out.push(DecodedFrame {
                pts,
                format: Arc::new(format),
                pixels: Pixels::Pool(Arc::new(PoolFrame {
                    planes,
                    owner: Some(Box::new(KeepFrames((c.clone(), c.clone())))),
                    copied: cp.copied,
                })),
                color: None,
            });
        }
        Ok(())
    }

    pub fn flush(&mut self) {
        self.color.flush();
        self.alpha.flush();
        self.colors.clear();
        self.masks.clear();
        self.opaque.clear();
    }
}

/// Keeps the decoders' frames alive while the paired frame is in use.
struct KeepFrames(#[allow(dead_code)] (DecodedFrame, DecodedFrame));
