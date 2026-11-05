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
fn packed_save_retains_the_source_contract_failure() {
    let result = pm_acceptance::runner::execute(
        "A506",
        "W031",
        "original",
        "Packed project export",
        headless::packed_save,
    );
    assert_eq!(result.status, pm_acceptance::runner::Status::Fail);
    assert!(
        result
            .reason
            .contains("save_project must export a separate packed project"),
        "{}",
        result.reason
    );
}

#[test]
fn utf16_ingress_retains_real_failure_and_leaves_source_bytes_exact() {
    let result = headless::utf16_ingress();
    assert_eq!(
        result.unwrap_err(),
        "MCP ingress rejects kernel-valid authored lone UTF-16; serde_json conversion panics"
    );
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
