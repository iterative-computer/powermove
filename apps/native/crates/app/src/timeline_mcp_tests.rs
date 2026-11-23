//! A real app MCP connection and a GPUI test timeline share a temporary project.
use super::*;
use pm_kernel::{KernelActor, KernelConfig, session::SessionSource};
use serde_json::{Value, json};

struct Session {
    tools: crate::mcp::Connection,
    stage: Stage,
    _profile: tempfile::TempDir,
}

struct McpFixture {
    tools: crate::mcp::Connection,
    rig: Rig,
    profile: tempfile::TempDir,
}

impl McpFixture {
    fn new() -> Self {
        let profile = tempfile::tempdir().unwrap();
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        crate::mcp::attach(&kernel, profile.path()).unwrap();
        let host = crate::mcp::Host::new(kernel, profile.path().to_owned());
        let mut project: Value =
            serde_json::from_str(&crate::v4_view_tests::project_json(1)).unwrap();
        project["layers"][0]["name"] = json!("Main baseline");
        project["comps"]["alternate"] = json!({
            "id":"alternate", "name":"Alternate", "w":640, "h":360,
            "fps":30, "dur":10, "bg":"#000000",
            "layers":[{"id":"alternate-solid","name":"Alternate solid","type":"solid",
                       "from":0,"dur":8,"on":true,"p":{},"d":{"w":32,"h":24}}]
        });
        host.open(
            PROJECT.into(),
            SessionSource::New(pm_model::json::parse(&project.to_string()).unwrap()),
        )
        .unwrap();
        host.save(host.current().unwrap(), None).unwrap();
        let tools = crate::mcp::Tools::new(host, profile.path().join("exports"));
        if let Some(worker) = std::env::var_os("PM_TIMELINE_ENGINE_WORKER") {
            crate::mcp_services::connect_engine_with_executable(
                &tools,
                Default::default(),
                Default::default(),
                worker.into(),
            )
            .unwrap();
        }
        let rig = crate::v4_view_tests::rig_opened("timeline", actor);
        Self {
            tools: tools.connect(),
            rig,
            profile,
        }
    }
}

impl Session {
    fn new(cx: &mut TestAppContext) -> Self {
        let McpFixture {
            tools,
            rig,
            profile,
        } = McpFixture::new();
        Self {
            tools,
            stage: stage_opened(cx, rig, true),
            _profile: profile,
        }
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        call(&mut self.tools, name, args)
    }

    fn start(&mut self) {
        self.call(
            "start_session",
            json!({"prompt":"Create an animated typewriter"}),
        );
    }

    fn add_scene(&mut self) -> String {
        add_scene(&mut self.tools)
    }

    fn refresh(&mut self, cx: &mut TestAppContext) {
        self.stage
            .surface
            .update(&mut self.stage.vcx, |t, cx| t.published(cx));
        self.stage.idle(cx);
    }
}

