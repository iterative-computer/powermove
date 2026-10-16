//! Compares the compat level 1 rasterizer with Chromium's Canvas2D
//! references (tools/canvas-ref). Development tool:
//!   cargo run -p pm-vector --features render --example canvas_compare -- <capture dir> [filter]
//! Environment: FLIP (references stored bottom-up), SHOW (every case),
//! PIX (differing pixels), ROW/COL (alpha along a row and a column).
use std::path::Path;

#[path = "../tests/canvas_cases.rs"]
mod canvas_cases;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]);
    let filter = args.get(2).cloned().unwrap_or_default();
    let flip = std::env::var("FLIP").is_ok();
    let show = std::env::var("SHOW").is_ok();
    let (mut exact, mut within1, mut worse, mut unsupported) = (0, 0, 0, 0);
    for c in &canvas_cases::cases() {
        let id = c["id"].as_str().unwrap();
        if !id.contains(&filter) {
            continue;
        }
        let path = dir.join(format!("{id}.png"));
        if std::fs::metadata(&path).is_err() {
            continue;
        }
        let Some(result) = canvas_cases::rasterize(c) else { continue };
        let (w, h, ours) = match result {
            Ok(r) => r,
            Err(e) => {
                unsupported += 1;
                if show { println!("{id:44} {e}"); }
                continue;
            }
        };
        let (rw, rh, mut reference) = canvas_cases::read_png(&path);
        if flip {
            let row = (rw * 4) as usize;
            let mut f = vec![0; reference.len()];
            for y in 0..rh as usize {
                f[y * row..(y + 1) * row].copy_from_slice(&reference[(rh as usize - 1 - y) * row..(rh as usize - y) * row]);
            }
            reference = f;
        }
        if (rw, rh) != (w, h) {
            println!("{id:44} SIZE ours {w}x{h} ref {rw}x{rh}");
            worse += 1;
            continue;
        }
        if let Ok(row) = std::env::var("ROW") {
            let y: u32 = row.parse().unwrap();
            let a: Vec<u8> = (0..w).map(|x| ours[((y * w + x) * 4 + 3) as usize]).collect();
            let b: Vec<u8> = (0..w).map(|x| reference[((y * w + x) * 4 + 3) as usize]).collect();
            println!("ours {:?}\nref  {:?}", &a[..a.len().min(40)], &b[..b.len().min(40)]);
            let col: u32 = std::env::var("COL").ok().and_then(|c| c.parse().ok()).unwrap_or(w / 2);
            let a: Vec<u8> = (0..h).map(|y| ours[((y * w + col) * 4 + 3) as usize]).collect();
            let b: Vec<u8> = (0..h).map(|y| reference[((y * w + col) * 4 + 3) as usize]).collect();
            println!("col {col} ours {:?}\ncol {col} ref  {:?}", a, b);
        }
        let (mut diff, mut over1, mut max) = (0usize, 0usize, 0i32);
        let mut shown = 0;
        for (i, (a, b)) in ours.iter().zip(&reference).enumerate() {
            let e = (i32::from(*a) - i32::from(*b)).abs();
            if e > 1 && i % 4 == 3 && shown < 8 && std::env::var("PIX").is_ok() {
                let p = i / 4;
                println!("   pixel ({}, {}) ours {:?} ref {:?}", p as u32 % w, p as u32 / w, &ours[p * 4..p * 4 + 4], &reference[p * 4..p * 4 + 4]);
                shown += 1;
            }
            diff += usize::from(e > 0);
            over1 += usize::from(e > 1);
            max = max.max(e);
        }
        match max {
            0 => exact += 1,
            1 => within1 += 1,
            _ => worse += 1,
        }
        if max > 0 || show {
            println!("{id:44} {w}x{h} differing {diff}, >1 {over1}, max {max}");
        }
    }
    println!("exact {exact}, within 1 {within1}, worse {worse}, unsupported {unsupported}");
}
