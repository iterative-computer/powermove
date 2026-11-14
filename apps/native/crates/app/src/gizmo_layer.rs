//! The viewer's on-canvas outline and handles as Core Animation shape
//! layers over the picture, set on the main thread whenever the gizmo
//! changes. A drag's box therefore follows the pointer at the pointer's pace
//! however long the engine takes to draw the picture under it (the picture
//! follows on its own frames). Guides and safe areas stay in the engine's
//! overlay.
use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use objc2_core_graphics::{CGColor, CGColorSpace, CGMutablePath};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAShapeLayer, CATransaction};
use pm_engine_service::OverlayLine;

/// Where the lines go: the composition's rectangle in the viewer view's
/// coordinates (points, top-left origin), and that view's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The composition's top-left corner and size in the viewer view (pt).
    pub origin: [f64; 2],
    pub size: [f64; 2],
    /// The viewer view's size (pt): its layer is y-up.
    pub view: [f64; 2],
}

/// The lines grouped by colour, in the viewer view's y-up layer points.
pub fn segments(lines: &[OverlayLine], at: Placement) -> Vec<([f32; 4], Vec<[f64; 4]>)> {
    let mut groups: Vec<([f32; 4], Vec<[f64; 4]>)> = vec![];
    for line in lines {
        let [x0, y0, x1, y1] = line.ends.map(f64::from);
        let point = |x: f64, y: f64| {
            (
                at.origin[0] + x * at.size[0],
                at.view[1] - (at.origin[1] + y * at.size[1]),
            )
        };
        let (a, b) = (point(x0, y0), point(x1, y1));
        if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
            continue;
        }
        let segment = [a.0, a.1, b.0, b.1];
        match groups.iter_mut().find(|(c, _)| *c == line.color) {
            Some((_, s)) => s.push(segment),
            None => groups.push((line.color, vec![segment])),
        }
    }
    groups
}

pub struct GizmoLayer {
    parent: Retained<CALayer>,
    root: Retained<CALayer>,
    shapes: Vec<Retained<CAShapeLayer>>,
}
impl GizmoLayer {
    pub fn new(parent: Retained<CALayer>) -> Self {
        let root = CALayer::new();
        root.setMasksToBounds(true);
        root.setZPosition(10.);
        parent.addSublayer(&root);
        Self {
            parent,
            root,
            shapes: vec![],
        }
    }
    /// Still attached to `parent` (the viewer's layer can be replaced).
    pub fn on(&self, parent: &CALayer) -> bool {
        std::ptr::eq(&*self.parent, parent)
    }
    pub fn set(&mut self, lines: &[OverlayLine], at: Placement, scale: f64) {
        let groups = segments(lines, at);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.root.setFrame(NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(at.view[0], at.view[1]),
        ));
        while self.shapes.len() < groups.len() {
            let shape = CAShapeLayer::new();
            shape.setFillColor(None);
            shape.setLineWidth(1.);
            self.root.addSublayer(&shape);
            self.shapes.push(shape);
        }
        let space = CGColorSpace::with_name(Some(unsafe {
            objc2_core_graphics::kCGColorSpaceExtendedLinearDisplayP3
        }));
        for (i, shape) in self.shapes.iter().enumerate() {
            let Some((color, segments)) = groups.get(i) else {
                shape.setHidden(true);
                continue;
            };
            let path = CGMutablePath::new();
            for [x0, y0, x1, y1] in segments {
                unsafe {
                    CGMutablePath::move_to_point(Some(&path), std::ptr::null(), *x0, *y0);
                    CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), *x1, *y1);
                }
            }
            let components = color.map(f64::from);
            let cg = space
                .as_deref()
                .and_then(|space| unsafe { CGColor::new(Some(space), components.as_ptr()) });
            shape.setHidden(false);
            shape.setContentsScale(scale);
            shape.setStrokeColor(cg.as_deref());
            shape.setPath(Some(&path));
        }
        CATransaction::commit();
    }
    pub fn clear(&mut self) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for shape in &self.shapes {
            shape.setHidden(true);
        }
        CATransaction::commit();
    }
}
impl Drop for GizmoLayer {
    fn drop(&mut self) {
        self.root.removeFromSuperlayer();
    }
}
/// The viewer view's layer, as a CALayer.
pub fn layer_of(object: &AnyObject) -> Option<Retained<CALayer>> {
    let layer: Option<Retained<CALayer>> = unsafe { msg_send![object, layer] };
    layer
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A line across the composition's top edge lands at the composition's
    /// rectangle in the viewer view, in its y-up layer coordinates.
    #[test]
    fn lines_land_on_the_composition_in_the_viewer() {
        let at = Placement {
            origin: [100., 50.],
            size: [400., 225.],
            view: [600., 400.],
        };
        let red = [1., 0., 0., 1.];
        let lines = [
            OverlayLine {
                ends: [0., 0., 1., 0.],
                color: red,
            },
            OverlayLine {
                ends: [0.5, 0.5, 0.5, 1.],
                color: red,
            },
            OverlayLine {
                ends: [0., 0., 0., 1.],
                color: [0., 1., 0., 1.],
            },
        ];
        let groups = segments(&lines, at);
        assert_eq!(groups.len(), 2, "one shape per colour");
        assert_eq!(groups[0].1[0], [100., 350., 500., 350.]);
        assert_eq!(groups[0].1[1], [300., 237.5, 300., 125.]);
        assert_eq!(groups[1].1[0], [100., 350., 100., 125.]);
    }
}
