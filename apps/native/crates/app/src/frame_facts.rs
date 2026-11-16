//! What the shell's menu state reads from the project on every frame: the
//! command palette's layers and effects and the status line's offline count.
//! Each is kept until what it is read from changes (the publication, the
//! effect registry, the media observations), so a frame never walks the
//! project: a walk of Paper's 867 layers in every frame made each one take
//! 40 ms (PM-430).
use pm_shell::integration::Names;
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

type Registries = Arc<BTreeMap<String, Vec<pm_kernel::state::Registration>>>;
type Publication = Arc<pm_kernel::Published>;

#[derive(Default)]
pub(crate) struct FrameFacts {
    layers: RefCell<Option<(Publication, Names)>>,
    effects: RefCell<Option<(Registries, Names)>>,
    offline: RefCell<Option<((Publication, u64), usize)>>,
    /// The preview rate measured while playing: since when, the frames
    /// presented then, and the last second's rate.
    preview: RefCell<Option<(std::time::Instant, u64, Option<u32>)>>,
}

impl FrameFacts {
    /// The palette's layers of the open composition, walked once per
    /// publication (held, so another one is never taken for it).
    pub fn palette_layers(&self, published: Option<&Publication>) -> Names {
        let Some(published) = published else {
            return Names::default();
        };
        let mut kept = self.layers.borrow_mut();
        if let Some((at, names)) = kept.as_ref()
            && Arc::ptr_eq(at, published)
        {
            return names.clone();
        }
        let names: Names = crate::layer_commands::palette_layers(published).into();
        *kept = Some((published.clone(), names.clone()));
        names
    }
    /// The palette's effects, read once per change of the effect registry.
    pub fn palette_effects(&self, state: &pm_kernel::state::StateSnapshot) -> Names {
        let mut kept = self.effects.borrow_mut();
        if let Some((registries, names)) = kept.as_ref()
            && Arc::ptr_eq(registries, &state.registries)
        {
            return names.clone();
        }
        let names: Names = crate::layer_commands::palette_effects(state).into();
        *kept = Some((state.registries.clone(), names.clone()));
        names
    }
    /// v1's playback state in the status line: "Playback paused", or while
    /// playing the frames presented in the last second ("Preview 24 fps"),
    /// "Measuring FPS…" until a second has passed. `presented` is the
    /// viewer's count of frames it presented.
    pub fn playback_state(&self, playing: bool, presented: u64, now: std::time::Instant) -> String {
        let mut kept = self.preview.borrow_mut();
        if !playing {
            *kept = None;
            return "Playback paused".into();
        }
        let (since, at, rate) = kept.get_or_insert((now, presented, None));
        let elapsed = now.saturating_duration_since(*since);
        if elapsed >= std::time::Duration::from_secs(1) {
            let frames = presented.saturating_sub(*at) as f64;
            *rate = Some((frames / elapsed.as_secs_f64()).round() as u32);
            (*since, *at) = (now, presented);
        }
        match rate {
            Some(fps) if *fps > 0 => format!("Preview {fps} fps"),
            _ => "Measuring FPS…".into(),
        }
    }
    /// The project's offline files, counted once per publication and media
    /// observation (`observed`, bumped whenever they are replaced).
    pub fn offline(
        &self,
        published: Option<&Publication>,
        observed: u64,
        count: impl FnOnce() -> usize,
    ) -> usize {
        let Some(published) = published else {
            return 0;
        };
        let mut kept = self.offline.borrow_mut();
        if let Some(((at, seen), n)) = kept.as_ref()
            && Arc::ptr_eq(at, published)
            && *seen == observed
        {
            return *n;
        }
        let n = count();
        *kept = Some(((published.clone(), observed), n));
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::model::layer_walks;

    /// A project of `count` layers in the open composition.
    fn large(count: usize) -> serde_json::Value {
        let layers: Vec<_> = (0..count)
            .map(|i| {
                serde_json::json!({"id":format!("l{i}"),"type":"solid","name":format!("Layer {i}"),
                    "from":0,"dur":10,"on":true,"group":if i % 3 == 0 {serde_json::Value::Null} else {serde_json::json!("g")}})
            })
            .chain(std::iter::once(serde_json::json!({"id":"g","type":"group","name":"Group","from":0,"dur":10,"on":true})))
            .collect();
        serde_json::json!({"id":"P","name":"Large","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000",
            "assets":{},"comps":{},"layers":layers})
    }

    /// The shell asks for the palette's lists on every frame: on a large
    /// document, a hundred frames walk its layers once, and once more only
    /// after an edit; the same list comes back as the same `Arc`, so the
    /// shell's comparison is a pointer's.
    /// The status line counts keys as v1 does: every key of the open
    /// composition's layers, an id or not (a path's vertex keys carry none),
    /// and none from a composition nested in it.
    #[test]
    fn the_key_count_takes_keys_without_ids_too() {
        let project = serde_json::json!({"id":"P","name":"Keys","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000",
            "assets":{},
            "comps":{"inner":{"name":"Inner","w":10,"h":10,"fps":30,"dur":1,"layers":[
                {"id":"x","type":"solid","name":"X","from":0,"dur":1,"on":true,
                 "p":{"opacity":{"v":1,"kf":[{"i":"z","t":0,"v":1}]}}}]}},
            "layers":[{"id":"l","type":"shape","name":"Path","from":0,"dur":10,"on":true,
                "p":{"opacity":{"v":100,"kf":[{"i":"a","t":0,"v":0},{"i":"b","t":1,"v":100}]}},
                "d":{"vertices":[{"p":{"x":{"v":1,"kf":[{"t":0,"v":1},{"t":1,"v":2}]}}}]}}]});
        let desk = crate::parity_tests::Desk::new(project);
        assert_eq!(crate::rows::keyframe_count(&desk.published().snapshot), 4);
    }
    /// v1's playback state: paused says so; playing measures the frames
    /// presented over a second.
    #[test]
    fn the_status_line_says_whether_playback_runs_and_how_fast() {
        let facts = FrameFacts::default();
        let start = std::time::Instant::now();
        assert_eq!(facts.playback_state(false, 0, start), "Playback paused");
        assert_eq!(facts.playback_state(true, 100, start), "Measuring FPS…");
        let later = start + std::time::Duration::from_millis(500);
        assert_eq!(facts.playback_state(true, 112, later), "Measuring FPS…");
        let second = start + std::time::Duration::from_secs(1);
        assert_eq!(facts.playback_state(true, 124, second), "Preview 24 fps");
        assert_eq!(facts.playback_state(false, 124, second), "Playback paused");
    }
    #[test]
    fn frames_on_a_large_document_never_walk_its_layers_again() {
        let desk = crate::parity_tests::Desk::new(large(3000));
        let facts = FrameFacts::default();
        let published = desk.published();
        let before = layer_walks();
        let first = facts.palette_layers(Some(&published));
        assert_eq!(first.len(), 3001);
        for _ in 0..100 {
            let again = facts.palette_layers(Some(&published));
            assert!(Arc::ptr_eq(&first, &again));
        }
        assert_eq!(layer_walks() - before, 1, "one walk for a hundred frames");
        let state = desk.kernel().state();
        let effects = facts.palette_effects(&state);
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&effects, &facts.palette_effects(&state)));
        }
        let mut counted = 0;
        for _ in 0..100 {
            facts.offline(Some(&published), 1, || {
                counted += 1;
                0
            });
        }
        assert_eq!(counted, 1);
        // An edit is a new revision: walked again, once.
        desk.call(pm_kernel::Call::Transact {
            label: Some("Rename".into()),
            body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Layer(
                pm_kernel::LayerCommand::Set {
                    layer_id: "l1".into(),
                    patch: pm_kernel::commands::layers::LayerPatch {
                        name: Some("Renamed".into()),
                        ..Default::default()
                    },
                },
            )]),
            expected_revision: None,
        });
        let edited = desk.published();
        assert_ne!(edited.revision(), published.revision());
        let names = facts.palette_layers(Some(&edited));
        assert_eq!(names[1].1, "Renamed");
        facts.palette_layers(Some(&edited));
        assert_eq!(layer_walks() - before, 2);
    }
}
