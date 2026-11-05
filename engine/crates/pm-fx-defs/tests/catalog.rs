use pm_fx_defs::{Definition, builtins, catalog::*};
use serde_json::{Value, json};

#[test]
fn pinned_main_has_every_family_and_full_historical_revisions() {
    assert_eq!(main_effects().commit, MAIN_COMMIT);
    assert_eq!(main_effects().effects.len(), 43);
    assert_eq!(main_catalog().definitions().count(), 53);
    assert_eq!(main_catalog().current_definitions().count(), 43);
    assert_eq!(main_effects().blobs.len(), 22);
    let raw: Value = serde_json::from_str(include_str!("../src/main-effects.json")).unwrap();
    for original in raw["effects"].as_array().unwrap() {
        let parsed: Definition = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *original);
    }
    for (id, version, definition) in main_catalog().definitions() {
        assert_eq!(id, definition.id);
        assert_eq!(
            effect_version(definition.metadata.get("version")).unwrap(),
            version
        );
        assert!(!definition.frag.is_empty());
        let raw = serde_json::to_value(definition).unwrap();
        assert_eq!(
            serde_json::from_value::<Definition>(raw).unwrap(),
            *definition
        );
    }
    for id in [
        "tint",
        "channelMixer",
        "threshold",
        "shiftChannels",
        "exposure",
        "levelsIndividual",
        "unsharpMask",
        "findEdges",
        "tritone",
        "colorBalanceHLS",
        "brightnessContrast",
        "blackWhite",
        "setMatte",
        "displacementMap",
        "compoundBlur",
        "setChannels",
        "texturize",
        "stroke",
        "applyColorLut",
        "blend",
        "calculations",
        "channelCombiner",
        "colorProfileConverter",
        "hdrCompander",
        "echo",
    ] {
        assert!(main_catalog().current(id).is_some(), "{id}");
    }
}

#[test]
fn older_art_keeps_its_shader_and_parameter_schema() {
    for old in &builtins().effects {
        let resolved = main_catalog().instance(&old.id, None).unwrap();
        if old.id == "gradient" {
            // Main's own historical v1 was changed after the tag without a
            // version bump. Keep this discrepancy explicit (N3/N4).
            assert_ne!(resolved.frag, old.frag);
            assert!(resolved.frag.contains("u_layerUV"));
            assert!(old.frag.contains("u_coordRes.x"));
        } else {
            assert_eq!(resolved.frag, old.frag, "{} shader", old.id);
        }
        assert_eq!(resolved.defaults(), old.defaults(), "{} values", old.id);
        assert_eq!(resolved.metadata.get("passes"), old.metadata.get("passes"));
    }
    for id in [
        "blur",
        "levels",
        "pixelate",
        "shadow",
        "invert",
        "gradient",
        "displacementMap",
        "texturize",
        "stroke",
        "channelCombiner",
    ] {
        let old = main_catalog().get(id, 1).unwrap();
        let current = main_catalog().get(id, 2).unwrap();
        assert_ne!(old, current, "{id}");
        if id == "stroke" {
            assert_eq!(old.frag, current.frag);
            assert_ne!(old.params, current.params); // Brush geometry changed.
        } else {
            assert_ne!(old.frag, current.frag, "{id}");
        }
        assert_eq!(main_catalog().current(id), main_catalog().get(id, 2));
        assert!(main_catalog().get(id, 3).is_none());
        assert!(
            main_catalog()
                .instance(id, Some(&json!(3)))
                .unwrap_err()
                .0
                .contains("version 3")
        );
    }
}

#[test]
fn saved_versions_follow_javascript_safe_integer_contract_without_coercion() {
    for raw in [Value::Null, json!(1), json!(1.0)] {
        assert_eq!(effect_version(Some(&raw)), Ok(1));
    }
    for raw in [
        json!(0),
        json!(-1),
        json!(1.5),
        json!("1"),
        json!(false),
        json!([]),
        json!({}),
        json!(9_007_199_254_740_992_u64),
    ] {
        assert!(effect_version(Some(&raw)).is_err(), "{raw}");
    }
    assert_eq!(
        effect_version(Some(&json!(MAX_SAFE_VERSION))),
        Ok(MAX_SAFE_VERSION)
    );
    assert!(main_catalog().instance("missing-provider", None).is_err());
    assert_eq!(
        main_catalog()
            .instance("invert", Some(&Value::Null))
            .unwrap(),
        main_catalog().get("invert", 1).unwrap()
    );
}

