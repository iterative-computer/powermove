//! v4 UI kit. Worker-built, token-resolved native descriptions, never a browser renderer.
mod accessibility;
mod numeric;
mod recipe;
mod validation;
pub use accessibility::*;
pub use numeric::*;
pub use recipe::*;
pub use validation::*;

use crate::{Id, Surface};
use std::{collections::BTreeMap, sync::Arc};

/// The shell supplies final logical-point hit rectangles after native layout.
pub type Layout = BTreeMap<Id, accesskit::Rect>;

/// Immutable worker publication. Scope this together with its panel/realm generation.
#[derive(Clone, Debug)]
pub struct Panel {
    pub revision: u64,
    pub surface: Arc<Surface>,
    pub components: BTreeMap<Id, Recipe>,
    pub accessibility: accesskit::TreeUpdate,
    pub diagnostics: Vec<crate::Diagnostic>,
}
impl Panel {
    pub fn prepare(
        surface: Arc<Surface>,
        tokens: &pm_design::Tokens,
        layout: &Layout,
        focus: Option<Id>,
        glass_available: bool,
    ) -> Self {
        let components = surface
            .nodes()
            .values()
            .filter_map(|node| {
                Component::of(node)
                    .map(|kind| (node.id, Recipe::new(kind, tokens, glass_available)))
            })
            .collect();
        Self {
            revision: surface.revision(),
            accessibility: accessibility_tree(&surface, layout, focus),
            diagnostics: validate_panel(&surface, tokens, layout),
            surface,
            components,
        }
    }
}
/// AppKit/GPUI implements this enqueue-only boundary; no JS or layout callback is allowed.
pub trait PanelHost {
    fn enqueue(&mut self, panel: Arc<Panel>);
}
#[derive(Default)]
pub struct InspectPanelHost(pub Option<Arc<Panel>>);
impl PanelHost for InspectPanelHost {
    fn enqueue(&mut self, panel: Arc<Panel>) {
        self.0 = Some(panel);
    }
}
