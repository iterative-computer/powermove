use super::*;
use gpui::{HeadlessAppContext, ParentElement, Styled};
use pm_compat_runner::host::{ExtensionHost, HostConfig};
use pm_kernel::{KernelActor, KernelConfig, OpenSource, Snapshot};
use pm_mcp::project_extensions::{Usage, library_roots, references};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

const SYNTHETIC: &str = r##"{
    "id":"film","name":"Synthetic Film","compId":"root","compName":"Main",
    "w":32,"h":32,"fps":30,"dur":2,"bg":"#000000","assets":{},"comps":{},
    "layers":[{
        "id":"plate","type":"solid","name":"Plate","on":true,"from":0,"dur":2,
        "blend":"normal","masks":[],
        "p":{"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]},
             "anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},
             "scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},
             "rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},
        "d":{"w":32,"h":32,"color":"#ffffff"},
        "fx":[{"id":"tint","type":"toy.tint","on":true,"p":{}}]
    }]
}"##;

fn write_extension(profile: &Path, version: &str, factor: f32) {
    let folder = profile.join("extensions/toy");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("manifest.json"),
        json!({
            "id":"toy","name":"Toy Tint","version":version,"apiVersion":3,
            "entry":"index.ts","contributes":["effects"]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(folder.join("index.ts"), format!(
        "export default function(api) {{ api.effects.register({{ id:'toy.tint', label:'Toy tint', group:'Test', params:[], frag:'o=vec4(vec3({factor:.2}), 1.0);' }}); }}"
    )).unwrap();
}
fn host(actor: &KernelActor, project: &str, profile: &Path, scope: ProjectScope) -> ExtensionHost {
    ExtensionHost::start_scoped(
        HostConfig {
            kernel: actor.handle(),
            project: project.into(),
            roots: library_roots(profile),
            cache: profile.join("extensions-build"),
            owner: None,
            text: None,
            wake: Arc::new(|| {}),
            frames: None,
            rasters: None,
            vars: None,
            clipboard: None,
        },
        scope,
    )
}
fn render(snapshot: &Snapshot, host: &ExtensionHost, apply_tint: bool) -> Vec<f32> {
    let (_, value) = host
        .wait_contributions(Duration::from_secs(30), &|| false)
        .unwrap();
    let mut renderer = pm_render::Renderer::headless().unwrap();
    renderer.set_isolate_layers(true);
    let definitions = crate::extensions::definitions(&value);
    assert_eq!(
        definitions.iter().any(|def| def.id.as_str() == "toy.tint"),
        apply_tint,
        "the synthetic effect did not register as expected"
    );
    let refused = renderer
        .set_document_effects(&snapshot.document, &definitions)
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    let frame = renderer
        .render(
            &snapshot.document,
            &snapshot.render_comp(),
            pm_protocol::common::Ticks::ZERO,
            &pm_render::RenderRequest::sized(
                pm_protocol::common::Size::new(32, 32),
                pm_protocol::color::Precision::Float16,
            ),
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

#[test]
fn project_extensions_keep_the_bundle_version_private_and_render_the_original_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let recipient = dir.path().join("recipient");
    write_extension(&source, "1.0.0", 0.40);
    write_extension(&recipient, "2.0.0", 0.75);
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = pm_model::json::parse(SYNTHETIC).unwrap();
    let kernel = actor.handle();
    kernel
        .open("writer", OpenSource::Untitled(project.clone()))
        .wait()
        .unwrap()
        .unwrap();
    let snapshot = &kernel.published("writer").unwrap().snapshot;
    let original = host(&actor, "writer", &source, Default::default());
    let original_pixels = render(snapshot, &original, true);
    assert!(
        original_pixels.chunks_exact(4).any(|pixel| pixel[0] > 0.0),
        "the synthetic plate did not draw"
    );
    let bundle = Bundle::collect(
        references(snapshot.root().unwrap()),
        &installed(&library_roots(&source)),
        &BTreeMap::new(),
        builtin_definitions(),
        &BTreeSet::new(),
        true,
    );
    let path = dir.path().join("Synthetic.pmv");
    pm_builtins::delivery::portable_with_extensions(
        snapshot,
        &BTreeMap::new(),
        &path,
        false,
        &AtomicBool::new(false),
        |plan| bundle.add_to(plan),
    )
    .unwrap();

    let opened = pm_kernel::project_io::open_project(&path, Default::default()).unwrap();
    let extra = opened.file.envelope.extra.clone();
    kernel
        .open("received", OpenSource::File(opened))
        .wait()
        .unwrap()
        .unwrap();
    let received = &kernel.published("received").unwrap().snapshot;
    let empty_profile = dir.path().join("fresh");
    let prepared = Open::prepare(&extra, &empty_profile);
    assert_eq!(prepared.frame().unwrap().default_action(), Some("project"));
    let (scope, errors) = prepared.install(Choice::Project, &empty_profile, "Synthetic Film");
    assert!(errors.is_empty());
    let accepted = host(&actor, "received", &empty_profile, scope);
    assert!(render(received, &accepted, true) == original_pixels);
    assert!(!empty_profile.join("extensions").exists());
    drop(accepted);

    // A different installed version stays intact and renders with its own
    // pixels even after this project installs its private version.
    kernel
        .open("library", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let library = host(&actor, "library", &recipient, Default::default());
    let library_snapshot = &kernel.published("library").unwrap().snapshot;
    let library_pixels = render(library_snapshot, &library, true);
    assert!(
        library_pixels != original_pixels,
        "the versions produced identical pixels"
    );
    let (scope, errors) =
        Open::prepare(&extra, &recipient).install(Choice::Project, &recipient, "Synthetic Film");
    assert!(errors.is_empty());
    let local = host(&actor, "received", &recipient, scope);
    assert!(render(received, &local, true) == original_pixels);
    assert!(render(library_snapshot, &library, true) == library_pixels);
    assert_eq!(
        installed(&library_roots(&recipient))["toy"]
            .manifest
            .as_ref()
            .unwrap()["version"],
        "2.0.0"
    );
    drop(local);

    let (scope, _) =
        Open::prepare(&extra, &recipient).install(Choice::Without, &recipient, "Synthetic Film");
    assert!(scope.excluded.contains("toy"));
    let bypass = host(&actor, "received", &recipient, scope);
    assert!(render(received, &bypass, false) != original_pixels);
    assert!(
        references(received.root().unwrap())
            .iter()
            .any(|usage| usage.definition == "toy.tint")
    );

    let (_, errors) =
        Open::prepare(&extra, &recipient).install(Choice::Library, &recipient, "Synthetic Film");
    assert!(errors.is_empty());
    assert!(Open::prepare(&extra, &recipient).frame().is_none());
    assert_eq!(
        fs::read_to_string(recipient.join("extensions/toy").join(ORIGIN)).unwrap(),
        r#"{"project":"Synthetic Film"}"#
    );
}

fn sample_entry(id: &str, name: &str, version: &str, bytes: usize) -> Entry {
    Entry {
        id: id.into(),
        name: name.into(),
        version: version.into(),
        author: "Synthetic Studio".into(),
        source: "local".into(),
        store_id: None,
        hash: None,
        bytes,
        items: vec![Usage {
            composition: "root".into(),
            layer: "plate".into(),
            kind: "effect".into(),
            definition: format!("{id}.fx"),
        }],
        status: Status::Included,
    }
}

#[test]
fn project_extensions_leave_builtin_3d_out_of_portable_exports() {
    let items = pm_protocol::definitions::BUILTIN_3D_IDS
        .into_iter()
        .map(|definition| Usage {
            composition: "root".into(),
            layer: definition.into(),
            kind: "layer".into(),
            definition: definition.into(),
        })
        .collect();
    let bundle = Bundle::collect(
        items,
        &BTreeMap::new(),
        &BTreeMap::new(),
        builtin_definitions(),
        &BTreeSet::new(),
        true,
    );
    assert!(bundle.entries.is_empty(), "{:?}", bundle.entries);
}

#[test]
fn project_extensions_open_old_builtin_3d_bundles_without_warning() {
    let profile = tempfile::tempdir().unwrap();
    let mut builtin = sample_entry("powermove", "Powermove", "", 0);
    builtin.status = Status::Unavailable;
    builtin.items[0].kind = "layer".into();
    builtin.items[0].definition = "powermove.3d.object".into();
    let prepare = |entry: &Entry| {
        let extra = json!({"extensions/manifest": {"version": 1, "entries": [entry]}});
        let extra = pm_model::json::parse(&extra.to_string()).unwrap();
        Open::prepare(extra.as_object().unwrap(), profile.path())
    };
    assert!(prepare(&builtin).frame().is_none());

    // A stale entry can be ignored only if every reference is builtin.
    builtin.items.extend(sample_entry("toy", "Toy", "", 0).items);
    assert!(prepare(&builtin).frame().is_some());
}

#[gpui::test]
fn project_extensions_each_choice_dismisses_the_sheet(cx: &mut gpui::TestAppContext) {
    let parent = cx.add_window(|_, _| gpui::EmptyView).into();
    let mut app = cx.update(|cx| cx.to_async());
    let frame = Open {
        bundle: Bundle::default(),
        required: vec![sample_entry("toy", "Toy", "1.0", 0)],
        excluded: BTreeSet::new(),
        warnings: vec![],
    }
    .frame()
    .unwrap();
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());

    for (button, choice) in [
        ("welcome-without", Choice::Without),
        ("welcome-library", Choice::Library),
        ("welcome-project", Choice::Project),
    ] {
        let reply = sheet(frame.clone(), tokens.clone(), parent, &mut app).unwrap();
        let dialog = cx.windows().into_iter().find(|&id| id != parent).unwrap();
        let mut view = gpui::VisualTestContext::from_window(dialog, cx);
        let at = view.debug_bounds(button).expect("visible sheet button").center();
        view.simulate_click(at, gpui::Modifiers::none());
        assert_eq!(reply.try_recv().unwrap(), choice);
        assert_eq!(cx.windows(), vec![parent], "{button} left the sheet open");
    }
}

fn finish(ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>) {
    crate::ticket::answer(ticket, Instant::now() + Duration::from_secs(30))
        .unwrap()
        .unwrap();
}

#[test]
#[ignore = "writes six synthetic review captures"]
fn capture_project_extensions_sheets() {
    let out = PathBuf::from(std::env::var_os("PM_PROJECT_EXTENSION_CAPTURE").unwrap());
    fs::create_dir_all(&out).unwrap();
    let entries = vec![
        sample_entry("corner-pin-snap", "Corner Pin Snap", "1.0", 6144),
        sample_entry("gradient-tint", "Gradient Tint", "2.1", 13280),
    ];
    let open = Open {
        bundle: Bundle::default(),
        required: entries.clone(),
        excluded: BTreeSet::new(),
        warnings: vec![],
    }
    .frame()
    .unwrap();
    for (name, scheme) in [
        ("light", pm_design::Scheme::Light),
        ("dark", pm_design::Scheme::Dark),
    ] {
        let tokens = pm_design::Tokens::new(scheme, pm_design::Accessibility::default());
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
        let width = px(tokens.metrics.sheet_width_wide);
        let open_window = cx
            .open_window(size(width, px(360.)), {
                let tokens = tokens.clone();
                let open = open.clone();
                move |_, cx| {
                    cx.new(|cx| {
                        pm_shell::welcome_view::WelcomeView::new(
                            tokens,
                            open,
                            Rc::new(|_, _| {}),
                            cx,
                        )
                    })
                }
            })
            .unwrap();
        cx.run_until_parked();
        cx.capture_screenshot(open_window.into())
            .unwrap()
            .save(out.join(format!("open-{name}.png")))
            .unwrap();

        let mut project: serde_json::Value = serde_json::from_str(SYNTHETIC).unwrap();
        project["exportDefaults"] = json!({"format":"json","scale":1,"fps":30,"range":"all",
            "quality":"high","mblur":false,"audio":false,"alpha":false});
        let rig = crate::v4_view_tests::rig_project("export", &project.to_string());
        rig.runtime.set_export_extensions(json!(entries));
        finish(rig.runtime.show("export").unwrap());
        let publication = crate::v4_view_tests::publication(&rig);
        let button = publication
            .surface
            .nodes()
            .values()
            .find(|node| {
                node.listeners.contains_key("click")
                    && node.props.values().any(|value| {
                        value.as_str() == Some("Includes 2 extensions needed to render")
                    })
            })
            .expect("extension disclosure in the Project export sheet");
        let button_id = button.id;
        let generation = publication.generation;
        let revision = publication.surface.revision();

        let runtime = rig.runtime.clone();
        let panel_color = tokens.colors.panel;
        let export_window = cx
            .open_window(size(width, px(445.)), |window, cx| {
                let view = cx.new(|cx| {
                    crate::v4_view::SurfaceView::new(
                        runtime,
                        publication,
                        tokens,
                        Default::default(),
                        window,
                        cx,
                    )
                });
                cx.new(|_| ExportCapture { view, panel_color })
            })
            .unwrap();
        cx.run_until_parked();
        cx.capture_screenshot(export_window.into())
            .unwrap()
            .save(out.join(format!("export-collapsed-{name}.png")))
            .unwrap();

        finish(
            rig.runtime
                .event(
                    "export",
                    generation,
                    revision,
                    button_id,
                    "click",
                    pm_surface::events::EventDetail::default(),
                )
                .unwrap(),
        );
        let expanded = crate::v4_view_tests::publication(&rig);
        cx.update(|cx| {
            let frame = export_window.root(cx).unwrap();
            let surface = frame.read(cx).view.clone();
            surface.update(cx, |view, cx| view.publish(expanded, cx));
        });
        cx.run_until_parked();
        cx.capture_screenshot(export_window.into())
            .unwrap()
            .save(out.join(format!("export-expanded-{name}.png")))
            .unwrap();
    }
}

struct ExportCapture {
    view: gpui::Entity<crate::v4_view::SurfaceView>,
    panel_color: u32,
}
impl gpui::Render for ExportCapture {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .size_full()
            .bg(gpui::rgb(self.panel_color))
            .font_family(pm_design::TEXT_FONT)
            .child(self.view.clone())
    }
}
