//! One-shot composition colour readback on the existing render worker.
use crate::{Shared, Viewer, render};
use pm_protocol::common::{Size, Ticks};
use std::sync::Arc;
pub type Reply = Result<[f32; 4], String>;
pub(crate) struct Pending {
    time: Ticks,
    deadline: std::time::Instant,
    stamp: crate::Stamp,
    size: Size,
    pixel: [u32; 2],
    reply: async_channel::Sender<Reply>,
}
impl Viewer {
    /// Coordinates are composition pixels, before viewer pan/zoom and display tone.
    pub fn sample_color(
        &self,
        time: Ticks,
        size: Size,
        pixel: [u32; 2],
    ) -> anyhow::Result<async_channel::Receiver<Reply>> {
        anyhow::ensure!(
            pixel[0] < size.width && pixel[1] < size.height,
            "pixel outside composition"
        );
        let (reply, rx) = async_channel::bounded(1);
        self.edit(|v| {
            anyhow::ensure!(v.picker_samples.len() < 8, "color sampling queue full");
            v.picker_samples.push_back(Pending {
                stamp: v.stamp,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
                time,
                size,
                pixel,
                reply,
            });
            Ok(())
        })?;
        Ok(rx)
    }
}
pub(crate) fn run_one(engine: &mut render::Engine, shared: &Arc<Shared>) {
    let work = {
        let mut state = shared.state.lock().unwrap();
        state.views.iter_mut().find_map(|(id, v)| {
            let p = v.picker_samples.pop_front()?;
            let scene = render::Scene {
                doc: v
                    .transient_snapshot
                    .as_ref()
                    .unwrap_or(&v.config.snapshot)
                    .document
                    .clone(),
                comp: v.config.composition.clone(),
                media: Arc::new(v.config.media.export_paths(false)),
                content: v.transient_content.unwrap_or(v.content),
            };
            Some((*id, p, scene, v.resources.clone(), v.stamp))
        })
    };
    let Some((id, p, scene, resources, stamp)) = work else {
        return;
    };
    if p.reply.is_closed() {
        return;
    }
    if p.stamp != stamp {
        let _ = p
            .reply
            .try_send(Err("Composition changed before sampling".into()));
        return;
    }
    engine.set_resources(resources);
    engine.use_compat_document(stamp.doc_id);
    let raw = engine.export(&scene, p.time, p.size);
    if raw.as_ref().is_err_and(render::transient) && std::time::Instant::now() < p.deadline {
        let mut state = shared.state.lock().unwrap();
        if let Some(v) = state.views.get_mut(&id).filter(|v| v.stamp == p.stamp) {
            v.picker_samples.push_front(p);
            return;
        }
    }
    let result = raw.map_err(|e| e.to_string()).and_then(|rgba| {
        let i = ((p.pixel[1] as usize * p.size.width as usize) + p.pixel[0] as usize) * 4;
        let c = rgba.get(i..i + 4).ok_or("composition pixel missing")?;
        // Renderer's final composition encoding is premultiplied E. Undo only
        // premultiplication: no display profile, loupe or OS sampler is involved.
        let a = c[3].clamp(0., 1.);
        let channel = |v: f32| if a > 0. { v / a } else { 0. };
        Ok([channel(c[0]), channel(c[1]), channel(c[2]), a])
    });
    let current = shared.state.lock().unwrap().views.get(&id).map(|v| v.stamp);
    let result = if current == Some(p.stamp) {
        result
    } else {
        Err("Composition changed while sampling".into())
    };
    let _ = p.reply.try_send(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colour_eyedropper_samples_headless_composition() {
        let source=pm_model::json::parse(r##"{"id":"sample","compId":"c","w":8,"h":8,"fps":30,"dur":1,"bg":"#0A84FF","assets":{},"comps":{},"layers":[]}"##).unwrap();
        let scene = crate::Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(source)),
                1,
            )),
            comp: "c".into(),
            media: Default::default(),
            content: 1,
        };
        let mut headless = crate::Headless::new().unwrap();
        let pixels = headless
            .export_reference(
                &scene,
                Ticks::ZERO,
                Size {
                    width: 8,
                    height: 8,
                },
            )
            .unwrap();
        let i = (4 * 8 + 4) * 4;
        let a = pixels[i + 3];
        for (actual, byte) in pixels[i..i + 3].iter().zip([10., 132., 255.]) {
            assert!((actual / a - byte / 255.).abs() < 0.01);
        }
    }
}
