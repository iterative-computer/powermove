//! Studio in a GPUI window: the native host for the GPUI rewrite
//! (docs/3d-studio-redesign.md section 4.1a).
//!
//! Drag to orbit, scroll to dolly, pick a scene along the top. While the view
//! moves Studio draws Live (half resolution, one sample, denoised); when it
//! rests it refines toward 256 samples. A worker thread owns the GPU and
//! sends finished frames to the window, so the UI never waits on a trace.
//!
//!   cargo run --release           (from engine/hosts/studio-gpui)

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use gpui::{
    App, AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Render, RenderImage,
    ScrollWheelEvent, SharedString, Styled, StyledImage as _, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, point, px, rgb, size,
};
use pm_studio::gpu::Renderer;
use pm_studio::math::{Vec3, v3};
use pm_studio::scene::{self, Scene, preset};

const SCENES: [&str; 7] = ["hero", "edge", "macro", "back", "chrome", "materials", "caustics"];
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const REFINE_SAMPLES: u32 = 256;

/// The orbit around a scene's aim point.
#[derive(Clone, Copy, PartialEq)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

enum Command {
    Scene(&'static str),
    View(Orbit),
}

struct Frame {
    image: Arc<RenderImage>,
    status: String,
}

fn orbit_of(s: &Scene) -> Orbit {
    let d = s.camera.position - s.camera.target;
    let distance = d.length();
    Orbit { yaw: d.x.atan2(d.z), pitch: (d.y / distance).asin(), distance }
}

fn place(s: &mut Scene, target: Vec3, o: Orbit) {
    let (cp, sp) = (o.pitch.cos(), o.pitch.sin());
    s.camera.position = target + v3(o.yaw.sin() * cp, sp, o.yaw.cos() * cp) * o.distance;
    s.camera.target = target;
    s.camera.focus = None;
}

/// RGBA from Studio → a GPUI image (BGRA).
fn image(mut rgba: Vec<u8>, w: u32, h: u32) -> Arc<RenderImage> {
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(w, h, rgba).expect("frame size");
    Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(buffer)]))
}

/// The worker: owns both renderers, renders Live on every view change and
/// refines between commands.
fn worker(commands: mpsc::Receiver<Command>, frames: async_channel::Sender<Frame>) {
    let mut full = Renderer::new(WIDTH, HEIGHT).expect("a Metal GPU");
    let mut live = Renderer::new(WIDTH / 2, HEIGHT / 2).expect("a Metal GPU");
    live.live = true;
    let (sw, sh) = (1024, 1488);
    for r in [&mut full, &mut live] {
        r.set_screen(&scene::screen_wallpaper(sw, sh, 0.0), sw, sh);
    }
    let adapter = full.adapter.clone();
    let mut current = preset(SCENES[0], "space-gray", 0.0).expect("scene");
    let mut target = current.camera.target;
    let mut name = SCENES[0];
    let load = |s: &Scene, full: &mut Renderer, live: &mut Renderer| {
        let p = s.prepare();
        full.set_scene(s, &p);
        live.set_scene(s, &p);
    };
    load(&current, &mut full, &mut live);
    let mut refining = true;
    let mut first = true;
    loop {
        // The newest command wins; while refining, do not block.
        let mut moved = false;
        let next = if refining { commands.try_recv().ok() } else { commands.recv().ok() };
        let mut pending: Vec<Command> = next.into_iter().collect();
        pending.extend(commands.try_iter());
        if !refining && pending.is_empty() {
            return; // the window closed
        }
        for c in pending {
            match c {
                Command::Scene(n) => {
                    current = preset(n, "space-gray", 0.0).expect("scene");
                    target = current.camera.target;
                    name = n;
                    load(&current, &mut full, &mut live);
                    moved = true;
                }
                Command::View(o) => {
                    place(&mut current, target, o);
                    moved = true;
                }
            }
        }
        if moved {
            full.set_camera(&current);
            live.set_camera(&current);
            let t = Instant::now();
            live.trace(1, 2, true, 6);
            let (px, _) = live.finish(4);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let status = format!("{name} · Live · {ms:.1} ms · {adapter}");
            if frames.send_blocking(Frame { image: image(px, WIDTH / 2, HEIGHT / 2), status }).is_err() {
                return;
            }
            refining = true;
            first = true;
            continue;
        }
        if refining {
            let t = Instant::now();
            full.trace(4, 6, first, 64);
            first = false;
            let (px, _) = full.finish(0);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let done = full.samples >= REFINE_SAMPLES;
            let label = if done { "Clean".to_string() } else { format!("Refining · {}%", 100 * full.samples / REFINE_SAMPLES) };
            let status = format!("{name} · {label} · {} spp · {ms:.0} ms per pass · {adapter}", full.samples);
            if frames.send_blocking(Frame { image: image(px, WIDTH, HEIGHT), status }).is_err() {
                return;
            }
            refining = !done;
        }
    }
}

struct StudioView {
    frame: Option<Arc<RenderImage>>,
    status: SharedString,
    scene: &'static str,
    orbit: Orbit,
    drag: Option<(f32, f32, Orbit)>,
    commands: mpsc::Sender<Command>,
}

