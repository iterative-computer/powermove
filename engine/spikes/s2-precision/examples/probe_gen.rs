//! Diagnostic: per-frame, per-region luma statistics of a decoded file against a reference.
use s2_precision::*;
use std::path::PathBuf;

fn main() {
    let a: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let r = vt::decode_file(&a[0], pixbuf::X422).unwrap();
    let t = vt::decode_file(&a[1], pixbuf::X422).unwrap();
    for (i, (fr, ft)) in r.frames.iter().zip(&t.frames).enumerate() {
        let (ry, w, h) = fr.buffer.read_u16(Some(0)).unwrap();
        let (ty, _, _) = ft.buffer.read_u16(Some(0)).unwrap();
        for band in 0..3 {
            let (y0, y1) = (band * h / 3, (band + 1) * h / 3);
            let (mut sd, mut sa, mut n) = (0f64, 0f64, 0f64);
            for y in y0..y1 {
                for x in 0..w {
                    let d = (ty[y * w + x] >> 6) as f64 - (ry[y * w + x] >> 6) as f64;
                    sd += d;
                    sa += d * d;
                    n += 1.0;
                }
            }
            println!(
                "frame {i} pts {} band {band}: mean diff {:.3} rms {:.3}",
                { ft.pts.value },
                sd / n,
                (sa / n).sqrt()
            );
        }
    }
}
