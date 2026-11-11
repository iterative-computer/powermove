//! Saved-definition pixel identity on the real headless Metal renderer.
mod support;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::{doc::*, ticks};
fn doc(ty: &str, version: Option<u32>) -> pm_model::Document {
    let mut chip = layer(
        "chip",
        "solid",
        json!({"w":16,"h":16,"color":"#4080C0"}),
        0.,
        0.,
        1.,
    );
    let mut fx = json!({"id":"f","type":ty,"on":true,"p":{}});
    if let Some(version) = version {
        fx["version"] = json!(version);
    }
    chip["fx"] = json!([fx]);
    load(&project(16, 16, 1., Value::Null, vec![chip]))
}
fn draw(r: &mut Renderer, doc: &pm_model::Document) -> Vec<f32> {
    let f = r
        .render(
            doc,
            "Ptest-root",
            ticks(0.),
            &RenderRequest::sized(Size::new(16, 16), Precision::Float16),
            &MapResolver::default(),
        )
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    r.read_rgba(&f).unwrap()
}
#[test]
fn unversioned_and_explicit_tag_effects_keep_every_pixel() {
    let mut renderer = Renderer::headless().unwrap();
    for ty in ["invert", "levels", "gradient", "blur", "shadow"] {
        let original = doc(ty, None);
        renderer
            .set_definitions(&pm_render::all_builtin_definitions())
            .unwrap();
        let expected = draw(&mut renderer, &original);
        for version in [None, Some(1)] {
            for app in [None, Some("1.2.0"), Some("1.0.0")] {
                let mut saved = doc(ty, version);
                saved.set_saving_app_version(app.map(str::to_owned));
                renderer.set_main_effects_for_document(&saved).unwrap();
                assert_eq!(
                    expected,
                    draw(&mut renderer, &saved),
                    "{ty} {version:?} {app:?}"
                );
            }
        }
    }
}
#[test]
fn current_invert_version_uses_its_own_program_and_unavailable_versions_fail() {
    let mut renderer = Renderer::headless().unwrap();
    let mut saved = doc("invert", Some(2));
    saved.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&saved).unwrap();
    let pixels = draw(&mut renderer, &saved);
    let center = &pixels[(8 * 16 + 8) * 4..][..4];
    assert_eq!(center, [767. / 1024., 255. / 512., 253. / 1024., 1.]);
    let missing = doc("invert", Some(999));
    renderer.set_main_effects_for_document(&missing).unwrap();
    assert!(
        renderer
            .render(
                &missing,
                "Ptest-root",
                ticks(0.),
                &RenderRequest::sized(Size::new(16, 16), Precision::Float16),
                &MapResolver::default()
            )
            .is_err()
    );
}

#[test]
fn explicit_continuous_raster_switch_renders_the_precomp_at_display_density() {
    let child = layer(
        "inner",
        "solid",
        json!({"w":16,"h":16,"color":"#FFFFFF"}),
        0.,
        0.,
        1.,
    );
    let mut pre = layer(
        "pre",
        "precomp",
        json!({"comp":"child","w":16,"h":16}),
        0.,
        0.,
        1.,
    );
    set(&mut pre, "scale.x", json!(400));
    set(&mut pre, "scale.y", json!(400));
    pre["continuousRaster"] = json!(true);
    let mut project = project(64, 64, 1., no_fill(), vec![pre]);
    composition(&mut project, "child", 16, 16, 1., vec![child]);
    let mut renderer = Renderer::headless().unwrap();
    let document = load(&project);
    let f = renderer
        .render(
            &document,
            "Ptest-root",
            ticks(0.),
            &RenderRequest::sized(Size::new(64, 64), Precision::Float16),
            &MapResolver::default(),
        )
        .unwrap();
    assert!(f.failures.is_empty());
    let pixels = renderer.read_rgba(&f).unwrap();
    assert!(pixels.chunks_exact(4).all(|p| p == [1., 1., 1., 1.]));
    let plan = renderer
        .plan(
            &document,
            "Ptest-root",
            ticks(0.),
            &RenderRequest::sized(Size::new(64, 64), Precision::Float16),
            &MapResolver::default(),
        )
        .unwrap();
    let (plan, failures) = plan;
    assert!(failures.is_empty());
    let content = match &plan.draws[0].content {
        pm_render::plan::Content::Placed(p) => &p.content,
        content => content,
    };
    let pm_render::plan::Content::Texture {
        source: pm_render::plan::Source::Comp(child),
        ..
    } = content
    else {
        panic!("expected precomp texture: {:?}", plan.draws[0]);
    };
    assert_eq!((child.width, child.height), (64, 64));
}

