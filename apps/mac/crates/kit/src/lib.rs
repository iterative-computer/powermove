//! Powermove native controls. Values in; transactional edit events out.
pub mod arith;
pub mod color;
pub mod input;
pub mod logic;
pub mod theme;

/// Install input key bindings once during application setup.
pub fn init(cx: &mut gpui::App) {
    input::init(cx);
}
pub mod field;
pub mod number;
pub use logic::EditEvent;
pub use number::{NumField, SliderField};
pub mod select;
pub mod text;
pub mod toggle;
pub use select::{FontField, MenuItem, PopoverMenu, Segmented, SelectField, SelectOption};
pub use text::TextField;
pub use toggle::{ToggleField, ToggleVariant};
pub mod point;
pub mod ramp;
pub use point::PointField;
pub use ramp::{FillStop, RampField, RampValue};
pub mod layout;
pub use layout::{
    ActionButton, BackTitle, Disclosure, KeyframeButton, KeyframeState, Row, Section, Tooltip,
};
pub mod picker;
pub use picker::ColorField;
pub mod fill;
pub use fill::{FillField, FillMode, FillValue};
pub mod binding;
pub mod motion;
pub mod native;
