//! Semantic field availability from the shipped panel. No labels, ordering
//! contracts for native layout, widget selection, or visual defaults are added.
use crate::input::NumericSpec;
use crate::view::{array, text};
use crate::{Binding, Inspector, InspectorError, PropertyId};
#[derive(Clone, Debug, PartialEq)]
pub enum FieldType {
    Number(NumericSpec),
    Boolean,
    Text,
    Choice,
    Color,
    Fill,
    Font,
    Opaque,
}
/// One parameter of a contributed layer type (`api.layers.register`), stored
/// as the animatable `x.<key>` channel in the layer's `d.params`, as main's
/// `ExtensionLayerParams` shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerParam {
    pub key: String,
    pub label: String,
    pub kind: FieldType,
    /// Its options as the definition gives them (a parameter with options is a
    /// choice whatever its type, as on main).
    pub options: Vec<crate::Value>,
    /// What Reset returns it to: `def`, or for a number with `defaultFrom`
    /// the layer's width or height times `factor` (main's `effectParameterDefault`).
    pub default: Option<crate::Value>,
    pub default_from: Option<(bool, f64)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDescriptor {
    pub binding: Binding,
    pub kind: FieldType,
    pub animatable: bool,
}
impl Inspector {
    pub fn fields(&self) -> Result<Vec<FieldDescriptor>, InspectorError> {
        let mut out = Vec::new();
        if self.composition_mode() {
            for (field, step, min) in [
                ("width", 2.0, Some(16.0)),
                ("height", 2.0, Some(16.0)),
                ("duration", 0.5, None),
                ("fps", 1.0, None),
            ] {
                out.push(FieldDescriptor {
                    binding: Binding::Composition(field.into()),
                    kind: FieldType::Number(NumericSpec {
                        step,
                        min,
                        ..Default::default()
                    }),
                    animatable: false,
                });
            }
            out.push(FieldDescriptor {
                binding: Binding::Composition("backgroundFill".into()),
                kind: FieldType::Fill,
                animatable: false,
            });
            let params = self.composition_field("params");
            if let Some(params) = params.as_object() {
                for (name, param) in params.iter() {
                    if text(param, "label").is_empty() && text(param, "name").is_empty() {
                        continue;
                    }
                    let kind = match text(param, "control") {
                        "color" => FieldType::Color,
                        "toggle" => FieldType::Boolean,
                        "select" => FieldType::Choice,
                        _ => FieldType::Number(NumericSpec {
                            step: match (
                                param.get("min").and_then(|v| v.as_f64()),
                                param.get("max").and_then(|v| v.as_f64()),
                            ) {
                                (Some(a), Some(b)) if a != b => (b - a) / 200.0,
                                _ => 0.01,
                            },
                            min: param.get("min").and_then(|v| v.as_f64()),
                            max: param.get("max").and_then(|v| v.as_f64()),
                            ..Default::default()
                        }),
                    };
                    out.push(FieldDescriptor {
                        binding: Binding::SceneParameter(name.to_string_lossy()),
                        kind,
                        animatable: false,
                    });
                }
            }
            return Ok(out);
        }
        let id = self.primary().expect("selected");
        let layer = self.layer(id)?;
        let kind = text(&layer, "type");
        let d = layer.get("d");
        let mut content = |field: &str, kind: FieldType| {
            let animatable = pm_eval::address::can_animate_content(
                self.layer_node(id).expect("present"),
                field.as_bytes(),
            );
            out.push(FieldDescriptor {
                binding: Binding::Content {
                    layer: id.into(),
                    field: field.into(),
                },
                kind,
                animatable,
            });
        };
        let number = |step, min, max| {
            FieldType::Number(NumericSpec {
                step,
                min,
                max,
                ..Default::default()
            })
        };
        // Time remapping (`RetimingSection.svelte`): shown once a video or
        // pre-comp layer has a source time channel.
        let remapped = d.is_some_and(|d| d.get("sourceTime").is_some());
        match kind {
            "text" => {
                for (field, kind) in [
                    ("text", FieldType::Text),
                    ("font", FieldType::Font),
                    ("weight", FieldType::Choice),
                    ("size", number(1.0, Some(4.0), None)),
                    ("tracking", number(0.5, None, None)),
                    ("leading", number(0.02, None, None)),
                    ("align", FieldType::Choice),
                    ("color", FieldType::Color),
                ] {
                    content(field, kind);
                }
            }
            "solid" | "shape" | "null" if d.is_none_or(|d| array(d.get("paths")).is_empty()) => {
                content("color", FieldType::Color);
                if kind == "shape" {
                    content("shape", FieldType::Choice);
                }
                content("w", number(1.0, Some(1.0), None));
                content("h", number(1.0, Some(1.0), None));
                if kind == "shape" {
                    if d.is_some_and(|d| text(d, "shape") == "rect") {
                        content("independentCorners", FieldType::Boolean);
                        content("smoothing", number(1.0, Some(0.0), Some(100.0)));
                        if d.and_then(|d| d.get("independentCorners"))
                            .and_then(|v| v.as_bool())
                            == Some(true)
                        {
                            for key in ["radiusTL", "radiusTR", "radiusBL", "radiusBR"] {
                                content(key, number(1.0, Some(0.0), None));
                            }
                        } else {
                            content("radius", number(1.0, Some(0.0), None));
                        }
                    }
                    content("stroke", number(0.5, Some(0.0), None));
                    content("strokeColor", FieldType::Color);
                    if d.is_some_and(|d| ["polygon", "star"].contains(&text(d, "shape"))) {
                        content("points", number(1.0, Some(3.0), Some(24.0)));
                    }
                }
            }
            "image" | "video" => {
                content("asset", FieldType::Choice);
                content("fit", FieldType::Choice);
                content("w", number(1.0, Some(1.0), None));
                content("h", number(1.0, Some(1.0), None));
                if kind == "video" {
                    content("trim", number(0.05, None, None));
                    content("speed", number(0.05, Some(0.05), None));
                    if d.and_then(|d| d.get("embeddedAudio"))
                        .and_then(|v| v.as_bool())
                        == Some(true)
                    {
                        content("audioMuted", FieldType::Boolean);
                        content("audioGain", number(0.05, Some(0.0), Some(4.0)));
                    }
                    if remapped {
                        content("timeRemap", FieldType::Boolean);
                        content("sourceTime", number(0.01, Some(0.0), None));
                    }
                }
            }
            "audio" => {
                for (field, step, min, max) in [
                    ("trim", 0.05, Some(0.0), None),
                    ("gain", 0.05, Some(0.0), Some(4.0)),
                    (
                        "fadeIn",
                        0.05,
                        Some(0.0),
                        layer.get("dur").and_then(|v| v.as_f64()),
                    ),
                    (
                        "fadeOut",
                        0.05,
                        Some(0.0),
                        layer.get("dur").and_then(|v| v.as_f64()),
                    ),
                ] {
                    content(field, number(step, min, max));
                }
            }
            "shader" | "extension" => {
                content("w", number(1.0, Some(1.0), None));
                content("h", number(1.0, Some(1.0), None));
                let definition = d.map(|d| text(d, "definition")).unwrap_or("");
                for param in self.layer_types.get(definition).into_iter().flatten() {
                    out.push(FieldDescriptor {
                        binding: Binding::Property(PropertyId::new(id, format!("x.{}", param.key))),
                        kind: param.kind.clone(),
                        animatable: true,
                    });
                }
            }
            "precomp" => {
                let comps = self
                    .published
                    .snapshot
                    .root()
                    .and_then(|r| r.get("comps"))
                    .map(|n| n.to_value())
                    .unwrap_or(crate::Value::Null);
                if crate::content::precomp_carries_audio(
                    d.map(|d| text(d, "comp")).unwrap_or(""),
                    &comps,
                ) {
                    content("audioMuted", FieldType::Boolean);
                    content("audioGain", number(0.05, Some(0.0), Some(4.0)));
                }
                if remapped {
                    content("timeRemap", FieldType::Boolean);
                    content("sourceTime", number(0.01, Some(0.0), None));
                }
            }
            _ => {}
        }
        if kind != "audio" {
            if let Some(properties) = layer.get("p").and_then(|v| v.as_object()) {
                for key in properties.keys() {
                    out.push(FieldDescriptor {
                        binding: Binding::Property(PropertyId::new(id, key.to_string_lossy())),
                        kind: FieldType::Opaque,
                        animatable: true,
                    });
                }
            }
            for (field, kind) in [
                ("blend", FieldType::Choice),
                ("mblur", FieldType::Boolean),
                ("matteMode", FieldType::Choice),
            ] {
                out.push(FieldDescriptor {
                    binding: Binding::Property(PropertyId::new(id, format!("l.{field}"))),
                    kind,
                    animatable: true,
                });
            }
            for (field, kind) in [
                ("parent", FieldType::Choice),
                ("matteSource", FieldType::Choice),
                ("scaleLinked", FieldType::Boolean),
                ("threeD", FieldType::Boolean),
            ] {
                out.push(FieldDescriptor {
                    binding: Binding::Layer {
                        layer: id.into(),
                        field: field.into(),
                    },
                    kind,
                    animatable: false,
                });
            }
        }
        if kind == "precomp" {
            out.push(FieldDescriptor {
                binding: Binding::Layer {
                    layer: id.into(),
                    field: "continuousRaster".into(),
                },
                kind: FieldType::Boolean,
                animatable: false,
            });
        }
        for (field, kind) in [
            ("solo", FieldType::Boolean),
            ("visible", FieldType::Boolean),
            ("color", FieldType::Color),
        ] {
            out.push(FieldDescriptor {
                binding: Binding::Layer {
                    layer: id.into(),
                    field: field.into(),
                },
                kind,
                animatable: false,
            });
        }
        Ok(out)
    }
}
