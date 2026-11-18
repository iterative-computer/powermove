//! The composer's Select part of the screen (v1's, PM-399): the person drags
//! a box over the editor; the agent gets what was under it. On the viewer's
//! picture it is the composition there, rendered at the playhead as the
//! agent's own render_frames renders it (the window's drawing holds no
//! picture: the engine draws it below the window's controls); elsewhere it is
//! the window as drawn. A click without a drag takes the whole frame (on the
//! picture) or the whole window.
use crate::session::Controller;
use gpui::*;

/// The composition's region, in its pixels, under a box in window points
/// (`x, y, w, h`), for a frame drawn at `origin` and `scale` with `size`;
/// `None` when the box misses the picture. A box of no size is a click: on
/// the picture, the whole frame.
pub(crate) fn composition_region(
    (x, y, w, h): (f64, f64, f64, f64),
    origin: (f64, f64),
    scale: f64,
    size: [f64; 2],
) -> Option<(u32, u32, u32, u32)> {
    let [cw, ch] = size;
    let to = |px: f64, py: f64| ((px - origin.0) / scale, (py - origin.1) / scale);
    if w == 0. && h == 0. {
        let (cx, cy) = to(x, y);
        return (cx >= 0. && cy >= 0. && cx < cw && cy < ch)
            .then(|| (0, 0, cw.round() as u32, ch.round() as u32));
    }
    let (x0, y0) = to(x, y);
    let (x1, y1) = to(x + w, y + h);
    let (x0, y0) = (x0.clamp(0., cw), y0.clamp(0., ch));
    let (x1, y1) = (x1.clamp(0., cw), y1.clamp(0., ch));
    let (rw, rh) = ((x1 - x0).round(), (y1 - y0).round());
    (rw >= 1. && rh >= 1.).then(|| (x0.round() as u32, y0.round() as u32, rw as u32, rh as u32))
}

/// `rgba`'s pixels (`width` wide) inside `x, y, w, h`, clamped to the image.
pub(crate) fn crop(
    rgba: &[u8],
    width: u32,
    height: u32,
    (x, y, w, h): (u32, u32, u32, u32),
) -> (u32, u32, Vec<u8>) {
    let (x, y) = (x.min(width), y.min(height));
    let (w, h) = (w.min(width - x), h.min(height - y));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * width + x) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    (w, h, out)
}

pub(crate) fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = vec![];
    let mut encoder = png::Encoder::new(&mut bytes, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    drop(writer);
    Ok(bytes)
}

pub(crate) fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("frame too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return Err("Unexpected frame format".into());
    }
    Ok((info.width, info.height, buf))
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.);
    format!("{}:{:05.2}", (s / 60.).floor() as u64, s % 60.)
}

impl Controller {
    /// The picture the viewer presented of this project at `time`, settled
    /// and with every effect in it; `None` otherwise (a render is needed).
    fn shown_picture(&self, time: f64) -> Option<pm_engine_service::Picture> {
        if self.settling.is_some() || self.awaiting_extension_effects() {
            return None;
        }
        let doc = self.published.as_ref()?.snapshot.doc_id;
        // The viewer's settled picture, copied when it came to rest, if it
        // is the frame shown now.
        let status = self.viewer.as_ref()?.status();
        let (frame, picture) = (status.frame?, status.settled_picture?);
        let at = picture.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
        (picture.stamp.doc_id == doc
            && picture.stamp.revision == frame.stamp.revision
            && picture.time == frame.time
            && (at - time).abs() < 1e-3)
            .then_some(picture)
    }
    /// Where the composition is drawn in the window, at what scale (the
    /// viewer's frame; `None` while it shows nothing).
    pub(crate) fn composition_frame(&self) -> Option<crate::gizmo::ViewFrame> {
        let slot = self.host.borrow().viewer_rect.get()?;
        let scale = self.view.scale.get();
        let (w, h) = (self.dimensions[0] * scale, self.dimensions[1] * scale);
        Some(crate::gizmo::ViewFrame {
            origin: point(
                px(slot.x + (slot.w - w as f32) / 2. + self.view.shown_pan()[0] as f32),
                px(slot.y + (slot.h - h as f32) / 2. + self.view.shown_pan()[1] as f32),
            ),
            scale,
            size: self.dimensions,
        })
    }

