use pm_kernel::{
    Caller, DocumentStore, HistoryCall, KernelActor, KernelConfig, OpenSource, Reply, Request,
};
use pm_model::{Map, Value, json::parse};
use pm_panel_media::*;
use std::{cell::Cell, collections::HashMap};

const PROJECT: &str = r#"{
"id":"P","name":"Fixture","compId":"C1","compName":"Main","w":640,"h":360,"fps":30,"dur":20,
"assets":{"a":{"id":"a","name":"Alpha.mov","kind":"video","dur":12,"sourcePath":"/synthetic/Alpha.mov","extensionData":{"keep":true}},"b":{"id":"b","name":"Beta.wav","kind":"audio","dur":20},"c":{"id":"c","name":"Still.svg","kind":"image","format":"svg"},"d":{"id":"d","name":"Mesh.glb","kind":"model"},"e":{"id":"e","name":"Nested.mov","kind":"video","folder":"f2"}},
"assetFolders":{"f1":{"id":"f1","name":"Bin 2","parent":null},"f2":{"id":"f2","name":"Inner","parent":"f1"},"f3":{"id":"f3","name":"Bin 10","parent":null}},
"layers":[{"id":"v","type":"video","from":2,"dur":8,"d":{"asset":"a"}},{"id":"child","type":"shape","parent":"v","d":{}}],
"comps":{"C2":{"id":"C2","name":"Second","w":640,"h":360,"fps":30,"dur":10,"layers":[]}}
}"#;

#[derive(Default)]
struct Runtime {
    states: HashMap<String, RuntimeState>,
    calls: Cell<usize>,
}
impl Runtime {
    fn ready() -> Self {
        Self {
            states: ["a", "b", "c", "d", "e"]
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        RuntimeState {
                            availability: Availability::Ready,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            calls: Cell::new(0),
        }
    }
    fn set(&mut self, id: &str, a: Availability) {
        self.states.insert(
            id.into(),
            RuntimeState {
                availability: a,
                ..Default::default()
            },
        );
    }
}
impl MediaRuntime for Runtime {
    fn state(&self, _: &Stamp, id: &str) -> RuntimeState {
        self.calls.set(self.calls.get() + 1);
        self.states.get(id).cloned().unwrap_or_default()
    }
}
fn catalog() -> Catalog {
    Catalog::new(
        "P".into(),
        DocumentStore::new_untitled(parse(PROJECT).unwrap()).snapshot(),
    )
    .unwrap()
}
fn setup() -> (Catalog, MediaPanel, Runtime) {
    let c = catalog();
    let r = Runtime::ready();
    let mut p = MediaPanel::default();
    p.refresh(&c, &r);
    (c, p, r)
}
fn asset(id: &str) -> ItemId {
    ItemId::Asset(id.into())
}
fn pointer(p: &mut MediaPanel, c: &Catalog, r: &Runtime, id: &str, m: Modifiers) {
    p.dispatch(
        c,
        r,
        Input::Pointer {
            id: asset(id),
            phase: PointerPhase::Down,
            button: 0,
            nested_control: false,
            modifiers: m,
        },
    )
    .unwrap();
}
fn key(p: &mut MediaPanel, c: &Catalog, r: &Runtime, id: &str, k: Key, m: Modifiers) -> Outcome {
    p.dispatch(
        c,
        r,
        Input::Key {
            target: KeyTarget::Row(asset(id)),
            key: k,
            modifiers: m,
        },
    )
    .unwrap()
}
fn primary() -> Modifiers {
    Modifiers {
        meta: true,
        ..Default::default()
    }
}
fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}
fn invoke(p: &mut MediaPanel, c: &Catalog, r: &Runtime, command: MenuCommand) -> Outcome {
    p.dispatch(c, r, Input::Invoke(command)).unwrap()
}
fn double(p: &mut MediaPanel, c: &Catalog, r: &Runtime, id: &str) -> Outcome {
    p.dispatch(
        c,
        r,
        Input::Pointer {
            id: asset(id),
            phase: PointerPhase::DoubleClick,
            button: 0,
            nested_control: false,
            modifiers: Modifiers::default(),
        },
    )
    .unwrap()
}
fn actor() -> (KernelActor, pm_kernel::KernelHandle, Catalog) {
    let a = KernelActor::spawn(KernelConfig::default());
    let h = a.handle();
    h.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let c = Catalog::published(&h.published("P").unwrap()).unwrap();
    (a, h, c)
}
fn applied(h: &pm_kernel::KernelHandle, edit: &PreparedEdit) {
    let ack = edit.submit(h, Caller::native()).wait().unwrap().unwrap();
    assert!(
        matches!(ack.reply, Reply::Applied { changed: true, .. }),
        "{:?}",
        ack.reply
    );
}
fn current(h: &pm_kernel::KernelHandle) -> Catalog {
    Catalog::published(&h.published("P").unwrap()).unwrap()
}
fn history(h: &pm_kernel::KernelHandle, call: HistoryCall) {
    h.call("P", Request::new(Caller::native(), call))
        .wait()
        .unwrap()
        .unwrap();
}
fn project(h: &pm_kernel::KernelHandle) -> Value {
    h.published("P")
        .unwrap()
        .snapshot
        .document
        .to_value()
        .unwrap()
}

