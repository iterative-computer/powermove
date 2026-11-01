use pm_kernel::DocumentStore;
use pm_panel_media::*;
use pm_project_panel_check::{accessibility, bridge::*};
fn setup() -> (Catalog, ProjectBridge) {
    let root = pm_model::json::parse(r#"{"compId":"c","compName":"Main","w":640,"h":360,"dur":20,"fps":30,"assets":{"a":{"id":"a","name":"A.mov","kind":"video"}},"assetFolders":{"f":{"id":"f","name":"Shots"}},"layers":[],"comps":{}}"#).unwrap();
    let c = Catalog::new("P".into(), DocumentStore::new_untitled(root).snapshot()).unwrap();
    let mut b = ProjectBridge::default();
    b.refresh(&c, &UnknownRuntime);
    (c, b)
}
#[test]
fn page_is_bounded_and_has_real_model_types() {
    let (c, mut b) = setup();
    let r = b
        .request(
            &c,
            &UnknownRuntime,
            Request {
                owner: c.stamp.clone(),
                action: Action::Page {
                    offset: 0,
                    limit: 2,
                },
            },
        )
        .unwrap();
    assert_eq!(r.view.page.total, 3);
    assert_eq!(r.view.page.rows.len(), 2);
    assert!(matches!(r.view.page.rows[0].data, RowData::Folder(_)));
    assert!(matches!(r.view.page.rows[1].data, RowData::Composition(_)));
}
#[test]
fn no_window_means_no_runtime_or_poster_requests() {
    let (c, b) = setup();
    assert!(b.view(&c, &UnknownRuntime).unwrap().page.rows.is_empty());
}
#[test]
fn stale_document_and_revision_refuse_before_model_input() {
    let (c, mut b) = setup();
    for owner in [
        Stamp {
            document: c.stamp.document + 1,
            ..c.stamp.clone()
        },
        Stamp {
            revision: c.stamp.revision + 1,
            ..c.stamp.clone()
        },
        Stamp {
            project: "other".into(),
            ..c.stamp.clone()
        },
    ] {
        assert!(matches!(
            b.request(
                &c,
                &UnknownRuntime,
                Request {
                    owner,
                    action: Action::Input(Input::FilesEnter)
                }
            ),
            Err(Error::Stale)
        ));
        assert!(!b.panel.file_drop_active());
    }
}
#[test]
fn rejects_large_window_before_mutating_view() {
    let (c, mut b) = setup();
    assert!(matches!(
        b.request(
            &c,
            &UnknownRuntime,
            Request {
                owner: c.stamp.clone(),
                action: Action::Page {
                    offset: 0,
                    limit: 4097
                }
            }
        ),
        Err(Error::InvalidTarget)
    ));
    assert!(b.view(&c, &UnknownRuntime).unwrap().page.rows.is_empty());
}
#[test]
fn root_drop_is_distinct_from_no_target_in_json() {
    let (c, mut b) = setup();
    b.panel.drop_target = Some(None);
    let v = serde_json::to_value(b.view(&c, &UnknownRuntime).unwrap()).unwrap();
    assert!(v["drop_target"]["folder"].is_null());
    assert!(v["drop_target"].is_object());
    b.panel.drop_target = None;
    assert!(
        serde_json::to_value(b.view(&c, &UnknownRuntime).unwrap()).unwrap()["drop_target"]
            .is_null()
    );
}
#[test]
fn folder_crumbs_and_input_are_model_owned() {
    let (c, mut b) = setup();
    let r = b
        .request(
            &c,
            &UnknownRuntime,
            Request {
                owner: c.stamp.clone(),
                action: Action::Input(Input::OpenFolder(Some("f".into()))),
            },
        )
        .unwrap();
    assert_eq!(r.view.crumbs[0].name, "Shots");
    assert_eq!(r.view.folder.as_deref(), Some("f"));
    assert!(r.outcome.unwrap().handled);
}
#[test]
fn menu_keeps_disabled_last_composition_delete() {
    let (c, mut b) = setup();
    let r = b
        .request(
            &c,
            &UnknownRuntime,
            Request {
                owner: c.stamp.clone(),
                action: Action::Menu(Some(ItemId::Composition("c".into()))),
            },
        )
        .unwrap();
    assert!(
        r.menu
            .unwrap()
            .entries
            .iter()
            .any(|e| matches!(e.command, MenuCommand::DeleteComposition(_)) && !e.enabled)
    );
}
#[test]
fn request_roundtrips_exact_agent_ids_and_modifiers() {
    let (c, _) = setup();
    let r = Request {
        owner: c.stamp,
        action: Action::Input(Input::Pointer {
            id: ItemId::Asset("🎬".into()),
            phase: PointerPhase::Down,
            button: 0,
            nested_control: false,
            modifiers: Modifiers {
                meta: true,
                shift: true,
                ..Default::default()
            },
        }),
    };
    let v = serde_json::to_value(&r).unwrap();
    let decoded: Request = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), v);
}