    fn region_target_context(
        &self,
        rect: (f64, f64, f64, f64),
        region: Option<(u32, u32, u32, u32)>,
        cx: &App,
    ) -> serde_json::Value {
        use serde_json::json;
        let mut panels = vec![];
        if let Some(shell) = self.shell.as_ref().and_then(|s| s.upgrade()) {
            let shell = shell.read(cx);
            let digest = shell.layout_digest(cx);
            if let Some(docks) = digest["docks"].as_array() {
                for panel in docks
                    .iter()
                    .filter_map(|d| d["panels"].as_array())
                    .flatten()
                {
                    let Some(id) = panel["id"].as_str() else {
                        continue;
                    };
                    let Some(b) = shell.panel_rect(id) else {
                        continue;
                    };
                    let (x, y, w, h) = rect;
                    if x <= (b.x + b.w) as f64
                        && x + w >= b.x as f64
                        && y <= (b.y + b.h) as f64
                        && y + h >= b.y as f64
                    {
                        panels.push(
                            json!({"panelId":id,"title":panel["title"],"area":[b.x,b.y,b.w,b.h]}),
                        );
                    }
                }
            }
        }
        json!({"projectId":self.session.as_ref().map(|s| &s.project),"documentId":self.published.as_ref().map(|p| p.snapshot.doc_id),"revision":self.published.as_ref().map(|p|p.revision()),"windowId":self.window_key,"compositionId":self.published.as_ref().map(|p|p.snapshot.render_comp().as_str().to_owned()),"time":self.time(),"panelId":if region.is_some(){Some("viewer")}else{None},"panels":panels,"coordinateSpace":if region.is_some(){"composition-pixels"}else{"window-points"},"area":region.map(|(x,y,w,h)|[x,y,w,h]),"windowArea":[rect.0,rect.1,rect.2,rect.3],"selection":self.published.as_ref().map(|p|p.selection.layers()),"layersUnderBox":[],"layerHitMethod":"transformed-bounds intersection","unavailableLayerBounds":[]})
    }
    /// The box the person selected, captured and attached to the visible
    /// thread (errors are told in the Agent's own place).
    pub(crate) fn attach_region(
        &mut self,
        rect: (f64, f64, f64, f64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let region = self.composition_frame().and_then(|f| {
            composition_region(
                rect,
                (f32::from(f.origin.x) as f64, f32::from(f.origin.y) as f64),
                f.scale,
                f.size,
            )
        });
        let captured_time = self.time();
        let mut target_context = self.region_target_context(rect, region, cx);
        target_context["time"] = serde_json::json!(captured_time);
        target_context["compositionTime"] = serde_json::json!(captured_time);
        let captured_publication = self.published.clone();
        let capture_kernel = self.kernel.clone();
        // Repeated picks at one playhead can share a visible filename. Keep
        // their bytes immutable until attachment_records pairs them with metadata.
        let folder = self
            .profile
            .join("caches")
            .join("agent-picks")
            .join(pm_kernel::new_id("pick"));
        let caption = if region.is_some() {
            format!("Viewer · {}", clock(captured_time))
        } else {
            "Powermove".into()
        };
        let runtime = self.builtins.runtime.clone();
        let job: Box<dyn FnOnce() -> Result<(String, Vec<u8>), String> + Send> = match region {
            Some(region) => {
                let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
                    return;
                };
                let (time, size) = (captured_time, self.dimensions);
                let shown = self.shown_picture(time);
                Box::new(move || {
                    // At most 1280 wide; the region scales with it. The
                    // picture the viewer shows at the playhead when it has
                    // settled (read back, no render: the chip is there at
                    // once), else the agent's own still, rendered.
                    let width = size[0].round().clamp(160., 1280.) as u32;
                    let (w, h, rgba) = match shown.and_then(|p| p.srgb8(width)) {
                        Some(read) => read,
                        None => decode_png(&crate::mcp::still(&project, time, width)?)?,
                    };
                    let k = w as f64 / size[0];
                    let at = |v: u32| (v as f64 * k).round() as u32;
                    let whole = region == (0, 0, size[0].round() as u32, size[1].round() as u32);
                    let (cw, ch, cut) = crop(
                        &rgba,
                        w,
                        h,
                        (
                            at(region.0),
                            at(region.1),
                            at(region.2).max(1),
                            at(region.3).max(1),
                        ),
                    );
                    let name = if whole {
                        format!("Frame {}.png", clock(time))
                    } else {
                        format!("Frame {} selection.png", clock(time))
                    };
                    Ok((name, encode_png(cw, ch, &cut)?))
                })
            }
            None => {
                // The window as drawn (a frame drawn, read back, never shown).
                window.draw(cx).clear(cx);
                let frame = window.render_to_image();
                window.refresh();
                let Ok(frame) = frame else { return };
                let scale = window.scale_factor() as f64;
                let (fw, fh) = (frame.width(), frame.height());
                let (x, y, w, h) = rect;
                let region = if w == 0. && h == 0. {
                    (0, 0, fw, fh)
                } else {
                    let s = |v: f64| (v * scale).round().max(0.) as u32;
                    (s(x), s(y), s(w).max(1), s(h).max(1))
                };
                target_context["area"] = serde_json::json!([
                    region.0.min(fw) as f64 / scale,
                    region.1.min(fh) as f64 / scale,
                    region.2.min(fw.saturating_sub(region.0)) as f64 / scale,
                    region.3.min(fh.saturating_sub(region.1)) as f64 / scale,
                ]);
                let rgba = frame.into_raw();
                Box::new(move || {
                    let (cw, ch, cut) = crop(&rgba, fw, fh, region);
                    Ok(("Powermove selection.png".into(), encode_png(cw, ch, &cut)?))
                })
            }
        };
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let current_capture = || {
                        captured_publication.as_ref().is_none_or(|p| {
                            capture_kernel.published(&p.project).is_some_and(|now| {
                                now.snapshot.doc_id == p.snapshot.doc_id
                                    && now.revision() == p.revision()
                                    && now.snapshot.open_comp() == p.snapshot.open_comp()
                            })
                        })
                    };
                    if !current_capture() {
                        return Err(
                            "The document changed before region capture. Select the region again."
                                .into(),
                        );
                    }
                    if let Some(p) = &captured_publication {
                        let (layers, unavailable) = region_layers(&p, region, captured_time);
                        target_context["layersUnderBox"] = serde_json::json!(layers);
                        target_context["unavailableLayerBounds"] = serde_json::json!(unavailable);
                    }
                    let (name, png) = job()?;
                    if !current_capture() {
                        return Err(
                            "The document changed during region capture. Select the region again."
                                .into(),
                        );
                    }
                    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                    let path = folder.join(&name);
                    std::fs::write(&path, png).map_err(|e| e.to_string())?;
                    let mut records = crate::media::attachment_records(&[path])?;
                    for record in &mut records {
                        record["targetContext"] = target_context.clone();
                        record["screenPick"] = true.into();
                        record["caption"] = caption.clone().into();
                        record["promptNote"] = format!(
                            "Screen selection: {}",
                            record["caption"].as_str().unwrap_or(&name)
                        )
                        .into();
                    }
                    match runtime {
                        Some(runtime) => runtime.agent_attach(records),
                        None => Err("The Agent panel closed.".into()),
                    }
                })
                .await;
            if let Err(message) = outcome {
                let _ = this.update(cx, |v, cx| {
                    v.errors.borrow_mut().tell("Agent", &message);
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{composition_region, crop, decode_png, encode_png, region_layers};

    #[test]
    fn region_metadata_finds_transformed_layers_and_excludes_distant_layers() {
        let source = pm_model::json::parse(r##"{"id":"region-context-fixture","w":640,"h":360,"fps":30,"dur":10,"layers":[{"id":"near","name":"Near","type":"solid","from":0,"dur":10,"on":true,"p":{"position.x":{"v":100,"kf":[]},"position.y":{"v":100,"kf":[]}},"d":{"w":20,"h":20,"color":"#ffffff"}},{"id":"far","name":"Far","type":"solid","from":0,"dur":10,"on":true,"p":{"position.x":{"v":500,"kf":[]},"position.y":{"v":300,"kf":[]}},"d":{"w":20,"h":20,"color":"#ffffff"}}]}"##).unwrap();
        let p = pm_kernel::detached_publication("region-context-fixture", source);
        let (layers, unavailable) = region_layers(&p, Some((80, 80, 40, 40)), 1.);
        assert_eq!(layers, vec![serde_json::json!("near")]);
        assert!(unavailable.is_empty(), "{unavailable:?}");
        assert!(region_layers(&p, None, 1.).0.is_empty());
    }

    /// A frame drawn at (100, 50), half size, of a 1920×1080 composition.
    const ORIGIN: (f64, f64) = (100., 50.);
    const SIZE: [f64; 2] = [1920., 1080.];

    #[test]
    fn a_box_on_the_picture_is_that_part_of_the_composition() {
        assert_eq!(
            composition_region((150., 100., 100., 50.), ORIGIN, 0.5, SIZE),
            Some((100, 100, 200, 100))
        );
        // Over the picture's edge: only what is on it.
        assert_eq!(
            composition_region((50., 30., 100., 100.), ORIGIN, 0.5, SIZE),
            Some((0, 0, 100, 160))
        );
    }
    #[test]
    fn a_box_off_the_picture_is_the_window() {
        assert_eq!(
            composition_region((0., 0., 90., 40.), ORIGIN, 0.5, SIZE),
            None
        );
    }
    #[test]
    fn a_click_on_the_picture_is_the_whole_frame_and_off_it_the_window() {
        assert_eq!(
            composition_region((200., 100., 0., 0.), ORIGIN, 0.5, SIZE),
            Some((0, 0, 1920, 1080))
        );
        assert_eq!(
            composition_region((20., 20., 0., 0.), ORIGIN, 0.5, SIZE),
            None
        );
    }
    #[test]
    fn a_crop_keeps_its_pixels_and_stays_inside_the_image() {
        // 4×2: pixel value = its index.
        let rgba: Vec<u8> = (0..8u8).flat_map(|i| [i, i, i, 255]).collect();
        let (w, h, cut) = crop(&rgba, 4, 2, (1, 0, 2, 2));
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            cut.chunks(4).map(|p| p[0]).collect::<Vec<_>>(),
            vec![1, 2, 5, 6]
        );
        let (w, h, _) = crop(&rgba, 4, 2, (3, 1, 9, 9));
        assert_eq!((w, h), (1, 1));
    }
    #[test]
    fn a_crop_goes_through_png_unchanged() {
        let rgba: Vec<u8> = (0..16u8).flat_map(|i| [i, 2 * i, 255 - i, 255]).collect();
        let png = encode_png(4, 4, &rgba).unwrap();
        assert_eq!(decode_png(&png).unwrap(), (4, 4, rgba));
    }
}

fn region_layers(
    p: &pm_kernel::Published,
    region: Option<(u32, u32, u32, u32)>,
    time: f64,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    use serde_json::json;
    let mut layers = vec![];
    let mut unavailable = vec![];
    if let Some((x, y, w, h)) = region {
        let mut mirror = pm_kernel::QueryMirror::new(p);
        for layer in pm_kernel::model::list_layers(
            p.snapshot.root().unwrap(),
            p.snapshot.open_comp().as_deref(),
        ) {
            let outcome = crate::text_host::query(p, |q| q.bounds(None, &layer.id, time));
            match (
                outcome,
                mirror.queries().world_matrix(None, &layer.id, time),
            ) {
                (Ok(Some(b)), Ok(m)) => {
                    let points = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]]
                        .map(|[x, y]| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
                    let min =
                        |axis: usize| points.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
                    let max = |axis: usize| {
                        points
                            .iter()
                            .map(|p| p[axis])
                            .fold(f64::NEG_INFINITY, f64::max)
                    };
                    if min(0) <= (x + w) as f64
                        && max(0) >= x as f64
                        && min(1) <= (y + h) as f64
                        && max(1) >= y as f64
                    {
                        layers.push(json!(layer.id));
                    }
                }
                (Ok(None), _) => {}
                _ => unavailable.push(json!(layer.id)),
            }
        }
    }

    (layers, unavailable)
}
