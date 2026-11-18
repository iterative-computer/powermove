//! An AppKit slot inside a GPUI scroll layer follows the layer's offset on
//! the frames that replay it (they run no paint of the slot's own): the
//! slot's view host stages it again from where it was painted, shifted.
use gpui::{Bounds, ContentMask, Pixels, ScrollLayerId, Window};
use std::cell::Cell;

#[derive(Default)]
pub struct LayerStage(Cell<Option<(ScrollLayerId, Bounds<Pixels>, ContentMask<Pixels>)>>);

impl LayerStage {
    /// Where a slot painted now stands: its bounds and clip, the clip held
    /// to its scroll layer's area when it is inside one (the layer paints
    /// past its edges).
    pub fn painted(
        &self,
        bounds: Bounds<Pixels>,
        mask: ContentMask<Pixels>,
        window: &Window,
    ) -> (Bounds<Pixels>, ContentMask<Pixels>) {
        let Some(layer) = window.scroll_layer() else {
            self.0.set(None);
            return (bounds, mask);
        };
        self.0.set(Some((layer, bounds, mask)));
        let clip = window
            .scroll_layer_shift(layer)
            .map_or(mask, |(_, area)| mask.intersect(&area));
        (bounds, clip)
    }

    pub fn recorded_in_scroll_layer(&self) -> bool {
        self.0.get().is_some()
    }

    /// Where the slot stands on a frame that did not paint it, when it is
    /// inside a scroll layer that is still there.
    pub fn kept(&self, window: &Window) -> Option<(Bounds<Pixels>, ContentMask<Pixels>)> {
        let (layer, bounds, mut mask) = self.0.get()?;
        let (delta, area) = window.scroll_layer_shift(layer)?;
        mask.bounds.origin += delta;
        mask.shape.origin += delta;
        Some((
            Bounds::new(bounds.origin + delta, bounds.size),
            mask.intersect(&area),
        ))
    }
}

pub fn vertical_fade(mask: ContentMask<Pixels>) -> [f64; 2] {
    [mask.fade_out.top, mask.fade_out.bottom].map(|value| f64::from(f32::from(value)))
}
