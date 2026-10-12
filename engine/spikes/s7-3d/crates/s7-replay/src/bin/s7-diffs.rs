//! Debug: lists pixels in a rectangle whose largest channel difference exceeds a threshold.
//!
//!   s7-diffs <a.png> <b.png> <x> <y> <w> <h> <threshold>
use anyhow::{Context, Result};
use s7_replay::image::read_png;
use std::path::PathBuf;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let p = read_png(&PathBuf::from(a.first().context("a")?))?;
    let q = read_png(&PathBuf::from(a.get(1).context("b")?))?;
    let n = |i: usize| -> Result<u32> { Ok(a.get(i).context("missing number")?.parse()?) };
    let (x0, y0, w, h, t) = (n(2)?, n(3)?, n(4)?, n(5)?, n(6)?);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = ((y * p.width + x) * 4) as usize;
            let d = (0..4).map(|c| (p.data[i + c] as i32 - q.data[i + c] as i32).unsigned_abs()).max().unwrap();
            if d > t {
                println!("{x},{y}: {:?} vs {:?} (d {d})", &p.data[i..i + 4], &q.data[i..i + 4]);
            }
        }
    }
    Ok(())
}