fn call(tools: &mut crate::mcp::Connection, name: &str, args: Value) -> Value {
    tools
        .dispatch(name, &args)
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

// The normal Rust test executable uses libtest's main and cannot serve
// --engine-job. Lane runs supply a separate headless worker.
fn render_frame(tools: &mut crate::mcp::Connection, args: Value) {
    if std::env::var_os("PM_TIMELINE_ENGINE_WORKER").is_some() {
        call(tools, "render_frames", args);
    }
}

fn add_scene(tools: &mut crate::mcp::Connection) -> String {
    call(
        tools,
        "apply_commands",
        json!({"commands":[
            {"type":"add_layer", "id":"cursor", "name":"Cursor", "layerType":"solid",
             "duration":8, "content":{"w":10,"h":40,"color":"#ee5555"}},
            {"type":"add_layer", "id":"headline", "name":"Headline", "layerType":"text",
             "duration":8, "content":{"text":"Just describe it."}},
            {"type":"group_layers", "targets":["cursor","headline"], "name":"Typewriter"},
            {"type":"add_layer", "id":"adjust", "name":"Adjustment Layer",
             "layerType":"adjustment", "duration":8}
        ]}),
    );
    let state = call(tools, "get_project_state", json!({}));
    let group = state["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["name"] == "Typewriter")
        .unwrap();
    let group = group["id"].as_str().unwrap().to_owned();
    call(
        tools,
        "apply_commands",
        json!({"commands":[
            {"type":"set_layer","target":group,"patch":{"collapsed":true}},
            {"type":"replace_keyframes","target":"cursor","path":"opacity","keyframes":[
                {"id":"cursor-on","time":0,"value":100},{"id":"cursor-off","time":1,"value":0}
            ]}
        ]}),
    );
    group
}

fn row_id(layer: &str) -> String {
    pm_panel_timeline::v4::row_id(&pm_panel_timeline::RowId::Layer(layer.into()))
}

fn row_visible(stage: &mut Stage, layer: &str) -> bool {
    let name = format!("timeline:row:{}", row_id(layer));
    stage
        .vcx
        .debug_bounds(Box::leak(name.into_boxed_str()))
        .is_some()
}

fn strip_visible(stage: &Stage, layer: &str) -> bool {
    stage.surface.read_with(&stage.vcx, |timeline, _| {
        timeline
            .scene()
            .is_some_and(|scene| scene.scroll_for(&row_id(layer)).is_some())
    })
}

fn assert_rows_match(stage: &mut Stage, layers: &[&str]) {
    for layer in layers {
        assert_eq!(
            row_visible(stage, layer),
            strip_visible(stage, layer),
            "outline and strips diverged for {layer}"
        );
    }
}

fn assert_visible_row_order(stage: &mut Stage) {
    let body = bounds(&mut stage.vcx, "timeline:surface");
    let (first, names) = stage
        .view
        .read_with(&stage.vcx, |view, _| view.timeline_outline_rows());
    let (range, strips) = stage.surface.read_with(&stage.vcx, |timeline, _| {
        let scene = timeline.scene().expect("a project timeline scene");
        let height = f32::from(body.size.height) as f64 - tokens().metrics.ruler as f64;
        let range = scene.visible_range(timeline.scroll_y(), height.max(0.));
        let strips = range
            .clone()
            .map(|index| scene.span(index).unwrap().0.id.clone())
            .collect::<Vec<_>>();
        (range, strips)
    });
    assert!(!strips.is_empty(), "the timeline has no visible strips");
    assert!(range.start >= first, "the outline begins after {range:?}");
    assert_eq!(
        names.get(range.start - first..range.end - first),
        Some(strips.as_slice()),
        "name and strip rows differ for visible indices {range:?}"
    );
}

fn press_timeline_keys(stage: &mut Stage, keys: &str, cx: &mut TestAppContext) {
    stage.vcx.simulate_keystrokes(keys);
    settle(cx, |cx| {
        stage.surface.read_with(cx, |timeline, _| timeline.idle())
    });
    stage
        .vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    // Draw after the native worker has run but before delivering a separate
    // JS publication, as can happen in the editor under load.
    assert_visible_row_order(stage);
    stage.idle(cx);
    stage.settle_outline(cx);
    assert_visible_row_order(stage);
}

#[gpui::test]
fn timeline_mcp_reveal_shortcuts_keep_nested_names_and_strips_together(cx: &mut TestAppContext) {
    let mut project: Value = serde_json::from_str(&crate::v4_view_tests::project_json(2)).unwrap();
    let layers = project["layers"].as_array_mut().unwrap();
    layers[0]["group"] = json!("inner");
    layers[0]["p"]["opacity"]["kf"] = json!([
        {"i":"in", "t":0.25, "v":100},
        {"i":"out", "t":1.0, "v":0}
    ]);
    layers[0]["masks"] = json!([
        {"id":"mask", "shape":"rect", "p":{"x":{"v":10,"kf":[]}}}
    ]);
    layers[1]["group"] = json!("closed");
    layers.extend([
        json!({"id":"inner","name":"Inner","type":"group","group":"outer","from":0,"dur":8,"collapsed":false}),
        json!({"id":"outer","name":"Outer","type":"group","from":0,"dur":8,"collapsed":false}),
        json!({"id":"closed","name":"Closed","type":"group","from":0,"dur":8,"collapsed":true}),
    ]);
    let mut stage = stage_with(cx, project.to_string(), true);
    assert_visible_row_order(&mut stage);
    assert!(!strip_visible(&stage, "l1"));
    select_row(&mut stage, "l0", cx);
    stage
        .surface
        .update(&mut stage.vcx, |timeline, cx| timeline.published(cx));
    stage.idle(cx);

    for (keys, expected) in [("u m", "m.mask.x"), ("m u", "opacity")] {
        press_timeline_keys(&mut stage, keys, cx);
        let property = pm_panel_timeline::v4::row_id(&pm_panel_timeline::RowId::Property {
            layer_id: "l0".into(),
            channel: expected.into(),
        });
        assert!(
            stage.surface.read_with(&stage.vcx, |timeline, _| {
                timeline.scene().unwrap().row(&property).is_some()
            }),
            "{keys} did not reveal {expected}"
        );
        seek_with_click(&mut stage, cx);
    }
    for keys in ["u m u m", "m u m u"] {
        press_timeline_keys(&mut stage, keys, cx);
        assert!(!strip_visible(&stage, "l1"));
        seek_with_click(&mut stage, cx);
    }
    select_row(&mut stage, "l0", cx);
    stage
        .rig
        .kernel
        .call_blocking(
            PROJECT,
            Request::new(Caller::native(), pm_kernel::SelectionCall::Clear),
        )
        .unwrap();
    stage
        .surface
        .update(&mut stage.vcx, |timeline, cx| timeline.published(cx));
    stage.idle(cx);
    stage.settle_outline(cx);
    assert!(
        stage
            .rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
            .is_empty()
    );
    // No-selection shortcuts must preserve the same alignment and respect
    // the disclosure that the model actually published for the unrelated group.
    press_timeline_keys(&mut stage, "u", cx);
    assert!(!strip_visible(&stage, "l1"));
    for _ in 0..3 {
        press_timeline_keys(&mut stage, "m", cx);
        let closed = stage.surface.read_with(&stage.vcx, |t, _| {
            t.model().reveal.layers.get("closed").map_or_else(
                || t.model().index.layer("closed").unwrap().group_collapsed,
                |disclosure| disclosure.group_collapsed,
            )
        });
        assert_eq!(strip_visible(&stage, "l1"), !closed);
    }
    // Exercise both disclosure states even when rapid reveal keys end closed.
    for opened in [true, false] {
        if strip_visible(&stage, "l1") != opened {
            click_disclosure(&mut stage, "closed", cx);
            stage.idle(cx);
            stage.settle_outline(cx);
        }
        assert_eq!(strip_visible(&stage, "l1"), opened);
        assert_visible_row_order(&mut stage);
    }
    seek_with_click(&mut stage, cx);
    assert!(stage.recorder.reports.borrow().is_empty());
}

#[gpui::test]
fn timeline_mcp_reveal_shortcuts_keep_the_saved_project_interactive(cx: &mut TestAppContext) {
    let Some(path) = std::env::var_os("PM_TIMELINE_MCP_PROJECT") else {
        return;
    };
    let opened =
        pm_kernel::open_project(&std::path::PathBuf::from(path), Default::default()).unwrap();
    let rig = crate::v4_view_tests::rig_source("timeline", pm_kernel::OpenSource::File(opened));
    let mut stage = stage_opened(cx, rig, true);
    // With no layer selected, M can open every group and cross the
    // boundary between a retained outline and one that pages in rows.
    press_timeline_keys(&mut stage, "u m", cx);
    press_timeline_keys(&mut stage, "m", cx);
    assert!(stage.surface.read_with(&stage.vcx, |timeline, _| {
        timeline.scene().unwrap().total() > 1536
    }));
    for keys in ["m u", "u m u m"] {
        press_timeline_keys(&mut stage, keys, cx);
        seek_with_click(&mut stage, cx);
    }
    let layer = stage.surface.read_with(&stage.vcx, |timeline, _| {
        timeline
            .model()
            .index
            .layers
            .iter()
            .find(|layer| layer.name.to_string_lossy() == "Cursor")
            .expect("the saved project contains the animated Cursor")
            .id
            .clone()
    });
    // Select through the kernel as the layer may be below the test viewport.
    stage
        .rig
        .kernel
        .call_blocking(
            PROJECT,
            Request::new(
                Caller::native(),
                Call::Selection(pm_kernel::SelectionCall::Layers {
                    ids: vec![layer],
                    add: false,
                }),
            ),
        )
        .unwrap();
    stage
        .surface
        .update(&mut stage.vcx, |timeline, cx| timeline.published(cx));
    stage.idle(cx);
    for keys in ["u m", "m u", "u m u m"] {
        press_timeline_keys(&mut stage, keys, cx);
        seek_with_click(&mut stage, cx);
    }
    assert!(stage.recorder.reports.borrow().is_empty());
}

fn click_disclosure(stage: &mut Stage, group: &str, cx: &mut TestAppContext) {
    let target = bounds(
        &mut stage.vcx,
        &format!("timeline:disclosure:{}", row_id(group)),
    );
    stage.vcx.simulate_click(target.center(), Modifiers::none());
    settle(cx, |cx| stage.view.read_with(cx, |view, _| view.idle()));
}

fn select_row(stage: &mut Stage, layer: &str, cx: &mut TestAppContext) {
    let row = bounds(&mut stage.vcx, &format!("timeline:row:{}", row_id(layer)));
    stage.vcx.simulate_click(
        point(row.left() + px(90.), row.center().y),
        Modifiers::none(),
    );
    settle(cx, |cx| stage.view.read_with(cx, |view, _| view.idle()));
    let published = stage.rig.kernel.published(PROJECT).unwrap();
    assert_eq!(published.selection.primary(), Some(layer));
}

fn seek_with_click(stage: &mut Stage, cx: &mut TestAppContext) {
    let column = stage.surface.read_with(&stage.vcx, |t, _| t.layer_column());
    let ruler = stage.origin() + point(px(column as f32 + 100.), px(12.));
    let count = stage.recorder.seeks.borrow().len();
    stage.vcx.simulate_click(ruler, Modifiers::none());
    stage.idle(cx);
    assert!(
        stage.recorder.seeks.borrow().len() > count,
        "the ruler did not receive a click"
    );
}

#[gpui::test]
fn timeline_mcp_large_outline_mounts_and_accepts_input(cx: &mut TestAppContext) {
    let mut stage = stage(cx, 656);
    assert!(row_visible(&mut stage, "l0"));
    assert!(strip_visible(&stage, "l0"));
    let (_, rows) = stage
        .view
        .read_with(&stage.vcx, |view, _| view.timeline_outline_rows());
    assert!(
        rows.len() < 128,
        "large outline must only send a window to the panel"
    );
    seek_with_click(&mut stage, cx);
}

#[gpui::test]
fn timeline_mcp_native_rows_stay_available_while_the_outline_starts(cx: &mut TestAppContext) {
    struct NativeOnly(Entity<TimelineSurface>);
    impl gpui::Render for NativeOnly {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement, Styled};
            gpui::div().size_full().child(self.0.clone())
        }
    }
    let rig = crate::v4_view_tests::rig("timeline", 2);
    let recorder = Rc::new(Recorder::default());
    let surface = cx.new(|cx| {
        TimelineSurface::new(
            rig.kernel.clone(),
            PROJECT.into(),
            tokens(),
            Rc::new(Recording(recorder)),
            Natives::default().outline,
            Default::default(),
            cx,
        )
    });
    cx.executor().allow_parking();
    let (_, vcx) = cx.add_window_view(|_, _| NativeOnly(surface.clone()));
    let mut vcx = vcx.clone();
    vcx.simulate_resize(gpui::size(px(1000.), px(600.)));
    settle(cx, |cx| {
        surface.read_with(cx, |timeline, _| timeline.idle())
    });
    vcx.simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    let row = bounds(&mut vcx, &format!("timeline:fallback:{}", row_id("l0")));
    vcx.simulate_click(row.center(), Modifiers::none());
    settle(cx, |cx| {
        surface.read_with(cx, |timeline, _| timeline.idle())
    });
    assert_eq!(
        rig.kernel.published(PROJECT).unwrap().selection.primary(),
        Some("l0")
    );
}

