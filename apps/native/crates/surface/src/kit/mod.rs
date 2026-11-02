//! v4 UI kit. Worker-built, token-resolved native descriptions, never a browser renderer.
mod accessibility;
mod additions;
mod colour;
mod numeric;
#[cfg(test)]
mod part2_tests;
mod recipe;
mod validation;
mod visibility;
mod wheel;
pub use accessibility::*;
pub use additions::*;
pub use colour::*;
pub use numeric::*;
pub use recipe::*;
pub use validation::*;
pub use visibility::*;
pub use wheel::*;

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
    pub decorations: Decorations,
    /// Includes plain text/symbol descendants: resolve their inline token styles
    /// with the dark palette as well as their component recipes.
    pub over_image: std::collections::BTreeSet<Id>,
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
        let table = DesignTokenTable::from_design(tokens);
        Self::prepare_with_token_table(surface, tokens, &table, layout, focus, glass_available)
    }
    /// Additive host seam for the newer design token catalog. Unresolved values
    /// remain neutral placeholders, reported in decorations and diagnostics.
    pub fn prepare_with_token_table(
        surface: Arc<Surface>,
        tokens: &pm_design::Tokens,
        table: &DesignTokenTable,
        layout: &Layout,
        focus: Option<Id>,
        glass_available: bool,
    ) -> Self {
        let dark = pm_design::Tokens::new(pm_design::Scheme::Dark, tokens.accessibility);
        let image_tokens = if tokens.scheme == pm_design::Scheme::Dark {
            tokens
        } else {
            &dark
        };
        Self::prepare_with_palettes(
            surface,
            tokens,
            image_tokens,
            table,
            layout,
            focus,
            glass_available,
        )
    }
    /// Supply the active theme's dark palette so over-image controls preserve
    /// custom dark-theme colours even while the window uses a light appearance.
    pub fn prepare_with_palettes(
        surface: Arc<Surface>,
        tokens: &pm_design::Tokens,
        image_tokens: &pm_design::Tokens,
        table: &DesignTokenTable,
        layout: &Layout,
        focus: Option<Id>,
        glass_available: bool,
    ) -> Self {
        // Controls nested in an InspectorRow sit in its `field`-high value well.
        let mut in_row = std::collections::BTreeSet::new();
        for node in surface.nodes().values() {
            if Component::of(node) == Some(Component::InspectorRow) {
                let mut pending: Vec<Id> = node.children.clone();
                while let Some(id) = pending.pop() {
                    if in_row.insert(id)
                        && let Some(child) = surface.node(id)
                    {
                        pending.extend(child.children.iter().copied());
                    }
                }
            }
        }
        let components = surface
            .nodes()
            .values()
            .filter_map(|node| {
                Component::of(node).map(|kind| {
                    let local = if additions::over_image(&surface, node) {
                        image_tokens
                    } else {
                        tokens
                    };
                    let mut recipe = Recipe::new(kind, local, glass_available);
                    additions::customize(&mut recipe, node, local, table);
                    if kind == Component::PopUpButton && in_row.contains(&node.id) {
                        recipe.visual_height = Some(tokens.metrics.field);
                    }
                    (node.id, recipe)
                })
            })
            .collect();
        let decorations: Decorations = surface
            .nodes()
            .values()
            .filter_map(|node| {
                let local = if additions::over_image(&surface, node) {
                    image_tokens
                } else {
                    tokens
                };
                additions::decorate(node, local, table).map(|d| (node.id, d))
            })
            .collect();
        let mut diagnostics = validate_panel_with_table(&surface, tokens, table, layout);
        for (&id, decoration) in &decorations {
            if surface
                .node(id)
                .and_then(Component::of)
                .is_some_and(|c| Component::ADDITIONS.contains(&c))
            {
                diagnostics.extend(decoration.unresolved_tokens().into_iter().map(|name| {
                    crate::Diagnostic {
                        id,
                        prop: name.into(),
                        code: "missingDesignToken".into(),
                    }
                }));
            }
        }
        Self {
            over_image: surface
                .nodes()
                .values()
                .filter(|n| additions::over_image(&surface, n))
                .map(|n| n.id)
                .collect(),
            decorations,
            revision: surface.revision(),
            accessibility: accessibility_tree(&surface, layout, focus),
            diagnostics,
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
