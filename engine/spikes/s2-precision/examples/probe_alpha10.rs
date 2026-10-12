//! Diagnostic: decode HEVC-with-alpha files to several formats through VideoToolbox's
//! converter and print samples (shows the 'x0a8' conversion defect at Main10).
use s2_precision::*;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    for p in &args[1..] {
        let p = PathBuf::from(p);
        println!("== {}", p.display());
        for fmt in [pixbuf::Y416, pixbuf::RGBA_HALF, pixbuf::X420, pixbuf::V420] {
            match vt::decode_file(&p, fmt) {
                Ok(d) => {
                    let b = &d.frames[0].buffer;
                    let w = b.width();
                    let px = [(100usize, 100usize), (960, 540), (1800, 1000)];
                    let mut s = String::new();
                    for (x, y) in px {
                        match fmt {
                            f if f == pixbuf::Y416 || f == pixbuf::RGBA_HALF => {
                                let (v, _, _) = b.read_u16(None).unwrap();
                                let o = (y * w + x) * 4;
                                s += &format!(" {:?}", &v[o..o + 4]);
                            }
                            f if f == pixbuf::X420 => {
                                let (v, _, _) = b.read_u16(Some(0)).unwrap();
                                s += &format!(" Y{}", v[y * w + x] >> 6);
                            }
                            _ => {
                                let (v, stride, _) = b.read_u8(Some(0)).unwrap();
                                let _ = stride;
                                s += &format!(" Y{}", v[y * w + x]);
                            }
                        }
                    }
                    println!("  {} delivered '{}':{}  attachments {:?}", cf::fourcc_str(fmt), cf::fourcc_str(b.format()), s,
                        b.attachments().iter().filter(|(k, _)| k.contains("Matrix") || k.contains("Transfer") || k.contains("Primaries")).collect::<Vec<_>>());
                }
                Err(e) => println!("  {}: error {e}", cf::fourcc_str(fmt)),
            }
        }
    }
}