#[test]
fn catalog_lists_bins_compositions_and_semantic_metadata() {
    let (c, p, r) = setup();
    let page = p.page(&c, &r, 0, 50).unwrap();
    assert_eq!(page.total, 8);
    assert_eq!(page.rows[0].id, ItemId::Folder("f1".into()));
    assert_eq!(page.rows[1].id, ItemId::Folder("f3".into()));
    assert_eq!(c.folders["f1"].items, 2);
    assert_eq!(c.assets["a"].references, 1);
    assert_eq!(c.assets["c"].format.as_deref(), Some("svg"));
    assert!(c.compositions["C1"].open);
    assert!(!c.compositions["C1"].can_nest);
    assert!(c.compositions["C2"].can_nest);
    assert_eq!(
        c.path(Some("f2"))
            .iter()
            .map(|f| f.id.as_str())
            .collect::<Vec<_>>(),
        ["f1", "f2"]
    );
}
#[test]
fn plain_primary_and_control_selection() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    pointer(&mut p, &c, &r, "c", primary());
    assert_eq!(p.selected_assets(), ["a", "c"]);
    pointer(
        &mut p,
        &c,
        &r,
        "a",
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(p.selected_assets(), ["c"]);
    pointer(&mut p, &c, &r, "b", Modifiers::default());
    assert_eq!(p.selected_assets(), ["b"]);
}
#[test]
fn shift_and_primary_shift_ranges_keep_anchor() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "b", Modifiers::default());
    pointer(&mut p, &c, &r, "d", shift());
    assert_eq!(p.selected_assets(), ["b", "c", "d"]);
    pointer(
        &mut p,
        &c,
        &r,
        "a",
        Modifiers {
            meta: true,
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(p.selected_assets(), ["b", "c", "d", "a"]);
}
#[test]
fn batch_pointer_down_preserves_drag_selection_and_click_collapses() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    pointer(&mut p, &c, &r, "b", primary());
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    assert_eq!(p.selected_assets(), ["a", "b"]);
    p.dispatch(
        &c,
        &r,
        Input::Pointer {
            id: asset("a"),
            phase: PointerPhase::Click,
            button: 0,
            nested_control: false,
            modifiers: Modifiers::default(),
        },
    )
    .unwrap();
    assert_eq!(p.selected_assets(), ["a"]);
}
#[test]
fn nested_buttons_and_secondary_press_do_not_select() {
    let (c, mut p, r) = setup();
    for (button, nested) in [(2, false), (0, true)] {
        assert!(
            !p.dispatch(
                &c,
                &r,
                Input::Pointer {
                    id: asset("a"),
                    phase: PointerPhase::Down,
                    button,
                    nested_control: nested,
                    modifiers: Modifiers::default()
                }
            )
            .unwrap()
            .handled
        );
    }
    assert!(p.selected_assets().is_empty());
}
#[test]
fn arrows_wrap_and_home_end_work_with_ranges() {
    let (c, mut p, r) = setup();
    // N2 / media-browser §5: navigation includes folders/compositions in grid order.
    for (k, from, to) in [
        (Key::Left, "b", "a"),
        (Key::Up, "b", "a"),
        (Key::Right, "a", "b"),
        (Key::Down, "a", "b"),
        (Key::End, "b", "d"),
    ] {
        key(&mut p, &c, &r, from, k, Modifiers::default());
        assert_eq!(p.selected_assets(), [to]);
        assert_eq!(p.focused(), Some(&asset(to)));
    }
    key(&mut p, &c, &r, "d", Key::Home, shift());
    assert_eq!(p.focused(), Some(&ItemId::Folder("f1".into())));
    assert!(p.selected_assets().is_empty());
}
#[test]
fn select_all_only_current_bin_assets() {
    let (c, mut p, r) = setup();
    key(&mut p, &c, &r, "a", Key::A, primary());
    assert_eq!(p.selected_assets(), ["a", "b", "c", "d"]);
    p.dispatch(&c, &r, Input::OpenFolder(Some("f2".into())))
        .unwrap();
    key(
        &mut p,
        &c,
        &r,
        "e",
        Key::A,
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(p.selected_assets(), ["e"]);
}
#[test]
fn fields_and_nested_controls_own_local_keys() {
    let (c, mut p, r) = setup();
    for target in [KeyTarget::TextField, KeyTarget::NestedControl] {
        for k in [Key::Enter, Key::Space, Key::Delete, Key::Left] {
            let o = p
                .dispatch(
                    &c,
                    &r,
                    Input::Key {
                        target: target.clone(),
                        key: k,
                        modifiers: Modifiers::default(),
                    },
                )
                .unwrap();
            assert!(o.effects.is_empty());
            assert!(o.edit.is_none());
        }
    }
    assert!(p.selected_assets().is_empty());
}
#[test]
fn enter_adds_double_click_previews_and_space_toggles() {
    let (c, mut p, r) = setup();
    let o = key(&mut p, &c, &r, "a", Key::Enter, Modifiers::default());
    assert_eq!(o.effects, vec![Effect::AddAsset("a".into())]);
    assert!(
        double(&mut p, &c, &r, "a")
            .effects
            .contains(&Effect::ShowPreview("a".into()))
    );
    assert!(
        key(&mut p, &c, &r, "a", Key::Space, Modifiers::default())
            .effects
            .contains(&Effect::TogglePreview("a".into()))
    );
    assert!(!p.preview.playing);
}
#[test]
fn model_never_previews_and_still_has_no_transport() {
    let (c, mut p, r) = setup();
    assert!(
        !double(&mut p, &c, &r, "d")
            .effects
            .iter()
            .any(|e| matches!(e, Effect::ShowPreview(_)))
    );
    double(&mut p, &c, &r, "c");
    assert!(!p.preview.transport);
    assert!(
        p.dispatch(&c, &r, Input::SeekSource(5.0))
            .unwrap()
            .effects
            .is_empty()
    );
}
#[test]
fn loading_and_unknown_are_not_declared_offline() {
    let (c, mut p, mut r) = setup();
    for state in [Availability::Unknown, Availability::Loading] {
        r.set("a", state);
        assert!(
            key(&mut p, &c, &r, "a", Key::Enter, Modifiers::default())
                .effects
                .is_empty()
        );
        assert_eq!(
            p.menu(&c, &r, Some(asset("a"))).unwrap().entries,
            vec![MenuEntry {
                command: MenuCommand::DeleteAssets,
                enabled: true,
                checked: false,
            }]
        );
    }
}
#[test]
fn offline_primary_locates_and_cloud_downloads_once() {
    let (c, mut p, mut r) = setup();
    r.set("a", Availability::Offline);
    assert!(
        double(&mut p, &c, &r, "a")
            .effects
            .contains(&Effect::PickFiles {
                replace: Some("a".into())
            })
    );
    r.set(
        "a",
        Availability::Cloud {
            provider: "fixture".into(),
            download: Download::Stored,
        },
    );
    assert!(
        key(&mut p, &c, &r, "a", Key::Enter, Modifiers::default())
            .effects
            .contains(&Effect::Download("a".into()))
    );
    r.set(
        "a",
        Availability::Cloud {
            provider: "fixture".into(),
            download: Download::Downloading,
        },
    );
    assert!(
        key(&mut p, &c, &r, "a", Key::Enter, Modifiers::default())
            .effects
            .is_empty()
    );
}
#[test]
fn escape_and_outside_press_release_preview_but_source_press_does_not() {
    let (c, mut p, r) = setup();
    double(&mut p, &c, &r, "a");
    p.dispatch(
        &c,
        &r,
        Input::OutsidePress {
            source_monitor: true,
            same_item: None,
            inside_panel: false,
        },
    )
    .unwrap();
    assert_eq!(p.selected_assets(), ["a"]);
    let o = p
        .dispatch(
            &c,
            &r,
            Input::OutsidePress {
                source_monitor: false,
                same_item: None,
                inside_panel: false,
            },
        )
        .unwrap();
    assert!(o.effects.contains(&Effect::ClearPreview));
    assert!(p.preview.asset.is_none());
    double(&mut p, &c, &r, "a");
    key(&mut p, &c, &r, "a", Key::Escape, Modifiers::default());
    assert!(p.selected_assets().is_empty());
}
#[test]
fn selecting_different_asset_folder_or_composition_clears_source() {
    let (c, mut p, r) = setup();
    double(&mut p, &c, &r, "a");
    pointer(&mut p, &c, &r, "b", Modifiers::default());
    assert!(p.preview.asset.is_none());
    double(&mut p, &c, &r, "a");
    p.menu(&c, &r, Some(ItemId::Folder("f1".into()))).unwrap();
    assert!(p.preview.asset.is_none());
    assert!(p.selected_assets().is_empty());
}
#[test]
fn source_scrub_keys_clamp_and_do_not_write_project() {
    let (c, mut p, r) = setup();
    double(&mut p, &c, &r, "a");
    let rev = c.stamp.revision;
    for (k, at) in [
        (Key::Right, 5.0),
        (Key::Right, 10.0),
        (Key::Right, 12.0),
        (Key::Left, 7.0),
        (Key::Home, 0.0),
        (Key::End, 12.0),
    ] {
        let o = p
            .dispatch(
                &c,
                &r,
                Input::Key {
                    target: KeyTarget::Source,
                    key: k,
                    modifiers: Modifiers::default(),
                },
            )
            .unwrap();
        assert_eq!(p.preview.seconds, at);
        assert!(o.edit.is_none());
    }
    assert_eq!(c.stamp.revision, rev);
    assert_eq!(
        p.dispatch(&c, &r, Input::SeekSource(f64::NAN)).unwrap_err(),
        Error::InvalidMedia
    );
}
#[test]
fn late_preview_callbacks_cannot_revive_released_source() {
    let (c, mut p, r) = setup();
    double(&mut p, &c, &r, "a");
    p.dispatch(&c, &r, Input::Clear).unwrap();
    assert_eq!(
        p.dispatch(
            &c,
            &r,
            Input::PreviewFeedback {
                stamp: c.stamp.clone(),
                asset: "a".into(),
                playing: true,
                seconds: 1.0,
                duration: Some(12.0)
            }
        )
        .unwrap_err(),
        Error::Stale
    );
}
#[test]
fn marquee_adds_all_hits_even_unmounted_and_obeys_pointer_owner() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    p.dispatch(
        &c,
        &r,
        Input::BeginMarquee {
            pointer: 7,
            modifiers: shift(),
        },
    )
    .unwrap();
    p.dispatch(
        &c,
        &r,
        Input::MarqueeHits {
            pointer: 8,
            activated: true,
            ids: vec!["c".into()],
        },
    )
    .unwrap();
    assert_eq!(p.selected_assets(), ["a"]);
    p.dispatch(
        &c,
        &r,
        Input::MarqueeHits {
            pointer: 7,
            activated: true,
            ids: vec!["c".into(), "d".into(), "e".into()],
        },
    )
    .unwrap();
    assert_eq!(p.selected_assets(), ["a", "c", "d"]);
    p.dispatch(
        &c,
        &r,
        Input::MarqueeHits {
            pointer: 7,
            activated: false,
            ids: vec![],
        },
    )
    .unwrap();
    assert_eq!(p.selected_assets(), ["a"]);
    p.dispatch(&c, &r, Input::EndMarquee { pointer: None })
        .unwrap();
}
#[test]
fn drag_batch_and_folder_cycle_guard() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    pointer(&mut p, &c, &r, "b", primary());
    p.dispatch(
        &c,
        &r,
        Input::DragStart {
            id: asset("a"),
            nested_control: false,
        },
    )
    .unwrap();
    assert_eq!(p.drag.as_ref().unwrap().assets, ["a", "b"]);
    assert!(p.drag.as_ref().unwrap().copy_to_timeline);
    assert!(p.can_drop(&c, Some("f1"), false));
    p.dispatch(&c, &r, Input::DragEnd).unwrap();
    p.dispatch(
        &c,
        &r,
        Input::DragStart {
            id: ItemId::Folder("f1".into()),
            nested_control: false,
        },
    )
    .unwrap();
    assert!(!p.can_drop(&c, Some("f2"), false));
    assert!(!p.can_drop(&c, Some("f1"), false));
    assert!(p.can_drop(&c, None, false));
}
#[test]
fn composition_drag_copies_and_cannot_move_into_bins() {
    let (c, mut p, r) = setup();
    p.dispatch(
        &c,
        &r,
        Input::DragStart {
            id: ItemId::Composition("C2".into()),
            nested_control: false,
        },
    )
    .unwrap();
    let d = p.drag.as_ref().unwrap();
    assert_eq!(d.kind, "comp");
    assert!(d.copy_to_timeline);
    assert!(!p.can_drop(&c, Some("f1"), false));
}
#[test]
fn unavailable_and_control_drag_decline() {
    let (c, mut p, mut r) = setup();
    r.set("a", Availability::Offline);
    for (id, nested) in [(asset("a"), false), (asset("b"), true)] {
        assert!(
            !p.dispatch(
                &c,
                &r,
                Input::DragStart {
                    id,
                    nested_control: nested
                }
            )
            .unwrap()
            .handled
        );
    }
    assert!(p.drag.is_none());
}
#[test]
fn file_drop_depth_option_and_bin_target() {
    let (c, mut p, r) = setup();
    for _ in 0..2 {
        p.dispatch(&c, &r, Input::FilesEnter).unwrap();
    }
    p.dispatch(&c, &r, Input::FilesLeave).unwrap();
    assert!(p.file_drop_active());
    let o = p
        .dispatch(
            &c,
            &r,
            Input::Drop {
                target: Some("f1".into()),
                files: vec!["/synthetic/sequence".into()],
                modifiers: Modifiers {
                    alt: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    assert_eq!(
        o.effects,
        vec![Effect::Import {
            files: vec!["/synthetic/sequence".into()],
            folder: Some("f1".into()),
            as_folder: true
        }]
    );
    assert!(!p.file_drop_active());
    assert!(o.edit.is_none());
}
#[test]
fn menu_actions_are_semantic_and_disable_illegal_composition_actions() {
    let (c, mut p, r) = setup();
    let m = p.menu(&c, &r, Some(asset("a"))).unwrap();
    for command in [
        MenuCommand::AddAsset("a".into()),
        MenuCommand::NewCompFromAsset("a".into()),
        MenuCommand::Replace("a".into()),
        MenuCommand::Reveal("a".into()),
        MenuCommand::DeleteAssets,
    ] {
        assert!(m.entries.contains(&MenuEntry {
            command,
            enabled: true,
            checked: false,
        }));
    }
    let m = p
        .menu(&c, &r, Some(ItemId::Composition("C1".into())))
        .unwrap();
    assert!(m.entries.contains(&MenuEntry {
        command: MenuCommand::OpenComposition("C1".into()),
        enabled: false,
        checked: false,
    }));
    assert!(m.entries.contains(&MenuEntry {
        command: MenuCommand::NestComposition("C1".into()),
        enabled: false,
        checked: false,
    }));
}
#[test]
fn batch_context_menu_manages_the_selected_videos_renditions_and_deletes() {
    let (c, mut p, mut r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    pointer(&mut p, &c, &r, "b", primary());
    let videos: Vec<String> = ["a", "b"]
        .into_iter()
        .filter(|id| c.assets[*id].kind == "video")
        .map(Into::into)
        .collect();
    let commands: Vec<MenuCommand> = p
        .menu(&c, &r, Some(asset("a")))
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.command)
        .collect();
    assert_eq!(commands.last(), Some(&MenuCommand::DeleteAssets));
    if videos.is_empty() {
        assert_eq!(commands, vec![MenuCommand::DeleteAssets]);
        return;
    }
    // A selection never offers Attach (one file per video).
    assert!(!commands.contains(&MenuCommand::Rendition(RenditionAction::AttachProxy)));
    let o = invoke(
        &mut p,
        &c,
        &r,
        MenuCommand::Rendition(RenditionAction::CreateProxy),
    );
    assert!(o.effects.contains(&Effect::Renditions {
        action: RenditionAction::CreateProxy,
        assets: videos.clone(),
    }));
    // Always Play Original shows checked once every video has it.
    for id in &videos {
        r.states.get_mut(id.as_str()).unwrap().play_original = true;
    }
    let menu = p.menu(&c, &r, Some(asset("a"))).unwrap();
    assert!(menu.entries.contains(&MenuEntry {
        command: MenuCommand::Rendition(RenditionAction::PlayOriginal(false)),
        enabled: true,
        checked: true,
    }));
}
#[test]
fn context_menu_selection_emits_preview_cleanup() {
    let (c, mut p, r) = setup();
    double(&mut p, &c, &r, "a");
    let m = p.menu(&c, &r, Some(ItemId::Folder("f1".into()))).unwrap();
    assert!(m.effects.contains(&Effect::ClearPreview));
}
#[test]
fn folder_keys_rename_commit_cancel_and_blur_intent() {
    let (c, mut p, r) = setup();
    p.dispatch(&c, &r, Input::BeginRename("f1".into())).unwrap();
    let o = p
        .dispatch(
            &c,
            &r,
            Input::CommitRename {
                folder: "f1".into(),
                value: " New bin ".into(),
            },
        )
        .unwrap();
    assert_eq!(
        o.edit,
        Some(Edit::RenameFolder {
            id: "f1".into(),
            name: "New bin".into()
        })
    );
    p.dispatch(&c, &r, Input::BeginRename("f1".into())).unwrap();
    p.dispatch(&c, &r, Input::CancelRename).unwrap();
    assert!(
        p.dispatch(
            &c,
            &r,
            Input::CommitRename {
                folder: "f1".into(),
                value: "ignored".into()
            }
        )
        .unwrap()
        .edit
        .is_none()
    );
}
#[test]
fn folder_navigation_and_invalid_folder_fall_back_to_root() {
    let (c, mut p, r) = setup();
    p.dispatch(
        &c,
        &r,
        Input::Key {
            target: KeyTarget::Row(ItemId::Folder("f1".into())),
            key: Key::Enter,
            modifiers: Modifiers::default(),
        },
    )
    .unwrap();
    assert_eq!(p.folder.as_deref(), Some("f1"));
    assert_eq!(p.page(&c, &r, 0, 100).unwrap().rows.len(), 1);
    p.dispatch(&c, &r, Input::OpenFolder(Some("absent".into())))
        .unwrap();
    assert!(p.folder.is_none());
}
#[test]
fn search_kind_missing_and_proxy_filters_do_not_edit() {
    let (c, mut p, mut r) = setup();
    p.dispatch(
        &c,
        &r,
        Input::Filter(Filter {
            query: "ALPHA".into(),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(p.total(), 1);
    r.set("a", Availability::Offline);
    p.dispatch(
        &c,
        &r,
        Input::Filter(Filter {
            kind: Some("video".into()),
            missing_only: true,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(p.total(), 1);
    r.states.get_mut("a").unwrap().proxy = ProxyStatus::Ready {
        version: "proxy-v1".into(),
    };
    p.dispatch(
        &c,
        &r,
        Input::Filter(Filter {
            proxy: Some(ProxyStatus::Ready {
                version: "proxy-v1".into(),
            }),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(p.total(), 1);
}
#[test]
fn proxy_ready_never_promotes_offline_original() {
    let (c, mut p, mut r) = setup();
    r.states.insert(
        "a".into(),
        RuntimeState {
            availability: Availability::Offline,
            proxy: ProxyStatus::Ready {
                version: "proxy-v1".into(),
            },
            poster: Some("poster-v1".into()),
            progress: None,
            ..Default::default()
        },
    );
    let o = key(&mut p, &c, &r, "a", Key::Enter, Modifiers::default());
    assert!(o.effects.contains(&Effect::PickFiles {
        replace: Some("a".into())
    }));
    let page = p.page(&c, &r, 0, 100).unwrap();
    assert!(
        page.rows
            .iter()
            .any(|row| matches!(&row.data,RowData::Asset(a,s)if a.id=="a"&&s.poster.is_some()))
    );
}
#[test]
fn referenced_delete_requires_confirmation_and_is_cancel_safe() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    let o = key(&mut p, &c, &r, "a", Key::Delete, Modifiers::default());
    assert!(o.edit.is_none());
    assert!(p.delete_pending());
    let token = match o.effects[0] {
        Effect::ConfirmDelete {
            token,
            references: 1,
            ..
        } => token,
        _ => panic!(),
    };
    p.dispatch(
        &c,
        &r,
        Input::ConfirmDelete {
            token,
            approved: false,
        },
    )
    .unwrap();
    assert!(!p.delete_pending());
    assert_eq!(p.selected_assets(), ["a"]);
}
#[test]
fn unreferenced_delete_is_immediate_batch_edit() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "b", Modifiers::default());
    pointer(&mut p, &c, &r, "c", primary());
    let o = key(&mut p, &c, &r, "b", Key::Backspace, Modifiers::default());
    assert_eq!(
        o.edit,
        Some(Edit::DeleteAssets {
            ids: vec!["b".into(), "c".into()],
            confirmed: false
        })
    );
}
#[test]
fn stale_confirmation_does_not_delete_changed_project() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "a", Modifiers::default());
    let o = key(&mut p, &c, &r, "a", Key::Delete, Modifiers::default());
    let token = match o.effects[0] {
        Effect::ConfirmDelete { token, .. } => token,
        _ => panic!(),
    };
    let mut newer = c.clone();
    newer.stamp.revision += 1;
    p.refresh(&newer, &r);
    assert_eq!(
        p.dispatch(
            &newer,
            &r,
            Input::ConfirmDelete {
                token,
                approved: true
            }
        )
        .unwrap_err(),
        Error::Stale
    );
}
#[test]
fn project_document_switch_resets_all_transient_ownership() {
    let (c, mut p, r) = setup();
    double(&mut p, &c, &r, "a");
    p.dispatch(&c, &r, Input::BeginRename("f1".into())).unwrap();
    let other = catalog();
    assert_ne!(c.stamp.document, other.stamp.document);
    assert!(p.refresh(&other, &r).contains(&Effect::ClearPreview));
    assert!(p.selected_assets().is_empty());
    assert!(p.renaming.is_none());
    assert!(p.drag.is_none());
}
#[test]
fn create_rename_move_and_ungroup_folder_round_trip_through_real_kernel() {
    let (_a, h, c) = actor();
    applied(
        &h,
        &c.prepare(Edit::CreateFolder {
            id: "new".into(),
            name: "bin 2".into(),
            parent: None,
        })
        .unwrap(),
    );
    assert_eq!(current(&h).folders["new"].name, "bin 2 2");
    let c = current(&h);
    applied(
        &h,
        &c.prepare(Edit::RenameFolder {
            id: "new".into(),
            name: "Renamed".into(),
        })
        .unwrap(),
    );
    let c = current(&h);
    applied(
        &h,
        &c.prepare(Edit::Move {
            assets: vec!["a".into(), "b".into()],
            folders: vec!["f2".into()],
            target: Some("new".into()),
        })
        .unwrap(),
    );
    let c = current(&h);
    assert_eq!(c.assets["a"].folder.as_deref(), Some("new"));
    applied(
        &h,
        &c.prepare(Edit::DeleteFolder { id: "new".into() }).unwrap(),
    );
    let c = current(&h);
    assert!(c.assets["a"].folder.is_none());
    assert!(c.folders["f2"].parent.is_none());
    history(&h, HistoryCall::Undo);
    let c = current(&h);
    assert_eq!(c.assets["a"].folder.as_deref(), Some("new"));
    assert!(c.folders.contains_key("new"));
    history(&h, HistoryCall::Redo);
    assert!(!current(&h).folders.contains_key("new"));
}
#[test]
fn bad_batch_move_and_cycle_apply_no_partial_work() {
    let c = catalog();
    assert_eq!(
        c.prepare(Edit::Move {
            assets: vec!["a".into()],
            folders: vec!["f1".into()],
            target: Some("f2".into())
        })
        .unwrap_err(),
        Error::FolderCycle
    );
    assert_eq!(
        c.prepare(Edit::Move {
            assets: vec!["a".into(), "absent".into()],
            folders: vec![],
            target: Some("f1".into())
        })
        .unwrap_err(),
        Error::MissingItem
    );
    assert!(c.assets["a"].folder.is_none());
}
#[test]
fn referenced_delete_retains_metadata_and_layers_for_one_undo() {
    let (_a, h, c) = actor();
    assert_eq!(
        c.prepare(Edit::DeleteAssets {
            ids: vec!["a".into()],
            confirmed: false
        })
        .unwrap_err(),
        Error::ConfirmationRequired
    );
    let before = project(&h);
    applied(
        &h,
        &c.prepare(Edit::DeleteAssets {
            ids: vec!["a".into(), "b".into()],
            confirmed: true,
        })
        .unwrap(),
    );
    let after = project(&h);
    let layers = after
        .get("comps")
        .unwrap()
        .get("C1")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].get("parent"), Some(&Value::Null));
    assert!(!current(&h).assets.contains_key("a"));
    assert_eq!(h.published("P").unwrap().history.entries, 1);
    history(&h, HistoryCall::Undo);
    assert_eq!(project(&h), before);
    history(&h, HistoryCall::Redo);
    assert_eq!(project(&h), after);
}
#[test]
fn stale_revision_and_reopened_document_are_rejected_atomically() {
    let (_a, h, c) = actor();
    let edit = c
        .prepare(Edit::RenameFolder {
            id: "f1".into(),
            name: "Stale".into(),
        })
        .unwrap();
    applied(
        &h,
        &c.prepare(Edit::RenameFolder {
            id: "f3".into(),
            name: "Other".into(),
        })
        .unwrap(),
    );
    assert!(matches!(
        edit.submit(&h, Caller::native())
            .wait()
            .unwrap()
            .unwrap()
            .reply,
        Reply::Throw(_)
    ));
    h.close("P").wait();
    h.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        edit.submit(&h, Caller::native())
            .wait()
            .unwrap()
            .unwrap()
            .reply,
        Reply::Throw(_)
    ));
    assert_eq!(current(&h).folders["f1"].name, "Bin 2");
}
#[test]
fn open_history_entry_refuses_restore_and_preserves_other_work() {
    let (_a, h, c) = actor();
    history(
        &h,
        HistoryCall::Begin {
            label: "Other gesture".into(),
        },
    );
    let edit = c
        .prepare(Edit::RenameFolder {
            id: "f1".into(),
            name: "New".into(),
        })
        .unwrap();
    assert!(matches!(
        edit.submit(&h, Caller::native())
            .wait()
            .unwrap()
            .unwrap()
            .reply,
        Reply::Throw(_)
    ));
    assert_eq!(current(&h).folders["f1"].name, "Bin 2");
    history(&h, HistoryCall::Commit { label: None });
}
#[test]
fn composition_duplicate_delete_and_last_composition_guard() {
    let (_a, h, c) = actor();
    applied(
        &h,
        &c.prepare(Edit::DuplicateComposition {
            id: "C2".into(),
            new_id: "C3".into(),
        })
        .unwrap(),
    );
    assert_eq!(current(&h).compositions.len(), 3);
    let c = current(&h);
    applied(
        &h,
        &c.prepare(Edit::DeleteComposition { id: "C3".into() })
            .unwrap(),
    );
    let c = current(&h);
    applied(
        &h,
        &c.prepare(Edit::DeleteComposition { id: "C2".into() })
            .unwrap(),
    );
    assert!(
        current(&h)
            .prepare(Edit::DeleteComposition { id: "C1".into() })
            .is_err()
    );
    history(&h, HistoryCall::Undo);
    assert!(current(&h).compositions.contains_key("C2"));
}
#[test]
fn import_is_batch_atomic_and_undoable() {
    let (_a, h, c) = actor();
    let metadata = parse(r#"{"name":"Imported.mov","kind":"video","dur":8}"#).unwrap();
    let edit = c
        .prepare_import(
            &c.stamp,
            vec![StoredAsset {
                id: "imported".into(),
                metadata,
                original_version: "fixture-original-v1".into(),
            }],
            Some("f1".into()),
            None,
        )
        .unwrap();
    applied(&h, &edit);
    assert_eq!(current(&h).assets["imported"].folder.as_deref(), Some("f1"));
    history(&h, HistoryCall::Undo);
    assert!(!current(&h).assets.contains_key("imported"));
    history(&h, HistoryCall::Redo);
    assert!(current(&h).assets.contains_key("imported"));
}
#[test]
fn relink_keeps_id_unknown_data_and_authored_layer_timing() {
    let (_a, h, c) = actor();
    let before = project(&h);
    let metadata=parse(r#"{"name":"Replacement.mov","kind":"video","sourcePath":"/synthetic/replacement.mov","dur":30}"#).unwrap();
    applied(
        &h,
        &c.prepare_import(
            &c.stamp,
            vec![StoredAsset {
                id: "ignored".into(),
                metadata,
                original_version: "fixture-original-v2".into(),
            }],
            None,
            Some("a".into()),
        )
        .unwrap(),
    );
    let after = project(&h);
    assert_eq!(before.get("comps"), after.get("comps"));
    assert_eq!(
        before
            .get("assets")
            .unwrap()
            .get("a")
            .unwrap()
            .get("extensionData"),
        after
            .get("assets")
            .unwrap()
            .get("a")
            .unwrap()
            .get("extensionData")
    );
    history(&h, HistoryCall::Undo);
    assert_eq!(before, project(&h));
}
#[test]
fn import_rejects_stale_owner_missing_version_and_duplicate_ids() {
    let c = catalog();
    let mut owner = c.stamp.clone();
    owner.document += 1;
    assert_eq!(
        c.prepare_import(&owner, vec![], None, None).unwrap_err(),
        Error::Stale
    );
    let item = StoredAsset {
        id: "new".into(),
        metadata: parse(r#"{"name":"x","kind":"image"}"#).unwrap(),
        original_version: "".into(),
    };
    assert_eq!(
        c.prepare_import(&c.stamp, vec![item.clone()], None, None)
            .unwrap_err(),
        Error::InvalidMedia
    );
    let item = StoredAsset {
        original_version: "fixture-original".into(),
        ..item
    };
    assert_eq!(
        c.prepare_import(&c.stamp, vec![item.clone(), item], None, None)
            .unwrap_err(),
        Error::InvalidMedia
    );
}
#[test]
fn corrupt_bin_cycles_and_orphans_remain_accessible() {
    let v=parse(r#"{"id":"P","assets":{"a":{"name":"orphan","kind":"image","folder":"gone"}},"assetFolders":{"a":{"name":"A","parent":"b"},"b":{"name":"B","parent":"a"},"tail":{"name":"T","parent":"b"}}}"#).unwrap();
    let c = Catalog::new("P".into(), DocumentStore::new_untitled(v).snapshot()).unwrap();
    assert_eq!(c.folders.len(), 3);
    assert!(c.folders.values().all(|f| f.parent.is_none()));
    assert!(c.assets["a"].folder.is_none());
    assert!(c.within(Some("a"), "b"));
    assert_eq!(
        c.prepare(Edit::Move {
            assets: vec![],
            folders: vec!["a".into()],
            target: Some("b".into())
        })
        .unwrap_err(),
        Error::FolderCycle
    );
}
#[test]
fn bounded_virtual_pages_and_id_navigation_over_sixty_thousand_rows() {
    let mut p = parse(PROJECT).unwrap();
    let mut assets = Map::new();
    for i in 0..60_001 {
        let id = format!("a{i:05}");
        let mut a = Map::new();
        a.insert("id", Value::from(id.as_str()));
        a.insert("name", Value::from(id.as_str()));
        a.insert("kind", Value::from("image"));
        assets.insert(id, Value::Object(a));
    }
    p.as_object_mut()
        .unwrap()
        .insert("assets", Value::Object(assets));
    let c = Catalog::new("P".into(), DocumentStore::new_untitled(p).snapshot()).unwrap();
    let r = Runtime::default();
    let mut p = MediaPanel::default();
    p.refresh(&c, &r);
    r.calls.set(0);
    let page = p.page(&c, &r, 59_990, 12).unwrap();
    assert_eq!(page.rows.len(), 12);
    assert_eq!(r.calls.get(), 12);
    assert_eq!(page.total, 60_005);
    assert_eq!(page.next, Some(60_002));
    key(&mut p, &c, &r, "a60000", Key::Right, Modifiers::default());
    assert_eq!(p.focused(), Some(&ItemId::Folder("f1".into())));
    assert!(p.page(&c, &r, usize::MAX, 50).unwrap().rows.is_empty());
    assert!(p.page(&c, &r, 0, 0).unwrap().next.is_none());
}

#[test]
fn every_project_and_asset_context_action_dispatches_without_ui() {
    let (c, mut p, r) = setup();
    for (command, effect) in [
        (
            MenuCommand::ImportFiles,
            Effect::PickFiles { replace: None },
        ),
        (
            MenuCommand::ImportFolder(Some("f1".into())),
            Effect::PickFolder {
                folder: Some("f1".into()),
            },
        ),
        (MenuCommand::NewComposition, Effect::NewComposition),
        (
            MenuCommand::Reveal("a".into()),
            Effect::Reveal {
                id: "a".into(),
                path: "/synthetic/Alpha.mov".into(),
            },
        ),
        (
            MenuCommand::Replace("a".into()),
            Effect::PickFiles {
                replace: Some("a".into()),
            },
        ),
        (
            MenuCommand::NewCompFromAsset("a".into()),
            Effect::NewCompFromAsset("a".into()),
        ),
        (
            MenuCommand::AddAsset("a".into()),
            Effect::AddAsset("a".into()),
        ),
        (
            MenuCommand::OpenComposition("C2".into()),
            Effect::OpenComposition("C2".into()),
        ),
        (
            MenuCommand::NestComposition("C2".into()),
            Effect::NestComposition("C2".into()),
        ),
        (
            MenuCommand::CompositionSettings("C2".into()),
            Effect::CompositionSettings("C2".into()),
        ),
    ] {
        assert!(invoke(&mut p, &c, &r, command).effects.contains(&effect));
    }
    assert!(matches!(
        invoke(
            &mut p,
            &c,
            &r,
            MenuCommand::DuplicateComposition("C2".into())
        )
        .edit,
        Some(Edit::DuplicateComposition { .. })
    ));
    assert_eq!(
        invoke(&mut p, &c, &r, MenuCommand::DeleteComposition("C2".into())).edit,
        Some(Edit::DeleteComposition { id: "C2".into() })
    );
    pointer(&mut p, &c, &r, "b", Modifiers::default());
    assert!(
        invoke(&mut p, &c, &r, MenuCommand::DeleteAssets)
            .edit
            .is_some()
    );
}

#[test]
fn folder_menu_actions_and_move_out_keep_parent_semantics() {
    let (c, mut p, r) = setup();
    let entries = p
        .menu(&c, &r, Some(ItemId::Folder("f1".into())))
        .unwrap()
        .entries;
    for command in [
        MenuCommand::OpenFolder("f1".into()),
        MenuCommand::RenameFolder("f1".into()),
        MenuCommand::NewFolder(Some("f1".into())),
        MenuCommand::ImportFolder(Some("f1".into())),
        MenuCommand::DeleteFolder("f1".into()),
    ] {
        assert!(entries.contains(&MenuEntry {
            command,
            enabled: true,
            checked: false,
        }));
    }
    invoke(&mut p, &c, &r, MenuCommand::OpenFolder("f2".into()));
    assert_eq!(
        invoke(&mut p, &c, &r, MenuCommand::MoveOut(asset("e"))).edit,
        Some(Edit::Move {
            assets: vec!["e".into()],
            folders: vec![],
            target: Some("f1".into())
        })
    );
    assert!(matches!(
        invoke(&mut p, &c, &r, MenuCommand::NewFolder(Some("f2".into()))).edit,
        Some(Edit::CreateFolder {
            parent: Some(_),
            ..
        })
    ));
    invoke(&mut p, &c, &r, MenuCommand::RenameFolder("f2".into()));
    assert_eq!(p.renaming.as_deref(), Some("f2"));
}

#[test]
fn acknowledged_new_folder_starts_rename_and_failure_does_not() {
    let (_actor, h, c) = actor();
    let r = Runtime::ready();
    let mut p = MediaPanel::default();
    p.refresh(&c, &r);
    let edit = c
        .prepare(Edit::CreateFolder {
            id: "new".into(),
            name: "Untitled Folder".into(),
            parent: Some("f1".into()),
        })
        .unwrap();
    let ack = edit.submit(&h, Caller::native()).wait().unwrap().unwrap();
    let next = current(&h);
    p.accept_edit(&edit, &ack, &next, &r).unwrap();
    assert_eq!(p.folder.as_deref(), Some("f1"));
    assert_eq!(p.renaming.as_deref(), Some("new"));
    assert_eq!(p.selected_item(), Some(&ItemId::Folder("new".into())));
    let mut failed = ack;
    failed.reply = Reply::Throw(pm_kernel::KernelError::Guard("synthetic refusal".into()));
    assert!(p.accept_edit(&edit, &failed, &next, &r).is_err());
}

#[test]
fn menu_cannot_nest_self_or_make_comp_from_audio_or_model() {
    let (c, mut p, r) = setup();
    for command in [
        MenuCommand::NestComposition("C1".into()),
        MenuCommand::NewCompFromAsset("b".into()),
        MenuCommand::NewCompFromAsset("d".into()),
    ] {
        assert_eq!(
            p.dispatch(&c, &r, Input::Invoke(command)).unwrap_err(),
            Error::InvalidTarget
        );
    }
}

#[test]
fn marquee_autoscroll_requires_active_owned_gesture_and_stops_on_blur() {
    let (c, mut p, r) = setup();
    p.dispatch(
        &c,
        &r,
        Input::BeginMarquee {
            pointer: 1,
            modifiers: Modifiers::default(),
        },
    )
    .unwrap();
    for (pointer, activated, expected) in [(2, true, false), (1, false, false), (1, true, true)] {
        let o = p
            .dispatch(
                &c,
                &r,
                Input::MarqueeEdge {
                    pointer,
                    activated,
                    direction: ScrollDirection::After,
                },
            )
            .unwrap();
        assert_eq!(
            o.effects
                .contains(&Effect::ScrollMarquee(ScrollDirection::After)),
            expected
        );
    }
    p.dispatch(&c, &r, Input::EndMarquee { pointer: None })
        .unwrap();
    assert!(
        p.dispatch(
            &c,
            &r,
            Input::MarqueeEdge {
                pointer: 1,
                activated: true,
                direction: ScrollDirection::Before
            }
        )
        .unwrap()
        .effects
        .is_empty()
    );
}

#[test]
fn host_double_receives_captured_ownership_with_every_effect() {
    #[derive(Default)]
    struct Host(Vec<HostRequest>);
    impl MediaHost for Host {
        fn submit(&mut self, request: HostRequest) {
            self.0.push(request);
        }
    }
    let (c, mut p, r) = setup();
    let outcome = double(&mut p, &c, &r, "a");
    let mut host = Host::default();
    outcome.submit_effects(&mut host);
    assert!(!host.0.is_empty());
    assert!(host.0.iter().all(|request| request.owner == c.stamp));
    assert!(
        host.0
            .iter()
            .any(|request| request.effect == Effect::ShowPreview("a".into()))
    );
}

#[test]
fn thumbnail_time_uses_authored_sample_and_waveforms_use_measured_means() {
    let c = catalog();
    assert_eq!(poster_time(&c.assets["a"]), Some(0.5));
    assert_eq!(poster_time(&c.assets["b"]), None);
    let mut short = c.assets["a"].clone();
    short.duration = Some(2.0);
    assert_eq!(poster_time(&short), Some(0.2));
    assert_eq!(waveform_bins(None, 4).unwrap(), None);
    assert_eq!(waveform_bins(Some(&[]), 4).unwrap(), None);
    assert_eq!(
        waveform_bins(Some(&[0.0, 1.0, 0.2, 0.6]), 2).unwrap(),
        Some(vec![0.5, 0.4])
    );
    assert_eq!(waveform_bins(Some(&[0.2]), 3).unwrap(), Some(vec![0.2; 3]));
    assert_eq!(
        waveform_bins(Some(&[f64::NAN]), 1).unwrap_err(),
        Error::InvalidMedia
    );
}

#[test]
fn structured_extension_sources_scene_descendants_and_maps_delete_and_undo() {
    let mut root = parse(PROJECT).unwrap();
    let layers=parse(r#"[
       {"id":"ext","type":"extension","d":{"data":{"object":{"source":{"assetId":"a"}}}}},
       {"id":"scene","type":"extension","d":{"data":{"scene":{"objects":[
          {"id":"source","source":{"assetId":"a"}}, {"id":"child","parent":"source"},
          {"id":"survivor","material":{"maps":{"albedo":"a","normal":"c"}}}
       ]}}}},
       {"id":"mat","type":"extension","d":{"data":{"object":{"material":{"maps":{"albedo":"a","normal":"c"}}}}}}
    ]"#).unwrap();
    root.as_object_mut().unwrap().insert("layers", layers);
    let actor = KernelActor::spawn(KernelConfig::default());
    let h = actor.handle();
    h.open("P", OpenSource::Untitled(root))
        .wait()
        .unwrap()
        .unwrap();
    let c = current(&h);
    assert_eq!(c.assets["a"].references, 4);
    let before = project(&h);
    applied(
        &h,
        &c.prepare(Edit::DeleteAssets {
            ids: vec!["a".into()],
            confirmed: true,
        })
        .unwrap(),
    );
    let next = project(&h);
    let layers = next
        .get("comps")
        .unwrap()
        .get("C1")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(layers.len(), 2);
    let scene = layers[0]
        .get("d")
        .unwrap()
        .get("data")
        .unwrap()
        .get("scene")
        .unwrap()
        .get("objects")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(scene.len(), 1);
    assert_eq!(
        scene[0]
            .get("material")
            .unwrap()
            .get("maps")
            .unwrap()
            .get("albedo"),
        None
    );
    history(&h, HistoryCall::Undo);
    assert_eq!(project(&h), before);
}

#[test]
fn unknown_utf16_metadata_survives_bin_edit_and_history() {
    let mut root = parse(PROJECT).unwrap();
    root.as_object_mut()
        .unwrap()
        .insert("opaqueExtension", parse(r#"{"opaque":"\ud800"}"#).unwrap());
    let actor = KernelActor::spawn(KernelConfig::default());
    let h = actor.handle();
    h.open("P", OpenSource::Untitled(root))
        .wait()
        .unwrap()
        .unwrap();
    let before = project(&h);
    let c = current(&h);
    applied(
        &h,
        &c.prepare(Edit::RenameFolder {
            id: "f1".into(),
            name: "Renamed".into(),
        })
        .unwrap(),
    );
    assert_eq!(
        project(&h).get("opaqueExtension"),
        before.get("opaqueExtension")
    );
    history(&h, HistoryCall::Undo);
    assert_eq!(project(&h), before);
}

#[test]
fn relink_cannot_move_media_to_import_worker_folder() {
    let (_actor, h, c) = actor();
    let edit = c
        .prepare_import(
            &c.stamp,
            vec![StoredAsset {
                id: "e".into(),
                metadata: parse(r#"{"name":"Relink","kind":"video","folder":"f3"}"#).unwrap(),
                original_version: "fixture-relink-v2".into(),
            }],
            None,
            Some("e".into()),
        )
        .unwrap();
    applied(&h, &edit);
    assert_eq!(current(&h).assets["e"].folder.as_deref(), Some("f2"));
}

#[test]
fn agents_can_roundtrip_inputs_pages_and_outcomes_with_escaped_ids() {
    let input = Input::Pointer {
        id: asset("unicode-🧭/\""),
        phase: PointerPhase::Down,
        button: 0,
        nested_control: false,
        modifiers: Modifiers {
            control: true,
            shift: true,
            ..Default::default()
        },
    };
    let bytes = serde_json::to_vec(&input).unwrap();
    let copy: Input = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(copy, input);
    let (c, mut p, r) = setup();
    let outcome = double(&mut p, &c, &r, "a");
    let bytes = serde_json::to_vec(&outcome).unwrap();
    let copy: Outcome = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(copy.owner, Some(c.stamp.clone()));
    assert_eq!(copy.effects, outcome.effects);
    let page = p.page(&c, &r, 0, 10).unwrap();
    let bytes = serde_json::to_vec(&page).unwrap();
    let copy: Page = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(copy, page);
}

#[test]
fn empty_external_drop_cleans_overlay_without_importing_or_moving() {
    let (c, mut p, r) = setup();
    p.dispatch(&c, &r, Input::FilesEnter).unwrap();
    let o = p
        .dispatch(
            &c,
            &r,
            Input::Drop {
                target: None,
                files: vec![],
                modifiers: Modifiers::default(),
            },
        )
        .unwrap();
    assert!(o.effects.is_empty());
    assert!(o.edit.is_none());
    assert!(!p.file_drop_active());
}

#[test]
fn locale_collation_can_be_supplied_without_changing_document() {
    let snapshot = DocumentStore::new_untitled(parse(PROJECT).unwrap()).snapshot();
    let before = snapshot.document.to_value();
    let c = Catalog::with_collation("P".into(), snapshot.clone(), |a, b| b.cmp(a)).unwrap();
    let mut p = MediaPanel::default();
    let r = Runtime::default();
    p.refresh(&c, &r);
    assert_eq!(
        p.page(&c, &r, 0, 1).unwrap().rows[0].id,
        ItemId::Folder("f1".into())
    );
    assert_eq!(snapshot.document.to_value(), before);
}

#[test]
fn precomp_dependency_cycles_disable_nesting_and_usage_counts_all_instances() {
    let v=parse(r#"{"id":"P","compId":"open","compName":"Zulu","layers":[],"comps":{
       "middle":{"id":"middle","name":"Bin 10","layers":[{"id":"l1","type":"precomp","d":{"comp":"open"}},{"id":"l2","type":"precomp","d":{"comp":"open"}}]},
       "outer":{"id":"outer","name":"Bin 2","layers":[{"id":"l3","type":"precomp","d":{"comp":"middle"}}]},
       "free":{"id":"free","name":"Alpha","layers":[]}
    }}"#).unwrap();
    let c = Catalog::new("P".into(), DocumentStore::new_untitled(v).snapshot()).unwrap();
    assert_eq!(c.compositions["open"].uses, 2);
    assert_eq!(c.compositions["middle"].uses, 1);
    assert!(!c.compositions["outer"].can_nest);
    assert!(!c.compositions["middle"].can_nest);
    assert!(c.compositions["free"].can_nest);
    let r = Runtime::default();
    let mut p = MediaPanel::default();
    p.refresh(&c, &r);
    let page = p.page(&c, &r, 0, 10).unwrap();
    assert_eq!(
        page.rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>(),
        [
            ItemId::Composition("free".into()),
            ItemId::Composition("outer".into()),
            ItemId::Composition("middle".into()),
            ItemId::Composition("open".into())
        ]
    );
}

#[test]
fn equal_object_and_scene_source_values_are_distinct_references() {
    let v=parse(r#"{"id":"P","compId":"C","assets":{"a":{"name":"A","kind":"model"}},"layers":[{"id":"l","type":"extension","d":{"data":{"object":{"source":{"assetId":"a"}},"scene":{"objects":[{"source":{"assetId":"a"}}]}}}}],"comps":{}}"#).unwrap();
    let c = Catalog::new("P".into(), DocumentStore::new_untitled(v).snapshot()).unwrap();
    assert_eq!(c.assets["a"].references, 2);
}

#[test]
fn nested_delete_button_targets_unselected_asset_and_reveal_button_selects_it() {
    let (c, mut p, r) = setup();
    pointer(&mut p, &c, &r, "b", Modifiers::default());
    pointer(&mut p, &c, &r, "c", primary());
    let o = p.dispatch(&c, &r, Input::DeleteAsset("a".into())).unwrap();
    assert!(
        matches!(&o.effects[0],Effect::ConfirmDelete{assets,..}if assets==&vec!["a".to_string()])
    );
    assert_eq!(p.selected_assets(), ["b", "c"]);
    invoke(&mut p, &c, &r, MenuCommand::Reveal("a".into()));
    assert_eq!(p.selected_assets(), ["a"]);
}

#[test]
fn authored_order_drives_paging_keyboard_ranges_and_drag_payloads() {
    let mut root = parse(PROJECT).unwrap();
    root.as_object_mut().unwrap().insert(
        pm_kernel::asset_order::FIELD,
        parse(r#"{"c":0,"b":1,"a":2,"d":3,"e":4}"#).unwrap(),
    );
    let snapshot = DocumentStore::new_untitled(root).snapshot();
    let c = Catalog::new("P".into(), snapshot).unwrap();
    let r = Runtime::ready();
    let mut p = MediaPanel::default();
    p.refresh(&c, &r);
    pointer(&mut p, &c, &r, "c", Modifiers::default());
    key(&mut p, &c, &r, "c", Key::Right, shift());
    assert_eq!(p.selected_assets(), ["c", "b"]);
    p.dispatch(
        &c,
        &r,
        Input::DragStart {
            id: asset("c"),
            nested_control: false,
        },
    )
    .unwrap();
    assert_eq!(p.drag.as_ref().unwrap().assets, ["c", "b"]);
    let page = p.page(&c, &r, 4, 4).unwrap();
    assert_eq!(
        page.rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>(),
        [asset("c"), asset("b"), asset("a"), asset("d")]
    );
}

#[test]
fn empty_direct_asset_field_falls_through_to_extension_source() {
    let mut root = parse(PROJECT).unwrap();
    root.as_object_mut().unwrap().insert("layers",parse(r#"[{"id":"l","type":"extension","d":{"asset":"","data":{"object":{"source":{"assetId":""}},"assetId":"a"}}}]"#).unwrap());
    let a = KernelActor::spawn(KernelConfig::default());
    let h = a.handle();
    h.open("P", OpenSource::Untitled(root))
        .wait()
        .unwrap()
        .unwrap();
    let c = current(&h);
    assert_eq!(c.assets["a"].references, 1);
    let before = project(&h);
    applied(
        &h,
        &c.prepare(Edit::DeleteAssets {
            ids: vec!["a".into()],
            confirmed: true,
        })
        .unwrap(),
    );
    assert!(
        project(&h)
            .get("comps")
            .unwrap()
            .get("C1")
            .unwrap()
            .get("layers")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    history(&h, HistoryCall::Undo);
    assert_eq!(project(&h), before);
}

#[test]
fn descendant_search_preserves_scope_paths_and_source() {
    let (c, mut p, r) = setup();
    p.dispatch(
        &c,
        &r,
        Input::Filter(Filter {
            query: "Nested".into(),
            ..Default::default()
        }),
    )
    .unwrap();
    let page = p.page(&c, &r, 0, 20).unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, asset("e"));
    assert_eq!(page.rows[0].folder_path, ["Bin 2", "Inner"]);
    p.dispatch(&c, &r, Input::OpenFolder(Some("f3".into())))
        .unwrap();
    p.dispatch(
        &c,
        &r,
        Input::Filter(Filter {
            query: "Nested".into(),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(p.total(), 0);
    assert_eq!(c.assets["e"].folder.as_deref(), Some("f2"));
}
#[test]
fn native_grid_columns_drive_rows_wrap_and_shift_ranges() {
    let (c, mut p, r) = setup();
    p.dispatch(&c, &r, Input::GridColumns(3)).unwrap();
    key(&mut p, &c, &r, "a", Key::Down, Modifiers::default());
    assert_eq!(p.focused(), Some(&asset("d")));
    key(&mut p, &c, &r, "d", Key::Up, shift());
    assert_eq!(p.selected_assets(), ["a", "b", "c", "d"]);
    key(&mut p, &c, &r, "d", Key::Down, Modifiers::default());
    assert_eq!(p.focused(), Some(&ItemId::Folder("f3".into())));
    assert_eq!(
        p.dispatch(&c, &r, Input::GridColumns(0)).unwrap_err(),
        Error::InvalidTarget
    );
}
#[test]
fn select_all_uses_sparse_exceptions_for_pages_commands_and_refresh() {
    let (c, mut p, r) = setup();
    key(&mut p, &c, &r, "a", Key::A, primary());
    assert_eq!(p.selected_count(), 4);
    pointer(&mut p, &c, &r, "b", primary());
    assert_eq!(p.selected_count(), 3);
    assert!(!p.is_selected("b"));
    assert_eq!(p.selected_assets(), ["a", "c", "d"]);
    p.refresh(&c, &r);
    assert_eq!(p.selected_assets(), ["a", "c", "d"]);
    assert_eq!(
        p.page(&c, &r, 0, 20)
            .unwrap()
            .rows
            .iter()
            .filter(|r| r.selected)
            .count(),
        3
    );
    pointer(&mut p, &c, &r, "b", primary());
    assert_eq!(p.selected_count(), 4);
    let plan = key(&mut p, &c, &r, "a", Key::Delete, Modifiers::default());
    assert!(matches!(&plan.effects[..], [Effect::ConfirmDelete { assets,.. }] if assets.len()==4));
}

#[test]
fn approved_undo_names_are_title_case() {
    let c = catalog();
    for (edit, label) in [
        (
            Edit::CreateFolder {
                id: "new".into(),
                name: "Fixture".into(),
                parent: None,
            },
            "New Folder",
        ),
        (
            Edit::RenameFolder {
                id: "f1".into(),
                name: "Renamed".into(),
            },
            "Rename Folder",
        ),
        (Edit::DeleteFolder { id: "f1".into() }, "Delete Folder"),
        (
            Edit::Move {
                assets: vec!["a".into()],
                folders: vec![],
                target: Some("f1".into()),
            },
            "Move to Folder",
        ),
        (
            Edit::DeleteAssets {
                ids: vec!["c".into()],
                confirmed: false,
            },
            "Delete Media",
        ),
    ] {
        assert_eq!(c.prepare(edit).unwrap().label, label);
    }
}
#[test]
fn changed_progress_and_composition_poster_remain_observed_facts() {
    struct Observed;
    impl MediaRuntime for Observed {
        fn state(&self, _: &Stamp, _: &str) -> RuntimeState {
            RuntimeState {
                availability: Availability::Changed {
                    source: "/synthetic/changed.mov".into(),
                },
                progress: Some(DownloadProgress {
                    completed: 25,
                    total: 100,
                }),
                ..Default::default()
            }
        }
        fn composition_poster(&self, _: &Stamp, id: &str) -> Option<String> {
            Some(format!("poster:{id}"))
        }
    }
    let mut value = parse(PROJECT).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("bg", Value::from("#112233"));
    let c = Catalog::new("P".into(), DocumentStore::new_untitled(value).snapshot()).unwrap();
    let mut p = MediaPanel::default();
    p.refresh(&c, &Observed);
    let page = p.page(&c, &Observed, 0, 100).unwrap();
    let comp = page
        .rows
        .iter()
        .find_map(|r| match &r.data {
            RowData::Composition(c) if c.id == "C1" => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(comp.background.as_deref(), Some("#112233"));
    assert_eq!(comp.poster.as_deref(), Some("poster:C1"));
    assert!(
        !p.dispatch(
            &c,
            &Observed,
            Input::DragStart {
                id: asset("a"),
                nested_control: false
            }
        )
        .unwrap()
        .handled
    );
    let asset = page
        .rows
        .iter()
        .find_map(|r| match &r.data {
            RowData::Asset(a, r) if a.id == "a" => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        asset.progress,
        Some(DownloadProgress {
            completed: 25,
            total: 100
        })
    );
}
