use super::*;
use crate::{
    Commit, Op, Props, Surface,
    binding::{BindingSink, Bindings, Edit},
};
use accesskit::{Action, Live, NodeId, Role, Toggled};
use pm_design::{Accessibility, Material, Scheme, Tokens};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

fn tokens() -> Tokens {
    Tokens::new(Scheme::Light, Accessibility::default())
}
// Synthetic host token table. Production must obtain these from the pm-design lane.
fn table(t: &Tokens) -> DesignTokenTable {
    let mut v = serde_json::to_value(t).unwrap();
    for (name, number) in [
        ("tree_indent", 16.),
        ("group_radius", 10.),
        ("form_row", 40.),
        ("gizmo_handle", 7.),
        ("gizmo_keyline", 1.),
        ("rotation_stem", 18.),
        ("anchor_mark", 11.),
        ("tangent_end", 6.),
        ("checker", 8.),
    ] {
        v["metrics"][name] = json!(number);
    }
    v["motion"]["wheel_rest"] = json!(0.5);
    v["motion"]["overlay_idle"] = json!(2.);
    DesignTokenTable::from_host(v)
}
fn props(v: Value) -> Props {
    v.as_object().unwrap().clone()
}
fn surface(kind: Component, extra: Value) -> Surface {
    let mut s = Surface::default();
    let mut patch = props(
        json!({"data-pm-component": serde_json::to_value(kind).unwrap(), "label":"Authored label", "binding":"property",
        "metadata":{"pmKit":{"kind":"transform"}},
        "style":{"minWidth":{"token":"metrics.hit"},"minHeight":{"token":"metrics.hit"}}}),
    );
    patch.extend(props(extra));
    let primitive = match kind {
        Component::ColourPopover => "anchored",
        Component::NumberField | Component::TextField => "input",
        _ => "div",
    };
    if kind == Component::ColourPopover {
        patch.insert("anchor".into(), json!(0));
    }
    s.apply(Commit {
        base_revision: 0,
        ops: vec![
            Op::Create {
                id: 1,
                r#type: primitive.into(),
            },
            Op::SetProps { id: 1, patch },
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
fn patch(s: &mut Surface, v: Value) {
    s.apply(Commit {
        base_revision: s.revision(),
        ops: vec![Op::SetProps {
            id: 1,
            patch: props(v),
        }],
    })
    .unwrap();
}
fn panel(s: Surface, t: &Tokens) -> Panel {
    Panel::prepare_with_token_table(Arc::new(s), t, &table(t), &Layout::new(), Some(1), true)
}
fn ax(p: &Panel, id: u64) -> &accesskit::Node {
    &p.accessibility
        .nodes
        .iter()
        .find(|(n, _)| *n == NodeId(id))
        .unwrap()
        .1
}
macro_rules! component {
    ($test:ident, $kind:ident, $role:ident, $widget:ident) => {
        #[test]
        fn $test() {
            let p = panel(surface(Component::$kind, json!({})), &tokens());
            assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
            assert_eq!(ax(&p, 1).role(), Role::$role);
            assert_eq!(ax(&p, 1).label(), Some("Authored label"));
            assert_eq!(p.components[&1].widget, Widget::$widget);
        }
    };
}
component!(secure_field_ax, SecureField, PasswordInput, SecureTextField);
component!(group_card_ax, FormGroup, Group, FormGroup);
component!(form_row_ax, FormRow, Group, FormRow);
component!(colour_popover_ax, ColourPopover, Dialog, ColourPopover);
component!(gizmo_handle_ax, ViewerGizmo, Button, ViewerGizmo);
component!(viewer_overlay_ax, ViewerOverlay, Group, ViewerOverlay);
component!(banner_ax, Banner, Status, Banner);
component!(push_button_ax, PushButton, Button, PushButton);

#[test]
fn switch_requires_top_level_settings_root_and_is_a_regular_native_switch() {
    let mut s = surface(Component::Switch, json!({"value":true}));
    assert!(
        validate_panel(&s, &tokens(), &Layout::new())
            .iter()
            .any(|d| d.code == "switchOutsideSettings")
    );
    patch(&mut s, json!({"metadata":{"hostWindow":"settings"}}));
    let p = panel(s, &tokens());
    assert!(p.diagnostics.is_empty());
    assert_eq!(p.components[&1].widget, Widget::Switch);
    assert_eq!(ax(&p, 1).role(), Role::Switch);
    assert_eq!(ax(&p, 1).toggled(), Some(Toggled::True));
    assert!(ax(&p, 1).supports_action(Action::Click));
    let mut nested = surface(Component::FormGroup, json!({}));
    nested.apply(Commit {base_revision:1,ops:vec![Op::Create{id:2,r#type:"div".into()},
        Op::SetProps{id:2,patch:props(json!({"data-pm-component":"switch","label":"Bad switch","metadata":{"hostWindow":"settings"},"style":{"minWidth":{"token":"metrics.hit"},"minHeight":{"token":"metrics.hit"}}}))},
        Op::InsertBefore{parent:1,id:2,before:None}]}).unwrap();
    assert!(
        validate_panel(&nested, &tokens(), &Layout::new())
            .iter()
            .any(|d| d.id == 2 && d.code == "switchOutsideSettings")
    );
}
#[test]
fn secure_field_never_projects_values_or_text_actions() {
    let p = panel(
        surface(
            Component::SecureField,
            json!({"value":"synthetic-sensitive-text","control":"text"}),
        ),
        &tokens(),
    );
    assert_eq!(ax(&p, 1).value(), None);
    assert!(!ax(&p, 1).supports_action(Action::SetValue));
    assert!(ax(&p, 1).supports_action(Action::Focus));
    assert!(
        p.diagnostics
            .iter()
            .any(|d| d.code == "secureValueOnSurface")
    );
}
#[test]
fn missing_tokens_are_explicit_and_never_replaced_with_literals() {
    let p = Panel::prepare(
        Arc::new(surface(Component::FormGroup, json!({}))),
        &tokens(),
        &Layout::new(),
        None,
        false,
    );
    assert_eq!(p.components[&1].radius, None);
    assert!(
        p.diagnostics
            .iter()
            .any(|d| d.prop == "metrics.group_radius" && d.code == "missingDesignToken")
    );
    let Decoration::FormGroup { radius, .. } = &p.decorations[&1] else {
        panic!()
    };
    assert_eq!(radius.value, None);
    for value in [json!(-1), json!(0), json!("0.5"), Value::Null] {
        let bad = DesignTokenTable::from_host(json!({"motion":{"wheel_rest":value}}));
        assert_eq!(bad.metric(WHEEL_REST_TOKEN), None);
    }
}
#[test]
fn group_cards_follow_scheme_and_increase_contrast_without_a_normal_border() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            let t = Tokens::new(
                scheme,
                Accessibility {
                    increase_contrast: contrast,
                    ..Accessibility::default()
                },
            );
            let p = panel(surface(Component::FormGroup, json!({})), &t);
            let r = &p.components[&1];
            assert_eq!(
                r.background,
                if scheme == Scheme::Light {
                    t.colors.field
                } else {
                    t.colors.floating
                }
            );
            assert_eq!(r.radius, Some(10.));
            assert_eq!(r.draw_border, contrast);
            assert_eq!(r.border, t.metrics.border);
            assert_eq!(r.border_color, t.colors.separator);
        }
    }
}
#[test]
fn all_field_borders_follow_increase_contrast() {
    for contrast in [false, true] {
        let t = Tokens::new(
            Scheme::Dark,
            Accessibility {
                increase_contrast: contrast,
                ..Accessibility::default()
            },
        );
        for kind in [
            Component::NumberField,
            Component::TextField,
            Component::SecureField,
            Component::PopUpButton,
            Component::SegmentedControl,
        ] {
            let r = Recipe::new(kind, &t, false);
            assert_eq!(r.draw_border, contrast);
            assert_eq!(r.border, t.metrics.border);
        }
    }
}
#[test]
fn colour_well_and_popover_use_native_recipe_with_exact_geometry() {
    let t = tokens();
    let p = panel(
        surface(
            Component::ColourWell,
            json!({"metadata":{"pmKit":{"minimal":true}}}),
        ),
        &t,
    );
    assert_eq!(p.components[&1].width, None);
    assert_eq!(p.components[&1].visual_height, None);
    assert_eq!(p.components[&1].radius, None);
    let standard = panel(surface(Component::ColourWell, json!({})), &t);
    assert_eq!(standard.components[&1].width, Some(t.metrics.field));
    let Decoration::ColourWell { minimal, checker } = &p.decorations[&1] else {
        panic!()
    };
    assert!(*minimal);
    assert_eq!(checker.value, Some(8.));
    let p = panel(
        surface(
            Component::ColourPopover,
            json!({"metadata":{"pmKit":{"fill":true}}}),
        ),
        &t,
    );
    let Decoration::ColourPopover(c) = &p.decorations[&1] else {
        panic!()
    };
    assert!(c.fill_modes);
    assert_eq!(c.sb_height, t.metrics.popover_width / 2.);
    assert_eq!(
        (c.thumb, c.ring, c.slider_track, c.slider_thumb, c.row_gap),
        (12., 2., 12., 16., 10.)
    );
    assert_eq!(c.recent_limit, 8);
    assert_eq!(c.sampler_label, "Pick a Colour From the Screen");
    assert_eq!(p.components[&1].material, Material::Glass);
    assert_eq!(p.components[&1].content_material, Material::Solid);
}
#[test]
fn gizmo_variants_use_dark_tokens_and_display_points_in_both_schemes() {
    for kind in ["transform", "rotation", "anchor", "path-vertex", "tangent"] {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let t = Tokens::new(scheme, Accessibility::default());
            let p = panel(
                surface(
                    Component::ViewerGizmo,
                    json!({"metadata":{"pmKit":{"kind":kind,"selected":true}}}),
                ),
                &t,
            );
            assert!(p.diagnostics.is_empty());
            let Decoration::ViewerGizmo(g) = &p.decorations[&1] else {
                panic!()
            };
            assert!(g.display_scaled);
            assert_eq!(g.keyline.value, Some(1.));
            assert!(p.over_image.contains(&1));
            let dark = Tokens::new(Scheme::Dark, t.accessibility);
            if kind == "transform" {
                assert_eq!(g.fill, dark.colors.accent);
                assert_eq!(g.stroke, dark.colors.viewer);
                assert_eq!(g.size.value, Some(7.));
            }
            if kind == "rotation" {
                assert_eq!(g.stem.as_ref().unwrap().value, Some(18.));
                assert_eq!(g.accent_stroke, Some(1.5));
            }
            if kind == "anchor" {
                assert_eq!(g.inner_ring, Some(1.));
            }
        }
    }
}
#[test]
fn overlay_dark_palette_includes_plain_descendants_and_accessibility_fallbacks() {
    for contrast in [false, true] {
        for transparency in [false, true] {
            for motion in [false, true] {
                let t = Tokens::new(
                    Scheme::Light,
                    Accessibility {
                        increase_contrast: contrast,
                        reduce_transparency: transparency,
                        reduce_motion: motion,
                    },
                );
                let mut s = surface(Component::ViewerOverlay, json!({}));
                s.apply(Commit {
                    base_revision: 1,
                    ops: vec![
                        Op::Create {
                            id: 2,
                            r#type: "text".into(),
                        },
                        Op::InsertBefore {
                            parent: 1,
                            id: 2,
                            before: None,
                        },
                    ],
                })
                .unwrap();
                let p = panel(s, &t);
                let dark = Tokens::new(Scheme::Dark, t.accessibility);
                assert_eq!(p.over_image.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
                assert_eq!(p.components[&1].background, dark.colors.floating);
                assert_eq!(
                    p.components[&1].material,
                    if transparency {
                        Material::Solid
                    } else {
                        Material::Glass
                    }
                );
                assert_eq!(
                    p.components[&1].motion,
                    Transition::CrossFade {
                        duration: if motion {
                            t.motion.reduced
                        } else {
                            t.motion.enter
                        }
                    }
                );
            }
        }
    }
}
#[test]
fn overlay_focus_hover_and_menus_prevent_idle_fade() {
    let mut v = OverlayVisibility::new(&table(&tokens()), Duration::ZERO).unwrap();
    assert!(v.visible(Duration::from_secs(1), false, false, false));
    assert!(!v.visible(Duration::from_secs(2), false, false, false));
    for flags in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert!(v.visible(Duration::from_secs(10), flags.0, flags.1, flags.2));
        assert!(v.visible(Duration::from_secs(11), false, false, false));
        assert!(!v.visible(Duration::from_secs(12), false, false, false));
    }
}
#[test]
fn banners_replace_dismiss_and_announce_with_severity() {
    let mut slot = BannerSlot::default();
    slot.show(1, Duration::ZERO, false);
    slot.show(2, Duration::from_secs(2), false);
    assert_eq!(slot.tick(Duration::from_secs(4)), None);
    assert_eq!(slot.tick(Duration::from_secs(6)), Some(2));
    assert_eq!(slot.id, None);
    slot.show(3, Duration::ZERO, true);
    assert_eq!(slot.tick(Duration::from_secs(100)), None);
    assert_eq!(slot.dismiss(), Some(3));
    for (severity, live) in [
        ("info", Live::Polite),
        ("warning", Live::Polite),
        ("error", Live::Assertive),
    ] {
        let p = panel(
            surface(
                Component::Banner,
                json!({"metadata":{"pmKit":{"severity":severity}}}),
            ),
            &tokens(),
        );
        assert_eq!(ax(&p, 1).live(), Some(live));
    }
    let p = panel(
        surface(
            Component::Banner,
            json!({"metadata":{"pmKit":{"placement":"inline"}}}),
        ),
        &tokens(),
    );
    assert_eq!(p.components[&1].material, Material::Solid);
    assert!(!p.over_image.contains(&1));
}
#[test]
fn tree_indent_resolves_named_token_and_disclosure_geometry() {
    let t = tokens();
    let p = panel(surface(Component::TreeItem, json!({})), &t);
    let Decoration::Tree {
        indent,
        disclosure_size,
        hit,
        leading,
    } = &p.decorations[&1]
    else {
        panic!()
    };
    assert_eq!(indent.name, TREE_INDENT_TOKEN);
    assert_eq!(indent.value, Some(16.));
    assert_eq!(*disclosure_size, t.metrics.symbol_inline);
    assert_eq!(*hit, t.metrics.hit);
    assert_eq!(
        *leading,
        t.metrics.inset - (t.metrics.hit - t.metrics.symbol_inline) / 2.
    );
}
#[derive(Default)]
struct Sink {
    edits: Vec<Edit>,
    fail: bool,
}
impl BindingSink for Sink {
    fn enqueue(&mut self, e: Edit) -> Result<(), String> {
        if self.fail {
            self.fail = false;
            Err("queue full".into())
        } else {
            self.edits.push(e);
            Ok(())
        }
    }
}
fn numeric() -> Surface {
    surface(
        Component::NumberField,
        json!({"control":"number","value":10.,"step":2.,"min":0.,"max":100.}),
    )
}
fn wheel(s: &Surface) -> NumericWheel {
    NumericWheel::new(s, 1, &tokens(), &table(&tokens()), Duration::ZERO, (0., 0.)).unwrap()
}
fn sample(ms: u64, device: WheelDevice, shift: bool, alt: bool) -> WheelSample {
    WheelSample {
        now: Duration::from_millis(ms),
        notches: 1.,
        device,
        shift,
        alt,
    }
}
#[test]
fn wheel_rests_for_half_second_and_container_scroll_resets_gate() {
    let s = numeric();
    let mut w = wheel(&s);
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    assert_eq!(
        w.sample(
            &s,
            sample(499, WheelDevice::Discrete, false, false),
            &mut b,
            &mut sink
        )
        .unwrap(),
        WheelDisposition::Scroll
    );
    w.container_scrolled(Duration::from_millis(400));
    assert!(!w.armed(Duration::from_millis(899)));
    assert!(w.armed(Duration::from_millis(900)));
    w.pointer(Duration::from_millis(900), (2., 0.));
    assert!(w.armed(Duration::from_millis(900)));
    w.pointer(Duration::from_millis(901), (4., 0.));
    assert!(!w.armed(Duration::from_millis(1400)));
    assert!(w.armed(Duration::from_millis(1401)));
    assert!(sink.edits.is_empty());
}
#[test]
fn wheel_trackpad_requires_option_and_modifiers_match_spec() {
    for (device, shift, alt, expected) in [
        (WheelDevice::Discrete, false, false, 12.),
        (WheelDevice::Discrete, true, false, 30.),
        (WheelDevice::Discrete, false, true, 10.2),
        (WheelDevice::Trackpad, false, true, 12.),
        (WheelDevice::Trackpad, true, true, 30.),
        (WheelDevice::Discrete, true, true, 30.),
    ] {
        let s = numeric();
        let mut w = wheel(&s);
        let mut b = Bindings::default();
        let mut sink = Sink::default();
        assert_eq!(
            w.sample(&s, sample(500, device, shift, alt), &mut b, &mut sink)
                .unwrap(),
            WheelDisposition::Edited(expected)
        );
    }
    let s = numeric();
    let mut w = wheel(&s);
    assert_eq!(
        w.sample(
            &s,
            sample(500, WheelDevice::Trackpad, false, false),
            &mut Bindings::default(),
            &mut Sink::default()
        )
        .unwrap(),
        WheelDisposition::Scroll
    );
}
#[test]
fn wheel_sequence_accumulates_and_commits_once_after_last_event() {
    let s = numeric();
    let mut w = wheel(&s);
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    for (ms, value) in [(500, 12.), (600, 14.), (900, 16.)] {
        assert_eq!(
            w.sample(
                &s,
                sample(ms, WheelDevice::Discrete, false, false),
                &mut b,
                &mut sink
            )
            .unwrap(),
            WheelDisposition::Edited(value)
        );
    }
    assert!(
        !w.tick(Duration::from_millis(1399), &mut b, &mut sink)
            .unwrap()
    );
    assert!(
        w.tick(Duration::from_millis(1400), &mut b, &mut sink)
            .unwrap()
    );
    assert!(
        !w.tick(Duration::from_millis(1500), &mut b, &mut sink)
            .unwrap()
    );
    assert_eq!(
        sink.edits
            .iter()
            .filter(|e| matches!(e, Edit::Begin { .. }))
            .count(),
        1
    );
    assert_eq!(
        sink.edits
            .iter()
            .filter(|e| matches!(e, Edit::Commit { .. }))
            .count(),
        1
    );
}
#[test]
fn wheel_cancel_on_rebind_targets_original_binding() {
    let mut s = numeric();
    let mut w = wheel(&s);
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    w.sample(
        &s,
        sample(500, WheelDevice::Discrete, false, false),
        &mut b,
        &mut sink,
    )
    .unwrap();
    patch(&mut s, json!({"binding":"replacement"}));
    assert_eq!(
        w.sample(
            &s,
            sample(600, WheelDevice::Discrete, false, false),
            &mut b,
            &mut sink
        )
        .unwrap(),
        WheelDisposition::Scroll
    );
    assert_eq!(
        sink.edits.last(),
        Some(&Edit::Cancel {
            binding: "property".into()
        })
    );
}
#[test]
fn wheel_queue_failures_are_retryable_without_duplicate_deltas() {
    let s = numeric();
    let mut w = wheel(&s);
    let mut b = Bindings::default();
    let mut sink = Sink {
        fail: true,
        ..Sink::default()
    };
    assert!(
        w.sample(
            &s,
            sample(500, WheelDevice::Discrete, false, false),
            &mut b,
            &mut sink
        )
        .is_err()
    );
    w.sample(
        &s,
        sample(500, WheelDevice::Discrete, false, false),
        &mut b,
        &mut sink,
    )
    .unwrap();
    sink.fail = true;
    assert!(
        w.sample(
            &s,
            sample(600, WheelDevice::Discrete, false, false),
            &mut b,
            &mut sink
        )
        .is_err()
    );
    assert_eq!(
        w.sample(
            &s,
            sample(600, WheelDevice::Discrete, false, false),
            &mut b,
            &mut sink
        )
        .unwrap(),
        WheelDisposition::Edited(14.)
    );
    sink.fail = true;
    assert!(
        w.tick(Duration::from_millis(1100), &mut b, &mut sink)
            .is_err()
    );
    assert!(
        w.tick(Duration::from_millis(1100), &mut b, &mut sink)
            .unwrap()
    );
    assert_eq!(sink.edits.len(), 4);
}
#[test]
fn wheel_clamps_rejects_overflow_and_missing_token() {
    let s = numeric();
    let mut w = wheel(&s);
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    let mut e = sample(500, WheelDevice::Discrete, false, false);
    e.notches = 1000.;
    assert_eq!(
        w.sample(&s, e, &mut b, &mut sink).unwrap(),
        WheelDisposition::Edited(100.)
    );
    e.notches = f64::MAX;
    assert!(w.sample(&s, e, &mut b, &mut sink).is_err());
    assert!(
        NumericWheel::new(
            &s,
            1,
            &tokens(),
            &DesignTokenTable::from_design(&tokens()),
            Duration::ZERO,
            (0., 0.)
        )
        .is_err()
    );
}
#[test]
fn colour_preview_cancel_commit_and_unmodified_close_are_one_transaction() {
    for cancel in [true, false] {
        let s = surface(
            Component::ColourWell,
            json!({"control":"color","value":"color(display-p3 1 0.125 0 / .7)"}),
        );
        let mut c = ColourEdit::open(&s, 1).unwrap();
        let mut b = Bindings::default();
        let mut sink = Sink::default();
        assert!(sink.edits.is_empty());
        c.preview(&s, "#abcdef", &mut b, &mut sink).unwrap();
        c.preview(&s, "#123456", &mut b, &mut sink).unwrap();
        c.close(cancel, &mut b, &mut sink).unwrap();
        c.close(cancel, &mut b, &mut sink).unwrap();
        assert_eq!(sink.edits.len(), 4);
        assert_eq!(
            sink.edits.last(),
            Some(&if cancel {
                Edit::Cancel {
                    binding: "property".into(),
                }
            } else {
                Edit::Commit {
                    binding: "property".into(),
                }
            })
        );
        assert_eq!(
            s.node(1).unwrap().props["value"],
            json!("color(display-p3 1 0.125 0 / .7)")
        );
        let mut c = ColourEdit::open(&s, 1).unwrap();
        let mut empty = Sink::default();
        c.close(false, &mut b, &mut empty).unwrap();
        assert!(empty.edits.is_empty());
    }
}
#[test]
fn colour_retirement_and_queue_errors_preserve_transaction() {
    let mut s = surface(Component::ColourWell, json!({"control":"color"}));
    let mut c = ColourEdit::open(&s, 1).unwrap();
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    c.preview(&s, "#abcdef", &mut b, &mut sink).unwrap();
    sink.fail = true;
    assert!(c.close(false, &mut b, &mut sink).is_err());
    patch(&mut s, json!({"disabled":true}));
    assert!(c.preview(&s, "#123456", &mut b, &mut sink).is_err());
    assert_eq!(
        sink.edits.last(),
        Some(&Edit::Cancel {
            binding: "property".into()
        })
    );
}
#[test]
fn native_recipes_have_no_literal_visual_assignments() {
    let source = include_str!("recipe.rs");
    let literals: Vec<_> = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .flat_map(|l| l.split(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        .filter(|w| w.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert!(literals.is_empty(), "{literals:?}");
    // New geometry is expressed by named tokens or the exact arithmetic approved
    // in inspector.md §7. There are no hex literals or fallback token values.
    let source = include_str!("additions.rs");
    assert!(!source.contains("0x"));
    assert!(!source.contains("unwrap_or(16"));
    assert!(!source.contains("unwrap_or(0.5"));
}

#[test]
fn native_only_colour_and_secure_hosts_have_linux_command_doubles() {
    let mut secure = InspectSecureFieldHost::default();
    secure.focus(7, "keychain:synthetic").unwrap();
    secure.commit(7).unwrap();
    secure.cancel(7).unwrap();
    assert_eq!(
        secure.0,
        vec![
            SecureFieldCommand::Focus {
                id: 7,
                binding: "keychain:synthetic".into()
            },
            SecureFieldCommand::Commit(7),
            SecureFieldCommand::Cancel(7)
        ]
    );
    let mut colour = InspectColourPopoverHost::default();
    colour.open(8, "background", true).unwrap();
    colour.sample_screen(8).unwrap();
    colour.close(8, true).unwrap();
    assert_eq!(
        colour.0,
        vec![
            ColourPopoverCommand::Open {
                anchor: 8,
                binding: "background".into(),
                fill: true
            },
            ColourPopoverCommand::SampleScreen(8),
            ColourPopoverCommand::Close {
                anchor: 8,
                cancel: true
            }
        ]
    );
}
#[test]
fn empty_state_symbol_is_decorative_in_ax() {
    let mut s = surface(Component::EmptyState, json!({}));
    s.apply(Commit{base_revision:1,ops:vec![Op::Create{id:2,r#type:"div".into()},Op::SetProps{id:2,patch:props(json!({"data-pm-slot":"symbol","metadata":{"pmKit":{"symbol":"sidebar.trailing","decorative":true}}}))},Op::InsertBefore{parent:1,id:2,before:None}]}).unwrap();
    let p = panel(s, &tokens());
    assert!(ax(&p, 2).is_hidden());
    assert_eq!(ax(&p, 1).role(), Role::Group);
    let Decoration::EmptyState {
        symbol_size,
        symbol_color,
    } = &p.decorations[&1]
    else {
        panic!()
    };
    assert_eq!(*symbol_size, tokens().metrics.symbol_panel * 2.);
    assert_eq!(*symbol_color, tokens().colors.muted);
}
#[test]
fn menu_presentation_is_published_with_separators_details_and_measurement_sample() {
    let p = panel(
        surface(
            Component::PopUpButton,
            json!({"metadata":{"pmKit":{"widthSample":"Auto (Quarter)","menu":[{"v":null,"label":"Auto","detail":"Lowers resolution while playing to keep up"},{"separator":true},{"v":false,"label":"Full","keyEquivalent":"⌘1"}]}}}),
        ),
        &tokens(),
    );
    let Decoration::PopUpButton { menu, width_sample } = &p.decorations[&1] else {
        panic!()
    };
    assert_eq!(width_sample.as_deref(), Some("Auto (Quarter)"));
    assert_eq!(menu[0].value, Some(Value::Null));
    assert_eq!(
        menu[0].detail.as_deref(),
        Some("Lowers resolution while playing to keep up")
    );
    assert!(menu[1].separator && menu[1].disabled);
    assert_eq!(menu[1].value, None);
    assert_eq!(menu[2].value, Some(json!(false)));
    assert_eq!(menu[2].key_equivalent.as_deref(), Some("⌘1"));
}
#[test]
fn wheel_bound_events_extend_sequence_and_unwritten_begin_cancels() {
    let mut s = numeric();
    let mut w = wheel(&s);
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    let mut e = sample(500, WheelDevice::Discrete, false, false);
    e.notches = 1000.;
    w.sample(&s, e, &mut b, &mut sink).unwrap();
    e.now = Duration::from_millis(900);
    w.sample(&s, e, &mut b, &mut sink).unwrap();
    assert!(
        !w.tick(Duration::from_millis(1000), &mut b, &mut sink)
            .unwrap()
    );
    assert!(
        w.tick(Duration::from_millis(1400), &mut b, &mut sink)
            .unwrap()
    );
    // A failed first write must still close the begin if no retry comes.
    struct FailWrite(Vec<Edit>);
    impl BindingSink for FailWrite {
        fn enqueue(&mut self, e: Edit) -> Result<(), String> {
            if matches!(e, Edit::Write { .. }) {
                Err("full".into())
            } else {
                self.0.push(e);
                Ok(())
            }
        }
    }
    patch(&mut s, json!({"value":1.}));
    let mut w = wheel(&s);
    let mut sink = FailWrite(vec![]);
    assert!(
        w.sample(
            &s,
            sample(500, WheelDevice::Discrete, false, false),
            &mut b,
            &mut sink
        )
        .is_err()
    );
    assert!(
        w.tick(Duration::from_millis(1000), &mut b, &mut sink)
            .unwrap()
    );
    assert_eq!(
        sink.0.last(),
        Some(&Edit::Cancel {
            binding: "property".into()
        })
    );
}
#[test]
fn increase_contrast_gizmo_keylines_and_reduced_overlay_deadlines_follow_tokens() {
    let t = Tokens::new(
        Scheme::Light,
        Accessibility {
            increase_contrast: true,
            reduce_motion: true,
            reduce_transparency: false,
        },
    );
    let p = panel(surface(Component::ViewerGizmo, json!({})), &t);
    let Decoration::ViewerGizmo(g) = &p.decorations[&1] else {
        panic!()
    };
    assert_eq!(g.keyline.value, Some(f64::from(t.metrics.border)));
    let p = panel(surface(Component::ViewerOverlay, json!({})), &t);
    let Decoration::ViewerOverlay { enter, exit, .. } = &p.decorations[&1] else {
        panic!()
    };
    assert_eq!((*enter, *exit), (t.motion.reduced, t.motion.reduced));
}
#[test]
fn active_dark_theme_patches_survive_over_image_recipe_publication() {
    let t = tokens();
    let mut dark = Tokens::new(Scheme::Dark, t.accessibility);
    dark.apply_patch([("colors.floating", f64::from(0x353535))])
        .unwrap();
    let s = surface(Component::ViewerOverlay, json!({}));
    let p = Panel::prepare_with_palettes(
        Arc::new(s),
        &t,
        &dark,
        &table(&t),
        &Layout::new(),
        None,
        true,
    );
    assert_eq!(p.components[&1].background, dark.colors.floating);
    let s = surface(Component::ViewerOverlay, json!({}));
    let p = panel(s, &dark);
    assert_eq!(p.components[&1].background, dark.colors.floating);
}
