//! The engine's render fonts (Phase 3c; `pm_node::fonts::render`), without
//! Electron. This process plays main (the bridge's rendezvous) and re-runs
//! itself as a child that plays the interactive engine with the document
//! frame source, its host on the main thread as the utility process's
//! JavaScript thread is (AppKit's font manager needs it, hence no harness).
//! The child checks:
//!
//! * a snapshot of a composition with a text layer is drawn (never refused
//!   for want of fonts), its fonts asked of the font service on the main
//!   thread while the render thread waits;
//! * the pixels equal a renderer laying the same text out over the CoreText
//!   registry directly (the scoped hub's replay over full fonts changes
//!   nothing);
//! * a text layer's raster (`render.raster`'s base) is the same raster
//!   `Renderer::text_raster` paints over the registry;
//! * a project font changes the generation and later frames use it;
//! * an engine whose font service never answers fails the snapshot in
//!   bounded time instead of drawing text with missing fonts.

#[cfg(target_os = "macos")]
fn main() {
    if std::env::var(child::CHILD_ENV).is_ok() {
        child::run();
        println!("CHILD-OK");
        return;
    }
    parent::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod parent {
    use std::process::{Command, Stdio};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use pm_bridge::audit::{ExpectedPeer, SignaturePolicy};
    use pm_bridge::host::{self, SurfaceTable};
    use pm_bridge::rendezvous::{Rendezvous, SERVICE_PREFIX, random_service_name};
    use pm_node::present::{BRIDGE_ENV, HOST_CLOCK_ENV, SOURCE_ENV};

    pub fn run() {
        let name = random_service_name(SERVICE_PREFIX).unwrap();
        let server = Rendezvous::register(&name).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .env(super::child::CHILD_ENV, "1")
            .env(BRIDGE_ENV, &name)
            .env(SOURCE_ENV, "document")
            .env(HOST_CLOCK_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let expected =
            ExpectedPeer::for_child(child.id() as i32, SignaturePolicy::UnsignedDevBypass).unwrap();
        let table = Arc::new(Mutex::new(SurfaceTable::default()));
        let main = std::thread::spawn(move || {
            let stop = AtomicBool::new(false);
            host::run(
                server,
                &expected,
                Duration::from_secs(90),
                &table,
                &stop,
                &mut |_| {},
            );
        });
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
        assert!(
            stdout.contains("CHILD-OK"),
            "child did not finish:\n{stdout}\n{stderr}"
        );
        main.join().unwrap();
        print!("{stdout}");
        println!("render_fonts: ok");
    }
}

#[cfg(target_os = "macos")]
mod child {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use pm_engine::Action;
    use pm_engine::engine::testing::hello;
    use pm_node::EngineHost;
    use pm_node::fonts::wire::{self, Op};
    use pm_protocol::codec;
    use pm_protocol::color::{Precision, ProjectColor};
    use pm_protocol::common::{Bytes, ContentHash, RequestId, Revision, SessionId, Size, Ticks};
    use pm_protocol::envelope::{Envelope, Message, OpenSession, Request, Response, SessionRole};
    use pm_protocol::error::{EngineProcess, ErrorCode};
    use pm_protocol::host::PortId;
    use pm_protocol::patch::{DocumentEncoding, Resync, ResyncSource};
    use pm_protocol::snapshot::{
        HostSnapshot, PixelPayload, SnapshotClass, SnapshotOutcome, SnapshotPixels,
        SnapshotRequest, SnapshotResult, SnapshotTarget,
    };
    use pm_protocol::version::PeerRole;
    use serde_json::{Value, json};

    pub const CHILD_ENV: &str = "PM_RENDER_FONTS_TEST_CHILD";
    const HOST: PortId = PortId(1);
    const WINDOW: PortId = PortId(7);
    const FONTS: u32 = 9;

    #[derive(Default)]
    struct Inbox {
        messages: Mutex<Vec<(PortId, Envelope)>>,
        wake: AtomicBool,
        arrived: Condvar,
    }

    impl Inbox {
        fn push(&self, actions: Vec<Action>) {
            let mut messages = self.messages.lock().unwrap();
            if actions.is_empty() {
                // The engine wants `pump` (a render thread's font ask).
                self.wake.store(true, Ordering::Release);
            }
            for action in actions {
                match action {
                    Action::Send { port, bytes } if port != PortId(FONTS) => {
                        messages
                            .push((port, codec::decode(&bytes).expect("engine output decodes")));
                    }
                    Action::Log { level, message } => eprintln!("engine {level:?}: {message}"),
                    _ => {}
                }
            }
            self.arrived.notify_all();
        }
    }

    struct Child {
        host: EngineHost,
        inbox: Arc<Inbox>,
        session: SessionId,
        /// Whether this thread answers the engine's font asks (`pump`).
        serve: bool,
    }

    fn ch(v: Value) -> Value {
        json!({ "v": v, "kf": [], "expr": null })
    }

    fn text_layer(text: &str, font: &str) -> Value {
        json!({
            "id": "title", "type": "text", "name": "title", "from": 0, "dur": 2, "on": true,
            "lock": false, "shy": false, "collapsed": true, "threeD": false, "solo": false,
            "color": "#4C8DFF", "blend": "normal", "mblur": false, "parent": null, "group": null,
            "matteSource": null,
            "p": {
                "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(160)),
                "position.y": ch(json!(90)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
                "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
            },
            "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
            "d": {
                "text": text, "boxWidth": ch(json!(0)), "boxHeight": ch(json!(0)), "font": font,
                "weight": 700, "size": 36, "tracking": 0, "leading": 1.2, "color": "#F2E8D0",
                "align": "center", "italic": false
            },
            "locked_intent": {}
        })
    }

    fn project(layers: Vec<Value>) -> Value {
        json!({
            "id": "Pfonts", "name": "fonts", "w": 320, "h": 180, "fps": 30, "dur": 2, "bg": "#000000",
            "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
            "compId": "Pfonts-root", "compName": "fonts", "markers": [], "work": [0, 2], "params": {},
            "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
        })
    }

    impl Child {
        fn start(serve: bool) -> Child {
            let inbox = Arc::new(Inbox::default());
            let sink_inbox = inbox.clone();
            let mut host = EngineHost::with_sink(
                EngineProcess::Interactive,
                Some(Arc::new(move |actions| sink_inbox.push(actions))),
                |key: &str| std::env::var(key).ok(),
            )
            .unwrap();
            host.attach(HOST.0, "host", None).unwrap();
            inbox.push(host.receive(HOST.0, &hello(PeerRole::Host)));
            host.attach(WINDOW.0, "renderer", Some(WINDOW.0)).unwrap();
            inbox.push(host.receive(WINDOW.0, &hello(PeerRole::Renderer)));
            let mut child = Child {
                host,
                inbox,
                session: SessionId::new(1).unwrap(),
                serve,
            };
            child.window(Envelope::new(Message::Request {
                id: RequestId::new(1).unwrap(),
                body: Request::OpenSession(OpenSession {
                    document: "Pfonts".into(),
                    role: SessionRole::Interactive,
                    process: EngineProcess::Interactive,
                    color: ProjectColor::DEFAULT,
                }),
            }));
            child.session = child.wait(Duration::from_secs(5), |port, m| match &m.message {
                Message::Response {
                    body: Response::SessionOpened { session },
                    ..
                } if port == WINDOW => Some(*session),
                _ => None,
            });
            child
        }

        /// Waits for a message, answering the engine's font asks meanwhile
        /// (as the utility process calls `pump` when the sink fires empty).
        fn wait<T>(
            &mut self,
            timeout: Duration,
            mut pick: impl FnMut(PortId, &Envelope) -> Option<T>,
        ) -> T {
            let deadline = Instant::now() + timeout;
            loop {
                {
                    let mut messages = self.inbox.messages.lock().unwrap();
                    if let Some(i) = messages.iter().position(|(p, m)| pick(*p, m).is_some()) {
                        let found = pick(messages[i].0, &messages[i].1).unwrap();
                        messages.remove(i);
                        return found;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "timed out; inbox: {:?}",
                        messages
                            .iter()
                            .map(|(p, m)| (p.0, &m.message))
                            .collect::<Vec<_>>()
                    );
                    let _ = self
                        .inbox
                        .arrived
                        .wait_timeout(messages, Duration::from_millis(5))
                        .unwrap();
                }
                if self.serve && self.inbox.wake.swap(false, Ordering::AcqRel) {
                    let actions = self.host.pump();
                    self.inbox.push(actions);
                }
            }
        }

        fn window(&mut self, envelope: Envelope) {
            let actions = self
                .host
                .receive(WINDOW.0, &codec::encode(&envelope).unwrap());
            self.inbox.push(actions);
        }

        fn resync(&mut self, id: u64, revision: u64, doc: &Value) {
            let parsed = pm_model::json::parse(&doc.to_string()).unwrap();
            let session = self.session;
            self.window(
                Envelope::new(Message::Request {
                    id: RequestId::new(id).unwrap(),
                    body: Request::Resync(Resync {
                        document: "Pfonts".into(),
                        revision: Revision::new(revision).unwrap(),
                        encoding: DocumentEncoding::CanonicalJsonV1,
                        source: ResyncSource::Inline {
                            body: Bytes(doc.to_string().into_bytes()),
                        },
                        hash: ContentHash(pm_model::hash::canonical_digest(&parsed)),
                    }),
                })
                .with_session(session),
            );
            self.wait(Duration::from_secs(5), |port, m| match &m.message {
                Message::Response {
                    id: got,
                    body: Response::PatchesApplied(_),
                } if port == WINDOW && got.get() == id => Some(()),
                _ => None,
            });
        }

        fn snapshot(
            &mut self,
            id: u64,
            target: SnapshotTarget,
            size: Size,
            pixels: SnapshotPixels,
        ) -> Result<SnapshotOutcome, ErrorCode> {
            let envelope = Envelope::new(Message::Request {
                id: RequestId::new(id).unwrap(),
                body: Request::Snapshot(HostSnapshot {
                    renderer: WINDOW,
                    session: self.session,
                    snapshot: SnapshotRequest {
                        composition: "Pfonts-root".into(),
                        time: Ticks::ZERO,
                        output_size: size,
                        target,
                        pixels,
                        class: SnapshotClass::Sync,
                        view_seq: None,
                        viewer: None,
                        wait_ms: 20_000,
                        blender_wait_ms: 0,
                        captions_hidden: false,
                        uploaded: Vec::new(),
                        compat_uploaded: Vec::new(),
                    },
                }),
            });
            let actions = self
                .host
                .receive(HOST.0, &codec::encode(&envelope).unwrap());
            self.inbox.push(actions);
            self.wait(Duration::from_secs(40), move |port, m| {
                if port != HOST {
                    return None;
                }
                match &m.message {
                    Message::Response {
                        id: got,
                        body: Response::Snapshot(outcome),
                    } if got.get() == id => Some(Ok((**outcome).clone())),
                    Message::Error { id: got, error } if got.get() == id => {
                        eprintln!("snapshot {id} failed: {}", error.message);
                        Some(Err(error.code))
                    }
                    _ => None,
                }
            })
        }

        /// Main's font port, opened as main opens it.
        fn font_port(&mut self) {
            self.host.attach(FONTS, pm_node::FONTS_PEER, None).unwrap();
            let actions = self.host.receive(
                FONTS,
                &wire::encode_request(1, Op::Open, "{\"generationBase\":100}", &[]),
            );
            assert_eq!(actions.len(), 1, "the open is answered");
            let actions = self.host.receive(
                FONTS,
                &wire::encode_request(2, Op::Seeds, "{\"seeds\":[]}", &[]),
            );
            assert_eq!(actions.len(), 1, "the final seed replay opens rendering");
        }
    }

    fn frame() -> SnapshotTarget {
        SnapshotTarget::Frame {
            motion_blur_samples: 0,
            background: true,
            draft_3d: false,
        }
    }

    fn pixels(result: &SnapshotResult) -> Vec<u8> {
        match &result.payload {
            PixelPayload::Inline { bytes } => bytes.0.clone(),
            PixelPayload::SharedMemory { name, length } => {
                pm_bridge::shm::take(name, length.get() as usize).expect("main takes the segment")
            }
        }
    }

    fn rendered(outcome: Result<SnapshotOutcome, ErrorCode>) -> SnapshotResult {
        match outcome {
            Ok(SnapshotOutcome::Rendered(result)) => result,
            other => panic!("expected pixels, got {other:?}"),
        }
    }

    /// The fonts a test renderer uses: the CoreText registry, directly, on
    /// this (the main) thread.
    struct Direct(pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>);
    // SAFETY: used on the main thread only, never moved to another.
    unsafe impl Send for Direct {}
    impl pm_render::text::TextFonts for Direct {
        fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
            &mut self.0
        }
        fn line_runs(
            &mut self,
            d: &pm_text::compat::TextContent,
        ) -> Vec<pm_text::compat::LineRuns> {
            self.0.compat().line_runs(d)
        }
        fn text_runs(
            &mut self,
            d: &pm_text::compat::TextContent,
            text: &str,
            tracking: f64,
        ) -> Vec<pm_text::compat::GlyphRun> {
            self.0.compat().text_runs(d, text, tracking).0
        }
        fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
            use pm_text::font::FontResolver;
            self.0.compat().resolver().face(id)
        }
        fn em_advance(&mut self, face: pm_text::font::FaceId, size: f32) -> Option<f32> {
            use pm_text::font::FontResolver;
            self.0.compat().resolver_mut().em_advance(face, size)
        }
    }

    fn direct_renderer() -> pm_render::Renderer {
        let mut r = pm_render::Renderer::headless().expect("a Metal device");
        r.set_text_fonts(Some(Box::new(Direct(
            pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
        ))));
        r
    }

    fn ink(px: &[u8], background: [u8; 3]) -> usize {
        px.chunks_exact(4)
            .filter(|p| {
                p[..3]
                    .iter()
                    .zip(background)
                    .any(|(a, b)| a.abs_diff(b) > 24)
            })
            .count()
    }

    pub fn run() {
        // 1. A text layer is drawn; the render thread's asks are answered here.
        let mut c = Child::start(true);
        c.font_port();
        let doc = project(vec![text_layer("Engine fonts", "Helvetica")]);
        c.resync(2, 1, &doc);
        let size = Size::new(320, 180);
        let result = rendered(c.snapshot(10, frame(), size, SnapshotPixels::Opaque));
        let engine_px = pixels(&result);
        let inked = ink(&engine_px, [0x20, 0x20, 0x20]);
        assert!(inked > 400, "the text draws ({inked} ink pixels)");
        let status: Value = serde_json::from_str(&c.host.status_json()).unwrap();
        let asked = status["fonts"]["roundTrips"].as_u64().unwrap();
        assert!(
            asked > 0,
            "the render thread asked the font service: {status}"
        );
        assert_eq!(status["renderFonts"]["failed"], 0, "{status}");
        println!("render fonts: {} ink pixels, {asked} asks", inked);

        // 2. The same pixels as a renderer over the registry directly.
        let document =
            pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1);
        let mut direct = direct_renderer();
        let request = pm_render::RenderRequest {
            output_size: size,
            scale: None,
            region: None,
            preview_canvas: false,
            precision: Precision::Float16,
            depth_id: false,
            background: true,
            export: true,
            view: None,
            blender: pm_render::blender::BlenderRender::Native,
            motion_blur: pm_render::motion::MotionBlur::Off,
            sampling: pm_render::sampling::Sampling::Final,
            video_end: pm_render::VideoEnd::of(pm_protocol::frame::FramePurpose::Export),
            origin: None,
        };
        let frame_out = direct
            .render(
                &document,
                "Pfonts-root",
                Ticks::ZERO,
                &request,
                &pm_node::present::document::MediaMap::default(),
            )
            .expect("the direct renderer draws the frame");
        let raw =
            pm_gpu::readback::read_raw(direct.device(), direct.queue(), &frame_out.color.texture)
                .unwrap();
        let direct_px = pm_node::present::snapshot::encode_rgba8(&raw, SnapshotPixels::Opaque);
        assert_eq!(direct_px.len(), engine_px.len());
        let differing = direct_px
            .iter()
            .zip(&engine_px)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            differing, 0,
            "the scoped hub's replay draws what the registry draws"
        );

        // 3. The layer raster equals `text_raster` over the registry.
        let raster = rendered(c.snapshot(
            11,
            SnapshotTarget::Layer {
                layer: "title".into(),
                scale: 1.0,
                crop: None,
                content: None,
            },
            Size::new(6000, 6000),
            SnapshotPixels::Premultiplied,
        ));
        let painted = direct
            .text_raster(
                &document,
                "Pfonts-root",
                "title",
                0.0,
                1.0,
                Precision::Float16,
            )
            .unwrap()
            .expect("the layer has ink");
        assert_eq!((raster.width, raster.height), painted.canvas);
        // The origin is today's `RasterSurface` `anchorX`, `anchorY`: the
        // anchor in the whole canvas the snapshot carries (legacy
        // gl/raster.ts `textGeometry`), not in its inked crop. It is the
        // layer raster's, and the crop's anchor moved by the crop's corner
        // in layer pixels.
        let whole = direct
            .layer_raster(
                &document,
                "Pfonts-root",
                "title",
                None,
                0.0,
                1.0,
                Precision::Float16,
            )
            .unwrap()
            .expect("the layer has ink");
        assert_eq!(whole.canvas, painted.canvas);
        assert_eq!(raster.origin, [whole.anchor.0, whole.anchor.1]);
        let [x0, y0, x1, y1] = painted.plan.crop.map(f64::from);
        let (sx, sy) = (
            painted.box_size.0 / (x1 - x0),
            painted.box_size.1 / (y1 - y0),
        );
        assert!(
            (whole.anchor.0 - (painted.anchor.0 + x0 * sx)).abs() < 1e-9
                && (whole.anchor.1 - (painted.anchor.1 + y0 * sy)).abs() < 1e-9,
            "whole-canvas anchor {:?}, crop anchor {:?}, crop {:?}",
            whole.anchor,
            painted.anchor,
            painted.plan.crop
        );
        assert!(
            (whole.box_size.0 - f64::from(painted.canvas.0) * sx).abs() < 1e-9
                && (whole.box_size.1 - f64::from(painted.canvas.1) * sy).abs() < 1e-9,
            "the box is the whole canvas's: {:?}",
            whole.box_size
        );
        let rgba = pm_node::present::snapshot::encode_rgba8_f32(
            &painted.pixels,
            SnapshotPixels::Premultiplied,
        );
        // Level 1 keeps the full Canvas2D quad: its transparent border is
        // part of the source geometry even when the ink has a smaller crop.
        assert_eq!(painted.plan.crop, [0, 0, raster.width, raster.height]);
        let [ink_left, ink_top, _, _] = painted.plan.scene.ink().expect("the layer has ink");
        assert!(
            ink_left > 0 && ink_top > 0,
            "exercise transparent canvas padding"
        );
        // `place_raster` takes `[x, y, w, h]` from the plan's `[x0, y0, x1, y1]`
        // crop (here the whole quad): the placed pixels are the raster's.
        let [px0, py0, px1, py1] = painted.plan.crop;
        let placed = pm_node::present::snapshot::place_raster(
            &rgba,
            [px0, py0, px1 - px0, py1 - py0],
            painted.canvas,
            None,
        )
        .expect("placed");
        assert_eq!(
            pixels(&raster),
            placed.0,
            "the placed raster is the registry's raster"
        );
        assert_eq!(
            rgba.len(),
            raster.width as usize * raster.height as usize * 4
        );
        assert_eq!(pixels(&raster), rgba, "the raster is the registry's raster");
        println!("layer raster: {}x{}", raster.width, raster.height);
        // Any scale, as today's `PM.raster` (`textRasterGeometry`): the
        // density is `min(scale, 8192 / w, 8192 / h)` of the same box, so
        // past the cap the canvas stops at the 8192-pixel edge, and a
        // vanishing scale is one pixel with no ink. Geometry is unchanged.
        let [bw, bh] = raster.layer_box.expect("a raster box");
        for (id, scale) in [(30u64, 40.0), (31, 1_000.0), (32, f64::MAX), (33, 1e-9)] {
            let at_scale = rendered(c.snapshot(
                id,
                SnapshotTarget::Layer {
                    layer: "title".into(),
                    scale,
                    crop: None,
                    content: None,
                },
                Size::new(8192, 8192),
                SnapshotPixels::Premultiplied,
            ));
            let density = scale.min(8192.0 / bw).min(8192.0 / bh);
            let canvas = (
                (bw * density).ceil().max(1.0) as u32,
                (bh * density).ceil().max(1.0) as u32,
            );
            assert_eq!(
                (at_scale.width, at_scale.height),
                canvas,
                "scale {scale}: today's canvas"
            );
            assert_eq!(at_scale.layer_box, raster.layer_box, "scale {scale}");
            assert_eq!(at_scale.origin, raster.origin, "scale {scale}");
            // At a vanishing density the text rasterizer can leave a faint
            // pixel (alpha 82 measured): the renderer's raster surface clears
            // it for today's non-positive scales (`withoutInk`).
            if scale >= 1.0 {
                let inked = pixels(&at_scale).chunks_exact(4).any(|p| p[3] > 0);
                assert!(inked, "scale {scale}: ink");
            }
            println!(
                "layer raster at scale {scale}: {}x{}",
                at_scale.width, at_scale.height
            );
        }
        // Today's placement: the selection of the text geometry; with
        // typography anchoring, the shift as `fontOffset` and in the anchor.
        let geometry = raster.raster.clone().expect("raster geometry");
        assert!(geometry.selection.is_some());
        assert_eq!(geometry.font_offset, None);
        let mut anchored = text_layer("Engine fonts", "Helvetica");
        anchored["d"]["fontAnchorBounds"] = json!({ "x0": -100, "y0": -30, "x1": 100, "y1": 10 });
        let content = Bytes(anchored.to_string().into_bytes());
        let shifted = rendered(c.snapshot(
            13,
            SnapshotTarget::Layer {
                layer: "title".into(),
                scale: 1.0,
                crop: None,
                content: Some(content),
            },
            Size::new(6000, 6000),
            SnapshotPixels::Premultiplied,
        ));
        let offset = shifted
            .raster
            .clone()
            .unwrap()
            .font_offset
            .expect("a font offset");
        assert!(offset[0] != 0.0 || offset[1] != 0.0, "{offset:?}");
        assert!((raster.origin[0] - offset[0] - shifted.origin[0]).abs() < 1e-9);
        assert!((raster.origin[1] - offset[1] - shifted.origin[1]).abs() < 1e-9);
        println!("font anchor: offset {offset:?}");

        // 4. A project font: a new generation, and the next frame draws with it.
        let before = c.host.font_broker().generation();
        let font = std::fs::read("/System/Library/Fonts/Supplemental/Courier New.ttf")
            .expect("a system font file");
        let actions = c.host.receive(
            FONTS,
            &wire::encode_request(
                2,
                Op::AddProjectFont,
                "{\"family\":\"Engine Test Face\",\"weight\":\"400\",\"style\":\"normal\",\"stretch\":\"normal\"}",
                &font,
            ),
        );
        assert_eq!(actions.len(), 1);
        let after = c.host.font_broker().generation();
        assert!(
            after > before,
            "a project font moves the generation ({before} → {after})"
        );
        let doc2 = project(vec![text_layer("Engine fonts", "Engine Test Face")]);
        c.resync(3, 2, &doc2);
        let projected = pixels(&rendered(c.snapshot(
            12,
            frame(),
            size,
            SnapshotPixels::Opaque,
        )));
        assert_ne!(
            projected, engine_px,
            "the project font draws (not Helvetica)"
        );
        assert!(ink(&projected, [0x20, 0x20, 0x20]) > 200);
        println!("project font: generation {before} → {after}");
        drop(c);

        // 5. A font service that never answers fails the snapshot in bounded
        //    time; it never draws text with missing fonts.
        let mut silent = Child::start(false);
        silent.resync(2, 1, &doc);
        let started = Instant::now();
        let outcome = silent.snapshot(20, frame(), size, SnapshotPixels::Opaque);
        let took = started.elapsed();
        assert_eq!(outcome.err(), Some(ErrorCode::Timeout), "a named failure");
        assert!(
            took < pm_node::fonts::hub::OPEN_TIMEOUT + Duration::from_secs(5),
            "bounded ({took:?})"
        );
        println!("silent font service: failed with timeout after {took:?}");
    }
}
