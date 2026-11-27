//! Semantic state only. Unknown compat/extension fields survive every merge.
use crate::{Error, Navigation, Point, finite};
use pm_kernel::state::UiState;
use pm_model::{Map, Value};

pub(crate) fn number(value: &Value, key: &str, fallback: f64) -> Result<f64, Error> {
    match value.get(key) {
        None => finite(fallback),
        Some(v) => finite(v.as_f64().ok_or(Error::Invalid("number"))?),
    }
}
pub(crate) fn boolean(value: &Value, key: &str, fallback: bool) -> Result<bool, Error> {
    match value.get(key) {
        None => Ok(fallback),
        Some(v) => v.as_bool().ok_or(Error::Invalid("boolean")),
    }
}
pub(crate) fn put(value: &mut Value, key: &str, next: Value) -> Result<(), Error> {
    value
        .as_object_mut()
        .ok_or(Error::Invalid("object"))?
        .insert(key, next);
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Auto,
    Full,
    Half,
    Quarter,
}
impl Quality {
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "1",
            Self::Half => "0.5",
            Self::Quarter => "0.25",
        }
    }
    pub fn scale(self) -> f64 {
        match self {
            Self::Auto | Self::Full => 1.0,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
        }
    }
    pub fn parse(id: &str) -> Result<Self, Error> {
        match id {
            "auto" => Ok(Self::Auto),
            "1" => Ok(Self::Full),
            "0.5" => Ok(Self::Half),
            "0.25" => Ok(Self::Quarter),
            _ => Err(Error::Invalid("quality")),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
    pub navigation: Navigation,
    pub show_controls: bool,
    pub quality: Quality,
    /// Auto may be at a lower measured quality; choosing Auto resets this to 1.
    pub effective_quality: f64,
    pub overlays: OverlayState,
}
impl Chrome {
    pub fn read(ui: &UiState) -> Result<Self, Error> {
        let viewer = &ui.viewer;
        let pan = match viewer.get("pan") {
            None => Point::default(),
            Some(Value::Array(a)) if a.len() == 2 => Point {
                x: a[0].as_f64().ok_or(Error::Invalid("pan"))?,
                y: a[1].as_f64().ok_or(Error::Invalid("pan"))?,
            }
            .checked()?,
            _ => return Err(Error::Invalid("pan")),
        };
        let navigation = Navigation {
            fit: boolean(viewer, "fit", true)?,
            zoom: number(viewer, "zoom", 1.0)?,
            pan,
        }
        .checked()?;
        let effective_quality = number(&ui.transport, "quality", 1.0)?;
        if !(0.25..=1.0).contains(&effective_quality) {
            return Err(Error::Invalid("effective quality"));
        }
        let quality = match ui
            .transport
            .get("previewResolution")
            .and_then(Value::as_str)
        {
            Some(id) => Quality::parse(id)?,
            None if ui
                .transport
                .get("perf")
                .map(|v| boolean(v, "auto", true))
                .transpose()?
                .unwrap_or(true) =>
            {
                Quality::Auto
            }
            None => match effective_quality {
                1.0 => Quality::Full,
                0.5 => Quality::Half,
                0.25 => Quality::Quarter,
                _ => return Err(Error::Invalid("quality")),
            },
        };
        Ok(Self {
            navigation,
            show_controls: boolean(viewer, "showControls", true)?,
            quality,
            effective_quality,
            overlays: OverlayState::read(viewer)?,
        })
    }
    pub fn write_navigation(ui: &mut UiState, nav: Navigation) -> Result<(), Error> {
        nav.checked()?;
        put(&mut ui.viewer, "fit", Value::Bool(nav.fit))?;
        put(&mut ui.viewer, "zoom", Value::Number(nav.zoom))?;
        put(
            &mut ui.viewer,
            "pan",
            Value::Array(vec![Value::Number(nav.pan.x), Value::Number(nav.pan.y)]),
        )
    }
    pub fn write_quality(ui: &mut UiState, quality: Quality) -> Result<(), Error> {
        put(
            &mut ui.transport,
            "previewResolution",
            Value::from(quality.id()),
        )?;
        put(&mut ui.transport, "quality", Value::Number(quality.scale()))?;
        let mut perf = ui
            .transport
            .get("perf")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        put(&mut perf, "auto", Value::Bool(quality == Quality::Auto))?;
        put(&mut ui.transport, "perf", perf)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    pub id: String,
    pub axis: Axis,
    pub position: f64,
}
/// Composition coordinates. No default margins or invented safe-area ratios.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeArea {
    pub id: String,
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayState {
    pub guides_visible: bool,
    pub safe_areas_visible: bool,
    pub guides: Vec<Guide>,
    pub safe_areas: Vec<SafeArea>,
}
impl OverlayState {
    fn read(viewer: &Value) -> Result<Self, Error> {
        let Some(v) = viewer.get("chromeOverlays") else {
            return Ok(Self::default());
        };
        let mut state = Self {
            guides_visible: boolean(v, "guidesVisible", false)?,
            safe_areas_visible: boolean(v, "safeAreasVisible", false)?,
            ..Self::default()
        };
        if let Some(guides) = v.get("guides") {
            for guide in guides.as_array().ok_or(Error::Invalid("guides"))? {
                state.guides.push(Guide {
                    id: id(guide)?,
                    axis: match guide.get("axis").and_then(Value::as_str) {
                        Some("horizontal") => Axis::Horizontal,
                        Some("vertical") => Axis::Vertical,
                        _ => return Err(Error::Invalid("guide axis")),
                    },
                    position: number(guide, "position", f64::NAN)?,
                });
            }
        }
        if let Some(areas) = v.get("safeAreas") {
            for area in areas.as_array().ok_or(Error::Invalid("safe areas"))? {
                let area = SafeArea {
                    id: id(area)?,
                    left: number(area, "left", f64::NAN)?,
                    top: number(area, "top", f64::NAN)?,
                    right: number(area, "right", f64::NAN)?,
                    bottom: number(area, "bottom", f64::NAN)?,
                };
                if area.left >= area.right || area.top >= area.bottom {
                    return Err(Error::Invalid("safe area bounds"));
                }
                state.safe_areas.push(area);
            }
        }
        let mut ids = std::collections::HashSet::new();
        if state
            .guides
            .iter()
            .map(|g| &g.id)
            .chain(state.safe_areas.iter().map(|a| &a.id))
            .any(|id| !ids.insert(id))
        {
            return Err(Error::Invalid("duplicate overlay id"));
        }
        Ok(state)
    }
    pub(crate) fn toggle(ui: &mut UiState, safe: bool) -> Result<(), Error> {
        let current = Self::read(&ui.viewer)?;
        let mut v = ui
            .viewer
            .get("chromeOverlays")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let (key, value) = if safe {
            ("safeAreasVisible", !current.safe_areas_visible)
        } else {
            ("guidesVisible", !current.guides_visible)
        };
        put(&mut v, key, Value::Bool(value))?;
        put(&mut ui.viewer, "chromeOverlays", v)
    }
}
fn id(value: &Value) -> Result<String, Error> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or(Error::Invalid("overlay id"))
}
/// Rendering geometry is provisional until Opus review. No colour, stroke,
/// material, ordering, padding, or fade policy is carried by the model.
/// DESIGN-TODO(opus): safe-area presets, guide editing, and overlay presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSize {
    pub width: u32,
    pub height: u32,
}
pub fn preview_size(
    size: crate::Size,
    zoom: f64,
    dpr: f64,
    quality: Quality,
    effective: f64,
) -> Result<RenderSize, Error> {
    size.checked()?;
    finite(zoom)?;
    finite(dpr)?;
    finite(effective)?;
    if !(0.25..=1.0).contains(&effective) {
        return Err(Error::Invalid("effective quality"));
    }
    let scale = if quality == Quality::Auto {
        (zoom.max(0.05) * dpr.clamp(1.0, 2.0)).min(1.0) * effective
    } else {
        quality.scale()
    }
    .clamp(0.05, 1.0);
    let width = (size.width * scale).round().max(2.0);
    let height = (size.height * scale).round().max(2.0);
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(Error::Invalid("render extent"));
    }
    Ok(RenderSize {
        width: width as u32,
        height: height as u32,
    })
}
