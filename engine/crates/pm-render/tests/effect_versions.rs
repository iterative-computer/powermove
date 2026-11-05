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