#[gpui::test]
fn timeline_mcp_a_malformed_layer_does_not_take_down_the_timeline(cx: &mut TestAppContext) {
    let mut project: Value = serde_json::from_str(&crate::v4_view_tests::project_json(3)).unwrap();
    project["layers"][1]["dur"] = json!(-1);
    project["layers"][2]["id"] = json!("l0");
    let mut stage = stage_project(cx, project.to_string());
    assert!(row_visible(&mut stage, "l0"));
    assert!(strip_visible(&stage, "l0"));
    assert!(!strip_visible(&stage, "l1"));
    seek_with_click(&mut stage, cx);
}

#[gpui::test]
fn timeline_mcp_open_repairs_a_legacy_composition_map(cx: &mut TestAppContext) {
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("composition.pmv");
    let mut project: Value = serde_json::from_str(&crate::v4_view_tests::project_json(1)).unwrap();
    project["comps"] = Value::Null;
    std::fs::write(&path, project.to_string()).unwrap();
    let opened = pm_kernel::open_project(&path, Default::default()).unwrap();
    let rig = crate::v4_view_tests::rig_source("timeline", pm_kernel::OpenSource::File(opened));
    let mut stage = stage_opened(cx, rig, true);
    assert!(row_visible(&mut stage, "l0"));
    assert!(strip_visible(&stage, "l0"));
    seek_with_click(&mut stage, cx);
}

/// The shell lays out the saved docks on TestPlatform. Only the Timeline
/// needs a live panel here, so the fixture can run without AppKit.
struct SavedWorkspaceDock {
    store: Rc<pm_shell::workspace::Store>,
    tokens: pm_design::Tokens,
    runtime: std::sync::Arc<pm_builtins::Builtins>,
    natives: Natives,
    first: RefCell<Option<pm_builtins::Publication>>,
    view: RefCell<Option<Entity<SurfaceView>>>,
}

impl pm_shell::integration::Integration for SavedWorkspaceDock {
    fn observe(&self, _: &mut gpui::Context<pm_shell::view::Editor>) {}

    fn command(&self, _: &pm_shell::Command, _: &mut Window, _: &mut App) -> bool {
        false
    }

    fn workspaces(&self, _: &App) -> Option<Rc<pm_shell::workspace::Store>> {
        Some(self.store.clone())
    }

    fn project_id(&self, _: &App) -> Option<String> {
        Some(PROJECT.into())
    }

    fn status(&self, _: &App) -> String {
        String::new()
    }

    fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}

    fn paints_own_ground(&self, id: &str, _: &App) -> bool {
        id == "timeline"
    }

    fn panel(
        &self,
        id: &str,
        rect: pm_shell::layout::Rect,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyElement> {
        use gpui::{IntoElement, ParentElement, Styled};
        if id != "timeline" {
            return None;
        }
        if let Some(first) = self.first.borrow_mut().take() {
            let view = cx.new(|cx| {
                SurfaceView::new(
                    self.runtime.clone(),
                    first,
                    self.tokens.clone(),
                    self.natives.clone(),
                    window,
                    cx,
                )
            });
            self.natives.outline.bind(view.downgrade());
            self.view.replace(Some(view));
        }
        let view = self.view.borrow().clone()?;
        view.update(cx, |view, cx| view.attach(window, cx));
        Some(
            gpui::div()
                .w_full()
                .h(px((rect.h - self.tokens.metrics.header).max(0.)))
                .overflow_hidden()
                .child(crate::builtins::fixed_body_view(id, &view))
                .into_any_element(),
        )
    }
}

