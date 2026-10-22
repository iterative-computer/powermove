//! What the bridge test source shows for a viewer state: the composition
//! rectangle under zoom and pan, the marker at its center, the editor camera,
//! and the overlay geometry the descriptor carries.
//!
//! Units: `ViewerState.size` is device pixels; zoom is CSS pixels per
//! composition pixel and pan is CSS pixels (as the viewer keeps them), so
//! both scale by the device pixel ratio. Overlay geometry is in frame
//! (device) pixels.

use pm_gpu::bridge_out::test_source::view_projection;
use pm_protocol::common::{Mat3, Rect, Size};
use pm_protocol::frame::{OverlayGeometry, OverlayLayer, ViewerState};

/// The test composition, in composition pixels (S1's 4K frame).
pub const COMPOSITION: Size = Size {
    width: 3840,
    height: 2160,
};

/// Side of the marker square in frame pixels.
pub const MARKER: f64 = 96.0;

/// The overlay layer id of the marker.
pub const MARKER_LAYER: &str = "bridge-test-marker";

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub composition: [f32; 4],
    pub marker: [f32; 4],
    pub camera: Option<[[f32; 4]; 4]>,
    pub overlay: OverlayGeometry,
}

/// The composition rectangle in frame pixels.
pub fn composition_rect(state: &ViewerState) -> Rect {
    let dpr = state.device_pixel_ratio;
    let scale = state.zoom * dpr;
    let width = f64::from(COMPOSITION.width) * scale;
    let height = f64::from(COMPOSITION.height) * scale;
    Rect {
        x: (f64::from(state.size.width) - width) / 2.0 + state.pan[0] * dpr,
        y: (f64::from(state.size.height) - height) / 2.0 + state.pan[1] * dpr,
        width,
        height,
    }
}

pub fn scene(state: &ViewerState) -> Scene {
    let rect = composition_rect(state);
    let scale = state.zoom * state.device_pixel_ratio;
    let cx = rect.x + rect.width / 2.0;
    let cy = rect.y + rect.height / 2.0;
    let half = MARKER / 2.0;
    let marker = [cx - half, cy - half, MARKER, MARKER];
    // Composition pixels to frame pixels (column-major 3×3).
    let world: Mat3 = [scale, 0.0, 0.0, 0.0, scale, 0.0, rect.x, rect.y, 1.0];
    let overlay = OverlayGeometry {
        viewport: rect,
        layers: vec![OverlayLayer {
            layer: MARKER_LAYER.into(),
            world_2d: Some(world),
            world_3d: None,
            bounds: [
                [marker[0], marker[1]],
                [marker[0] + MARKER, marker[1]],
                [marker[0] + MARKER, marker[1] + MARKER],
                [marker[0], marker[1] + MARKER],
            ],
            paths: Vec::new(),
        }],
        caret: None,
    };
    let aspect = f64::from(state.size.width) / f64::from(state.size.height.max(1));
    Scene {
        composition: [
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        ],
        marker: marker.map(|v| v as f32),
        camera: state
            .view
            .as_ref()
            .and_then(|view| view.camera.as_ref())
            .map(|camera| view_projection(camera, aspect)),
        overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::color::ViewerColorSpace;
    use pm_protocol::common::Seq;
    use pm_protocol::frame::{MediaPreference, PreviewResolution};

    pub(crate) fn state(zoom: f64, pan: [f64; 2]) -> ViewerState {
        ViewerState {
            headless: None,
            composition: "main".into(),
            size: Size::new(3840, 2160),
            device_pixel_ratio: 2.0,
            zoom,
            pan,
            resolution: PreviewResolution::Full,
            media: MediaPreference::Automatic,
            color_space: ViewerColorSpace::Srgb,
            view: None,
            play_every_frame: false,
            edr: false,
            view_seq: Seq::new(1).unwrap(),
            vsync: None,
            depth_id: None,
            chrome_selection: None,
        }
    }

    #[test]
    fn fit_zoom_fills_the_frame_and_pan_moves_the_marker() {
        let s = scene(&state(0.5, [0.0, 0.0]));
        assert_eq!(s.composition, [0.0, 0.0, 3840.0, 2160.0]);
        assert_eq!(s.marker, [1872.0, 1032.0, 96.0, 96.0]);
        let moved = scene(&state(0.5, [100.0, -20.0]));
        assert_eq!(moved.marker, [2072.0, 992.0, 96.0, 96.0]);
        assert_eq!(moved.overlay.layers[0].bounds[0], [2072.0, 992.0]);
        assert_eq!(moved.overlay.viewport.x, 200.0);
        assert!(moved.camera.is_none());
    }
}
