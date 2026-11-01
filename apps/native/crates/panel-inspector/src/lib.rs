//! Headless Inspector / Effect Controls. The kernel is the only document writer.
//!
//! Plans contain typed kernel requests, never document patches. Submit with
//! [`Inspector::submit`], await the returned future off the event handler, then
//! adopt the publication. No JS, platform UI, or blocking wait is used here.
//! See `docs/native/panels/inspector.md` for the shipped-source inventory.
mod edit;
mod effects;
pub mod input;
mod view;

pub use edit::{Binding, EditSession, PropertyAction, ServiceIntent};
pub use effects::{EffectClipboard, EffectSelection, EffectUiAction, KeyDisposition};
pub use pm_kernel::commands::effects::{EffectDefinition, ParamDefinition};
pub use pm_model::Value;
pub use view::{
    FieldState, Inspector, InspectorError, PropertyId, PropertyRow, ReadoutClock, RowWindow,
    ScrollMemory,
};
