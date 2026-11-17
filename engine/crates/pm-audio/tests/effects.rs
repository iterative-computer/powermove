mod common;
use common::*;
use pm_audio::effects::{Chain, Prepared, Registry};
use pm_audio::{MixFormat, Mixer, RenderOptions};
use pm_fx_defs::audio::{Settings, parse};
use serde_json::{Value, json};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;

struct GuardAllocator;
thread_local! {
    static GUARDED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
fn note() {
    let _ = GUARDED.try_with(|g| {
        if g.get() {
            ALLOCATIONS.with(|n| n.set(n.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for GuardAllocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: GuardAllocator = GuardAllocator;
fn settings(nodes: Value) -> Settings {
    parse(json!({"id":"test.effect","label":"Test","group":"Audio","params":[],"graph":nodes}))
        .unwrap()
        .resolve(&BTreeMap::new())
        .unwrap()
}
fn chain(nodes: Value) -> Chain {
    Chain::new(&settings(nodes), 48_000).unwrap()
}
fn filter(mode: &str, frequency: f64, gain: f64) -> Value {
    json!({"type":"biquad","mode":{"value":mode},"frequency":{"value":frequency},"q":{"value":std::f64::consts::FRAC_1_SQRT_2},"gainDb":{"value":gain}})
}
fn magnitude(node: Value, frequency: f64) -> f64 {
    let mut dsp = chain(json!([node]));
    dsp.begin_block(0);
    let mut power = 0.0;
    for i in 0..96_000 {
        let x = (i as f64 * std::f64::consts::TAU * frequency / 48_000.0).sin() as f32;
        let y = dsp.process_frame([x, x])[0];
        if i >= 48_000 {
            power += f64::from(y).powi(2);
        }
    }
    (power / 24_000.0).sqrt()
}
#[test]
fn biquad_magnitude_matches_known_responses() {
    for (mode, frequency, gain, expected, tolerance) in [
        ("lowPass", 100.0, 0.0, 1.0, 0.001),
        (
            "lowPass",
            1000.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
            0.001,
        ),
        ("lowPass", 10000.0, 0.0, 0.00729, 0.001),
        ("highPass", 100.0, 0.0, 0.01, 0.001),
        (
            "highPass",
            1000.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
            0.001,
        ),
        ("highPass", 10000.0, 0.0, 1.0, 0.001),
        ("peaking", 1000.0, 6.0, 10.0f64.powf(6.0 / 20.0), 0.001),
        ("notch", 1000.0, 0.0, 0.0, 0.00001),
        ("lowShelf", 20.0, 6.0, 10.0f64.powf(6.0 / 20.0), 0.001),
        ("highShelf", 20000.0, 6.0, 10.0f64.powf(6.0 / 20.0), 0.001),
    ] {
        let result = magnitude(filter(mode, 1000.0, gain), frequency);
        assert!(
            (result - expected).abs() < tolerance,
            "{mode} at {frequency}: {result} vs {expected}"
        );
    }
}
#[test]
fn gain_is_exact_and_stereo_independent() {
    let mut dsp = chain(json!([{"type":"gain","gainDb":{"value":0}}]));
    assert_eq!(dsp.process_frame([0.25, -0.75]), [0.25, -0.75]);
}
#[test]
fn compressor_reaches_expected_gain_reduction_on_steady_tone() {
    let mut dsp = chain(
        json!([{"type":"compressor","threshold":{"value":-18},"ratio":{"value":4},"attack":{"value":0.0001},"release":{"value":10},"knee":{"value":0},"makeup":{"value":0}}]),
    );
    let mut peak = 0.0f32;
    for i in 0..96_000 {
        let x = (i as f64 * std::f64::consts::TAU * 1000.0 / 48_000.0).sin() as f32 * 0.5;
        let y = dsp.process_frame([x, x])[0];
        if i >= 48_000 {
            peak = peak.max(y.abs());
        }
    }
    let input_db = 20.0 * 0.5f64.log10();
    let expected = 10.0f64.powf((-18.0 + (input_db + 18.0) / 4.0) / 20.0);
    assert!(
        (f64::from(peak) - expected).abs() < 0.003,
        "{peak} vs {expected}"
    );
}
#[test]
fn limiter_never_exceeds_ceiling_in_either_channel() {
    let mut dsp = chain(json!([{"type":"limiter","ceiling":{"value":-6},"release":{"value":0.1}}]));
    let ceiling = 10.0f32.powf(-6.0 / 20.0);
    for i in 0..48_000 {
        let x = (i as f32 * 0.17).sin() * 12.0;
        assert!(
            dsp.process_frame([x, -2.0 * x])
                .iter()
                .all(|y| y.abs() <= ceiling)
        );
    }
}
#[test]
fn delay_impulse_arrives_at_exact_sample_and_feedback_is_bounded() {
    let mut dsp = chain(
        json!([{"type":"delay","time":{"value":0.001},"feedback":{"value":0.5},"mix":{"value":1},"maxTime":0.1}]),
    );
    for i in 0..145 {
        let y = dsp.process_frame(if i == 0 { [1.0, -1.0] } else { [0.0; 2] });
        let expected = match i {
            48 => 1.0,
            96 => 0.5,
            144 => 0.25,
            _ => 0.0,
        };
        assert_eq!(y, [expected, -expected], "{i}");
    }
}
#[test]
fn processing_seeks_and_parameter_smoothing_allocate_nothing() {
    let original = settings(json!([
        {"type":"gain","gainDb":{"value":0}},
        filter("lowPass",1200.0,0.0),
        {"type":"compressor","threshold":{"value":-18},"ratio":{"value":4},"attack":{"value":0.01},"release":{"value":0.1},"knee":{"value":6},"makeup":{"value":0}},
        {"type":"limiter","ceiling":{"value":-1},"release":{"value":0.1}},
        {"type":"delay","time":{"value":0.01},"feedback":{"value":0.2},"mix":{"value":0.2},"maxTime":0.1}
    ]));
    let mut dsp = Chain::new(&original, 48_000).unwrap();
    ALLOCATIONS.with(|n| n.set(0));
    GUARDED.with(|g| g.set(true));
    dsp.retarget(&original).unwrap();
    for block in 0..128 {
        dsp.begin_block(if block == 64 { -1024 } else { block * 256 });
        for _ in 0..256 {
            std::hint::black_box(dsp.process_frame([0.5, -0.5]));
        }
    }
    GUARDED.with(|g| g.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
#[test]
fn seek_bridges_filter_output_and_clears_delay_tails() {
    let mut dsp = chain(json!([filter("lowPass", 1000.0, 0.0)]));
    dsp.begin_block(0);
    let mut previous = 0.0;
    for _ in 0..4800 {
        previous = dsp.process_frame([0.5; 2])[0];
    }
    dsp.begin_block(24_000);
    for _ in 0..480 {
        let next = dsp.process_frame([-0.5; 2])[0];
        assert!(
            (next - previous).abs() < 0.01,
            "seek step {previous} -> {next}"
        );
        previous = next;
    }
    let mut delay = chain(
        json!([{"type":"delay","time":{"value":0.1},"feedback":{"value":0.9},"mix":{"value":1},"maxTime":0.2}]),
    );
    delay.begin_block(0);
    delay.process_frame([1.0; 2]);
    delay.begin_block(48000);
    for _ in 0..24000 {
        assert_eq!(delay.process_frame([0.0; 2]), [0.0; 2]);
    }
}
#[test]
fn nonfinite_inputs_are_sanitised_and_gain_edits_are_smoothed() {
    let mut dsp = chain(json!([{"type":"gain","gainDb":{"value":0}}]));
    assert_eq!(dsp.process_frame([f32::NAN, f32::INFINITY]), [0.0; 2]);
    let target = settings(json!([{"type":"gain","gainDb":{"value":-96}}]));
    dsp.retarget(&target).unwrap();
    let mut previous = 1.0;
    for _ in 0..480 {
        let next = dsp.process_frame([1.0; 2])[0];
        assert!((next - previous).abs() < 0.003);
        previous = next;
    }
    assert!((previous - 10.0f32.powf(-96.0 / 20.0)).abs() < 1e-7);
}
fn rendered_mixer(rig: &Rig, doc: &pm_model::Document, registry: &Registry) -> Mixer {
    let graph = Arc::new(rig.graph(doc, "C"));
    let prepared = Prepared::build(doc, &graph, registry, 48_000).unwrap();
    assert!(prepared.problems.is_empty(), "{:?}", prepared.problems);
    let mut mixer = Mixer::new(
        MixFormat::stereo(48_000),
        Arc::new(doc.clone()),
        graph,
        pm_eval::Evaluator::new(),
        rig.cache.clone(),
    );
    mixer.set_audio_effects(prepared);
    mixer
}
#[test]
fn playback_and_offline_share_bit_exact_effects_after_clip_envelope() {
    let dir = temp_dir("effects");
    let wav = dir.join("tone.wav");
    write_wav(
        &wav,
        48_000,
        &[
            tone(48_000, 1.0, 1000.0, 0.5),
            tone(48_000, 1.0, 700.0, 0.5),
        ],
    );
    let rig = Rig::new(&[("s", &wav)]);
    let doc = project(30.0,1.0,&[r#"{"id":"sound","type":"audio","from":0,"dur":1,"on":true,"d":{"asset":"s","gain":0.5,"trim":0,"fadeIn":0.01,"fadeOut":0.01},"audioFx":[{"id":"f","type":"test.effect","name":"Test","on":true,"p":{}}]}"#.into()],&[],"");
    let def = parse(json!({"id":"test.effect","label":"Test","group":"Audio","params":[],"graph":[filter("lowPass",1200.0,0.0),{"type":"delay","time":{"value":0.01},"feedback":{"value":0.2},"mix":{"value":0.2},"maxTime":0.1}]})).unwrap();
    let registry = Registry::from([("test.effect".into(), def)]);
    let mut export = rendered_mixer(&rig, &doc, &registry);
    let expected = rig.render(&mut export, 0, 48000, 997);
    let mut playback = rendered_mixer(&rig, &doc, &registry);
    let mut actual = [Vec::new(), Vec::new()];
    let mut planar = [Vec::new(), Vec::new()];
    for first in (0..48000).step_by(256) {
        let n = (48000 - first).min(256);
        let report = playback
            .render(first as i64, n, &mut planar, &RenderOptions::default())
            .unwrap();
        assert_eq!(report.starved, 0);
        for (out, block) in actual.iter_mut().zip(&planar) {
            out.extend_from_slice(block);
        }
    }
    assert_eq!(actual[0], expected[0]);
    assert_eq!(actual[1], expected[1]);
    let mut dry = rig.mixer(doc, "C", MixFormat::stereo(48_000));
    assert_ne!(expected, rig.render(&mut dry, 0, 48000, 997));
}
#[test]
fn missing_effect_is_bypassed_and_reported_by_saved_name() {
    let dir = temp_dir("missing-effect");
    let wav = dir.join("tone.wav");
    write_wav(&wav, 48_000, &[tone(48_000, 0.1, 440.0, 0.2)]);
    let rig = Rig::new(&[("s", &wav)]);
    let doc = project(30.0,0.1,&[r#"{"id":"s","type":"audio","from":0,"dur":0.1,"d":{"asset":"s"},"audioFx":[{"id":"f","type":"missing.eq","name":"My EQ","on":true,"p":{}}]}"#.into()],&[],"");
    let graph = rig.graph(&doc, "C");
    let prepared = Prepared::build(&doc, &graph, &Registry::new(), 48_000).unwrap();
    assert_eq!(prepared.problems.len(), 1);
    assert!(prepared.problems[0].2.contains("My EQ"));
    let mut with = rig.mixer(doc.clone(), "C", MixFormat::stereo(48_000));
    with.set_audio_effects(prepared);
    let mut dry = rig.mixer(doc, "C", MixFormat::stereo(48_000));
    assert_eq!(
        rig.render(&mut with, 0, 4800, 256),
        rig.render(&mut dry, 0, 4800, 256)
    );
}

#[test]
fn retry_replays_the_same_output_without_advancing_delay_history() {
    let graph = json!([{"type":"delay","time":{"value":0.001},"feedback":{"value":0.5},"mix":{"value":1},"maxTime":0.1}]);
    let mut dsp = chain(graph.clone());
    let mut reference = chain(graph);
    dsp.begin_block(0);
    reference.begin_block(0);
    let mut first = Vec::new();
    for i in 0..128 {
        let input = if i == 0 { [1.0; 2] } else { [0.0; 2] };
        first.push(dsp.process_frame(input));
        reference.process_frame(input);
    }
    for _ in 0..3 {
        dsp.begin_block(0);
        for expected in &first {
            assert_eq!(dsp.process_frame([0.0; 2]), *expected);
        }
    }
    dsp.begin_block(128);
    reference.begin_block(128);
    for _ in 0..256 {
        assert_eq!(
            dsp.process_frame([0.0; 2]),
            reference.process_frame([0.0; 2])
        );
    }
}

#[test]
fn processor_overflow_bypasses_the_node_without_poisoning_output() {
    let mut dsp = chain(json!([{"type":"gain","gainDb":{"value":24}}]));
    assert_eq!(dsp.process_frame([f32::MAX; 2]), [f32::MAX; 2]);
    assert_eq!(dsp.process_frame([0.25; 2]), [0.25; 2]);
}

#[test]
fn retimed_and_reversed_clips_feed_the_chain_after_source_mapping() {
    let dir = temp_dir("retimed-effects");
    let wav = dir.join("tone.wav");
    write_wav(&wav, 48_000, &[tone(48_000, 1.0, 440.0, 0.3)]);
    let rig = Rig::new(&[("s", &wav)]);
    let definition = parse(json!({"id":"test.effect","label":"Gain","group":"Audio","params":[],"graph":[{"type":"gain","gainDb":{"value":-6}}]})).unwrap();
    let registry = Registry::from([("test.effect".into(), definition)]);
    for speed in [2.0, -1.0] {
        let layer = json!({"id":"sound","type":"audio","from":0,"dur":0.4,"d":{"asset":"s","gain":0.5,"trim":0.5,"speed":speed,"pitch":"follow","fadeIn":0.01,"fadeOut":0.01},"audioFx":[{"id":"f","type":"test.effect","name":"Gain","on":true,"p":{}}]}).to_string();
        let doc = project(30.0, 0.4, &[layer], &[], "");
        let mut wet = rendered_mixer(&rig, &doc, &registry);
        let mut dry = rig.mixer(doc, "C", MixFormat::stereo(48_000));
        let wet = rig.render(&mut wet, 0, 19200, 256);
        let dry = rig.render(&mut dry, 0, 19200, 256);
        assert!(dry[0].iter().any(|v| v.abs() > 0.01));
        let gain = 10.0f32.powf(-6.0 / 20.0);
        for (wet, dry) in wet.iter().flatten().zip(dry.iter().flatten()) {
            assert!(
                (wet - dry * gain).abs() < 1e-6,
                "speed {speed}: {wet} vs {dry}"
            );
        }
    }
}

#[test]
fn rebuilt_compatible_chain_keeps_delay_history_at_handoff() {
    let dir = temp_dir("handoff-effects");
    let wav = dir.join("tone.wav");
    write_wav(&wav, 48_000, &[tone(48_000, 0.1, 440.0, 0.3)]);
    let rig = Rig::new(&[("s", &wav)]);
    let doc = project(30.0,0.1,&[r#"{"id":"sound","type":"audio","from":0,"dur":0.1,"d":{"asset":"s","gain":1},"audioFx":[{"id":"f","type":"test.effect","name":"Delay","on":true,"p":{}}]}"#.into()],&[],"");
    let definition = parse(json!({"id":"test.effect","label":"Delay","group":"Audio","params":[],"graph":[{"type":"delay","time":{"value":0.01},"feedback":{"value":0.5},"mix":{"value":1},"maxTime":0.1}]})).unwrap();
    let registry = Registry::from([("test.effect".into(), definition)]);
    let mut actual = rendered_mixer(&rig, &doc, &registry);
    let mut reference = rendered_mixer(&rig, &doc, &registry);
    assert_eq!(
        rig.render(&mut actual, 0, 1024, 256),
        rig.render(&mut reference, 0, 1024, 256)
    );
    let prepared = Prepared::build(&doc, actual.graph(), &registry, 48_000).unwrap();
    actual.set_audio_effects(prepared);
    assert_eq!(
        rig.render(&mut actual, 1024, 4800, 256),
        rig.render(&mut reference, 1024, 4800, 256)
    );
}

#[test]
fn retimed_delay_tail_continues_while_clip_gain_is_zero() {
    let dir = temp_dir("zero-gain-tail");
    let wav = dir.join("tone.wav");
    write_wav(&wav, 48_000, &[tone(48_000, 1.0, 440.0, 0.3)]);
    let rig = Rig::new(&[("s", &wav)]);
    let definition = parse(json!({"id":"test.effect","label":"Delay","group":"Audio","params":[],"graph":[{"type":"delay","time":{"value":0.02},"feedback":{"value":0.8},"mix":{"value":1},"maxTime":0.1}]})).unwrap();
    let registry = Registry::from([("test.effect".into(), definition)]);
    let layer = json!({"id":"sound","type":"audio","from":0,"dur":0.1,
        "d":{"asset":"s","gain":{"v":1,"kf":[{"t":0,"v":1,"i":"a"},{"t":0.04,"v":0,"i":"b"}],"expr":null},"trim":0.2,"speed":2,"pitch":"follow"},
        "audioFx":[{"id":"f","type":"test.effect","name":"Delay","on":true,"p":{}}]}).to_string();
    let doc = project(30.0, 0.1, &[layer], &[], "");
    let mut wet = rendered_mixer(&rig, &doc, &registry);
    let mut dry = rig.mixer(doc, "C", MixFormat::stereo(48_000));
    let wet = rig.render(&mut wet, 0, 4800, 256);
    let dry = rig.render(&mut dry, 0, 4800, 256);
    assert!(dry[0][3000..3600].iter().all(|v| *v == 0.0));
    assert!(wet[0][3000..3600].iter().any(|v| v.abs() > 0.001));
}
