//! Atomic, inspectable v4 native UI state. No JS, DOM, renderer, or platform dependency.
pub mod binding;
pub mod interaction;
pub mod style;
pub mod tree;
pub use tree::*;
pub mod controls;

/// macOS owns the renderer. Implementations enqueue immutable accepted trees;
/// this interface never synchronously calls JS or performs platform layout reads.
pub trait NativeHost {
    fn enqueue(&mut self, tree: std::sync::Arc<Surface>, diagnostics: Vec<Diagnostic>);
}
/// Linux/headless publication double; also useful for an agent inspector.
#[derive(Default)]
pub struct InspectHost {
    pub tree: Option<std::sync::Arc<Surface>>,
    pub diagnostics: Vec<Diagnostic>,
}
impl NativeHost for InspectHost {
    fn enqueue(&mut self, tree: std::sync::Arc<Surface>, diagnostics: Vec<Diagnostic>) {
        self.tree = Some(tree);
        self.diagnostics = diagnostics;
    }
}
