//! Debug view: crops the same region from a reference and a test PNG and writes
//! reference | test | |difference| × gain, magnified with nearest-neighbor sampling.
//!
//!   s7-zoom <ref.png> <test.png> <x> <y> <w> <h> <scale> <gain> <out.png>
use anyhow::{Context, Result};
use s7_replay::image::{read_png, write_png};
use std::path::PathBuf;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let r = read_png(&PathBuf::from(a.first().context("ref")?))?;
    let t = read_png(&PathBuf::from(a.get(1).context("test")?))?;
    let n = |i: usize| -> Result<u32> { Ok(a.get(i).context("missing number")?.parse()?) };
    let (x0, y0, w, h, s, gain) = (n(2)?, n(3)?, n(4)?, n(5)?, n(6)?, n(7)?);
    let out = PathBuf::from(a.get(8).context("out")?);
    let (ow, oh) = (w * s * 3, h * s);
    let mut img = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let panel = x / (w * s);
            let (sx, sy) = (x0 + (x % (w * s)) / s, y0 + y / s);
            let i = ((sy * r.width + sx) * 4) as usize;
            let px: [u8; 4] = match panel {
                0 => r.data[i..i + 4].try_into()?,
                1 => t.data[i..i + 4].try_into()?,
                _ => {
                    let d = |c: usize| ((r.data[i + c] as i32 - t.data[i + c] as i32).unsigned_abs() * gain).min(255) as u8;
                    [d(0), d(1), d(2), 255]
                }
            };
            let o = ((y * ow + x) * 4) as usize;
            // Composite over mid grey so transparent pixels stay readable.
            let al = px[3] as u32;
            for c in 0..3 {
                img[o + c] = if panel == 2 { px[c] } else { (px[c] as u32 + (255 - al) * 64 / 255).min(255) as u8 };
            }
            img[o + 3] = 255;
        }
    }
    write_png(&out, ow, oh, &img)
}
