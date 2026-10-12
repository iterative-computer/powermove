//! Debug: prints RGBA values of a pixel rectangle from several PNGs, one block per file.
//!
//!   s7-pixels <x> <y> <w> <h> <png> [png ...]
use anyhow::{Context, Result};
use s7_replay::image::read_png;
use std::path::PathBuf;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize| -> Result<u32> { Ok(a.get(i).context("missing number")?.parse()?) };
    let (x0, y0, w, h) = (n(0)?, n(1)?, n(2)?, n(3)?);
    for path in &a[4..] {
        let img = read_png(&PathBuf::from(path))?;
        println!("{path}");
        for y in y0..y0 + h {
            let row: Vec<String> = (x0..x0 + w).map(|x| { let i = ((y * img.width + x) * 4) as usize; format!("{:3},{:3},{:3}", img.data[i], img.data[i + 1], img.data[i + 2]) }).collect();
            println!("  y{y:3}: {}", row.join(" | "));
        }
    }
    Ok(())
}
