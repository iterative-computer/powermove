//! Asynchronous viewer surface queries. GPU work stays on the render worker.
use crate::{Shared, Stamp, Viewer, render, studio};
use pm_protocol::common::Ticks;
use std::sync::Arc;

pub use pm_render::scene3d::provider::SurfaceHit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceQueryKind {
    Hover,
    Click,
}

#[derive(Clone, Debug)]
pub struct SurfaceReply {
    pub stamp: Stamp,
    pub time: Ticks,
    pub hit: Option<SurfaceHit>,
}
pub type Reply = Result<SurfaceReply, String>;
pub(crate) struct Pending {
    stamp: Stamp,
    time: Ticks,
    point: [f32; 2],
    kind: SurfaceQueryKind,
    reply: async_channel::Sender<Reply>,
}
pub(crate) fn cancel(view: &mut crate::View, reason: &str) {
    for pending in view.surface_queries.drain(..) {
        let _ = pending.reply.try_send(Err(reason.into()));
    }
}
impl Viewer {
    /// Query a displayed frame in normalized top-left composition coordinates.
    /// Convert fit/pan/zoom/crop into composition coordinates before calling.
    /// Await the receiver away from GPUI callbacks; stale replies are errors.
    /// Pending hovers coalesce; clicks are retained in a bounded queue.
    pub fn pick_surface(
        &self,
        stamp: Stamp,
        time: Ticks,
        point: [f32; 2],
        kind: SurfaceQueryKind,
    ) -> anyhow::Result<async_channel::Receiver<Reply>> {
        anyhow::ensure!(
            point
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
            "surface coordinates must be finite and in 0..=1"
        );
        let (reply, receive) = async_channel::bounded(1);
        let mut state = self.shared.state.lock().unwrap();
        anyhow::ensure!(
            !state.stop && state.halted.is_none(),
            "surface query worker is stopped"
        );
        let v = state
            .views
            .get_mut(&self.id)
            .ok_or_else(|| anyhow::anyhow!("viewer retired"))?;
        {
            anyhow::ensure!(v.stamp == stamp, "surface query frame is stale");
            if kind == SurfaceQueryKind::Hover
                && v.surface_queries
                    .back()
                    .is_some_and(|p| p.kind == SurfaceQueryKind::Hover)
            {
                if let Some(old) = v.surface_queries.pop_back() {
                    let _ = old.reply.try_send(Err("surface hover superseded".into()));
                }
            }
            anyhow::ensure!(v.surface_queries.len() < 8, "surface query queue is full");
            v.surface_queries.push_back(Pending {
                stamp,
                time,
                point,
                kind,
                reply,
            });
        }
        drop(state);
        self.shared.interactive.set(true);
        let _ = self.shared.wake.try_send(());
        Ok(receive)
    }
}

pub(crate) fn run_one(engine: &mut render::Engine, shared: &Arc<Shared>) {
    let work = {
        let mut state = shared.state.lock().unwrap();
        state.views.iter_mut().find_map(|(id, v)| {
            let p = v.surface_queries.pop_front()?;
            let scene = render::Scene {
                doc: v.config.snapshot.document.clone(),
                comp: v.config.composition.clone(),
                media: v.paths.clone(),
                content: v.content,
            };
            let scale = v
                .status
                .frame
                .as_ref()
                .filter(|f| f.stamp == p.stamp && f.time == p.time)
                .map_or(1., |f| f.scale);
            let (w, h) = v
                .config
                .output
                .crop
                .map_or((v.config.output.width, v.config.output.height), |c| {
                    (c.canvas_width, c.canvas_height)
                });
            let side = |v: u32| ((v as f64 * scale).round() as u32).max(1);
            let request = studio::SceneRequest {
                composition: scene.comp.clone(),
                revision: scene.doc.revision(),
                ticks: p.time.get(),
                width: side(w),
                height: side(h),
            };
            Some((
                *id,
                p,
                scene,
                request,
                v.resources.clone(),
                v.studio_preview,
                v.stamp,
            ))
        })
    };
    let Some((id, p, scene, request, resources, preview, current)) = work else {
        return;
    };
    let result = if current != p.stamp {
        Err("surface query frame is stale".into())
    } else {
        engine.set_resources(resources);
        engine.studio_preview(preview);
        engine.use_compat_document(p.stamp.doc_id);
        engine
            .pick_surface(&scene, &request, p.point)
            .map_err(|e| e.to_string())
    };
    let current = shared.state.lock().unwrap().views.get(&id).map(|v| v.stamp);
    let result = if current != Some(p.stamp) {
        Err("surface query frame changed before completion".into())
    } else {
        result.map(|hit| SurfaceReply {
            stamp: p.stamp,
            time: p.time,
            hit,
        })
    };
    let _ = p.reply.try_send(result);
}
