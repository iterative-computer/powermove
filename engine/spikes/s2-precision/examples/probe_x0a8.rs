//! Diagnostic: the 'x0a8' layout. Decode the same file to 'x0a8' and to 'x420' and
//! compare the luma, chroma, and alpha planes as raw samples.
use s2_precision::*;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    for p in &args[1..] {
        let p = PathBuf::from(p);
        println!("== {}", p.display());
        let a = vt::decode_file(&p, u32::from_be_bytes(*b"x0a8")).unwrap();
        let b = vt::decode_file(&p, pixbuf::X420).unwrap();
        let y4 = vt::decode_file(&p, pixbuf::Y416).unwrap();
        let fa = &a.frames[0].buffer;
        let fb = &b.frames[0].buffer;
        println!("x0a8 planes {} format '{}'", fa.plane_count(), cf::fourcc_str(fa.format()));
        for pl in 0..fa.plane_count() {
            let (bytes, bpr, w, h) = fa.read_plane_raw(Some(pl)).unwrap();
            println!("  plane {pl}: {w}x{h} bpr {bpr} first bytes {:?}", &bytes[..16]);
        }
        let (ya, bpr, w, h) = fa.read_plane_raw(Some(0)).unwrap();
        let (yb, _, _) = fb.read_u16(Some(0)).unwrap();
        // Interpret x0a8 plane 0 as u16 LE.
        let mut same = 0;
        let mut same_shift = 0;
        for y in 0..h {
            for x in 0..w {
                let o = y * bpr + x * 2;
                let v = u16::from_le_bytes([ya[o], ya[o + 1]]);
                if v == yb[y * w + x] {
                    same += 1;
                }
                if (v << 6) == yb[y * w + x] {
                    same_shift += 1;
                }
            }
        }
        println!("  x0a8 Y u16 == x420 Y: {same}/{}; (x0a8 << 6) == x420 Y: {same_shift}", w * h);
        let (yd, _, _) = y4.frames[0].buffer.read_u16(None).unwrap();
        for (px, py) in [(100usize, 100usize), (960, 540), (1800, 1000)] {
            let o = py * bpr + px * 2;
            let v = u16::from_le_bytes([ya[o], ya[o + 1]]);
            println!(
                "  ({px},{py}) x0a8 Y raw {v} (>>6 {}), x420 Y {} (>>6 {}), y416 Y {} A {}",
                v >> 6,
                yb[py * w + px],
                yb[py * w + px] >> 6,
                yd[(py * w + px) * 4 + 1],
                yd[(py * w + px) * 4]
            );
        }
        if fa.plane_count() > 2 {
            let (al, abpr, aw, _) = fa.read_plane_raw(Some(2)).unwrap();
            let _ = aw;
            for (px, py) in [(100usize, 100usize), (960, 540), (1800, 1000)] {
                println!("  ({px},{py}) x0a8 A {}", al[py * abpr + px]);
            }
        }
    }
}
