//! Powermove native controls. Values in; transactional edit events out.
pub mod arith;
pub mod color;
pub mod field;
pub mod input;
pub mod logic;
pub mod native;
pub mod number;
pub mod text;
pub mod theme;
pub mod toggle;
pub use logic::EditEvent;
pub use number::{NumField, SliderField};
pub use text::TextField;
pub use toggle::{ToggleField, ToggleVariant};
/// Install input key bindings once during application setup.
pub fn init(cx: &mut gpui::App) {
    input::init(cx);
}
