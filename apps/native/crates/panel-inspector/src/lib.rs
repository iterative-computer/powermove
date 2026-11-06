//! Headless Inspector / Effect Controls. The kernel is the only document writer.
//!
//! Plans contain typed kernel requests, never document patches. Submit with
//! [`Inspector::submit`], await the returned future off the event handler, then
//! adopt the publication. No JS, platform UI, or blocking wait is used here.
//! See `docs/native/panels/inspector.md` for the shipped-source inventory.
mod catalog;
pub use catalog::{FieldDescriptor, FieldType};
pub mod content;
mod edit;
mod effects;
mod effect_definitions;
pub mod input;
mod relations;
pub mod text_animators;
pub mod v4_effects;
pub mod v4_properties;
mod view;

pub use edit::{Binding, EditSession, PropertyAction, ServiceIntent};
pub use effects::{EffectClipboard, EffectSelection, EffectUiAction, KeyDisposition};
pub use pm_kernel::commands::effects::{EffectDefinition, ParamDefinition};
pub use pm_model::Value;
pub use relations::ParentPick;
pub use view::{
    FieldState, Inspector, InspectorError, PropertyId, PropertyRow, ReadoutClock, RowWindow,
    ScrollMemory,
};
