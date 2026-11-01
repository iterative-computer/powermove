use pm_fx_defs::{DefaultContext, Definition, PropertyChannel, builtins};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[test]
fn every_definition_round_trips_without_losing_metadata_or_shader_source() {
    let raw: Value = serde_json::from_str(include_str!("../src/builtins.json")).unwrap();
    assert_eq!(builtins().effects.len(), 16);
    assert_eq!(builtins().transitions.len(), 4);
    for key in ["effects", "transitions"] {
        for source in raw[key].as_array().unwrap() {
            let def: Definition = serde_json::from_value(source.clone()).unwrap();
            assert_eq!(serde_json::to_value(&def).unwrap(), *source);
            assert!(!def.frag.is_empty());
            let unique: BTreeSet<_> = def.params.iter().map(|p| &p.k).collect();
            assert_eq!(unique.len(), def.params.len());
            assert_eq!(def.defaults().len(), def.params.len());
            for p in &def.params {
                let channels = def.channels();
                assert_eq!(channels[&p.k].v, p.def);
                assert!(channels[&p.k].kf.is_empty());
                assert_eq!(channels[&p.k].expr, Value::Null);
                assert_eq!(def.default_for(&p.k), Some(&p.def));
            }
            assert_eq!(def.default_for("notRegistered"), None);
        }
    }
}

#[test]
fn defaults_match_shipped_effects_and_typed_parameters() {
    let b = builtins();
    assert_eq!(b.effect("blur").unwrap().defaults()["amount"], 24);
    assert_eq!(b.effect("duotone").unwrap().defaults()["shadow"], "#1B2A4A");
    assert_eq!(b.effect("gradient").unwrap().defaults()["radial"], false);
    assert_eq!(b.transition("wipe").unwrap().defaults()["softness"], 8);
    assert!(b.effect("notInstalled").is_none());
    assert!(b.transition("notInstalled").is_none());
}

#[test]
fn reset_context_uses_layer_type_and_owning_composition() {
    let ctx = DefaultContext {
        width: 4096.0,
        height: 2160.0,
    };
    let b = builtins();
    assert_eq!(
        b.channel_default("text", "position.x", ctx),
        Some(json!(2048.0))
    );
    assert_eq!(
        b.channel_default("text", "position.y", ctx),
        Some(json!(1080.0))
    );
    assert_eq!(
        b.channel_default("solid", "position.x", ctx),
        Some(json!(0))
    );
    assert_eq!(b.channel_default("solid", "c.w", ctx), Some(json!(4096.0)));
    assert_eq!(b.channel_default("image", "c.w", ctx), Some(json!(1920)));
    assert_eq!(b.channel_default("null", "anchor.x", ctx), Some(json!(50)));
    assert_eq!(b.channel_default("null", "opacity", ctx), Some(json!(0)));
    assert_eq!(
        b.channel_default("group", "position.x", ctx),
        Some(json!(0))
    );
    assert_eq!(b.channel_default("text", "c.boxWidth", ctx), Some(json!(0)));
    assert_eq!(
        b.channel_default("text", "perspective", ctx),
        Some(json!(50))
    );
    assert!(b.channel_default("audio", "position.x", ctx).is_none());
    assert!(
        b.channel_default("extensionType", "position.x", ctx)
            .is_none()
    );
    assert!(b.channel_default("text", "fontAxis.fake", ctx).is_none());
}

#[test]
fn fresh_channels_are_isolated_and_extension_definitions_supply_their_own_defaults() {
    let b = builtins();
    let mut definition = b.effect("blur").unwrap().clone();
    definition.params[0].def = json!(99);
    let mut fresh = definition.channels();
    fresh
        .get_mut("amount")
        .unwrap()
        .kf
        .push(json!({"t":1,"v":6}));
    assert_eq!(
        definition.channels()["amount"],
        PropertyChannel::new(json!(99))
    );
    assert_eq!(
        b.effect("blur").unwrap().default_for("amount"),
        Some(&json!(24))
    );
    assert_eq!(
        serde_json::to_value(PropertyChannel::new(json!(true))).unwrap(),
        json!({"v":true,"kf":[],"expr":null})
    );
}

#[test]
fn reset_uses_the_explicit_inspector_contract_instead_of_guessing_factory_values() {
    let b = builtins();
    let ctx = DefaultContext {
        width: 2048.0,
        height: 1152.0,
    };
    assert_eq!(
        b.reset_default("solid", "position.x", ctx),
        Some(json!(1024.0))
    );
    assert_eq!(
        b.channel_default("solid", "position.x", ctx),
        Some(json!(0))
    );
    assert_eq!(
        b.reset_default("adjustment", "position.y", ctx),
        Some(json!(0))
    );
    assert_eq!(b.reset_default("group", "position.x", ctx), Some(json!(0)));
    assert_eq!(b.reset_default("null", "anchor.x", ctx), Some(json!(50)));
    assert_eq!(b.reset_default("text", "c.text", ctx), None);
    assert_eq!(b.reset_default("audio", "position.x", ctx), None);
}

#[test]
fn unavailable_composition_dimensions_cannot_become_null_channel_values() {
    let ctx = DefaultContext {
        width: f64::NAN,
        height: f64::INFINITY,
    };
    assert_eq!(builtins().channel_default("text", "position.x", ctx), None);
    assert_eq!(builtins().reset_default("text", "position.y", ctx), None);
    assert_eq!(
        builtins().channel_default("text", "opacity", ctx),
        Some(json!(100))
    );
}
