//! Every channel layout our FFmpeg build names (the layouts files reach the
//! engine with; today's engine takes any of them through Chromium's decode)
//! decodes with its speakers and folds into the stereo bus as today's Web
//! Audio graph folds it: by channel count alone (Chromium's
//! `AudioBus::SumFromByDownMixing`): 4 channels as quad, 6 as 5.1, every
//! other count keeping its first two channels.

mod common;

use common::*;
use pm_audio::MixFormat;
use pm_audio::layout;
use pm_media::audio::Speaker;

const SR: u32 = 48_000;

/// Amplitude of a tone at `freq` in `x` (least squares).
fn amplitude(x: &[f32], freq: f64) -> f64 {
    let w = std::f64::consts::TAU * freq / SR as f64;
    let (mut xs, mut xc) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    2.0 * (xs * xs + xc * xc).sqrt() / x.len() as f64
}

const LAYOUTS: &[&str] = &[
    "mono",
    "stereo",
    "2.1",
    "3.0",
    "3.0(back)",
    "4.0",
    "quad",
    "quad(side)",
    "3.1",
    "5.0",
    "5.0(side)",
    "4.1",
    "5.1",
    "5.1(side)",
    "6.0",
    "6.0(front)",
    "3.1.2",
    "hexagonal",
    "6.1",
    "6.1(back)",
    "6.1(front)",
    "7.0",
    "7.0(front)",
    "7.1",
    "7.1(wide)",
    "7.1(wide-side)",
    "5.1.2",
    "5.1.2(back)",
    "octagonal",
    "cube",
    "5.1.4",
    "7.1.2",
    "7.1.4",
    "7.2.3",
    "9.1.4",
    "9.1.6",
    "hexadecagonal",
    "binaural",
    "downmix",
    "22.2",
];

/// Today's stereo fold of `n` channels: rows L and R.
fn web_audio_fold(n: usize) -> [Vec<f32>; 2] {
    let h = 0.5f32.sqrt();
    let mut m = [vec![0f32; n], vec![0f32; n]];
    match n {
        1 => {
            m[0][0] = 1.0;
            m[1][0] = 1.0;
        }
        4 => {
            (m[0][0], m[0][2], m[1][1], m[1][3]) = (0.5, 0.5, 0.5, 0.5);
        }
        6 => {
            (m[0][0], m[0][2], m[0][4]) = (1.0, h, h);
            (m[1][1], m[1][2], m[1][5]) = (1.0, h, h);
        }
        _ => {
            m[0][0] = 1.0;
            m[1][1] = 1.0;
        }
    }
    m
}

#[test]
fn every_named_layout_folds_into_stereo() {
    let dir = temp_dir("layouts-all");
    let mut summary = vec![];
    for (li, name) in LAYOUTS.iter().enumerate() {
        let file = dir.join(format!("layout-{li}.wav"));
        // Channel c carries a tone at 300 + 100·c Hz (whole cycles in the
        // measured window), at 0.3.
        let probe = std::process::Command::new(ffmpeg())
            .args(["-hide_banner", "-layouts"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&probe.stdout);
        let decomposition = text
            .lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or_else(|| panic!("{name} is not a layout of our FFmpeg"));
        let channels = decomposition.split('+').count();
        let exprs: Vec<String> = (0..channels)
            .map(|c| format!("0.3*sin(2*PI*{}*t)", 300 + 100 * c))
            .collect();
        generate(
            &file,
            &format!("aevalsrc={}:c={name}:s=48000:d=1", exprs.join("|")),
            &["-c:a", "pcm_f32le"],
        );
        let rig = Rig::new(&[("s", &file)]);
        let graph = rig.graph(
            &project(30.0, 1.0, &[audio("A", "s", 0.0, 1.0, "", "")], &[], ""),
            "C",
        );
        let speakers = graph.voices[0].source.info.speakers.clone();
        assert_eq!(speakers.len(), channels, "{name}");
        let named = speakers
            .iter()
            .filter(|s| !matches!(s, Speaker::Other(_)))
            .count();
        let doc = project(30.0, 1.0, &[audio("A", "s", 0.0, 1.0, "", "")], &[], "");
        let mut m = rig.mixer(doc, "C", MixFormat::stereo(SR));
        let out = rig.render(&mut m, 0, SR as i64, 1 << 16);
        let want = web_audio_fold(channels);
        for (c, s) in speakers.iter().enumerate() {
            let f = 300.0 + 100.0 * c as f64;
            let (l, r) = (
                amplitude(&out[0][4800..43200], f),
                amplitude(&out[1][4800..43200], f),
            );
            let (el, er) = (0.3 * want[0][c] as f64, 0.3 * want[1][c] as f64);
            assert!(
                (l - el).abs() < 1e-4 && (r - er).abs() < 1e-4,
                "{name} channel {c} ({s:?}): heard L {l:.4} R {r:.4}, matrix L {el:.4} R {er:.4}"
            );
        }
        summary.push(format!("{name} ({channels} ch, {named} named)"));
    }
    eprintln!("{} layouts: {}", summary.len(), summary.join(", "));
}

/// Ambisonics and unknown channels fold by count, as Web Audio does today.
#[test]
fn channels_without_meaning_fold_by_count() {
    use Speaker::*;
    let o = |n: usize| (0..n).map(|i| Other(1024 + i as i32)).collect::<Vec<_>>();
    let st = layout::stereo();
    assert_eq!(
        layout::matrix(&o(2), &st),
        vec![vec![1.0, 0.0], vec![0.0, 1.0]]
    );
    assert_eq!(
        layout::matrix(&o(4), &st),
        layout::matrix(&[FrontLeft, FrontRight, BackLeft, BackRight], &st)
    );
    assert_eq!(
        layout::matrix(&o(6), &st),
        layout::matrix(
            &[
                FrontLeft,
                FrontRight,
                FrontCenter,
                LowFrequency,
                BackLeft,
                BackRight
            ],
            &st
        )
    );
    // Discrete: three or nine unknown channels keep the first two.
    let m = layout::matrix(&o(9), &st);
    assert_eq!(m[0][0], 1.0);
    assert_eq!(m[1][1], 1.0);
    assert!(m.iter().all(|r| r[2..].iter().all(|g| *g == 0.0)));
    // Inside a named layout, a channel without meaning is a center.
    let m = layout::matrix(&[FrontLeft, FrontRight, Other(77)], &st);
    assert_eq!(m[0][2], layout::FOLD);
    assert_eq!(m[1][2], layout::FOLD);
}
