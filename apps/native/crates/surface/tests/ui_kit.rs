use accesskit::{Action, ActionData, ActionRequest, NodeId, Rect, Role, Toggled, TreeId};
use pm_design::{Accessibility, Material, Scheme, Tokens};
use pm_surface::{
    Commit, Op, Props, Surface,
    binding::{BindingSink, Bindings, Edit},
    controls::Controls,
    kit::*,
};
use serde_json::{Value, json};
use std::sync::Arc;
fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
fn props(value: Value) -> Props {
    value.as_object().unwrap().clone()
}
fn surface(kind: Component) -> Surface {
    let mut s = Surface::default();
    let primitive = match kind {
        Component::NumberField | Component::TextField => "input",
        Component::PopoverAnchor | Component::SheetAnchor => "anchored",
        _ => "div",
    };
    let mut p = props(
        json!({"data-pm-component": serde_json::to_value(kind).unwrap(), "label":"Authored label",
        "style":{"minWidth":{"token":"metrics.hit"},"minHeight":{"token":"metrics.hit"}}}),
    );
    if kind.interactive() {
        p.insert("tabIndex".into(), json!(0));
    }
    if kind == Component::NumberField {
        p.extend(props(
            json!({"control":"number", "value":10., "binding":"property", "step":2., "unit":"px"}),
        ));
    }
    if kind == Component::TextField {
        p.extend(props(
            json!({"control":"text", "value":"Exact text", "binding":"property"}),
        ));
    }
    if matches!(kind, Component::Checkbox | Component::Toggle) {
        p.extend(props(
            json!({"control":"toggle", "value":true, "binding":"property"}),
        ));
    }
    if matches!(kind, Component::PopoverAnchor | Component::SheetAnchor) {
        p.insert("anchor".into(), json!(0));
    }
    s.apply(Commit {
        base_revision: 0,
        ops: vec![
            Op::Create {
                id: 1,
                r#type: primitive.into(),
            },
            Op::SetProps { id: 1, patch: p },
            Op::InsertBefore {
                parent: 0,
                id: 1,
                before: None,
            },
        ],
    })
    .unwrap();
    s
}
fn patch(s: &mut Surface, p: Value) {
    s.apply(Commit {
        base_revision: s.revision(),
        ops: vec![Op::SetProps {
            id: 1,
            patch: props(p),
        }],
    })
    .unwrap();
}
fn check_component(kind: Component, expected_role: Role, widget: Widget) {
    let s = surface(kind);
    let panel = Panel::prepare(Arc::new(s), &tokens(), &Layout::new(), Some(1), false);
    assert!(panel.diagnostics.is_empty(), "{:?}", panel.diagnostics);
    assert_eq!(panel.components[&1].widget, widget);
    let node = &panel
        .accessibility
        .nodes
        .iter()
        .find(|n| n.0 == NodeId(1))
        .unwrap()
        .1;
    assert_eq!(node.role(), expected_role);
    assert_eq!(node.label(), Some("Authored label"));
    assert_eq!(node.supports_action(Action::Focus), kind.interactive());
    assert_eq!(panel.accessibility.nodes.len(), 2);
    assert_eq!(panel.accessibility.focus, NodeId(1));
    assert_eq!(panel.components[&1].content_material, Material::Solid);
    let mut host = InspectPanelHost::default();
    host.enqueue(Arc::new(panel));
    assert_eq!(host.0.unwrap().revision, 1);
}
macro_rules! component_test {
    ($name:ident, $component:ident, $role:ident, $widget:ident) => {
        #[test]
        fn $name() {
            check_component(Component::$component, Role::$role, Widget::$widget);
        }
    };
}
component_test!(inspector_row, InspectorRow, Group, InspectorRow);
component_test!(number_field, NumberField, NumberInput, ScrubbableTextField);
component_test!(text_field, TextField, TextInput, TextField);
component_test!(pop_up_button, PopUpButton, ComboBox, PopUpButton);
component_test!(
    segmented_control,
    SegmentedControl,
    RadioGroup,
    SegmentedControl
);
component_test!(segment, Segment, RadioButton, Segment);
component_test!(checkbox, Checkbox, CheckBox, Checkbox);
component_test!(toggle, Toggle, CheckBox, Checkbox);
component_test!(colour_well, ColourWell, ColorWell, ColourWell);
component_test!(disclosure_group, DisclosureGroup, Group, DisclosureGroup);
component_test!(
    disclosure_button,
    DisclosureButton,
    DisclosureTriangle,
    DisclosureButton
);
component_test!(list, List, List, List);
component_test!(tree, Tree, Tree, Outline);
component_test!(list_item, ListItem, ListItem, ListItem);
component_test!(tree_item, TreeItem, TreeItem, TreeItem);
component_test!(toolbar_item, ToolbarItem, Button, ToolbarItem);
component_test!(sheet_anchor, SheetAnchor, Dialog, Sheet);
component_test!(popover_anchor, PopoverAnchor, Dialog, Popover);
component_test!(progress_row, ProgressRow, ProgressIndicator, Progress);
component_test!(empty_state, EmptyState, Group, EmptyState);