fn stage_saved_workspace(cx: &mut TestAppContext, rig: Rig, saved: Value) -> Stage {
    use pm_shell::{view::Editor, workspace};
    let published = rig.kernel.published(PROJECT).unwrap();
    let shell_tokens = tokens();
    let timeline_tokens =
        crate::builtins::timeline_chrome(shell_tokens.clone(), &published.workspace, &Value::Null);
    let natives = Natives::default();
    let recorder = Rc::new(Recorder::default());
    recorder.runtime.replace(Some(rig.runtime.clone()));
    recorder.published.replace(Some(rig.kernel.clone()));
    let surface = cx.new(|cx| {
        TimelineSurface::new(
            rig.kernel.clone(),
            PROJECT.into(),
            timeline_tokens.clone(),
            Rc::new(Recording(recorder.clone())),
            natives.outline.clone(),
            Default::default(),
            cx,
        )
    });
    natives
        .views
        .borrow_mut()
        .insert("timeline".into(), surface.clone().into());
    let dock = Rc::new(SavedWorkspaceDock {
        store: Rc::new(workspace::Store {
            records: workspace::seeded(&[]),
            project: Some(saved.clone()),
            project_id: Some(PROJECT.into()),
            ..Default::default()
        }),
        tokens: timeline_tokens,
        runtime: rig.runtime.clone(),
        natives,
        first: RefCell::new(Some(publication(&rig))),
        view: RefCell::new(None),
    });
    cx.executor().allow_parking();
    let (editor, vcx) = cx.add_window_view(|_, cx| {
        let mut editor = Editor::new(shell_tokens, None, cx);
        editor.integration = Some(dock.clone());
        editor
    });
    let mut vcx = vcx.clone();
    vcx.simulate_resize(gpui::size(px(1440.), px(900.)));
    vcx.simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    editor.read_with(&vcx, |editor, _| {
        assert_eq!(editor.workspace_record(), &saved);
        assert!(editor.tab_rect("viewer").is_some());
        assert!(editor.tab_rect("timeline").is_some());
    });
    let view = dock
        .view
        .borrow()
        .clone()
        .expect("saved Timeline dock mounted");
    let mut stage = Stage {
        rig,
        view,
        surface,
        recorder,
        vcx,
    };
    stage.idle(cx);
    stage
}

fn assert_saved_workspace_timeline(
    cx: &mut TestAppContext,
    actor: KernelActor,
    saved: &str,
) -> Stage {
    let mut stage = stage_saved_workspace(
        cx,
        crate::v4_view_tests::rig_opened("timeline", actor),
        serde_json::from_str(saved).unwrap(),
    );
    let published = stage.rig.kernel.published(PROJECT).unwrap();
    let root = published.snapshot.root().unwrap();
    let comp = published.snapshot.render_comp();
    let node = pm_kernel::model::comp_node(root, Some(&comp)).unwrap_or(root);
    let layers = pm_kernel::model::layers_of(node).count();
    assert!(
        layers > 0,
        "expected the saved composition to contain layers"
    );
    stage.surface.read_with(&stage.vcx, |timeline, _| {
        assert_eq!(timeline.model().index.doc_id, published.snapshot.doc_id);
        assert_eq!(
            timeline.model().index.comp_id,
            published.snapshot.open_comp()
        );
        assert!(timeline.model().index.has_composition());
        assert_eq!(timeline.model().index.layers.len(), layers);
    });
    assert!(stage.view.read_with(&stage.vcx, |view, _| {
        view.id_of_text("No Composition").is_none()
    }));
    seek_with_click(&mut stage, cx);
    stage
}

#[gpui::test]
fn timeline_mcp_saved_project_opens_on_the_viewers_composition(cx: &mut TestAppContext) {
    let Some(path) = std::env::var_os("PM_TIMELINE_MCP_PROJECT") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    let copy = directory.path().join("copy.pmv");
    std::fs::copy(path, &copy).unwrap();
    // The supplied source stays read-only. The isolated library needs a
    // writable copy to exercise the same path as an ordinary reopen.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o600)).unwrap();

    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    crate::mcp::attach(&kernel, &profile).unwrap();
    let host = crate::mcp::Host::new(kernel.clone(), profile.clone());
    host.open(PROJECT.into(), SessionSource::File(copy))
        .unwrap();
    let opened = kernel.published(PROJECT).unwrap();
    let saved = pm_model::json::to_canonical_string(&opened.workspace);
    let direct = assert_saved_workspace_timeline(cx, actor, &saved);
    host.close_headless().unwrap();
    drop(direct);
    drop(host);
    drop(kernel);

    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    crate::mcp::attach(&kernel, &profile).unwrap();
    let host = crate::mcp::Host::new(kernel.clone(), profile.clone());
    host.open(
        PROJECT.into(),
        SessionSource::Library {
            profile,
            project: PROJECT.into(),
        },
    )
    .unwrap();
    let restored = kernel.published(PROJECT).unwrap();
    assert_eq!(
        pm_model::json::to_canonical_string(&restored.workspace),
        saved
    );
    let reopened = assert_saved_workspace_timeline(cx, actor, &saved);
    host.close_headless().unwrap();
    drop(reopened);
}

#[gpui::test]
fn timeline_mcp_paints_one_row_set_while_publications_arrive(cx: &mut TestAppContext) {
    let mut mcp = Session::new(cx);
    mcp.start();
    let group = mcp.add_scene();

    // The native worker may finish before the V8 worker publishes an outline.
    // Yield a display frame in that order without delaying either model's work.
    mcp.stage
        .surface
        .update(&mut mcp.stage.vcx, |t, cx| t.published(cx));
    settle(cx, |cx| mcp.stage.surface.read_with(cx, |t, _| t.idle()));
    mcp.stage
        .vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    assert_rows_match(&mut mcp.stage, &[&group, "cursor", "headline", "adjust"]);

    mcp.stage.idle(cx);
    assert!(row_visible(&mut mcp.stage, &group));
    assert!(strip_visible(&mcp.stage, "adjust"));
    assert_rows_match(&mut mcp.stage, &[&group, "cursor", "headline", "adjust"]);
}

#[gpui::test]
fn timeline_mcp_commit_rollback_and_undo_leave_timeline_interactive(cx: &mut TestAppContext) {
    let mut mcp = Session::new(cx);
    mcp.start();
    let group = mcp.add_scene();
    mcp.refresh(cx);
    std::thread::scope(|scope| {
        let tools = &mut mcp.tools;
        let rendering = scope.spawn(|| {
            render_frame(tools, json!({"times":[0.5], "width":160}));
        });
        seek_with_click(&mut mcp.stage, cx);
        rendering.join().unwrap();
    });

    // Click the real outline control while MCP still owns the open session.
    for _ in 0..2 {
        let expanded = strip_visible(&mcp.stage, "cursor");
        click_disclosure(&mut mcp.stage, &group, cx);
        mcp.refresh(cx);
        assert_eq!(strip_visible(&mcp.stage, "cursor"), !expanded);
        assert_rows_match(&mut mcp.stage, &[&group, "cursor", "headline", "adjust"]);
    }
    mcp.call("finish_session", json!({"commit":true}));
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, &group));
    assert_rows_match(&mut mcp.stage, &[&group, "cursor", "headline", "adjust"]);
    seek_with_click(&mut mcp.stage, cx);
    select_row(&mut mcp.stage, &group, cx);

    mcp.start();
    mcp.call(
        "apply_commands",
        json!({"commands":[
            {"type":"add_layer","id":"temporary","layerType":"solid","name":"Temporary"}
        ]}),
    );
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, "temporary"));
    mcp.stage.undo(cx);
    assert!(!strip_visible(&mcp.stage, "temporary"));
    seek_with_click(&mut mcp.stage, cx);
    mcp.call("finish_session", json!({"commit":true}));
    mcp.refresh(cx);

    mcp.start();
    mcp.call(
        "apply_commands",
        json!({"commands":[
            {"type":"add_layer","id":"rolled-back","layerType":"solid","name":"Rolled back"}
        ]}),
    );
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, "rolled-back"));
    mcp.call("finish_session", json!({"commit":false}));
    mcp.refresh(cx);
    assert!(!strip_visible(&mcp.stage, "rolled-back"));
    assert_rows_match(
        &mut mcp.stage,
        &[&group, "cursor", "headline", "adjust", "rolled-back"],
    );
    seek_with_click(&mut mcp.stage, cx);
}

