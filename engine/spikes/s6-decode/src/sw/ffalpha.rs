//! VP8 or VP9 with alpha through two FFmpeg native decoders: one for the color
//! stream, one for the alpha stream carried in Matroska BlockAdditional id 1
//! (an independent VP8 or VP9 stream whose luma is the alpha plane). This is
//! the structure of FFmpeg's libvpx wrapper, with FFmpeg's native decoders,
//! which render into `get_buffer2` frames: both decode straight into the
//! Metal-shared pool, so VP8 alpha needs no copy even though libvpx's VP8
//! decoder has no external frame buffers.
//!
//! VP8 and VP9 decoding is exact (the reference decoder defines the output), so
//! the native decoders must produce libvpx's pixels; the matrix checks that
//! against an FFmpeg libvpx framemd5 and the other backends.

use super::SwDecoder;
use crate::ff::{FfDecoder, Input, Packet};
use crate::pool::{MetalPool, PoolFrame};
use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct FfAlphaDecoder {
    main: FfDecoder,
    alpha: FfDecoder,
    color: BTreeMap<i64, PoolFrame>,
    mask: BTreeMap<i64, PoolFrame>,
}

impl FfAlphaDecoder {
    pub fn new(inp: &Input, pool: Arc<MetalPool>, threads: i32) -> Result<Self> {
        let main = FfDecoder::new(inp, None, pool.clone(), threads)?;
        let alpha = FfDecoder::new(inp, None, pool, threads)?;
        ensure!(main.name == "vp8" || main.name == "vp9", "native alpha pairing needs FFmpeg's vp8 or vp9, got {}", main.name);
        ensure!(main.dr1 && alpha.dr1, "{} does not render into get_buffer2 frames", main.name);
        Ok(FfAlphaDecoder { main, alpha, color: BTreeMap::new(), mask: BTreeMap::new() })
    }

    /// Emits every frame whose color and alpha pictures have both arrived.
    fn pair(&mut self, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        let ready: Vec<i64> = self.color.keys().copied().filter(|k| self.mask.contains_key(k)).collect();
        for pts in ready {
            let mut c = self.color.remove(&pts).unwrap();
            let mut a = self.mask.remove(&pts).unwrap();
            ensure!(c.planes.len() == 3 && !a.planes.is_empty(), "unexpected plane counts {} and {}", c.planes.len(), a.planes.len());
            ensure!(
                a.planes[0].width == c.planes[0].width && a.planes[0].height == c.planes[0].height && a.depth == c.depth,
                "alpha picture {}x{} {}-bit differs from color {}x{} {}-bit",
                a.planes[0].width,
                a.planes[0].height,
                a.depth,
                c.planes[0].width,
                c.planes[0].height,
                c.depth
            );
            c.planes.push(a.planes.swap_remove(0));
            c.label = c.label.replacen("yuv", "yuva", 1);
            c.copied |= a.copied;
            let owners = (c.owner.take(), a.owner.take());
            c.owner = Some(Box::new(owners));
            out.push((pts, c));
        }
        Ok(())
    }
}

impl SwDecoder for FfAlphaDecoder {
    fn send(&mut self, p: Option<&Packet>, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        let mut got = vec![];
        match p {
            Some(p) => {
                let Some(add) = &p.additional else { bail!("alpha stream packet at pts {} without BlockAdditional", p.pts) };
                let ap = Packet { data: add.clone(), additional: None, ..p.clone() };
                self.main.send(Some(p), &mut got)?;
                self.color.extend(got.drain(..));
                self.alpha.send(Some(&ap), &mut got)?;
                self.mask.extend(got.drain(..));
            }
            None => {
                self.main.send(None, &mut got)?;
                self.color.extend(got.drain(..));
                self.alpha.send(None, &mut got)?;
                self.mask.extend(got.drain(..));
            }
        }
        self.pair(out)?;
        if p.is_none() {
            ensure!(
                self.color.is_empty() && self.mask.is_empty(),
                "unpaired pictures after drain: color {:?}, alpha {:?}",
                self.color.keys().collect::<Vec<_>>(),
                self.mask.keys().collect::<Vec<_>>()
            );
        }
        Ok(())
    }
    fn name(&self) -> String {
        format!("{} + {} alpha decoder", self.main.name, self.alpha.name)
    }
    fn direct(&self) -> bool {
        self.main.dr1 && self.alpha.dr1
    }
    fn copies(&self) -> usize {
        self.main.copies + self.alpha.copies
    }
    fn flush(&mut self) {
        self.main.flush();
        self.alpha.flush();
        self.color.clear();
        self.mask.clear();
    }
}