#[test]
fn current_color_defaults_bind_named_parameters_and_preserve_identity_pixels() {
    let mut renderer = Renderer::headless().unwrap();
    for id in [
        "channelMixer",
        "shiftChannels",
        "exposure",
        "levelsIndividual",
        "colorBalanceHLS",
        "brightnessContrast",
        "arithmetic",
    ] {
        let document = doc(id, Some(1));
        renderer.set_main_effects_for_document(&document).unwrap();
        let pixels = draw(&mut renderer, &document);
        assert_eq!(
            &pixels[(8 * 16 + 8) * 4..][..4],
            [257. / 1024., 257. / 512., 771. / 1024., 1.],
            "{id}"
        );
    }
}

#[test]
fn current_gradient_uses_layer_pixels_without_reinterpreting_the_tag_revision() {
    let source = doc("gradient", Some(2));
    let mut value: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &source.to_value().unwrap(),
    ))
    .unwrap();
    value["layers"][0]["fx"][0]["p"] = json!({"startX":ch(json!(0)),"startY":ch(json!(0)),"endX":ch(json!(16)),"endY":ch(json!(0))});
    let document =
        pm_model::Document::new(Some(pm_model::json::parse(&value.to_string()).unwrap()), 1);
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_main_effects_for_document(&document).unwrap();
    let pixels = draw(&mut renderer, &document);
    assert_eq!(
        &pixels[(8 * 16 + 8) * 4..][..4],
        [0.53125, 0.53125, 0.53125, 1.]
    );
}

/// A fork that wins a built-in id draws every saved version 1 instance with
/// its own program, as the app that made the project did; a later saved
/// revision has no provider and is refused, never drawn by the built-in.
#[test]
fn a_winning_fork_draws_saved_version_one_and_refuses_later_revisions() {
    use pm_protocol::{common::DefinitionId, definitions::DefinitionOrigin};
    let mut renderer = Renderer::headless().unwrap();
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .unwrap();
    let inverted = draw(&mut renderer, &doc("invert", None));
    let mut fork = pm_render::all_builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    fork.id = DefinitionId::new("gradient");
    fork.origin = DefinitionOrigin::Fork;
    for version in [None, Some(1)] {
        let saved = doc("gradient", version);
        let refused = renderer
            .set_document_effects(&saved, std::slice::from_ref(&fork))
            .unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(inverted, draw(&mut renderer, &saved), "{version:?}");
    }
    let later = doc("gradient", Some(2));
    renderer
        .set_document_effects(&later, std::slice::from_ref(&fork))
        .unwrap();
    let frame = renderer.render(
        &later,
        "Ptest-root",
        ticks(0.),
        &RenderRequest::sized(Size::new(16, 16), Precision::Float16),
        &MapResolver::default(),
    );
    assert!(
        frame.is_err() || !frame.unwrap().failures.is_empty(),
        "a saved revision the fork never had must not draw"
    );
}

