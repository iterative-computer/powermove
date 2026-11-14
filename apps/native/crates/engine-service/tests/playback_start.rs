//! The first pictures of a looping text-and-audio composition. CoreText
//! answers on the main run loop, as in the app; no native window opens.
use pm_engine_service::{
    FrameSpec, Headless, Output, Scene, clock::TransportClock, policy::Moment,
};
use pm_protocol::{
    clock::{ClockSample, TransportCommand, TransportState},
    common::{HostMicros, PlaybackRate, Seq, TICKS_PER_SECOND, TickRange, Ticks},
};
use std::{sync::Arc, time::Duration};

fn main() {
    let broker = pm_text::host::FontBroker::new();
    broker.set_wake(Arc::new(|| {}));
    let mut fonts = pm_text::host::FontService::new().unwrap();
    let scenario = std::thread::spawn({
        let broker = broker.clone();
        move || {
            first_pictures_keep_the_text(broker.clone());
            static_text_matches_across_stop(broker.clone());
            paused_text_includes_zero_and_excludes_its_out_point(broker);
        }
    });
    while !scenario.is_finished() {
        let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.005);
        objc2_foundation::NSRunLoop::mainRunLoop().runUntilDate(&until);
        pm_text::host::serve(&mut fonts, broker.take_asks());
        std::thread::sleep(Duration::from_millis(1));
    }
    if scenario.join().is_err() {
        std::process::exit(101);
    }
    println!("test first_pictures_keep_the_text ... ok");
}

fn scene() -> Scene {
    let value = pm_model::json::parse(
        r##"{
        "id":"playback-start","name":"Playback start","compId":"C",
        "w":640,"h":360,"fps":30,"dur":3,"bg":"#000000",
        "layers":[
          {"id":"title","name":"Title","type":"text","from":0,"dur":1,"on":true,
           "p":{"position.x":{"v":320,"kf":[],"expr":null},
                "position.y":{"v":180,"kf":[],"expr":null},
                "scale.x":{"v":100,"kf":[],"expr":null},
                "scale.y":{"v":100,"kf":[],"expr":null},
                "opacity":{"v":100,"kf":[],"expr":null}},
           "d":{"text":"The new Powermove","font":"Arial","size":48,"color":"#FFFFFF"},
           "fx":[],"masks":[]},
          {"id":"sound","name":"Sound","type":"audio","from":0,"dur":3,"on":true,
           "p":{},"d":{"asset":"sound","gain":1,"trim":0},"fx":[],"masks":[]}
        ],"assets":{},"comps":{},"params":{}
    }"##,
    )
    .unwrap();
    Scene {
        doc: Arc::new(pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(value)),
            1,
        )),
        comp: "C".into(),
        media: Arc::default(),
        content: 1,
    }
}

fn first_pictures_keep_the_text(broker: pm_text::host::FontBroker) {
    let scene = scene();
    let output = Output {
        width: 640,
        height: 360,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    let mut host = Headless::with_fonts(pm_render::text::host::RenderFonts::new(broker)).unwrap();
    let frame = TICKS_PER_SECOND / 30;
    let range = TickRange {
        start: Ticks::ZERO,
        end: Ticks::from_whole_seconds(3),
    };
    for latency in [20_000_u64, 160_000] {
        let mut clock = TransportClock::new(frame);
        clock.apply(TransportCommand::SetLoop { range: Some(range) }, 0);
        clock.hold(0, 1_000_000);
        clock.follow(&ClockSample {
            seq: Seq::new(1).unwrap(),
            epoch: 7,
            state: TransportState::Playing,
            host_time: HostMicros::new(1_000_000 + latency).unwrap(),
            media_time: Ticks::ZERO,
            rate: PlaybackRate::NORMAL,
            output_latency_us: latency as u32,
            sample_rate: 48_000,
            loop_range: Some(range),
        });
        // At 60 Hz these include every pre-start refresh and the first ten
        // content frames. The title has ended at the erroneous wrapped time.
        for refresh in 0..30 {
            let at = 1_000_000 + refresh * 1_000_000 / 60;
            let time = Ticks::new(clock.snap(clock.picture_at(at))).unwrap();
            let pixels = host
                .frame_spec_settled(
                    &scene,
                    FrameSpec {
                        moment: Moment::Playing,
                        exact: false,
                        ..FrameSpec::exact(time, output)
                    },
                    &Default::default(),
                )
                .unwrap();
            let lit = pixels
                .iter()
                .filter(|p| p[0].max(p[1]).max(p[2]) > 0.1)
                .count();
            assert!(
                lit > 1_000,
                "missing text: latency={latency}, refresh={refresh}, ticks={time:?}, lit={lit}"
            );
        }
    }
}

// Fractional Fit sizes change CoreText hinting when playback and pause
// choose different raster grids. Compare the actual shown RGB pixels,
// including the immediate paused picture, its settled picture and capture.
fn static_text_matches_across_stop(broker: pm_text::host::FontBroker) {
    let scene = scene();
    let mut host = Headless::with_fonts(pm_render::text::host::RenderFonts::new(broker)).unwrap();
    for (width, height) in [(470, 264), (471, 265), (640, 360)] {
        let output = Output {
            width,
            height,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        let scale = FrameSpec::text_grid_scale(&scene, &output, 160 << 20).unwrap();
        let spec = FrameSpec {
            moment: Moment::Playing,
            scale,
            exact: false,
            ..FrameSpec::exact(Ticks::ZERO, output)
        };
        let playing = host
            .frame_spec_settled(&scene, spec, &Default::default())
            .unwrap();
        for (exact, settled) in [(false, false), (false, true), (true, false)] {
            let paused = host
                .frame_spec_settled(
                    &scene,
                    FrameSpec {
                        moment: Moment::Paused,
                        exact,
                        settled,
                        ..spec
                    },
                    &Default::default(),
                )
                .unwrap();
            let error = playing
                .iter()
                .zip(&paused)
                .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
                .fold(0., f64::max);
            assert!(
                error <= 1e-6,
                "{width}x{height}: Stop changed text, exact={exact}, settled={settled}, max RGB error={error}"
            );
        }
        assert!(
            FrameSpec::text_grid_scale(&scene, &output, 1 << 20).is_none(),
            "a native grid must fit its existing memory grant"
        );
    }
    println!("test static_text_matches_across_stop ... ok");
}

// Layer spans are [in, out): a paused picture at exactly zero must draw
// the title, as must its last content frame; the exact out point must not.
fn paused_text_includes_zero_and_excludes_its_out_point(broker: pm_text::host::FontBroker) {
    let scene = scene();
    let mut host = Headless::with_fonts(pm_render::text::host::RenderFonts::new(broker)).unwrap();
    let output = Output {
        width: 470,
        height: 264,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    for (ticks, visible) in [
        (0, true),
        (TICKS_PER_SECOND - TICKS_PER_SECOND / 30, true),
        (TICKS_PER_SECOND, false),
    ] {
        let pixels = host
            .frame_spec_settled(
                &scene,
                FrameSpec {
                    scale: FrameSpec::paused_full_scale(&scene, &output),
                    exact: false,
                    ..FrameSpec::exact(Ticks::new(ticks).unwrap(), output)
                },
                &Default::default(),
            )
            .unwrap();
        let lit = pixels
            .iter()
            .filter(|p| p[0].max(p[1]).max(p[2]) > 0.1)
            .count();
        assert_eq!(lit > 500, visible, "paused ticks={ticks}: lit pixels={lit}");
    }
    println!("test paused_text_includes_zero_and_excludes_its_out_point ... ok");
}
