//! v4 extensions' viewer overlays (`overlays.viewer`): display-only surfaces
//! drawn over the composition and below the transform gizmos, as v1–3
//! extensions drew on the viewer's overlay canvas. Pointer input passes
//! through to the viewer. Each overlay learns where the composition sits.
use crate::v4_view::{Natives, SurfaceView};
use gpui::*;
use pm_builtins::{Builtins, Publication};
use serde_json::json;
use std::{cell::Cell, collections::BTreeMap, rc::Rc, sync::Arc};

pub struct ViewerOverlays {
    runtime: Option<Arc<Builtins>>,
    host: Rc<dyn crate::gizmo::Host>,
    tokens: pm_design::Tokens,
    natives: Natives,
    pending: BTreeMap<String, Publication>,
    views: BTreeMap<String, Entity<SurfaceView>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The frame last told to the realms, rounded to a hundredth of a point.
    sent: Option<[i64; 5]>,
}
impl ViewerOverlays {
    pub fn new(host: Rc<dyn crate::gizmo::Host>, tokens: pm_design::Tokens) -> Self {
        Self {
            runtime: None,
            host,
            tokens,
            natives: Natives::default(),
            pending: BTreeMap::new(),
            views: BTreeMap::new(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            sent: None,
        }
    }
    pub fn publish(&mut self, runtime: Arc<Builtins>, p: Publication, cx: &mut Context<Self>) {
        self.runtime = Some(runtime);
        match self.views.get(&p.panel) {
            Some(view) => view.update(cx, |v, cx| v.publish(p, cx)),
            None => {
                self.pending.insert(p.panel.clone(), p);
            }
        }
        // A new overlay needs the frame too.
        self.sent = None;
        cx.notify();
    }
    pub fn clear(&mut self, id: &str, cx: &mut Context<Self>) {
        self.views.remove(id);
        self.pending.remove(id);
        cx.notify();
    }
    /// The composition's rectangle in overlay points, sent when it moves.
    fn tell_frame(&mut self, cx: &App) {
        let (Some(runtime), Some(frame)) = (&self.runtime, self.host.frame(self.bounds.get(), cx))
        else {
            return;
        };
        let origin = self.bounds.get().origin;
        let x = f64::from(f32::from(frame.origin.x - origin.x));
        let y = f64::from(f32::from(frame.origin.y - origin.y));
        let (w, h) = (frame.size[0] * frame.scale, frame.size[1] * frame.scale);
        let key = [x, y, w, h, frame.scale].map(|v| (v * 100.).round() as i64);
        if self.sent == Some(key) {
            return;
        }
        self.sent = Some(key);
        runtime.viewer_frame(&json!({"x":x,"y":y,"width":w,"height":h,"scale":frame.scale}));
    }
}
impl Render for ViewerOverlays {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(runtime) = self.runtime.clone() {
            for (id, publication) in std::mem::take(&mut self.pending) {
                let (tokens, natives) = (self.tokens.clone(), self.natives.clone());
                let runtime = runtime.clone();
                let view = cx
                    .new(|cx| SurfaceView::new(runtime, publication, tokens, natives, window, cx));
                self.views.insert(id, view);
            }
        }
        self.tell_frame(cx);
        let bounds = self.bounds.clone();
        let this = cx.entity().downgrade();
        div()
            .id("viewer-extension-overlays")
            .size_full()
            .relative()
            .child(
                canvas(
                    move |b, _, cx| {
                        if bounds.get() != b {
                            bounds.set(b);
                            // Laid out somewhere new: tell the overlays after paint.
                            let this = this.clone();
                            cx.defer(move |cx| {
                                let _ = this.update(cx, |v, cx| {
                                    v.sent = None;
                                    v.tell_frame(cx);
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .children(
                self.views
                    .values()
                    .map(|view| div().absolute().inset_0().child(view.clone())),
            )
    }
}