/// An extension layer whose extension isn't installed draws today's
/// missing-definition checkerboard where frames show the project (the
/// viewer, agent stills: today's compositor draws it in every frame), and an
/// export refuses it, as it refuses an uninstalled effect.
#[test]
fn an_extension_layer_with_no_installed_provider_is_the_checkerboard_in_the_viewer_and_refused_in_exports()
 {
    let mut renderer = Renderer::headless().unwrap();
    let field = layer(
        "field",
        "extension",
        json!({"definition":"acme.field.layer","version":1,"w":16,"h":16,"data":{},"params":{}}),
        0.,
        0.,
        1.,
    );
    let mut absent = load(&project(16, 16, 1., Value::Null, vec![field]));
    absent.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&absent).unwrap();
    let request = RenderRequest::sized(Size::new(16, 16), Precision::Float16);
    let export = renderer.render(
        &absent,
        "Ptest-root",
        ticks(0.),
        &request,
        &MapResolver::default(),
    );
    assert!(export.is_err(), "an export never omits a layer silently");
    renderer.set_isolate_layers(true);
    let shown = draw(&mut renderer, &absent);
    renderer.set_isolate_layers(false);
    // `mix(vec3(.055,.05,.07), vec3(.16,.09,.15), checker)`, opaque.
    let cells = [[0.055, 0.05, 0.07], [0.16, 0.09, 0.15]];
    for (x, y) in [(2, 2), (8, 8), (13, 5)] {
        let px = &shown[(y * 16 + x) * 4..][..4];
        assert!((px[3] - 1.).abs() < 1e-3, "{px:?}");
        assert!(
            cells.iter().any(|c| c.iter().zip(px).all(|(w, g)| (w - g).abs() < 0.02)),
            "({x}, {y}) {px:?}"
        );
    }
}
#[test]
fn an_effect_with_no_installed_provider_is_bypassed_in_the_viewer_and_refused_in_exports() {
    let mut renderer = Renderer::headless().unwrap();
    let plain = {
        let mut d = doc("invert", None);
        d.set_saving_app_version(Some("1.3.0".into()));
        d
    };
    let mut missing = doc("acme.glow", None);
    missing.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&missing).unwrap();
    assert_eq!(
        renderer.missing_effects(),
        [pm_render::effects::versions::MissingEffect {
            effect: "acme.glow".into(),
            revision: None,
            layers: 1
        }]
    );
    let request = RenderRequest::sized(Size::new(16, 16), Precision::Float16);
    let export = renderer.render(
        &missing,
        "Ptest-root",
        ticks(0.),
        &request,
        &MapResolver::default(),
    );
    assert!(export.is_err(), "an export never omits an effect silently");
    renderer.set_isolate_layers(true);
    let bypassed = draw(&mut renderer, &missing);
    renderer.set_isolate_layers(false);
    // The layer draws without the effect: the chip's own colour.
    let center = &bypassed[(8 * 16 + 8) * 4..][..4];
    let chip = [64. / 255., 128. / 255., 192. / 255., 1.];
    for (got, want) in center.iter().zip(chip) {
        assert!((got - want).abs() < 2e-3, "{center:?}");
    }
    renderer.set_main_effects_for_document(&plain).unwrap();
    assert!(renderer.missing_effects().is_empty());
    // Switched off, or in a composition nothing draws: it never renders.
    let mut chip = layer(
        "chip",
        "solid",
        json!({"w":16,"h":16,"color":"#4080C0"}),
        0.,
        0.,
        1.,
    );
    chip["fx"] = json!([{"id":"f","type":"acme.glow","on":false,"missing":true,"p":{}}]);
    let mut project = project(16, 16, 1., Value::Null, vec![chip.clone()]);
    chip["fx"][0]["on"] = json!(true);
    project["comps"] = json!({"unused":{"id":"unused","name":"Unused","w":16,"h":16,"fps":30,"dur":1,"layers":[chip]}});
    let quiet = load(&project);
    renderer.set_main_effects_for_document(&quiet).unwrap();
    assert!(renderer.missing_effects().is_empty());
    assert!(renderer.missing_effects_in(&quiet, "Ptest-root").is_empty());
    assert_eq!(renderer.missing_effects_in(&quiet, "unused").len(), 1);
}

/// A revision a newer Powermove saved (current main writes revisions this
/// build may not have): the viewer draws the layer without that effect and
/// names it; an export refuses; the saved effect is never rewritten.
#[test]
fn a_saved_revision_this_build_lacks_is_bypassed_and_named_in_the_viewer_and_refused_in_exports() {
    let mut renderer = Renderer::headless().unwrap();
    let mut newer = doc("invert", Some(999));
    newer.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&newer).unwrap();
    assert_eq!(
        renderer.missing_effects(),
        [pm_render::effects::versions::MissingEffect {
            effect: "invert".into(),
            revision: Some("999".into()),
            layers: 1
        }]
    );
    assert_eq!(
        renderer.missing_effects_in(&newer, "Ptest-root"),
        renderer.missing_effects()
    );
    let request = RenderRequest::sized(Size::new(16, 16), Precision::Float16);
    assert!(
        renderer
            .render(
                &newer,
                "Ptest-root",
                ticks(0.),
                &request,
                &MapResolver::default()
            )
            .is_err(),
        "an export never omits an effect silently"
    );
    renderer.set_isolate_layers(true);
    let bypassed = draw(&mut renderer, &newer);
    renderer.set_isolate_layers(false);
    let center = &bypassed[(8 * 16 + 8) * 4..][..4];
    let chip = [64. / 255., 128. / 255., 192. / 255., 1.];
    for (got, want) in center.iter().zip(chip) {
        assert!((got - want).abs() < 2e-3, "{center:?}");
    }
    // A revision this build has draws as before and names nothing.
    let mut current = doc("invert", Some(2));
    current.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&current).unwrap();
    assert!(renderer.missing_effects().is_empty());
}
