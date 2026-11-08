//! The Select tool's marquee (viewer.ts `startSelectionMarquee`): a drag from
//! empty canvas selects the active layers it fully encloses; Shift toggles
//! them in the selection instead.
use super::{GizmoSurface, transform};
use gpui::{prelude::*, *};
use pm_kernel::{Caller, QueryMirror, Request, SelectionCall};

pub struct Marquee {
    pub origin: Point<Pixels>,
    pub start: [f64; 2],
    pub shift: bool,
    pub shape: Option<crate::shape_tool::ShapeBox>,
}
/// Layers whose composition-space bounds sit inside the box.
fn enclosed(p: &pm_kernel::Published, b: crate::shape_tool::ShapeBox, time: f64) -> Vec<String> {
    let Some(root) = p.snapshot.root() else {
        return vec![];
    };
    let mut mirror = QueryMirror::new(p);
    let mut out = vec![];
    for layer in pm_kernel::model::list_layers(root, None) {
        if matches!(layer.kind.as_str(), "audio" | "camera") {
            continue;
        }
        let q = mirror.queries();
        if !q.active(None, &layer.id, time).unwrap_or(false) {
            continue;
        }
        let bounds = if layer.kind == "group" {
            crate::text_host::query(p, |q| q.group_bounds(None, &layer.id, time))
        } else {
            crate::text_host::query(p, |q| q.bounds(None, &layer.id, time))
        };
        let (Ok(Some(bounds)), Ok(world)) =
            (bounds, mirror.queries().world_matrix(None, &layer.id, time))
        else {
            continue;
        };
        // Group bounds are already in the composition.
        let world = if layer.kind == "group" {
            [1., 0., 0., 1., 0., 0.]
        } else {
            world
        };
        let inside = [
            [bounds.x0, bounds.y0],
            [bounds.x1, bounds.y0],
            [bounds.x1, bounds.y1],
            [bounds.x0, bounds.y1],
        ]
        .iter()
        .all(|&c| {
            let p = transform(&world, c);
            p[0] >= b.x0 && p[0] <= b.x1 && p[1] >= b.y0 && p[1] <= b.y1
        });
        if inside {
            out.push(layer.id);
        }
    }
    out
}
impl GizmoSurface {
    pub(super) fn marquee_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return false;
        };
        let at = self.to_comp(event.position, frame);
        let threshold = self.tokens.metrics.drag_threshold;
        let Some(m) = self.marquee.as_mut() else {
            return false;
        };
        let moved = event.position - m.origin;
        if m.shape.is_none() && f32::from(moved.x).hypot(f32::from(moved.y)) < threshold {
            return true;
        }
        m.shape = Some(crate::shape_tool::ShapeBox::from_drag(
            m.start, at, false, false,
        ));
        cx.notify();
        true
    }
    pub(super) fn marquee_up(&mut self, commit: bool, cx: &mut Context<Self>) -> bool {
        let Some(m) = self.marquee.take() else {
            return false;
        };
        cx.notify();
        if !commit {
            return true;
        }
        let Some(shape) = m.shape else {
            // A click on empty canvas clears the selection (Shift keeps it).
            if !m.shift {
                self.queue.push_back(super::Job::Select(None));
                self.pump(cx);
            }
            return true;
        };
        let (kernel, project, time) = (
            self.kernel.clone(),
            self.project.clone(),
            self.host.time(cx),
        );
        self.picks += 1;
        cx.spawn(async move |this, cx| {
            let poses = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    let inside = enclosed(&p, shape, time);
                    let ids = if m.shift {
                        let mut next: Vec<String> = p.selection.layers().to_vec();
                        for id in inside {
                            match next.iter().position(|s| *s == id) {
                                Some(i) => {
                                    next.remove(i);
                                }
                                None => next.push(id),
                            }
                        }
                        next
                    } else {
                        inside
                    };
                    let select = SelectionCall::Layers { ids, add: false };
                    kernel
                        .call(&project, Request::new(Caller::native(), select))
                        .await?
                        .ok()?;
                    super::read_at(kernel.published(&project)?.as_ref(), time)
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.picks -= 1;
                if let Some(poses) = poses {
                    s.poses = std::sync::Arc::new(poses);
                    s.draw(cx);
                }
            });
        })
        .detach();
        true
    }
    /// viewer-chrome.md: 1 pt `selected_text` at 70 % over a 1 pt black
    /// 40 % outer stroke, no fill.
    pub(super) fn marquee_paint(&self, frame: super::ViewFrame, window: &mut Window) {
        let Some(b) = self.marquee.as_ref().and_then(|m| m.shape) else {
            return;
        };
        let origin = frame.origin
            + point(
                px((b.x0 * frame.scale) as f32),
                px((b.y0 * frame.scale) as f32),
            );
        let size = size(
            px((b.w() * frame.scale) as f32),
            px((b.h() * frame.scale) as f32),
        );
        let inner = Bounds::new(origin, size);
        let outer = inner.dilate(px(1.));
        window.paint_quad(
            PaintQuad::new(outer)
                .border_widths(1f32)
                .border_color(rgba(0x0000_0066)),
        );
        let ink = rgba((self.tokens.colors.selected_text << 8) | 0xB3);
        window.paint_quad(PaintQuad::new(inner).border_widths(1f32).border_color(ink));
    }
}
