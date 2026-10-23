//! Glitch-safety rule: the render the real-time producer runs must
//! allocate nothing once warm. A counting allocator counts this thread's
//! heap allocations around 200 renders of a mix with every kind of voice.
//!
//! Not there yet: measured 316,504 allocations in 200 renders (about 1,580
//! a render), almost all inside `pm_eval` (channel evaluation: `ev_p`, its
//! key strings and keyframe curves, `Comp::in_root` in `MixGraph::resolve`),
//! then the mixer's own per-render views (`render_today`, `read_linear`,
//! `stretch::produce`). This is a ratchet towards 0: the ceiling may only
//! go down. `PM_ALLOC_SITES=1` prints where they come from.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;
use pm_audio::{MixFormat, RenderOptions};

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

thread_local! {
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}
static SITES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static SAMPLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn note() {
    // `try_with`: thread-local teardown may allocate-free after destruction.
    let _ = COUNTING.try_with(|on| {
        if on.get() {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            // PM_ALLOC_SITES=1: where they come from (a diagnostic only).
            if SAMPLE.load(std::sync::atomic::Ordering::Relaxed)
                && !INSIDE.with(|i| i.replace(true))
            {
                let bt = std::backtrace::Backtrace::force_capture().to_string();
                let frames: Vec<&str> = bt
                    .lines()
                    .filter(|l| {
                        (l.contains("pm_") || l.contains("symphonia") || l.contains("rubato"))
                            && !l.contains("render_allocations")
                            && !l.contains("alloc::")
                    })
                    .take(3)
                    .collect();
                SITES
                    .lock()
                    .unwrap()
                    .push(frames.join(" <- ").trim().to_string());
                INSIDE.with(|i| i.set(false));
            }
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// This thread's allocations while `f` runs.
fn allocations(f: impl FnOnce()) -> u64 {
    COUNT.with(|c| c.set(0));
    COUNTING.with(|on| on.set(true));
    f();
    COUNTING.with(|on| on.set(false));
    COUNT.with(|c| c.get())
}

const SR: u32 = 48_000;

#[test]
fn warm_renders_allocate_nothing() {
    let dir = temp_dir("render-alloc");
    let t48 = dir.join("t48.wav");
    let t44 = dir.join("t44.wav");
    let wide = dir.join("wide.wav");
    write_wav(
        &t48,
        SR,
        &[tone(SR, 8.0, 440.0, 0.3), tone(SR, 8.0, 550.0, 0.3)],
    );
    write_wav(
        &t44,
        44_100,
        &[tone(44_100, 8.0, 660.0, 0.3), tone(44_100, 8.0, 770.0, 0.3)],
    );
    write_wav(
        &wide,
        SR,
        &(0..6)
            .map(|c| tone(SR, 8.0, 300.0 + 100.0 * c as f64, 0.1))
            .collect::<Vec<_>>(),
    );
    let gain = keys(&[(0.0, 0.2), (3.0, 1.0), (6.0, 0.4)]);
    let inner = comp(
        30.0,
        8.0,
        &[audio("I", "t44", 0.2, 7.0, r#","trim":0.1"#, "")],
        r#","audioGain":0.7"#,
    );
    let layers = [
        audio(
            "A",
            "t48",
            0.0,
            8.0,
            &format!(r#","gain":{gain},"fadeIn":0.5,"fadeOut":0.5"#),
            "",
        ),
        layer(
            "P",
            "precomp",
            0.1,
            7.5,
            r#"{"comp":"X","audioGain":0.8}"#,
            "",
        ),
        audio("S", "t48", 0.0, 8.0, r#","speed":1.5,"gain":0.4"#, ""),
        audio("W", "wide", 1.0, 5.0, r#","gain":0.5"#, ""),
    ];
    let doc = project(30.0, 8.0, &layers, &[("X".into(), inner)], "");
    let rig = Rig::new(&[("t48", &t48), ("t44", &t44), ("wide", &wide)]);
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
    let mut out = vec![vec![0f32; 512]; 2];
    let options = RenderOptions {
        meters: true,
        wait: true,
        ..RenderOptions::default()
    };
    // Warm: decoders, scratch, stretch state, conversions.
    for k in 0..64 {
        m.render(k * 512, 512, &mut out, &options).unwrap();
    }
    let mut at = 64 * 512;
    SAMPLE.store(
        std::env::var_os("PM_ALLOC_SITES").is_some(),
        std::sync::atomic::Ordering::Relaxed,
    );
    let n = allocations(|| {
        for _ in 0..200 {
            m.render(at, 512, &mut out, &options).unwrap();
            at += 512;
        }
    });
    SAMPLE.store(false, std::sync::atomic::Ordering::Relaxed);
    if std::env::var_os("PM_ALLOC_SITES").is_some() {
        let sites = SITES.lock().unwrap();
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for s in sites.iter() {
            *counts.entry(s.as_str()).or_default() += 1;
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (s, c) in v.iter().take(25) {
            eprintln!("{c:7} {s}");
        }
    }
    // The ratchet (target 0; lower it with every removal).
    const CEILING: u64 = 316_504;
    eprintln!("heap allocations in 200 warm renders: {n} (ceiling {CEILING}, target 0)");
    assert!(
        n <= CEILING,
        "heap allocations in 200 warm renders: {n} > {CEILING}"
    );
}
