use pm_design::Tokens;
use pm_panel_timeline::render::*;
pub fn viewport(t: &Tokens) -> Viewport {
    Viewport {
        width: 960.,
        height: 440.,
        scroll_y: 0.,
        scroll_time: 1.,
        pps: 100.,
        layer_column: t.metrics.layer_column as f64,
    }
}
pub fn clip(id: &str) -> Clip {
    Clip {
        id: id.into(),
        name: format!("{id}.mov"),
        from: 0.125,
        duration: 5.375,
        label: Label::Teal,
        selected: false,
        locked: false,
        hidden: false,
        media: Media::Ready,
        audio: None,
        trim: None,
        missing_effect: None,
        cues: vec![],
    }
}
pub fn key(id: &str, time: f64) -> Key {
    Key {
        id: id.into(),
        seconds: time,
        selected: false,
        focused: false,
        interpolation: Interpolation::Linear,
    }
}
pub fn row(id: &str, content: RowContent) -> PaintRow {
    PaintRow {
        id: id.into(),
        accessibility_label: format!("{id}, video, 5 seconds"),
        depth: 0,
        selected: false,
        hovered: false,
        focused: false,
        content,
    }
}
pub fn scene(t: &Tokens) -> Scene {
    let mut selected = clip("selected");
    selected.selected = true;
    selected.trim = Some(Edge::Out);
    let mut offline = clip("offline");
    offline.media = Media::Offline;
    let mut loading = clip("loading");
    loading.media = Media::Loading;
    let mut locked = clip("locked");
    locked.locked = true;
    locked.trim = Some(Edge::In);
    let mut audio = clip("audio");
    audio.label = Label::Green;
    audio.audio = Some(Some(Waveform {
        peaks: vec![0., 0.8, 0.4, 1., 1., 0.2].into(),
        rate: 1.,
        trim: 0.,
    }));
    let mut keys = vec![key("linear", 1.1), key("ease", 2.25), key("hold", 3.5)];
    keys[1].interpolation = Interpolation::Ease;
    keys[1].selected = true;
    keys[1].focused = true;
    keys[2].interpolation = Interpolation::Hold;
    let mut rows = vec![];
    for c in [selected, offline, loading, locked, audio] {
        let id = c.id.clone();
        let mut r = row(
            &id,
            RowContent::Layer {
                clip: Some(c),
                summary: keys.clone().into(),
            },
        );
        r.selected = id == "selected";
        r.hovered = id == "offline";
        r.focused = id == "selected";
        rows.push(r);
    }
    for (id, state) in [
        ("static", KeyState::Static),
        ("animated", KeyState::Animated),
        ("at-playhead", KeyState::AtPlayhead),
    ] {
        rows.push(row(
            id,
            RowContent::Property {
                state,
                keys: if state == KeyState::Static {
                    vec![]
                } else {
                    keys.clone()
                },
            },
        ));
    }
    let mut scene = Scene::new(rows, 12., 30., t).unwrap();
    scene.work_area = (2., 8.);
    scene.markers = vec![Marker {
        id: "cue".into(),
        seconds: 3.25,
        name: "Cue".into(),
    }];
    scene.bars = [
        BarState::Missing,
        BarState::Rendering,
        BarState::Rendered,
        BarState::Kept,
        BarState::Stale,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, state)| Bar {
        from: i as f64 + 1.,
        to: i as f64 + 2.,
        state,
    })
    .collect();
    scene
}
pub fn large_scene(count: usize, t: &Tokens) -> Scene {
    Scene::new(
        (0..count)
            .map(|i| {
                row(
                    &format!("layer-{i}"),
                    RowContent::Layer {
                        clip: Some(clip(&format!("clip-{i}"))),
                        summary: vec![].into(),
                    },
                )
            })
            .collect(),
        30.,
        30.,
        t,
    )
    .unwrap()
}

/// One layer row holding `clip`.
#[allow(dead_code)]
pub fn scene_with(t: &Tokens, clip: Clip) -> Scene {
    let id = clip.id.clone();
    let r = row(
        &id,
        RowContent::Layer {
            clip: Some(clip),
            summary: vec![].into(),
        },
    );
    Scene::new(vec![r], 12., 30., t).unwrap()
}