#[test]
fn all_accessibility_variants_resolve_from_tokens() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            for transparency in [false, true] {
                for motion in [false, true] {
                    let t = Tokens::new(
                        scheme,
                        Accessibility {
                            increase_contrast: contrast,
                            reduce_transparency: transparency,
                            reduce_motion: motion,
                        },
                    );
                    for &kind in Component::ALL {
                        let r = Recipe::new(kind, &t, true);
                        assert_eq!(r.hit, t.metrics.hit);
                        assert_eq!(r.border, t.metrics.border);
                        assert_eq!(r.border_color, t.colors.separator);
                        assert_eq!(r.muted, t.colors.muted);
                        assert_eq!(r.focus, t.colors.focus);
                        assert_eq!(r.selected_fill, t.colors.selected_fill);
                        assert_eq!(r.selected_segment, t.colors.floating);
                        assert_eq!(r.material, t.material(kind.surface(), true));
                        if transparency {
                            assert_eq!(r.material, Material::Solid);
                        }
                        if motion {
                            assert_eq!(
                                r.motion,
                                Transition::CrossFade {
                                    duration: t.motion.reduced
                                }
                            );
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn inspector_numbers_and_sheet_content_follow_the_spec() {
    let mut t = tokens();
    t.apply_patch([("metrics.value_column", 144.), ("metrics.field", 26.)])
        .unwrap();
    assert_eq!(
        Recipe::new(Component::InspectorRow, &t, false).value_column,
        Some(144.)
    );
    let r = Recipe::new(Component::NumberField, &t, false);
    assert_eq!(r.visual_height, Some(26.));
    assert!(r.tabular_figures && r.units_follow_value && r.values_left_aligned);
    assert_eq!(
        Recipe::new(Component::SheetAnchor, &t, true).background,
        t.colors.panel
    );
    assert_eq!(
        Recipe::new(Component::PopoverAnchor, &t, true).background,
        t.colors.floating
    );
    assert_eq!(
        Recipe::new(Component::ListItem, &t, true).material,
        Material::Solid
    );
}
#[test]
fn missing_labels_non_tokens_wrong_token_groups_and_small_hits_are_flagged() {
    let mut s = surface(Component::TextField);
    patch(
        &mut s,
        json!({"label":" ","style":{"height":22,"background":"#ff0000","minWidth":{"token":"metrics.field"},"minHeight":{"token":"metrics.hit"},"padding":{"token":"colors.accent"},"fontSize":{"token":"typography.missing"}}}),
    );
    let diagnostics = validate_panel(&s, &tokens(), &Layout::new());
    assert!(diagnostics.iter().any(|d| d.code == "missingLabel"));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.code == "nonTokenValue")
            .count(),
        4
    );
    assert!(diagnostics.iter().any(|d| d.code == "smallHitTarget"));
    patch(
        &mut s,
        json!({"label":"Label","style":{"minWidth":{"token":"metrics.hit"},"minHeight":{"token":"metrics.hit"}}}),
    );
    let layout = Layout::from([(
        1,
        Rect {
            x0: 0.,
            y0: 0.,
            x1: 100.,
            y1: 22.,
        },
    )]);
    assert_eq!(
        validate_panel(&s, &tokens(), &layout)[0].code,
        "smallHitTarget"
    );
    let layout = Layout::from([(
        1,
        Rect {
            x0: 0.,
            y0: 0.,
            x1: 100.,
            y1: f64::NAN,
        },
    )]);
    assert_eq!(
        validate_panel(&s, &tokens(), &layout)[0].code,
        "smallHitTarget"
    );
}
#[test]
fn labels_resolve_by_id_without_reading_field_values() {
    let mut s = surface(Component::TextField);
    patch(&mut s, json!({"label":null,"labelledBy":"name"}));
    assert!(
        validate_panel(&s, &tokens(), &Layout::new())
            .iter()
            .any(|d| d.code == "missingLabel")
    );
    s.apply(Commit {
        base_revision: s.revision(),
        ops: vec![
            Op::Create {
                id: 2,
                r#type: "text".into(),
            },
            Op::SetProps {
                id: 2,
                patch: props(json!({"id":"name"})),
            },
            Op::SetText {
                id: 2,
                text: "名前 🎬".into(),
            },
            Op::InsertBefore {
                parent: 0,
                id: 2,
                before: Some(1),
            },
        ],
    })
    .unwrap();
    assert!(validate_panel(&s, &tokens(), &Layout::new()).is_empty());
    assert_eq!(accessible_label(&s, s.node(1).unwrap()), "名前 🎬");
    assert_eq!(focus_order(&s), vec![1]);
}
#[test]
fn stylesheet_visual_literals_are_validated_after_the_cascade() {
    let mut s = surface(Component::TextField);
    s.apply(Commit{base_revision:s.revision(),ops:vec![Op::SetProps{id:0,patch:props(json!({"styleRules":[{"selector":{"kind":"id","value":"field"},"style":{"color":"red"}}]}))},
        Op::SetProps{id:1,patch:props(json!({"id":"field"}))}]}).unwrap();
    assert!(
        validate_panel(&s, &tokens(), &Layout::new())
            .iter()
            .any(|d| d.prop == "style.color")
    );
}
#[test]
fn anchors_progress_and_unknown_component_are_diagnostic_not_destructive() {
    let mut s = surface(Component::PopoverAnchor);
    patch(&mut s, json!({"anchor":999}));
    assert_eq!(
        validate_panel(&s, &tokens(), &Layout::new())[0].code,
        "missingAnchor"
    );
    patch(
        &mut s,
        json!({"data-pm-component":"progress-row","value":2}),
    );
    assert!(
        validate_panel(&s, &tokens(), &Layout::new())
            .iter()
            .any(|d| d.code == "invalidProgress")
    );
    patch(&mut s, json!({"data-pm-component":"future-widget"}));
    assert!(
        validate_panel(&s, &tokens(), &Layout::new())
            .iter()
            .any(|d| d.code == "unknownComponent")
    );
    assert_eq!(s.node(1).unwrap().props["value"], json!(2));
}
fn request(action: Action, data: Option<ActionData>) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: NodeId(1),
        data,
    }
}
#[test]
fn accessibility_actions_are_typed_revision_guarded_and_disabled_aware() {
    let mut s = surface(Component::NumberField);
    let tree = accessibility_tree(&s, &Layout::new(), None);
    let node = &tree.nodes[1].1;
    assert_eq!(node.value(), Some("10 pixels"));
    assert_eq!(node.numeric_value(), Some(10.));
    // A long fraction reads like the sheets show it; the exact value is kept.
    let mut long = surface(Component::NumberField);
    patch(&mut long, json!({"value":73.53333333333333,"unit":"s"}));
    let tree = accessibility_tree(&long, &Layout::new(), None);
    assert_eq!(tree.nodes[1].1.value(), Some("73.533 seconds"));
    assert_eq!(tree.nodes[1].1.numeric_value(), Some(73.53333333333333));
    assert_eq!(
        accessibility_action(&s, s.revision(), request(Action::Increment, None)).unwrap(),
        NativeAction::Step(1, true)
    );
    assert!(accessibility_action(&s, 0, request(Action::Increment, None)).is_err());
    assert!(
        accessibility_action(
            &s,
            s.revision(),
            request(Action::SetValue, Some(ActionData::NumericValue(f64::NAN)))
        )
        .is_err()
    );
    assert_eq!(
        accessibility_action(
            &s,
            s.revision(),
            request(Action::SetValue, Some(ActionData::NumericValue(12.)))
        )
        .unwrap(),
        NativeAction::SetNumber(1, 12.)
    );
    patch(&mut s, json!({"disabled":true}));
    assert!(accessibility_action(&s, s.revision(), request(Action::Increment, None)).is_err());
    assert!(focus_order(&s).is_empty());
}
#[test]
fn accessibility_states_units_and_descendants_are_preserved() {
    let mut s = surface(Component::Checkbox);
    patch(&mut s, json!({"mixed":true}));
    assert_eq!(
        accessibility_tree(&s, &Layout::new(), None).nodes[1]
            .1
            .toggled(),
        Some(Toggled::Mixed)
    );
    let mut s = surface(Component::TreeItem);
    patch(&mut s, json!({"aria-expanded":true,"aria-selected":true}));
    let tree = accessibility_tree(&s, &Layout::new(), Some(999));
    assert_eq!(tree.nodes[1].1.is_selected(), Some(true));
    assert!(tree.nodes[1].1.supports_action(Action::Collapse));
    assert_eq!(tree.focus, NodeId(0));
}
#[derive(Default)]
struct Sink {
    edits: Vec<Edit>,
    fail: bool,
    fail_write: bool,
}
impl BindingSink for Sink {
    fn enqueue(&mut self, edit: Edit) -> Result<(), String> {
        if self.fail || (self.fail_write && matches!(edit, Edit::Write { .. })) {
            return Err("queue full".into());
        }
        self.edits.push(edit);
        Ok(())
    }
}
#[test]
fn native_scrub_threshold_modifiers_and_one_undo_step() {
    let s = surface(Component::NumberField);
    let mut gesture = NumericScrub::new(&s, 1, (0., 0.), &tokens()).unwrap();
    let mut bindings = Bindings::default();
    let mut sink = Sink::default();
    assert_eq!(
        gesture
            .sample(&s, (2., 0.), false, false, &mut bindings, &mut sink)
            .unwrap(),
        vec![ScrubEffect::Pending]
    );
    assert!(sink.edits.is_empty());
    assert_eq!(
        gesture
            .sample(&s, (3., 0.), false, false, &mut bindings, &mut sink)
            .unwrap(),
        vec![ScrubEffect::CapturePointer, ScrubEffect::Value(13.)]
    );
    gesture
        .sample(&s, (3., 0.), true, false, &mut bindings, &mut sink)
        .unwrap();
    gesture
        .sample(&s, (0., -10.), false, true, &mut bindings, &mut sink)
        .unwrap();
    assert_eq!(
        gesture.finish(false, &mut bindings, &mut sink).unwrap(),
        vec![ScrubEffect::ReleasePointer]
    );
    assert_eq!(
        sink.edits,
        vec![
            Edit::Begin {
                binding: "property".into()
            },
            Edit::Write {
                binding: "property".into(),
                value: json!(13.)
            },
            Edit::Write {
                binding: "property".into(),
                value: json!(40.)
            },
            Edit::Write {
                binding: "property".into(),
                value: json!(11.)
            },
            Edit::Commit {
                binding: "property".into()
            }
        ]
    );
}
#[test]
fn native_scrub_cancel_click_bounds_invalid_and_queue_retry() {
    let mut s = surface(Component::NumberField);
    patch(&mut s, json!({"max":12}));
    let mut bindings = Bindings::default();
    let mut sink = Sink::default();
    let mut gesture = NumericScrub::new(&s, 1, (0., 0.), &tokens()).unwrap();
    assert_eq!(
        gesture.finish(false, &mut bindings, &mut sink).unwrap(),
        vec![ScrubEffect::EditText]
    );
    let mut gesture = NumericScrub::new(&s, 1, (0., 0.), &tokens()).unwrap();
    sink.fail = true;
    assert!(
        gesture
            .sample(&s, (5., 0.), false, false, &mut bindings, &mut sink)
            .is_err()
    );
    sink.fail = false;
    assert_eq!(
        gesture
            .sample(&s, (5., 0.), false, false, &mut bindings, &mut sink)
            .unwrap(),
        vec![ScrubEffect::CapturePointer, ScrubEffect::Value(12.)]
    );
    assert!(
        gesture
            .sample(&s, (f64::NAN, 0.), false, false, &mut bindings, &mut sink)
            .is_err()
    );
    patch(&mut s, json!({"binding":"replacement"}));
    assert!(
        gesture
            .sample(&s, (6., 0.), false, false, &mut bindings, &mut sink)
            .is_err()
    );
    gesture.finish(true, &mut bindings, &mut sink).unwrap();
    assert_eq!(
        sink.edits.last(),
        Some(&Edit::Cancel {
            binding: "property".into()
        })
    );
}
#[test]
fn relative_typed_arithmetic_then_arrow_uses_the_original_value() {
    let s = surface(Component::NumberField);
    let mut controls = Controls::default();
    let mut sink = Sink::default();
    controls.focus(&s, 1, &mut sink).unwrap();
    controls.input(1, "+10".into(), &mut sink).unwrap();
    let effects = controls
        .numeric_key(&s, 1, "ArrowUp", false, &mut sink)
        .unwrap();
    assert_eq!(
        effects,
        vec![pm_surface::controls::Effect::Draft(1, "22".into())]
    );
    controls.finish(&s, 1, false, &mut sink).unwrap();
    assert_eq!(
        sink.edits[1],
        Edit::Write {
            binding: "property".into(),
            value: json!(22.)
        }
    );
}
fn visual_literals(source: &str) -> Vec<String> {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| line.split(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        .filter(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_owned)
        .collect()
}
#[test]
fn recipes_have_no_literal_colours_sizes_radii_type_or_motion() {
    assert!(visual_literals(include_str!("../src/kit/recipe.rs")).is_empty());
    assert!(!visual_literals("visual_height: 22., background: 0xFF0000").is_empty());
}

#[test]
fn diagonal_scrubs_sum_axes_and_use_tagged_drag_precision() {
    use pm_surface::binding::{Scrub, scrub};
    let sample = Scrub {
        start: 10.,
        delta: (4., -3.),
        step: 1.,
        speed: 0.5,
        shift: false,
        alt: false,
        bounds: (-100., 100.),
    };
    assert_eq!(scrub(sample).unwrap(), 13.5);
    assert_eq!(
        scrub(Scrub {
            delta: (4., 4.),
            ..sample
        })
        .unwrap(),
        10.
    );
    assert_eq!(
        scrub(Scrub {
            delta: (0.1234, 0.),
            ..sample
        })
        .unwrap(),
        10.062
    );
}
#[test]
fn accessibility_reads_native_text_observations_and_progress_ranges() {
    let mut s = surface(Component::TextField);
    s.observe_input(1, "入力 🎬".into(), (0, 2), true).unwrap();
    assert_eq!(
        accessibility_tree(&s, &Layout::new(), None).nodes[1]
            .1
            .value(),
        Some("入力 🎬")
    );
    let mut s = surface(Component::ProgressRow);
    assert_eq!(
        accessibility_tree(&s, &Layout::new(), None).nodes[1]
            .1
            .numeric_value(),
        None
    );
    patch(&mut s, json!({"value":0.25}));
    let tree = accessibility_tree(&s, &Layout::new(), None);
    assert_eq!(tree.nodes[1].1.numeric_value(), Some(0.25));
    assert_eq!(tree.nodes[1].1.min_numeric_value(), Some(0.));
    assert_eq!(tree.nodes[1].1.max_numeric_value(), Some(1.));
}

#[test]
fn failed_scrub_write_retries_pointer_capture_and_finished_gestures_do_not_restart() {
    let s = surface(Component::NumberField);
    let mut bindings = Bindings::default();
    let mut sink = Sink {
        fail_write: true,
        ..Sink::default()
    };
    let mut gesture = NumericScrub::new(&s, 1, (0., 0.), &tokens()).unwrap();
    assert!(
        gesture
            .sample(&s, (3., 0.), false, false, &mut bindings, &mut sink)
            .is_err()
    );
    sink.fail_write = false;
    assert_eq!(
        gesture
            .sample(&s, (3., 0.), false, false, &mut bindings, &mut sink)
            .unwrap(),
        vec![ScrubEffect::CapturePointer, ScrubEffect::Value(13.)]
    );
    gesture.finish(false, &mut bindings, &mut sink).unwrap();
    assert!(
        gesture
            .finish(false, &mut bindings, &mut sink)
            .unwrap()
            .is_empty()
    );
    assert!(
        gesture
            .sample(&s, (4., 0.), false, false, &mut bindings, &mut sink)
            .is_err()
    );
    assert_eq!(sink.edits.len(), 3);
}

#[test]
fn relative_arithmetic_step_uses_the_current_published_value() {
    let mut s = surface(Component::NumberField);
    let mut controls = Controls::default();
    let mut sink = Sink::default();
    controls.focus(&s, 1, &mut sink).unwrap();
    controls.input(1, "+10".into(), &mut sink).unwrap();
    patch(&mut s, json!({"value":100}));
    assert_eq!(
        controls
            .numeric_key(&s, 1, "ArrowUp", false, &mut sink)
            .unwrap(),
        vec![pm_surface::controls::Effect::Draft(1, "112".into())]
    );
}

#[test]
fn finite_authored_numbers_do_not_overflow_during_decimal_rounding() {
    use pm_surface::{
        binding::{Scrub, scrub},
        controls::numeric_draft,
    };
    assert_eq!(numeric_draft("1e308", 0.), Some(1e308));
    assert_eq!(
        scrub(Scrub {
            start: 1e308,
            delta: (0., 0.),
            step: 1.,
            speed: 0.5,
            shift: false,
            alt: false,
            bounds: (-f64::MAX, f64::MAX)
        })
        .unwrap(),
        1e308
    );
}

#[test]
fn accessibility_popups_use_authored_choice_labels_and_toolbar_sheet_states() {
    let mut s = surface(Component::PopUpButton);
    patch(
        &mut s,
        json!({"value":false,"options":[{"v":false,"label":"Draft"}]}),
    );
    let tree = accessibility_tree(&s, &Layout::new(), None);
    assert_eq!(tree.nodes[1].1.value(), Some("Draft"));
    assert_eq!(tree.nodes[1].1.has_popup(), Some(accesskit::HasPopup::Menu));
    let mut s = surface(Component::ToolbarItem);
    patch(&mut s, json!({"aria-pressed":true}));
    assert_eq!(
        accessibility_tree(&s, &Layout::new(), None).nodes[1]
            .1
            .toggled(),
        Some(Toggled::True)
    );
    let mut s = surface(Component::SheetAnchor);
    patch(&mut s, json!({"aria-modal":true}));
    assert!(
        accessibility_tree(&s, &Layout::new(), None).nodes[1]
            .1
            .is_modal()
    );
}

#[test]
fn design_review_shapes_follow_the_panel_specs() {
    let t = tokens();
    let seg = Recipe::new(Component::SegmentedControl, &t, false);
    assert_eq!(
        seg.track_padding,
        Some(t.metrics.control_radius - t.metrics.inner_radius)
    );
    let well = Recipe::new(Component::ColourWell, &t, false);
    assert_eq!(well.visual_height, Some(t.metrics.field));
    assert_eq!(well.radius, Some(t.metrics.control_radius));
    assert_eq!(
        Recipe::new(Component::ListItem, &t, false).radius,
        Some(t.metrics.control_radius)
    );
    assert_eq!(
        Recipe::new(Component::Toggle, &t, false).widget,
        Widget::Checkbox
    );
    assert_eq!(
        Recipe::new(Component::PopUpButton, &t, false).visual_height,
        Some(t.metrics.control)
    );
    assert_eq!(pm_surface::kit::spoken_unit("%"), "percent");
    assert_eq!(pm_surface::kit::spoken_unit("°"), "degrees");
    assert_eq!(pm_surface::kit::spoken_number(73.53333333333333), "73.533");
    assert_eq!(pm_surface::kit::spoken_number(100.0), "100");
    assert_eq!(pm_surface::kit::spoken_number(-0.0001), "0");
}
