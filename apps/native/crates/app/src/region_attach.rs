//! The composer's Select part of the screen (v1's, PM-399): the person drags
//! a box over the editor; the agent gets what was under it. On the viewer's
//! picture it is the composition there, rendered at the playhead as the
//! agent's own render_frames renders it (the window's drawing holds no
//! picture: the engine draws it below the window's controls); elsewhere it is
//! the window as drawn. A click without a drag takes the whole frame (on the
//! picture) or the whole window.
use crate::session::Controller;
use gpui::*;
use serde_json::{Value, json};

const MAX_CAPTURE_EDGE: u32 = 2048;
type CapturePixels = (String, u32, u32, Vec<u8>);

fn viewer_readback_width([w, h]: [f64; 2]) -> u32 {
    (w * (f64::from(MAX_CAPTURE_EDGE) / w.max(h)).min(1.))
        .floor()
        .clamp(1., 1280.) as u32
}

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
    if ![x, y, w, h, origin.0, origin.1, scale, cw, ch]
        .into_iter().all(f64::is_finite)
        || scale <= 0. || cw <= 0. || ch <= 0. || w < 0. || h < 0.
    {
        return None;
    }
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
    let stride = width as usize * 4;
    if stride.checked_mul(height as usize).is_none_or(|len| len > rgba.len()) {
        return (0, 0, Vec::new());
    }
    let (x, y) = (x.min(width), y.min(height));
    let (w, h) = (w.min(width - x), h.min(height - y));
    let row_bytes = w as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * h as usize);
    for row in y..y + h {
        let start = row as usize * stride + x as usize * 4;
        out.extend_from_slice(&rgba[start..start + row_bytes]);
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

/// Build the single durable attachment record directly from the capture.
fn region_record(
    (name, width, height, pixels): CapturePixels,
    target_context: Value,
    caption: &str,
) -> Result<Value, String> {
    use base64::Engine;
    if width == 0 || height == 0 {
        return Err("The selected region is outside the picture.".into());
    }
    let pixels = image::RgbaImage::from_raw(width, height, pixels)
        .ok_or("The selected region could not be read.")?;
    let edge = width.max(height);
    let pixels = if edge > MAX_CAPTURE_EDGE {
        let scaled = |value: u32| {
            ((u64::from(value) * u64::from(MAX_CAPTURE_EDGE)) / u64::from(edge)).max(1) as u32
        };
        image::imageops::resize(
            &pixels,
            scaled(width),
            scaled(height),
            image::imageops::FilterType::Triangle,
        )
    } else {
        pixels
    };
    let (width, height) = pixels.dimensions();
    let png = encode_png(width, height, pixels.as_raw())?;
    let thumbnail = crate::media_thumbs::capture_thumbnail(&png, width, height, pixels.as_raw());
    let data = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(json!({
        "id": pm_kernel::new_id("attachment"),
        "name": name,
        "size": png.len(),
        "type": "image/png",
        "dataUrl": format!("data:image/png;base64,{data}"),
        "thumbnail": thumbnail,
        "targetContext": target_context,
        "screenPick": true,
        "caption": caption,
        "promptNote": format!("Screen selection: {caption}"),
    }))
}

fn run_capture(
    executor: &BackgroundExecutor,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Task<Result<(), String>> {
    executor.spawn(async move { work() })
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.);
    format!("{}:{:05.2}", (s / 60.).floor() as u64, s % 60.)
}

fn region_panels(
    shell: Option<&pm_shell::view::Editor>,
    bounds: (f64, f64, f64, f64),
    cx: &App,
) -> Vec<serde_json::Value> {
    let mut panels = vec![];
    if let Some(shell) = shell {
        let known = shell.panels(cx);
        for id in shell
            .layout
            .docks
            .iter()
            .flat_map(|dock| &dock.groups)
            .filter_map(|group| group.tabs.get(group.active))
        {
            let Some(b) = shell.panel_rect(id) else {
                continue;
            };
            let (x, y, w, h) = bounds;
            if x <= (b.x + b.w) as f64
                && x + w >= b.x as f64
                && y <= (b.y + b.h) as f64
                && y + h >= b.y as f64
            {
                panels.push(json!({"panelId":id,"title":known.title(id),"area":[b.x,b.y,b.w,b.h]}));
            }
        }
    }
    panels
}

/// Capture from the editor after GPUI returns the Controller to the app.
/// Both layout metadata and a window redraw call back into the Controller.
fn with_region_panels(
    shell: Option<WeakEntity<pm_shell::view::Editor>>,
    rect: (f64, f64, f64, f64),
    window: &mut Window,
    cx: &mut App,
    capture_window: bool,
    capture: impl FnOnce(Vec<serde_json::Value>, &mut Window, &mut App) + 'static,
) {
    window.defer(cx, move |window, cx| {
        if capture_window {
            // Draw into the buffer for readback without presenting a frame.
            window.draw(cx).clear(cx);
        }
        let shell = shell.as_ref().and_then(WeakEntity::upgrade);
        let bounds = if capture_window && rect.2 == 0. && rect.3 == 0. {
            let Size { width, height } = window.viewport_size();
            (0., 0., f32::from(width) as f64, f32::from(height) as f64)
        } else {
            rect
        };
        let panels = region_panels(shell.as_ref().map(|shell| shell.read(cx)), bounds, cx);
        capture(panels, window, cx);
    });
}

/// Read the viewer's settled picture on the capture worker. Fetching its status
/// can wait for the engine mutex, even when a completed frame is available.
fn shown_picture(
    viewer: &pm_engine_service::Viewer,
    doc: u64,
    revision: u64,
    time: f64,
) -> Option<pm_engine_service::Picture> {
    let status = viewer.status();
    let (frame, picture) = (status.frame?, status.settled_picture?);
    let at = picture.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
    (picture.stamp.doc_id == doc
        && picture.stamp.revision == revision
        && picture.stamp.revision == frame.stamp.revision
        && picture.time == frame.time
        && (at - time).abs() < 1e-3)
        .then_some(picture)
}

impl Controller {
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
        captured_time: f64,
    ) -> serde_json::Value {
        json!({"projectId":self.session.as_ref().map(|s| &s.project),"documentId":self.published.as_ref().map(|p| p.snapshot.doc_id),"revision":self.published.as_ref().map(|p|p.revision()),"windowId":self.window_key,"compositionId":self.published.as_ref().map(|p|p.snapshot.render_comp().as_str().to_owned()),"time":captured_time,"compositionTime":captured_time,"panelId":if region.is_some(){Some("viewer")}else{None},"coordinateSpace":if region.is_some(){"composition-pixels"}else{"window-points"},"area":region.map(|(x,y,w,h)|[x,y,w,h]),"windowArea":[rect.0,rect.1,rect.2,rect.3],"selection":null,"layersUnderBox":[],"layerHitMethod":"transformed-bounds intersection","unavailableLayerBounds":[]})
    }
    /// The box the person selected, captured and attached to the visible
    /// thread (errors are told in the Agent's own place).
    pub(crate) fn attach_region(
        &mut self,
        rect: (f64, f64, f64, f64),
        agent_project: String,
        agent_thread: String,
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
        let mut target_context = self.region_target_context(rect, region, captured_time);
        let captured_publication = self.published.clone();
        let capture_kernel = self.kernel.clone();
        let caption = if region.is_some() {
            format!("Viewer · {}", clock(captured_time))
        } else {
            "Powermove".into()
        };
        let runtime = self.builtins.runtime.clone();
        let dimensions = self.dimensions;
        let project = self.session.as_ref().map(|s| s.project.clone());
        let viewer =
            if region.is_some() && self.settling.is_none() && !self.awaiting_extension_effects() {
                self.viewer.clone().zip(
                    self.published
                        .as_ref()
                        .map(|p| (p.snapshot.doc_id, p.revision())),
                )
            } else {
                None
            };
        let this = cx.weak_entity();
        with_region_panels(
            self.shell.clone(),
            rect,
            window,
            cx,
            region.is_none(),
            move |panels, window, cx| {
                target_context["panels"] = serde_json::json!(panels);
                let job: Box<dyn FnOnce() -> Result<CapturePixels, String> + Send> = match region {
                    Some(region) => {
                        let Some(project) = project else { return };
                        let (time, size) = (captured_time, dimensions);
                        Box::new(move || {
                            // At most 1280 wide; the region scales with it. The
                            // picture the viewer shows at the playhead when it has
                            // settled (read back, no render: the chip is there at
                            // once), else the agent's own still, rendered.
                            let width = viewer_readback_width(size);
                            let shown = viewer.and_then(|(viewer, (doc, revision))| {
                                shown_picture(&viewer, doc, revision, time)
                            });
                            let (w, h, rgba) = match shown.and_then(|p| p.srgb8(width)) {
                                Some(read) => read,
                                None => decode_png(&crate::mcp::still(&project, time, width)?)?,
                            };
                            let at_x = |v: u32| (v as f64 * f64::from(w) / size[0]).round() as u32;
                            let at_y = |v: u32| (v as f64 * f64::from(h) / size[1]).round() as u32;
                            let whole =
                                region == (0, 0, size[0].round() as u32, size[1].round() as u32);
                            let (cw, ch, cut) = crop(
                                &rgba,
                                w,
                                h,
                                (
                                    at_x(region.0).min(w.saturating_sub(1)),
                                    at_y(region.1).min(h.saturating_sub(1)),
                                    at_x(region.2).max(1),
                                    at_y(region.3).max(1),
                                ),
                            );
                            let name = if whole {
                                format!("Frame {}.png", clock(time))
                            } else {
                                format!("Frame {} selection.png", clock(time))
                            };
                            Ok((name, cw, ch, cut))
                        })
                    }
                    None => {
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
                            Ok(("Powermove selection.png".into(), cw, ch, cut))
                        })
                    }
                };
                cx.spawn(async move |cx| {
                    let outcome = run_capture(cx.background_executor(), move || {
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
                            let (layers, unavailable) = region_layers(p, region, captured_time);
                            target_context["selection"] = json!(p.selection.layers());
                            target_context["layersUnderBox"] = serde_json::json!(layers);
                            target_context["unavailableLayerBounds"] =
                                serde_json::json!(unavailable);
                        }
                        let record = region_record(job()?, target_context, &caption)?;
                        if !current_capture() {
                            return Err(
                            "The document changed during region capture. Select the region again."
                                .into(),
                        );
                        }
                        runtime.ok_or("The Agent panel closed.")?.agent_attach_for(
                            &agent_project,
                            &agent_thread,
                            vec![record],
                        )
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
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{composition_region, crop, decode_png, encode_png, region_layers};

    use gpui::{AnyElement, App, AppContext, Context, Entity, TestAppContext, Window};
    use pm_shell::{integration::Integration, view::Editor};
    use serde_json::json;
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn viewer_capture_at_289_percent_schedules_work_before_the_watchdog(cx: &mut TestAppContext) {
        use base64::Engine;
        use pm_agents::persistence::Store;
        use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::ShellCall};
        use pm_panel_agentpanel::{model::Panel, store::KernelStore};
        use std::{
            fs,
            time::{Duration, Instant},
        };

        let profile = tempfile::tempdir().unwrap();
        let root = profile.path().canonicalize().unwrap();
        let legacy = root.join("kernel-state.json");
        let payload = "QUJD".repeat(24 * 1024 * 1024 / 4);
        let settings = json!({
            "extensions": [],
            "settings": {
                "theme": "dark",
                "agentThreads.pick":{"version":1,"activeId":"t","threads":[{
                    "id":"t","composerDraft":"Keep both pictures","conversation":[],
                    "attachments":[{"id":"legacy-0","name":"Existing.png","payloadStored":true}]
                }]},
                "agentAttachment.legacy-0":{"dataBase64": &payload},
                "agentAttachment.legacy-1":{"dataBase64": &payload},
                "agentAttachment.legacy-2":{"dataBase64": &payload},
                "agentAttachment.legacy-3":{"dataBase64": &payload}
            }
        });
        fs::write(&legacy, settings.to_string()).unwrap();
        drop((payload, settings));
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        // Kernel replies are blocking and must never be awaited on AppKit.
        let (settings_path, settings, mut store, mut panel) = std::thread::spawn(move || {
            kernel
                .shell(ShellCall::AttachLibrary {
                    profile: root.clone(),
                    format: DiskFormat::V1_2,
                    electron_profile: None,
                })
                .wait()
                .unwrap()
                .unwrap();
            let mut store = KernelStore::bind(kernel.clone()).unwrap();
            let panel = Panel::load(&mut store, "pick", "fallback", 0).unwrap();
            assert!(!root.join("kernel-state.json").exists());
            let settings_path = root.join("store/theme.json");
            let settings = fs::read(&settings_path).unwrap();
            assert_eq!(serde_json::from_slice::<String>(&settings).unwrap(), "dark");
            (settings_path, settings, store, panel)
        })
        .join()
        .unwrap();
        let zoom = 2.89;
        let size = [1080., 3840.];
        let region = composition_region(
            (0., 0., size[0] * zoom, size[1] * zoom),
            (0., 0.),
            zoom,
            size,
        );
        assert_eq!(region, Some((0, 0, 1080, 3840)));
        let width = super::viewer_readback_width(size);
        assert!(
            (f64::from(width) * size[1] / size[0]).ceil() <= f64::from(super::MAX_CAPTURE_EDGE)
        );
        let start = Instant::now();
        let task = super::run_capture(&cx.background_executor, move || {
            // This synthetic stall stands in for a GPU fence that may take
            // longer than a frame. The capture event must already have returned.
            std::thread::sleep(Duration::from_millis(275));
            let record = super::region_record(
                ("Viewer.png".into(), 1080, 3840, vec![127; 1080 * 3840 * 4]),
                json!({"area":region}),
                "Viewer",
            )?;
            let data = record["dataUrl"]
                .as_str()
                .unwrap()
                .split_once(',')
                .unwrap()
                .1;
            let png = base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap();
            let (w, h, _) = decode_png(&png)?;
            assert_eq!((w, h), (576, super::MAX_CAPTURE_EDGE));
            let id = record["id"].as_str().unwrap().to_owned();
            // This harness uses a deterministic GPUI executor. Kernel IO still
            // obeys the thread restriction on a real worker.
            std::thread::spawn(move || {
                panel
                    .attach_records("t", vec![record], &mut store, 1)
                    .unwrap();
                panel.save(&mut store, 1).unwrap();
                assert!(
                    store
                        .get(&format!("agentAttachment.{id}"))
                        .unwrap()
                        .is_some()
                );
                assert_eq!(
                    panel.inspect("t").unwrap()["attachments"]
                        .as_array()
                        .unwrap()
                        .len(),
                    2
                );
                let saved = store.get("agentThreads.pick").unwrap().unwrap();
                let capture = &saved["threads"][0]["attachments"][1];
                assert_eq!(capture["id"], id);
                assert_eq!(capture["payloadStored"], true);
                assert!(capture.get("dataUrl").is_none());
            })
            .join()
            .unwrap();
            Ok(())
        });
        let main = start.elapsed();
        assert!(
            main < Duration::from_millis(250),
            "region capture blocked the event thread for {main:?}"
        );
        println!("viewer selection at 289%: queued in {main:?}");
        cx.foreground_executor().block_test(task).unwrap();
        assert!(!legacy.exists());
        assert_eq!(fs::read(settings_path).unwrap(), settings);
        actor.shutdown();
    }

    // The native Controller requires an AppKit window. Reproduce its lease
    // with a headless entity and the real editor's panel lookup.
    struct PanelBridge(Entity<Vec<(String, String)>>);

    impl Integration for PanelBridge {
        fn observe(&self, _: &mut Context<Editor>) {}

        fn command(&self, _: &pm_shell::Command, _: &mut Window, _: &mut App) -> bool {
            false
        }

        fn panel(
            &self,
            _: &str,
            _: pm_shell::layout::Rect,
            _: &mut Window,
            _: &mut App,
        ) -> Option<AnyElement> {
            None
        }

        fn project_id(&self, _: &App) -> Option<String> {
            None
        }

        fn status(&self, _: &App) -> String {
            String::new()
        }

        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}

        fn contributed_panels(&self, cx: &App) -> Vec<(String, String)> {
            self.0.read(cx).clone()
        }
    }

    fn region_attach_panels(cx: &mut TestAppContext, whole_window: bool) {
        let owner = cx.new(|_| vec![("capture.panel".into(), "Capture Panel".into())]);
        let bridge = Rc::new(PanelBridge(owner.clone()));
        let (shell, cx) = cx.add_window_view(|_, cx| {
            let mut shell = Editor::new(crate::v4_view_tests::tokens(), None, cx);
            shell.integration = Some(bridge);
            shell
                .layout
                .show_panel("capture.panel", &shell.panels(cx))
                .unwrap();
            shell
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let id = if whole_window {
            "capture.panel"
        } else {
            "viewer"
        };
        let (b, viewer) = shell.read_with(cx, |shell, _| {
            (
                shell.panel_rect(id).expect("visible selected panel"),
                shell.panel_rect("viewer").expect("visible viewer"),
            )
        });
        let rect = if whole_window {
            (b.x as f64 + 1., b.y as f64 + 1., 0., 0.)
        } else {
            (b.x as f64 + 1., b.y as f64 + 1., 4., 4.)
        };
        let captured = Rc::new(RefCell::new(None));
        let result = captured.clone();
        cx.update(|window, cx| {
            owner.update(cx, |_, cx| {
                super::with_region_panels(
                    Some(shell.downgrade()),
                    rect,
                    window,
                    cx,
                    whole_window,
                    move |panels, _, _| *result.borrow_mut() = Some(panels),
                );
            });
        });
        let panels = captured.borrow_mut().take().expect("capture completed");
        let title = if whole_window {
            "Capture Panel"
        } else {
            "Viewer"
        };
        assert!(
            panels.contains(&json!({
                "panelId": id,
                "title": title,
                "area": [b.x, b.y, b.w, b.h]
            })),
            "{panels:?}"
        );
        if whole_window {
            assert!(
                panels.contains(&json!({
                    "panelId": "viewer",
                    "title": "Viewer",
                    "area": [viewer.x, viewer.y, viewer.w, viewer.h]
                })),
                "{panels:?}"
            );
        } else {
            assert_eq!(panels.len(), 1, "{panels:?}");
        }
    }

    #[gpui::test]
    fn region_attach_viewer_keeps_panel_metadata(cx: &mut TestAppContext) {
        region_attach_panels(cx, false);
    }

    #[gpui::test]
    fn region_attach_whole_window_keeps_panel_metadata(cx: &mut TestAppContext) {
        region_attach_panels(cx, true);
    }

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