impl StudioView {
    fn new(cx: &mut Context<Self>) -> Self {
        let (tx, rx) = mpsc::channel();
        let (ftx, frx) = async_channel::unbounded::<Frame>();
        std::thread::spawn(move || worker(rx, ftx));
        cx.spawn(async move |this, cx| {
            while let Ok(mut frame) = frx.recv().await {
                // Only the newest frame matters.
                while let Ok(newer) = frx.try_recv() {
                    frame = newer;
                }
                if this.update(cx, |v, cx| {
                    v.frame = Some(frame.image);
                    v.status = frame.status.into();
                    cx.notify();
                }).is_err() {
                    break;
                }
            }
        })
        .detach();
        let orbit = orbit_of(&preset(SCENES[0], "space-gray", 0.0).expect("scene"));
        StudioView { frame: None, status: "Starting Studio…".into(), scene: SCENES[0], orbit, drag: None, commands: tx }
    }

    fn pick(&mut self, name: &'static str, cx: &mut Context<Self>) {
        self.scene = name;
        self.orbit = orbit_of(&preset(name, "space-gray", 0.0).expect("scene"));
        let _ = self.commands.send(Command::Scene(name));
        cx.notify();
    }
}

impl Render for StudioView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pills = SCENES.iter().map(|&name| {
            let active = name == self.scene;
            div()
                .px_3()
                .py_1()
                .rounded_full()
                .text_sm()
                .bg(if active { rgb(0x1d1d1f) } else { rgb(0xe8e8ed) })
                .text_color(if active { rgb(0xffffff) } else { rgb(0x1d1d1f) })
                .child(name)
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| this.pick(name, cx)))
        });
        let viewport = div()
            .flex_1()
            .size_full()
            .bg(rgb(0xf6f5f8))
            .children(self.frame.clone().map(|f| img(f).size_full().object_fit(ObjectFit::Contain)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, _| {
                this.drag = Some((e.position.x.as_f32(), e.position.y.as_f32(), this.orbit));
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| {
                let Some((x0, y0, start)) = this.drag else { return };
                if e.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    return;
                }
                let (dx, dy) = (e.position.x.as_f32() - x0, e.position.y.as_f32() - y0);
                let o = Orbit { yaw: start.yaw - dx * 0.008, pitch: (start.pitch + dy * 0.006).clamp(-1.4, 1.45), distance: start.distance };
                if o != this.orbit {
                    this.orbit = o;
                    let _ = this.commands.send(Command::View(o));
                }
            }))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, _| {
                let dy = e.delta.pixel_delta(px(16.0)).y.as_f32();
                this.orbit.distance = (this.orbit.distance * (1.0 + dy * 0.002)).max(1.0);
                let _ = this.commands.send(Command::View(this.orbit));
            }));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .child(div().flex().gap_2().pt(px(38.0)).px_4().pb_2().children(pills))
            .child(viewport)
            .child(div().px_4().py_2().text_xs().text_color(rgb(0x6e6e73)).child(self.status.clone()))
    }
}

/// `--headless DIR`: the window's worker driven by a scripted drag, writing
/// the frames the window would show (for checks without a display).
fn headless(dir: &str) {
    std::fs::create_dir_all(dir).expect("output directory");
    let (tx, rx) = mpsc::channel();
    let (ftx, frx) = async_channel::unbounded::<Frame>();
    let worker = std::thread::spawn(move || worker(rx, ftx));
    let start = orbit_of(&preset(SCENES[0], "space-gray", 0.0).expect("scene"));
    let save = |f: &Frame, name: &str| {
        let bytes = f.image.as_bytes(0).expect("frame");
        let mut rgba = bytes.to_vec();
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        let (w, h) = (f.image.size(0).width.0 as u32, f.image.size(0).height.0 as u32);
        pm_studio::gpu::write_png(&std::path::Path::new(dir).join(name), w, h, &rgba).expect("png");
        println!("{name}: {}", f.status);
    };
    // A drag: twelve small orbit steps, each answered by a Live frame.
    for i in 1..=12 {
        let o = Orbit { yaw: start.yaw - 0.05 * i as f32, pitch: start.pitch + 0.01 * i as f32, distance: start.distance };
        tx.send(Command::View(o)).expect("worker");
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let mut last_live = None;
    loop {
        let f = frx.recv_blocking().expect("frame");
        if f.status.contains("Live") {
            last_live = Some(f);
            continue;
        }
        if let Some(l) = last_live.take() {
            save(&l, "live.png");
        }
        if f.status.contains("Clean") {
            save(&f, "clean.png");
            break;
        }
    }
    drop(tx);
    let _ = worker.join();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--headless") {
        return headless(args.get(i + 1).map_or("studio-gpui-out", |s| s.as_str()));
    }
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Studio".into()), appears_transparent: true, traffic_light_position: Some(point(px(14.0), px(14.0))) }),
            ..Default::default()
        };
        cx.open_window(options, |_, cx| cx.new(StudioView::new)).expect("a window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}
