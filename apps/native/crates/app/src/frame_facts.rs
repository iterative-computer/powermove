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
