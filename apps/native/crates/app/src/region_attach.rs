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
pub(crate) fn crop(rgba: &[u8], width: u32, height: u32, (x, y, w, h): (u32, u32, u32, u32)) -> (u32, u32, Vec<u8>) {
    let (x, y) = (x.min(width), y.min(height));
    let (w, h) = (w.min(width - x), h.min(height - y));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * width + x) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    (w, h, out)
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = vec![];
    let mut encoder = png::Encoder::new(&mut bytes, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    drop(writer);
    Ok(bytes)
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
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
        let frame = self.viewer.as_ref()?.status().frame?;
        let at = frame.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
        (frame.settled && frame.stamp.doc_id == doc && (at - time).abs() < 1e-3)
            .then_some(frame.picture)
            .flatten()
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
        let folder = self.profile.join("caches").join("agent-picks");
        let runtime = self.builtins.runtime.clone();
        let job: Box<dyn FnOnce() -> Result<(String, Vec<u8>), String> + Send> = match region {
            Some(region) => {
                let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
                    return;
                };
                let (time, size) = (self.time(), self.dimensions);
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
                    let (cw, ch, cut) = crop(&rgba, w, h, (at(region.0), at(region.1), at(region.2).max(1), at(region.3).max(1)));
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
                    let (name, png) = job()?;
                    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                    let path = folder.join(&name);
                    std::fs::write(&path, png).map_err(|e| e.to_string())?;
                    let records = crate::media::attachment_records(&[path])?;
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
    use super::{composition_region, crop, decode_png, encode_png};

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
        assert_eq!(composition_region((0., 0., 90., 40.), ORIGIN, 0.5, SIZE), None);
    }
    #[test]
    fn a_click_on_the_picture_is_the_whole_frame_and_off_it_the_window() {
        assert_eq!(composition_region((200., 100., 0., 0.), ORIGIN, 0.5, SIZE), Some((0, 0, 1920, 1080)));
        assert_eq!(composition_region((20., 20., 0., 0.), ORIGIN, 0.5, SIZE), None);
    }
    #[test]
    fn a_crop_keeps_its_pixels_and_stays_inside_the_image() {
        // 4×2: pixel value = its index.
        let rgba: Vec<u8> = (0..8u8).flat_map(|i| [i, i, i, 255]).collect();
        let (w, h, cut) = crop(&rgba, 4, 2, (1, 0, 2, 2));
        assert_eq!((w, h), (2, 2));
        assert_eq!(cut.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), vec![1, 2, 5, 6]);
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