fn ax_fixture() -> (Catalog, ProjectAccessibilityFixture) {
    let (c, mut bridge) = setup();
    let view = bridge
        .request(
            &c,
            &UnknownRuntime,
            Request {
                owner: c.stamp.clone(),
                action: Action::Page {
                    offset: 0,
                    limit: 10,
                },
            },
        )
        .unwrap()
        .view;
    let mut ops = vec![
        serde_json::json!({"op":"create","id":1,"type":"div"}),
        serde_json::json!({"op":"setProps","id":1,"patch":{"data-pm-component":"list","label":"Project media"}}),
        serde_json::json!({"op":"insertBefore","parent":0,"id":1,"before":null}),
    ];
    for (index, row) in view.page.rows.iter().enumerate() {
        let id = index as u64 + 2;
        ops.extend([serde_json::json!({"op":"create","id":id,"type":"div"}),serde_json::json!({"op":"setProps","id":id,"patch":{"role":"cell","label":"Fixture","attr":{"item":row.id}}}),serde_json::json!({"op":"insertBefore","parent":1,"id":id,"before":null})]);
    }
    let mut surface = pm_surface::Surface::default();
    surface
        .apply_json(&serde_json::json!({"baseRevision":0,"ops":ops}).to_string())
        .unwrap();
    let ax = accessibility::ProjectAccessibility::prepare(
        &surface,
        &view,
        &pm_surface::kit::Layout::new(),
        Some(1),
    );
    (c, ProjectAccessibilityFixture { ax, view })
}
struct ProjectAccessibilityFixture {
    ax: accessibility::ProjectAccessibility,
    view: Projection,
}
#[test]
fn accessibility_is_a_multiselect_list_of_real_cells_with_full_count() {
    let (_, f) = ax_fixture();
    let grid = &f.ax.tree.nodes.iter().find(|(id, _)| id.0 == 1).unwrap().1;
    assert_eq!(grid.role(), accesskit::Role::List);
    assert!(grid.is_multiselectable());
    assert_eq!(grid.row_count(), Some(3));
    for (_, cell) in f.ax.tree.nodes.iter().skip(2) {
        assert_eq!(cell.role(), accesskit::Role::Cell);
        assert_eq!(cell.is_selected(), Some(false));
    }
    assert_eq!(f.view.page.rows.len(), 3);
}
#[test]
fn voiceover_folder_actions_use_exact_model_inputs_and_refuse_stale_owners() {
    let (c, f) = ax_fixture();
    let cell = &f.ax.tree.nodes.iter().find(|(id, _)| id.0 == 2).unwrap().1;
    assert_eq!(
        cell.custom_actions()
            .iter()
            .map(|a| a.description.as_ref())
            .collect::<Vec<_>>(),
        vec!["Open", "Rename", "Delete"]
    );
    let request = accesskit::ActionRequest {
        action: accesskit::Action::CustomAction,
        target_tree: accesskit::TreeId::ROOT,
        target_node: accesskit::NodeId(2),
        data: Some(accesskit::ActionData::CustomAction(1)),
    };
    assert!(matches!(f.ax.input(1,&c.stamp,&request),Ok(Input::BeginRename(id)) if id=="f"));
    assert!(f.ax.input(2, &c.stamp, &request).is_err());
    assert!(
        f.ax.input(
            1,
            &Stamp {
                document: c.stamp.document + 1,
                ..c.stamp.clone()
            },
            &request
        )
        .is_err()
    );
    assert!(
        f.ax.input(
            1,
            &c.stamp,
            &accesskit::ActionRequest {
                data: Some(accesskit::ActionData::CustomAction(-1)),
                ..request.clone()
            }
        )
        .is_err()
    );
    assert!(
        f.ax.input(
            1,
            &c.stamp,
            &accesskit::ActionRequest {
                target_node: accesskit::NodeId(999),
                ..request
            }
        )
        .is_err()
    );
}
#[test]
fn unknown_originals_and_last_comp_do_not_advertise_unsafe_actions() {
    let (_, f) = ax_fixture();
    let comp = &f.ax.tree.nodes.iter().find(|(id, _)| id.0 == 3).unwrap().1;
    assert!(
        !comp
            .custom_actions()
            .iter()
            .any(|a| a.description.as_ref() == "Delete")
    );
    let asset = &f.ax.tree.nodes.iter().find(|(id, _)| id.0 == 4).unwrap().1;
    assert_eq!(
        asset
            .custom_actions()
            .iter()
            .map(|a| a.description.as_ref())
            .collect::<Vec<_>>(),
        vec!["Delete"]
    );
}

#[test]
fn every_folder_activation_resets_the_virtual_window_to_its_new_scope() {
    for input in [
        Input::Pointer {
            id: ItemId::Folder("f".into()),
            phase: PointerPhase::DoubleClick,
            button: 0,
            nested_control: false,
            modifiers: Modifiers::default(),
        },
        Input::Invoke(MenuCommand::OpenFolder("f".into())),
        Input::Key {
            target: KeyTarget::Row(ItemId::Folder("f".into())),
            key: Key::Enter,
            modifiers: Modifiers::default(),
        },
    ] {
        let root = pm_model::json::parse(r#"{"compId":"c","compName":"Main","w":640,"h":360,"dur":20,"fps":30,"assets":{"a":{"id":"a","name":"A.mov","kind":"video","folder":"f"}},"assetFolders":{"f":{"id":"f","name":"Shots"}},"layers":[],"comps":{}}"#).unwrap();
        let c = Catalog::new("P".into(), DocumentStore::new_untitled(root).snapshot()).unwrap();
        let mut bridge = ProjectBridge::default();
        bridge.refresh(&c, &UnknownRuntime);
        bridge
            .request(
                &c,
                &UnknownRuntime,
                Request {
                    owner: c.stamp.clone(),
                    action: Action::Page {
                        offset: 1,
                        limit: 1,
                    },
                },
            )
            .unwrap();
        let reply = bridge
            .request(
                &c,
                &UnknownRuntime,
                Request {
                    owner: c.stamp.clone(),
                    action: Action::Input(input),
                },
            )
            .unwrap();
        assert_eq!(reply.view.folder.as_deref(), Some("f"));
        assert_eq!(reply.view.page.offset, 0);
        assert_eq!(reply.view.page.rows.len(), 1);
        assert_eq!(reply.view.page.rows[0].id, ItemId::Asset("a".into()));
    }
}
