use pm_acceptance::headless;
macro_rules! tools {
    ($($name:ident),* $(,)?) => {$ (
        #[test] fn $name() { headless::check(stringify!($name)).unwrap(); }
    )*};
}
tools! { list_projects, create_project, open_project, manage_project, start_session,
finish_session, rollback_changes, get_timeline, select_layers, apply_commands,
edit_layer_strips, edit_video, export_captions, manage_media,
get_panel_layout, get_3d_scene, validate_effect }
#[test]
fn packed_save_preserves_bytes_history_source_and_unique_export_contract() {
    headless::packed_save().unwrap();
}

#[test]
fn utf16_ingress_preserves_authored_code_units_and_source_bytes() {
    headless::utf16_ingress().unwrap();
}

#[test]
fn registry_doubles_cannot_promote_full_media_bodies() {
    for tool in [
        "list_media",
        "check_project",
        "get_project_state",
        "project_memory",
    ] {
        assert!(headless::check(tool).is_err());
        let report = pm_acceptance::main_reference::run(
            &pm_acceptance::catalog::Catalog::embedded().unwrap(),
            &pm_acceptance::runner::Filter {
                workflow: None,
                case: Some(format!("MAIN.MCP.{tool}.behavior")),
            },
            &[],
            None,
        )
        .unwrap();
        assert_eq!(report.counts(), (0, 0, 1));
        assert_eq!(report.exit_code(true), 2);
    }
}
