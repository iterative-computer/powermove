//! Headless presentation and action models from Opus specifications at ce36cb6fc.
//! No rendering, invented tokens, permissions, engine mutation or OS prompt here.
//! Hosts drive actions by stable ids on workers and apply the existing tokens.
pub mod migration;
pub mod native;
pub mod updates;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub id: &'static str,
    pub label: &'static str,
    pub enabled: bool,
}
impl Button {
    pub fn new(id: &'static str, label: &'static str) -> Self {
        Self {
            id,
            label,
            enabled: true,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub surface: &'static str,
    pub title: String,
    pub line: String,
    pub status: Option<String>,
    pub buttons: Vec<Button>,
    pub default: Option<&'static str>,
}
