//! The index an edit updates is the index a fresh read gives, and it shares
//! every layer the edit did not touch.
use std::sync::Arc;

use pm_kernel::commands::properties::SetProperty;
use pm_kernel::commands::{KeyframeCommand, PropertyCommand};
use pm_kernel::*;
use pm_panel_timeline::TimelineIndex;

fn same(a: &TimelineIndex, b: &TimelineIndex) {
    assert_eq!(a.layers.len(), b.layers.len());
    for (x, y) in a.layers.iter().zip(&b.layers) {
        assert_eq!(
            (
                &x.id,
                &x.name,
                x.visible,
                x.locked,
                x.effectively_locked,
                x.group.as_deref()
            ),
            (
                &y.id,
                &y.name,
                y.visible,
                y.locked,
                y.effectively_locked,
                y.group.as_deref()
            )
        );
        assert_eq!(x.channels.len(), y.channels.len());
        for (c, d) in x.channels.iter().zip(&y.channels) {
            assert_eq!(c.path, d.path);
            assert_eq!(c.value, d.value);
            let keys = |ch: &pm_panel_timeline::Channel| {
                ch.keys
                    .iter()
                    .map(|k| (k.id.clone(), k.time))
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(c), keys(d));
            for key in &c.keys {
                let (layer, channel, found) = a.key(&key.id).expect("key resolves");
                assert_eq!(
                    (layer.id.as_str(), channel.path.as_str()),
                    (x.id.as_str(), c.path.as_str())
                );
                assert_eq!(found.time, key.time);
                assert!(b.key(&key.id).is_some());
            }
            assert!(a.channel(&x.id, &c.path).is_some());
        }
    }
}

fn transact(kernel: &KernelHandle, commands: Vec<Command>) {
    let ack = kernel
        .call_blocking(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("edit".into()),
                    body: WriteBody::Commands(commands),
                    expected_revision: None,
                },
            ),
        )
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
}

#[test]
fn an_edit_reads_only_the_layers_it_touched() {
    std::thread::spawn(|| {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        kernel
            .open(
                "p",
                OpenSource::Untitled(pm_kernel::synthetic::project(400)),
            )
            .wait()
            .unwrap()
            .unwrap();
        let mut index = TimelineIndex::from_snapshot(&kernel.published("p").unwrap().snapshot);
        let edits: Vec<Vec<Command>> = vec![
            vec![PropertyCommand::Set(SetProperty::new("L57", "position.x", 5.0, 1.0)).into()],
            vec![
                KeyframeCommand::Add {
                    layer_id: "L130".into(),
                    channel: "rotation".into(),
                    time: 2.0,
                    key_id: "fresh".into(),
                    value: Some(pm_model::Value::Number(45.0)),
                }
                .into(),
            ],
            vec![Command::SetLayerName {
                layer_id: "L3".into(),
                name: "Renamed".into(),
            }],
            vec![Command::SetLayerVisible {
                layer_id: "L99".into(),
                visible: false,
            }],
            vec![Command::SetLayerLocked {
                layer_id: "L0".into(),
                locked: true,
            }],
        ];
        for edit in edits {
            transact(&kernel, edit);
            let snapshot = kernel.published("p").unwrap().snapshot.clone();
            let next = index.updated(&snapshot);
            same(&next, &TimelineIndex::from_snapshot(&snapshot));
            let shared = next
                .layers
                .iter()
                .zip(&index.layers)
                .filter(|(a, b)| Arc::ptr_eq(a, b))
                .count();
            assert!(
                shared >= next.layers.len() - 1,
                "{shared} of {}",
                next.layers.len()
            );
            index = next;
        }
        assert!(index.key("fresh").is_some());
        // Undo restores what a fresh read sees.
        for _ in 0..5 {
            kernel
                .call_blocking("p", Request::new(Caller::native(), HistoryCall::Undo))
                .unwrap();
            let snapshot = kernel.published("p").unwrap().snapshot.clone();
            let next = index.updated(&snapshot);
            same(&next, &TimelineIndex::from_snapshot(&snapshot));
            index = next;
        }
        assert!(index.key("fresh").is_none());
        actor.shutdown();
    })
    .join()
    .unwrap();
}

struct Presenter;
impl pm_panel_timeline::render::ModelPresenter for Presenter {
    fn accessibility_label(&self, row: &pm_panel_timeline::RowId) -> String {
        format!("{row:?}")
    }
    fn layer_label(&self, layer: &pm_panel_timeline::Layer) -> pm_panel_timeline::render::Label {
        pm_panel_timeline::render::Label::for_kind(&layer.kind)
    }
    fn media(&self, _: &pm_panel_timeline::Layer) -> pm_panel_timeline::render::Media {
        pm_panel_timeline::render::Media::Ready
    }
    fn interpolation(
        &self,
        _: &pm_panel_timeline::Key,
    ) -> pm_panel_timeline::render::Interpolation {
        pm_panel_timeline::render::Interpolation::Linear
    }
}

#[test]
fn collapsed_rows_share_their_key_marks_until_a_key_of_theirs_is_selected() {
    use pm_panel_timeline::render::{RowContent, Scene};
    std::thread::spawn(|| {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        kernel
            .open(
                "p",
                OpenSource::Untitled(pm_kernel::synthetic::project(200)),
            )
            .wait()
            .unwrap()
            .unwrap();
        let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
        let mut client =
            pm_panel_timeline::TimelineClient::new(kernel.clone(), "p", Caller::native()).unwrap();
        let summary = |scene: &Scene, layer: &str| {
            let row = scene.row(&pm_panel_timeline::v4::row_id(
                &pm_panel_timeline::RowId::Layer(layer.into()),
            ));
            match &row.expect("layer row").content {
                RowContent::Layer { summary, .. } => summary.clone(),
                _ => panic!("layer row"),
            }
        };
        let before = Scene::from_model(&client.model, &Presenter, &tokens, 0.).unwrap();
        let key = client
            .model
            .index
            .layer("L5")
            .unwrap()
            .channels
            .iter()
            .find(|c| !c.keys.is_empty())
            .unwrap()
            .keys[0]
            .id
            .clone();
        kernel
            .call_blocking(
                "p",
                Request::new(Caller::native(), SelectionCall::Keys(vec![key.clone()])),
            )
            .unwrap();
        client.sync().unwrap();
        let after = Scene::from_model(&client.model, &Presenter, &tokens, 0.).unwrap();
        let marks = summary(&after, "L5");
        assert!(marks.iter().any(|k| k.id == key && k.selected));
        assert_eq!(marks.iter().filter(|k| k.selected).count(), 1);
        assert!(!summary(&before, "L5").iter().any(|k| k.selected));
        assert!(Arc::ptr_eq(&summary(&before, "L6"), &summary(&after, "L6")));
        actor.shutdown();
    })
    .join()
    .unwrap();
}