#[gpui::test]
fn timeline_mcp_key_edits_respect_a_closed_group(cx: &mut TestAppContext) {
    let mut mcp = Session::new(cx);
    mcp.start();
    let group = mcp.add_scene();
    mcp.refresh(cx);
    if strip_visible(&mcp.stage, "cursor") {
        click_disclosure(&mut mcp.stage, &group, cx);
        mcp.refresh(cx);
    }
    assert!(!strip_visible(&mcp.stage, "cursor"));
    mcp.call(
        "apply_commands",
        json!({"commands":[
            {"type":"replace_keyframes","target":"cursor","path":"position.x","keyframes":[
                {"id":"cursor-start","time":0,"value":0},{"id":"cursor-end","time":1,"value":120}
            ]}
        ]}),
    );
    mcp.refresh(cx);
    assert!(!strip_visible(&mcp.stage, "cursor"));
    assert_rows_match(&mut mcp.stage, &[&group, "cursor", "headline", "adjust"]);
    seek_with_click(&mut mcp.stage, cx);
}

#[gpui::test]
fn timeline_mcp_private_composition_does_not_take_over_the_editor(cx: &mut TestAppContext) {
    let mut mcp = Session::new(cx);
    mcp.start();
    let group = mcp.add_scene();
    mcp.refresh(cx);
    let comp = mcp
        .stage
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .snapshot
        .open_comp();
    mcp.call(
        "apply_commands",
        json!({"compositionId":"alternate","commands":[
            {"type":"add_layer","id":"alternate-added","layerType":"solid","name":"Private edit"}
        ]}),
    );
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, &group));
    assert!(!strip_visible(&mcp.stage, "alternate-added"));
    render_frame(
        &mut mcp.tools,
        json!({"compositionId":"alternate", "times":[0.5], "width":160}),
    );
    mcp.call("finish_session", json!({"commit":true}));
    mcp.refresh(cx);
    let active = mcp.stage.rig.kernel.published(PROJECT).unwrap();
    assert_eq!(active.snapshot.open_comp(), comp);
    mcp.stage.surface.read_with(&mcp.stage.vcx, |timeline, _| {
        assert_eq!(timeline.model().index.comp_id, active.snapshot.open_comp());
        assert!(timeline.model().index.has_composition());
    });
    assert!(strip_visible(&mcp.stage, &group));
    for layer in ["alternate-solid", "alternate-added"] {
        assert!(!strip_visible(&mcp.stage, layer));
        assert!(!row_visible(&mut mcp.stage, layer));
    }
    assert_rows_match(&mut mcp.stage, &[&group, "cursor", "headline", "adjust"]);
    seek_with_click(&mut mcp.stage, cx);
    mcp.stage.undo(cx);
    seek_with_click(&mut mcp.stage, cx);
}

#[gpui::test]
fn timeline_mcp_composition_switch_retires_the_previous_scene_immediately(cx: &mut TestAppContext) {
    let mut mcp = Session::new(cx);
    mcp.start();
    let group = mcp.add_scene();
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, &group));
    let document = mcp
        .stage
        .rig
        .kernel
        .published(PROJECT)
        .unwrap()
        .snapshot
        .doc_id;
    mcp.stage
        .rig
        .kernel
        .call_blocking(
            PROJECT,
            Request::new(
                Caller::native(),
                Call::OpenComposition {
                    comp_id: "alternate".into(),
                    document,
                },
            ),
        )
        .unwrap();
    assert_eq!(
        mcp.stage
            .rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .snapshot
            .open_comp()
            .as_deref(),
        Some("alternate")
    );

    // A native worker may still be producing the next composition's scene.
    // That cannot leave the old composition paintable in the current frame.
    mcp.stage
        .surface
        .update(&mut mcp.stage.vcx, |timeline, cx| {
            timeline.published(cx);
            assert!(
                !timeline
                    .scene()
                    .is_some_and(|scene| scene.scroll_for(&row_id(&group)).is_some())
            );
        });
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, "alternate-solid"));
    assert_rows_match(&mut mcp.stage, &[&group, "alternate-solid"]);
    seek_with_click(&mut mcp.stage, cx);
    mcp.call("finish_session", json!({"commit":true}));
    mcp.refresh(cx);
    assert!(strip_visible(&mcp.stage, "alternate-solid"));
    seek_with_click(&mut mcp.stage, cx);
}