#[test]
fn catalog_rejects_ambiguous_or_foreign_histories() {
    let raw = serde_json::to_value(main_catalog().current("invert").unwrap()).unwrap();
    let mut cases = Vec::new();
    for path in ["id", "version", "legacy"] {
        let mut altered = raw.clone();
        altered["legacy"][0][path] = match path {
            "id" => json!("other"),
            "version" => json!(2),
            _ => json!([]),
        };
        cases.push(altered);
    }
    let mut duplicate = raw.clone();
    duplicate["legacy"]
        .as_array_mut()
        .unwrap()
        .push(raw["legacy"][0].clone());
    cases.push(duplicate);
    let mut implicit = raw.clone();
    implicit["legacy"][0]
        .as_object_mut()
        .unwrap()
        .remove("version");
    cases.push(implicit);
    for bad in cases {
        let def: Definition = serde_json::from_value(bad).unwrap();
        assert!(EffectCatalog::new(vec![def]).is_err());
    }
    let def: Definition = serde_json::from_value(raw).unwrap();
    assert!(EffectCatalog::new(vec![def.clone(), def]).is_err());
}

#[test]
fn provider_history_is_not_borrowed_from_builtin_with_same_id() {
    let mut provider = main_catalog().get("invert", 1).unwrap().clone();
    provider.metadata.remove("version");
    provider.frag = "extension-authored replacement".into();
    let catalog = EffectCatalog::new(vec![provider.clone()]).unwrap();
    assert_eq!(catalog.instance("invert", None).unwrap(), &provider);
    assert!(catalog.instance("invert", Some(&json!(2))).is_err());
}

#[test]
fn source_sized_defaults_fall_back_without_clamping_authored_values() {
    let gradient = main_catalog().current("gradient").unwrap();
    let p = gradient
        .params
        .iter()
        .find(|p| p.presentation.contains_key("defaultFrom"))
        .unwrap();
    let ctx = &p.presentation["defaultFrom"];
    let dimension = if ctx["dimension"] == "width" {
        8192.
    } else {
        4096.
    };
    assert_eq!(
        parameter_default(p, 8192., 4096.),
        json!(dimension * ctx["factor"].as_f64().unwrap())
    );
    for (width, height) in [
        (f64::NAN, f64::NAN),
        (0., 0.),
        (-1., -1.),
        (f64::INFINITY, f64::INFINITY),
    ] {
        assert_eq!(parameter_default(p, width, height), p.def);
    }
    let tint = &main_catalog().current("tint").unwrap().params[0];
    assert_eq!(parameter_default(tint, 800., 600.), tint.def);
}

#[test]
fn changed_historical_gradient_requires_provenance_instead_of_silent_reinterpretation() {
    assert!(
        saved_definition("gradient", None, None)
            .unwrap_err()
            .0
            .contains("ambiguous")
    );
    let old = saved_definition("gradient", None, Some(ReferenceGeneration::V120)).unwrap();
    let main = saved_definition("gradient", None, Some(ReferenceGeneration::Main)).unwrap();
    assert_ne!(old.frag, main.frag);
    assert_eq!(old, builtins().effect("gradient").unwrap());
    assert!(saved_definition("gradient", Some(&json!(2)), None).is_ok());
    assert!(saved_definition("blur", None, None).is_ok());
}

#[test]
fn assembled_programs_are_keyed_by_implementation_revision_and_preserve_raw_sources() {
    use std::collections::BTreeSet;
    let shaders = &main_effects().shaders;
    assert_eq!(shaders.len(), 53);
    let mut keys = BTreeSet::new();
    for shader in shaders {
        assert!(keys.insert((&shader.id, shader.version)));
        let definition = main_catalog().get(&shader.id, shader.version).unwrap();
        if definition.metadata.get("rawShader") == Some(&json!(true)) {
            assert_eq!(shader.source, definition.frag);
        } else {
            assert!(shader.source.starts_with("#version 300 es\n"));
            assert!(shader.source.contains(&definition.frag));
            assert!(shader.source.contains("void main()"));
            for param in &definition.params {
                assert!(shader.source.contains(&format!("u_{}", param.k)));
            }
        }
    }
}

#[test]
fn typed_references_create_static_inputs_instead_of_animated_channels() {
    let channels = main_catalog().current("setChannels").unwrap();
    assert_eq!(channels.input_defaults().len(), 4);
    for key in ["redLayer", "greenLayer", "blueLayer", "alphaLayer"] {
        assert!(!channels.channels().contains_key(key));
        assert!(!channels.defaults().contains_key(key));
        assert_eq!(
            serde_json::to_value(&channels.input_defaults()[key]).unwrap(),
            json!({"layerId":null,"stage":"source"})
        );
    }
    let stroke = main_catalog().current("stroke").unwrap();
    assert!(!stroke.channels().contains_key("path"));
    assert_eq!(
        serde_json::to_value(stroke.input_defaults()["path"].clone()).unwrap(),
        json!({"kind":"path","layerId":null,"pathId":null})
    );
    let lut = main_catalog().current("applyColorLut").unwrap();
    assert!(!lut.channels().contains_key("lut"));
    assert_eq!(
        serde_json::to_value(lut.input_defaults()["lut"].clone()).unwrap(),
        json!({"kind":"asset","assetId":null})
    );
}
