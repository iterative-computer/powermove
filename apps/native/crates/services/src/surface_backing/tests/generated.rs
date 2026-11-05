use super::*;
use crate::surface_backing::generated::*;
struct Source {
    selected: String,
}
impl GeneratedSource for Source {
    fn layer_id(&self, _: &Session, target: &str) -> Result<String> {
        Ok(if target == "$selection" {
            self.selected.clone()
        } else {
            target.into()
        })
    }
    fn composition_command(&self, path: &str, value: Value) -> Result<Value> {
        if path != "composition.width" {
            return Err(unavailable("missing composition binding"));
        }
        Ok(json!({"type":"set_composition","patch":{"width":value}}))
    }
}
#[test]
fn generated_binding_precedence_is_source_then_local_then_parameter() {
    let c = json!({"type":"slider","label":"L","target":"$selection","path":"content.gain","stateKey":"local","param":"p"});
    assert_eq!(
        binding(&c).unwrap(),
        Binding::Source {
            target: "$selection".into(),
            path: "content.gain".into()
        }
    );
    assert_eq!(
        binding(&json!({"type":"color","label":"L","stateKey":"local","param":"p"})).unwrap(),
        Binding::Local("local".into())
    );
    assert_eq!(
        binding(&json!({"type":"curve","label":"L","param":"p"})).unwrap(),
        Binding::Parameter("p".into())
    );
    assert_eq!(
        binding(&json!({"type":"future","secret":1})).unwrap(),
        Binding::Unsupported("future".into())
    );
}
#[test]
fn selection_source_is_resolved_on_every_edit() {
    let b = Binding::Source {
        target: "$selection".into(),
        path: "transform.x".into(),
    };
    let mut source = Source {
        selected: "a".into(),
    };
    let first = edit_command(&source, &session(1), &b, json!(4), 3.).unwrap();
    source.selected = "b".into();
    let second = edit_command(&source, &session(1), &b, json!(4), 3.).unwrap();
    assert_eq!(first["target"], "a");
    assert_eq!(second["target"], "b");
    assert_eq!(second["path"], "x");
    assert_eq!(second["time"], 3.);
}
#[test]
fn composition_binding_does_not_fall_back_to_a_parameter_on_missing_support() {
    let source = Source {
        selected: "a".into(),
    };
    let b = Binding::Source {
        target: "composition".into(),
        path: "composition.background.angle".into(),
    };
    assert_eq!(
        edit_command(&source, &session(1), &b, json!(90), 0.)
            .unwrap_err()
            .code,
        ServiceErrorCode::Unavailable
    );
}
#[test]
fn all_generated_edit_routes_preserve_values() {
    let source = Source {
        selected: "a".into(),
    };
    for (path, kind, key) in [
        ("content.fill", "set_content", "fill"),
        ("layer.visible", "set_layer", "visible"),
        ("properties.x", "set_property", "x"),
    ] {
        let b = Binding::Source {
            target: "$selection".into(),
            path: path.into(),
        };
        let value = json!({"future":[true,2]});
        let cmd = edit_command(&source, &session(1), &b, value.clone(), 2.).unwrap();
        assert_eq!(cmd["type"], kind);
        if kind == "set_property" {
            assert_eq!(cmd["value"], value);
            assert_eq!(cmd["path"], key);
        } else {
            assert_eq!(cmd["patch"][key], value);
        }
    }
}
#[test]
fn manifest_arrays_cannot_smuggle_edit_authority() {
    let commands = strip_manifest_commands(&json!([{"type":"set_content","overrideLock":true,"preserveHandEdits":false,"markIntent":"human","future":7}])).unwrap();
    assert_eq!(commands[0], json!({"type":"set_content","future":7}));
    assert!(strip_manifest_commands(&json!([null])).is_err());
}
#[test]
fn button_modes_and_action_precedence_are_explicit() {
    for kind in ["script", "transform", "easing"] {
        let effect = button_effect(&json!({"action":{"type":kind,"mode":"preview","code":"never evaluated"},"commands":[{}]}),&json!({"x":1})).unwrap();
        assert!(matches!(effect, ButtonEffect::Tool { apply: false, .. }));
    }
    assert!(matches!(
        button_effect(&json!({"action":{"type":"reset"}}), &json!({})).unwrap(),
        ButtonEffect::Reset
    ));
    assert!(
        button_effect(
            &json!({"action":{"type":"future"},"cmd":"danger"}),
            &json!({})
        )
        .is_err()
    );
}
struct Host {
    fail: bool,
    saved: Vec<Value>,
}
impl GeneratedHost for Host {
    fn run(&mut self, _: &ToolRequest) -> Result<Value> {
        Err(unavailable("tool absent"))
    }
    fn save_local(&mut self, _: &str, state: &Value) -> Result<()> {
        if self.fail {
            return Err(failure());
        }
        self.saved.push(state.clone());
        Ok(())
    }
}
fn panel() -> GeneratedPanel {
    GeneratedPanel::new(json!({"id":"p","controls":[],"state":{"x":1,"future":[1,2]},"unknown":7}))
        .unwrap()
}
#[test]
fn generated_local_state_survives_failed_flush_and_reset() {
    let mut p = panel();
    let mut h = Host {
        fail: true,
        saved: vec![],
    };
    p.set_local("x", json!(2)).unwrap();
    assert!(p.flush_local(&mut h).is_err());
    h.fail = false;
    p.flush_local(&mut h).unwrap();
    assert_eq!(h.saved[0]["future"], json!([1, 2]));
    p.reset();
    p.flush_local(&mut h).unwrap();
    assert_eq!(h.saved[1]["x"], 1);
}
#[test]
fn changed_tool_state_or_session_rejects_stale_preview() {
    let mut p = panel();
    let control = json!({"action":{"type":"script","mode":"preview","code":"x"}});
    let request = p.begin(session(1), 4, &control).unwrap();
    p.set_local("x", json!(5)).unwrap();
    assert!(
        p.settle(&request, &session(1), 4, Ok(json!({"ok":true})))
            .is_err()
    );
    assert!(p.preview.is_none());
    let next = p.begin(session(1), 4, &control).unwrap();
    assert!(
        p.settle(&next, &session(2), 4, Ok(json!({"ok":true})))
            .is_err()
    );
}
#[test]
fn tool_failure_is_not_a_successful_preview_and_busy_is_guarded() {
    let mut p = panel();
    let control = json!({"action":{"type":"script","mode":"preview"}});
    let r = p.begin(session(1), 2, &control).unwrap();
    assert!(p.begin(session(1), 2, &control).is_err());
    assert_eq!(
        p.settle(&r, &session(1), 2, Err(failure()))
            .unwrap_err()
            .code,
        ServiceErrorCode::DiskFull
    );
    assert!(p.preview.is_none());
}