/// Replays the same stages into two offscreen Metal textures. The input and
/// row assertions live in the GPUI tests above and do not depend on pixels.
#[::core::prelude::v1::test]
fn timeline_mcp_capture_publication_sequence() {
    use gpui::{HeadlessAppContext, KeyDownEvent, Keystroke, size};
    use std::sync::Arc;

    let Some(dir) = std::env::var_os("PM_TIMELINE_MCP_CAPTURE") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for (theme, scheme) in [
        ("light", pm_design::Scheme::Light),
        ("dark", pm_design::Scheme::Dark),
    ] {
        let McpFixture {
            mut tools,
            rig,
            profile: _profile,
        } = McpFixture::new();
        let tokens = pm_design::Tokens::new(scheme, Default::default());
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(
                gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                    .with_automatic_optical_sizing(),
            ),
            Arc::new(()),
            || {
                Some(Box::new(
                    gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                ))
            },
        );
        cx.allow_parking();
        let natives = Natives::default();
        let outline = natives.outline.clone();
        let recorder = Rc::new(Recorder::default());
        *recorder.runtime.borrow_mut() = Some(rig.runtime.clone());
        let surface = cx.update(|cx| {
            cx.new(|cx| {
                TimelineSurface::new(
                    rig.kernel.clone(),
                    PROJECT.into(),
                    tokens.clone(),
                    Rc::new(Recording(recorder)),
                    outline.clone(),
                    Default::default(),
                    cx,
                )
            })
        });
        natives
            .views
            .borrow_mut()
            .insert("timeline".into(), surface.clone().into());
        let first = publication(&rig);
        let runtime = rig.runtime.clone();
        let window = cx
            .open_window(size(px(1000.), px(360.)), |window, cx| {
                let child =
                    cx.new(|cx| SurfaceView::new(runtime, first, tokens, natives, window, cx));
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                cx.new(|_| Docked {
                    child,
                    focus,
                    timeline: surface.clone(),
                })
            })
            .unwrap();
        let view = cx.update(|cx| window.root(cx).unwrap().read(cx).child.clone());
        outline.bind(view.downgrade());
        let settle_native = |cx: &mut HeadlessAppContext| {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                cx.run_until_parked();
                if cx.update(|cx| surface.read(cx).idle() && view.read(cx).idle()) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timeline capture timed out"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let update_native = |cx: &mut HeadlessAppContext| {
            cx.update(|cx| surface.update(cx, |s, cx| s.published(cx)));
            settle_native(cx);
        };
        let refresh = |cx: &mut HeadlessAppContext| {
            settle_native(cx);
            update_native(cx);
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            for (_, ticket) in rig.runtime.publish() {
                let ticket = ticket.unwrap();
                loop {
                    if let Some(result) = ticket.try_take() {
                        result.unwrap();
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "outline capture timed out"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            cx.update(|cx| {
                for publication in rig.runtime.take_publications() {
                    view.update(cx, |v, cx| v.publish(publication, cx));
                }
            });
            cx.run_until_parked();
        };
        let capture = |cx: &mut HeadlessAppContext, stage: &str| {
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(dir.join(format!("{stage}-{theme}.png")))
                .unwrap();
        };
        settle_native(&mut cx);
        capture(&mut cx, "01-before-session");

        call(
            &mut tools,
            "start_session",
            json!({"prompt":"Create an animated typewriter"}),
        );
        let group = add_scene(&mut tools);
        update_native(&mut cx);
        capture(&mut cx, "02-first-edit-frame");
        refresh(&mut cx);
        capture(&mut cx, "03-live-rows");

        let target = cx
            .debug_bounds(
                window.into(),
                &format!("timeline:disclosure:{}", row_id(&group)),
            )
            .unwrap()
            .unwrap()
            .center();
        cx.simulate_event(
            window.into(),
            MouseDownEvent {
                button: MouseButton::Left,
                position: target,
                modifiers: Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            },
        );
        cx.simulate_event(
            window.into(),
            MouseUpEvent {
                button: MouseButton::Left,
                position: target,
                modifiers: Modifiers::none(),
                click_count: 1,
            },
        );
        refresh(&mut cx);
        capture(&mut cx, "04-disclosure");

        rig.kernel
            .call_blocking(
                PROJECT,
                Request::new(
                    Caller::native(),
                    pm_kernel::SelectionCall::Layers {
                        ids: vec!["cursor".into()],
                        add: false,
                    },
                ),
            )
            .unwrap();
        refresh(&mut cx);
        for (key, label) in [("u", "04a-reveal-u"), ("m", "04b-reveal-u-m")] {
            cx.simulate_event(
                window.into(),
                KeyDownEvent {
                    keystroke: Keystroke::parse(key).unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
            );
            settle_native(&mut cx);
            capture(&mut cx, label);
        }
        refresh(&mut cx);

        render_frame(&mut tools, json!({"times":[0.5], "width":160}));
        call(&mut tools, "finish_session", json!({"commit":true}));
        refresh(&mut cx);
        capture(&mut cx, "05-committed");

        rig.kernel
            .call_blocking(
                PROJECT,
                Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
            )
            .unwrap();
        refresh(&mut cx);
        capture(&mut cx, "06-after-undo");

        rig.kernel
            .call_blocking(
                PROJECT,
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some("Lock Layer".into()),
                        body: pm_kernel::WriteBody::Commands(vec![
                            pm_kernel::Command::SetLayerLocked {
                                layer_id: "l0".into(),
                                locked: true,
                            },
                        ]),
                        expected_revision: None,
                    },
                ),
            )
            .unwrap();
        refresh(&mut cx);
        capture(&mut cx, "07-locked-strip");
    }
}

/// Window position of a spot on a layer's strip, from the painted frame.
fn strip_point(stage: &mut Stage, layer: &str, fraction: f64, inset: f64) -> Point<Pixels> {
    let origin = stage.origin();
    let frame = stage
        .surface
        .read_with(&stage.vcx, |t, _| t.frame().expect("a painted frame"));
    let op = frame
        .ops
        .iter()
        .find(|op| op.id == format!("clip:{layer}"))
        .unwrap_or_else(|| panic!("no strip is painted for {layer}"));
    let x = op.bounds.x + op.bounds.width * fraction + inset;
    let y = op.bounds.y + op.bounds.height / 2.;
    origin + point(px(x as f32), px(y as f32))
}

/// A strip dragged by its body moves, and by each edge trims, whatever the
/// editor did before (agent session, reveal keys, undo).
fn assert_strip_edits(stage: &mut Stage, layer: &str, cx: &mut TestAppContext) {
    use pm_panel_timeline::render::Edge;
    fn edit_point(stage: &mut Stage, layer: &str, edge: Option<Edge>) -> Point<Pixels> {
        let (fraction, inset) = match edge {
            None => (0.5, 0.),
            Some(Edge::In) => (0., 2.),
            Some(Edge::Out) => (1., -2.),
        };
        let preferred = strip_point(stage, layer, fraction, inset);
        let origin = stage.origin();
        let at = preferred - origin;
        let frame = stage
            .surface
            .read_with(&stage.vcx, |t, _| t.frame().unwrap());
        let (x, y) = (f32::from(at.x) as f64, f32::from(at.y) as f64);
        // Keyframes take their own presses even if an underlying strip also
        // hits. Use a reachable part of the same strip or trim handle.
        let clear = |x| {
            frame.hit_clip(x, y) == Some((layer, edge))
                && frame
                    .hit_layer_key_across(
                        x,
                        y,
                        crate::timeline_surface::KEY_REACH + 0.5,
                        tokens().metrics.row as f64,
                    )
                    .is_none()
        };
        let x = std::iter::once(x)
            .chain((1..=32).flat_map(|i| {
                let shift = f64::from(i) * 0.5;
                [x + shift, x - shift]
            }))
            .find(|x| clear(*x))
            .unwrap_or_else(|| {
                panic!("no reachable {edge:?} target for strip {layer} near {x},{y}")
            });
        origin + point(px(x as f32), at.y)
    }
    stage.surface.update(&mut stage.vcx, |t, cx| t.fit_time(cx));
    stage.idle(cx);
    let (start, out) = (stage.start_of(layer), stage.out_of(layer));
    let body = edit_point(stage, layer, None);
    stage.drag(cx, body, body + point(px(40.), px(0.)), Modifiers::none());
    let moved = stage.start_of(layer);
    assert!(
        moved > start,
        "{layer}: a body drag left the strip at {moved} (was {start}); reports {:?}, previews {}, selection {:?}",
        stage.recorder.reports.borrow(),
        stage
            .recorder
            .timing_previews
            .borrow()
            .iter()
            .flatten()
            .count(),
        stage
            .rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
    );
    stage.undo(cx);
    assert_eq!(stage.start_of(layer), start);

    let edge = edit_point(stage, layer, Some(Edge::In));
    let mods = Modifiers::none();
    stage.vcx.simulate_mouse_down(edge, MouseButton::Left, mods);
    stage.idle(cx);
    assert_eq!(
        stage
            .surface
            .read_with(&stage.vcx, |t, _| t.gesture_state()),
        "pressed Some(Trim(In))",
        "{layer}: the in-edge press at {edge:?} must arm trim"
    );
    let mut samples = Vec::new();
    for travel in [7.5, 15., 22.5, 30.] {
        stage
            .vcx
            .simulate_mouse_move(edge + point(px(travel), px(0.)), MouseButton::Left, mods);
        stage.idle(cx);
        samples.push((
            stage.start_of(layer),
            stage
                .surface
                .read_with(&stage.vcx, |t, _| t.gesture_state()),
        ));
    }
    stage
        .vcx
        .simulate_mouse_up(edge + point(px(30.), px(0.)), MouseButton::Left, mods);
    stage.idle(cx);
    assert!(
        stage.start_of(layer) > start,
        "{layer}: the in edge at {edge:?} did not trim: reports {:?}, samples {samples:?}, selection {:?}",
        stage.recorder.reports.borrow(),
        stage
            .rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
    );
    stage.undo(cx);
    assert_eq!(stage.start_of(layer), start);

    let edge = edit_point(stage, layer, Some(Edge::Out));
    stage.drag(cx, edge, edge - point(px(30.), px(0.)), Modifiers::none());
    assert!(
        stage.out_of(layer) < out,
        "{layer}: the out edge did not trim: {:?}",
        stage.recorder.reports.borrow()
    );
    stage.undo(cx);
    assert_eq!(stage.out_of(layer), out);
}

#[gpui::test]
fn timeline_mcp_strips_move_and_trim_during_and_after_a_session(cx: &mut TestAppContext) {
    let mut mcp = Session::new(cx);
    mcp.start();
    let group = mcp.add_scene();
    mcp.refresh(cx);
    assert_strip_edits(&mut mcp.stage, "adjust", cx);
    press_timeline_keys(&mut mcp.stage, "u m u m", cx);
    assert_strip_edits(&mut mcp.stage, "adjust", cx);
    click_disclosure(&mut mcp.stage, &group, cx);
    mcp.refresh(cx);
    assert_strip_edits(&mut mcp.stage, "cursor", cx);
    mcp.call("finish_session", json!({"commit":true}));
    mcp.refresh(cx);
    assert_strip_edits(&mut mcp.stage, "adjust", cx);
    assert_strip_edits(&mut mcp.stage, "cursor", cx);
}

#[gpui::test]
fn timeline_mcp_saved_project_strips_move_and_trim(cx: &mut TestAppContext) {
    let Some(path) = std::env::var_os("PM_TIMELINE_MCP_PROJECT") else {
        return;
    };
    let opened =
        pm_kernel::open_project(&std::path::PathBuf::from(path), Default::default()).unwrap();
    let rig = crate::v4_view_tests::rig_source("timeline", pm_kernel::OpenSource::File(opened));
    let mut stage = stage_opened(cx, rig, true);
    let locked = stage.surface.read_with(&stage.vcx, |t, _| {
        let layers = &t.model().index.layers;
        (
            layers.len(),
            layers.iter().filter(|l| l.effectively_locked).count(),
        )
    });
    eprintln!(
        "SAVED_PROJECT_LOCKED layers={} locked={}",
        locked.0, locked.1
    );
    let target = stage.surface.read_with(&stage.vcx, |t, _| {
        t.model()
            .index
            .layers
            .iter()
            .find(|l| l.name.to_string_lossy() == "Adjustment Layer")
            .map(|l| l.id.clone())
    });
    let Some(target) = target else { return };
    stage
        .rig
        .kernel
        .call_blocking(
            PROJECT,
            Request::new(
                Caller::native(),
                Call::Selection(pm_kernel::SelectionCall::Layers {
                    ids: vec![target.clone()],
                    add: false,
                }),
            ),
        )
        .unwrap();
    stage
        .surface
        .update(&mut stage.vcx, |t, cx| t.published(cx));
    stage.idle(cx);
    assert_strip_edits(&mut stage, &target, cx);
}

fn press_select_all(stage: &mut Stage, cx: &mut TestAppContext) {
    stage.vcx.simulate_keystrokes("cmd-a");
    stage.idle(cx);
}

#[gpui::test]
fn timeline_mcp_select_all_keeps_disclosure_scroll_and_direct_sibling_scope(
    cx: &mut TestAppContext,
) {
    let mut project: Value = serde_json::from_str(&crate::v4_view_tests::project_json(30)).unwrap();
    let layers = project["layers"].as_array_mut().unwrap();
    layers[0]["group"] = json!("expanded");
    layers[1]["group"] = json!("expanded");
    layers[1]["lock"] = json!(true);
    layers[2]["group"] = json!("closed");
    layers[3]["group"] = json!("nested");
    layers[4]["lock"] = json!(true);
    layers.splice(0..0, [
        json!({"id":"expanded","name":"Expanded","type":"group","from":0,"dur":8,"collapsed":false}),
        json!({"id":"nested","name":"Nested","group":"expanded","type":"group","from":0,"dur":8,"collapsed":true}),
        json!({"id":"closed","name":"Closed","type":"group","from":0,"dur":8,"collapsed":true}),
    ]);
    let mut stage = stage_with(cx, project.to_string(), true);
    let read_ids = |stage: &Stage| {
        stage
            .rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
            .to_vec()
    };
    let at = stage.at(3, 1.);
    let scroll_by = |stage: &mut Stage, dy, cx: &mut TestAppContext| {
        stage.vcx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        stage.idle(cx);
    };
    scroll_by(&mut stage, -72., cx);
    let rows = stage
        .surface
        .read_with(&stage.vcx, |t, _| t.model().rows().len());
    let scroll = stage.surface.read_with(&stage.vcx, |t, _| t.scroll_y());
    assert!(scroll > 0., "the test should check a scrolled timeline");
    press_select_all(&mut stage, cx);
    assert_eq!(
        stage
            .surface
            .read_with(&stage.vcx, |t, _| t.model().rows().len()),
        rows
    );
    assert_eq!(
        stage.surface.read_with(&stage.vcx, |t, _| t.scroll_y()),
        scroll
    );
    assert_eq!(read_ids(&stage).len(), 27);
    assert!(
        !read_ids(&stage)
            .iter()
            .any(|id| matches!(id.as_str(), "l0" | "l1" | "l2" | "l3" | "l4"))
    );

    scroll_by(&mut stage, 72., cx);
    let name = bounds(&mut stage.vcx, &format!("timeline:name:{}", row_id("l0")));
    stage.vcx.simulate_click(name.center(), Modifiers::none());
    stage.idle(cx);
    press_select_all(&mut stage, cx);
    assert_eq!(read_ids(&stage), vec!["nested", "l0"]);
    assert!(!row_visible(&mut stage, "l3"));
}

#[gpui::test]
fn timeline_mcp_main3_select_all_can_move_a_strip_with_a_locked_nested_layer(
    cx: &mut TestAppContext,
) {
    let (Some(path), Ok(comp_name), Ok(locked_id)) = (
        std::env::var_os("PM_TIMELINE_MAIN3_PROJECT"),
        std::env::var("PM_TIMELINE_LOCKED_COMP_NAME"),
        std::env::var("PM_TIMELINE_LOCKED_LAYER"),
    ) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("main3.pmv");
    std::fs::copy(path, &copy).unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open(
            PROJECT,
            pm_kernel::OpenSource::File(
                pm_kernel::open_project(&copy, Default::default()).unwrap(),
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let publication = kernel.published(PROJECT).unwrap();
    let root = publication.snapshot.root().unwrap();
    let main3 = root
        .get("comps")
        .unwrap()
        .to_value()
        .as_object()
        .unwrap()
        .iter()
        .find_map(|(id, comp)| {
            (comp.get("name").and_then(pm_model::Value::as_str) == Some(comp_name.as_str()))
                .then(|| id.to_string_lossy())
        })
        .or_else(|| {
            (root
                .get("compName")
                .map(|name| name.to_value())
                .and_then(|name| name.as_str().map(str::to_owned))
                .as_deref()
                == Some(comp_name.as_str()))
            .then(|| pm_kernel::model::open_comp_id(root))
            .flatten()
        })
        .expect("fixture contains Main 3");
    kernel
        .call_blocking(
            PROJECT,
            Request::new(
                Caller::native(),
                Call::OpenComposition {
                    comp_id: main3,
                    document: publication.snapshot.doc_id,
                },
            ),
        )
        .unwrap();
    let rig = crate::v4_view_tests::rig_opened("timeline", actor);
    let mut stage = stage_opened(cx, rig, true);
    stage.surface.update(&mut stage.vcx, |t, cx| t.fit_time(cx));
    stage.idle(cx);

    let (target, locked_before, roots) = stage.surface.read_with(&stage.vcx, |t, _| {
        let index = &t.model().index;
        let locked = index
            .layer(&locked_id)
            .expect("fixture includes the nested locked layer");
        assert!(locked.effectively_locked && locked.group.is_some());
        let roots = index
            .layers
            .iter()
            .filter(|l| l.group.is_none() && !l.effectively_locked)
            .count();
        let frame = t.frame().unwrap();
        let target = frame
            .ops
            .iter()
            .filter_map(|op| {
                let id = op.id.strip_prefix("clip:")?;
                let l = index.layer(id)?;
                let x = op.bounds.x + op.bounds.width * 0.5;
                let y = op.bounds.y + op.bounds.height * 0.5;
                (l.group.is_none()
                    && !l.effectively_locked
                    && frame.hit_clip(x, y) == Some((id, None)))
                .then(|| id.to_owned())
            })
            .next()
            .expect("an unlocked visible top-level strip");
        (target, locked.from, roots)
    });
    press_select_all(&mut stage, cx);
    let before = stage.start_of(&target);
    let body = strip_point(&mut stage, &target, 0.5, 0.);
    stage.drag(cx, body, body + point(px(40.), px(0.)), Modifiers::none());
    assert!(
        stage.start_of(&target) > before,
        "the strip did not move after Command-A in Main 3"
    );
    assert_eq!(stage.start_of(&locked_id), locked_before);
    assert_eq!(
        stage
            .rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .selection
            .layers()
            .len(),
        roots
    );
}

#[gpui::test]
fn timeline_mcp_drag_recovers_when_the_mouse_up_is_lost(cx: &mut TestAppContext) {
    let mut stage = stage(cx, 2);
    let start = stage.start_of("l0");
    let body = strip_point(&mut stage, "l0", 0.5, 0.);
    let moved = body + point(px(40.), px(0.));
    stage
        .vcx
        .simulate_mouse_down(body, MouseButton::Left, Modifiers::none());
    stage
        .vcx
        .simulate_mouse_move(moved, MouseButton::Left, Modifiers::none());
    stage.idle(cx);
    stage.vcx.simulate_event(gpui::MouseMoveEvent {
        position: moved,
        pressed_button: None,
        modifiers: Modifiers::none(),
    });
    stage.idle(cx);
    assert_eq!(stage.start_of("l0"), start);
    assert_strip_edits(&mut stage, "l1", cx);
}

#[gpui::test]
fn timeline_mcp_single_locked_strip_explains_the_lock_and_row_unlocks_in_one_step(
    cx: &mut TestAppContext,
) {
    let mut project: Value = serde_json::from_str(&crate::v4_view_tests::project_json(1)).unwrap();
    project["layers"][0]["lock"] = json!(true);
    let mut stage = stage_with(cx, project.to_string(), true);
    let start = stage.start_of("l0");
    let body = strip_point(&mut stage, "l0", 0.5, 0.);
    stage.drag(cx, body, body + point(px(40.), px(0.)), Modifiers::none());
    assert_eq!(stage.start_of("l0"), start);
    assert!(
        stage
            .recorder
            .effects
            .borrow()
            .iter()
            .any(|effect| { format!("{effect:?}").starts_with("LockedStrip { layer: \"l0\",") }),
        "a blocked drag should offer Unlock"
    );
    let before = stage.rig.kernel.published(PROJECT).unwrap().history.entries;
    let lock = bounds(&mut stage.vcx, &format!("timeline:lock:{}", row_id("l0")));
    stage.vcx.simulate_click(lock.center(), Modifiers::none());
    settle(cx, |cx| stage.view.read_with(cx, |view, _| view.idle()));
    stage
        .surface
        .update(&mut stage.vcx, |t, cx| t.published(cx));
    stage.idle(cx);
    let published = stage.rig.kernel.published(PROJECT).unwrap();
    assert!(!pm_kernel::model::list_layers(published.snapshot.root().unwrap(), None)[0].locked);
    assert_eq!(published.history.entries, before + 1);
    stage.undo(cx);
    let published = stage.rig.kernel.published(PROJECT).unwrap();
    assert!(pm_kernel::model::list_layers(published.snapshot.root().unwrap(), None)[0].locked);
}
