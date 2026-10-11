//! Diagnostic: v216 layout and precision of a ProRes 422 decode versus x422.
use s2_precision::*;
use std::path::PathBuf;

fn main() {
    let p = PathBuf::from(std::env::args().nth(1).unwrap());
    let v = vt::decode_file(&p, pixbuf::V216).unwrap();
    let x = vt::decode_file(&p, pixbuf::X422).unwrap();
    let (vd, comps, h) = v.frames[0].buffer.read_u16(None).unwrap();
    let (xy, w, _) = x.frames[0].buffer.read_u16(Some(0)).unwrap();
    let (xc, _, _) = x.frames[0].buffer.read_u16(Some(1)).unwrap();
    println!("v216 comps/row {comps} rows {h}; x422 width {w}");
    for i in 0..4 {
        println!("v216 texel {i}: {:?}  x422 Y {:?} C {:?}", &vd[i * 4..i * 4 + 4], &xy[i * 2..i * 2 + 2], &xc[i * 2..i * 2 + 2]);
    }
    let low6 = vd.iter().filter(|s| *s & 63 != 0).count() as f64 / vd.len() as f64;
    // Does x422 equal round(v216 / 64) * 64?
    let mut same = 0usize;
    let mut n = 0usize;
    for y in 0..h {
        for t in 0..w / 2 {
            let b = (y * w / 2 + t) * 4;
            let pairs = [(vd[b + 1], xy[y * w + 2 * t]), (vd[b + 3], xy[y * w + 2 * t + 1]), (vd[b], xc[y * w + 2 * t]), (vd[b + 2], xc[y * w + 2 * t + 1])];
            for (a, c) in pairs {
                n += 1;
                if ((a as u32 + 32) >> 6).min(1023) == (c as u32 >> 6) {
                    same += 1;
                }
            }
        }
    }
    println!("fraction of v216 samples with nonzero low 6 bits: {low6:.4}");
    println!("x422 == round(v216/64): {same}/{n}");
}
