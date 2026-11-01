//! Run with the engine environment and shared target dir. Optional first arg:
//! another .pmv/JSON project. Left/right step one exact frame, Shift steps ten.
use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, IntoElement, Render, Styled, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use pm_mac_engine_host::{EngineHost, MainThreadFonts, RenderRequest, Size, Ticks};
use pm_mac_panel_viewer::{Viewport, frame_element};
use pm_model::{
    Document, Value, canonical,
    container::{ReadOptions, read_container},
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

struct Project {
    document: Arc<Document>,
    composition: String,
    size: Size,
    rate: pm_time::Rate,
    frames: i64,
    assets: Arc<pm_render::MapResolver>,
    _media: tempfile::TempDir,
}
fn load(path: &Path) -> Result<Project, Box<dyn std::error::Error>> {
    let project_dir = path.parent().unwrap_or(Path::new(".")).to_owned();
    let mut file = File::open(path)?;
    let container = read_container(&file, ReadOptions { verify_media: true })?;
    let project = canonical::canonical_project(canonical::into_project(container.document));
    let composition = canonical::open_comp_id(&project)
        .ok_or("project has no open composition")?
        .to_owned();
    let comp = project
        .get("comps")
        .and_then(|v| v.get(&composition))
        .ok_or("missing composition")?;
    let number = |k: &str| match comp.get(k) {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    };
    let size = Size::new(number("w") as u32, number("h") as u32);
    let rate = pm_model::ingest::IngestRate::of_composition(comp)?.rate;
    let frames = rate
        .frame_of(pm_time::ticks_from_seconds(number("dur"))?)
        .max(1);
    let media = tempfile::tempdir()?;
    let mut resolver = pm_render::MapResolver::default();
    for (i, record) in container.media.iter().enumerate() {
        let extension = if record.media_type.to_string_lossy() == "image/png" {
            "png"
        } else {
            "bin"
        };
        let output = media.path().join(format!("media-{i}.{extension}"));
        file.seek(SeekFrom::Start(record.offset))?;
        let mut writer = File::create(&output)?;
        std::io::copy(
            &mut Read::by_ref(&mut file).take(record.length),
            &mut writer,
        )?;
        writer.flush()?;
        resolver.paths.insert(record.id.to_string_lossy(), output);
    }
    // Linked media remains explicit, relative to the opened project.
    if let Some(assets) = project.get("assets").and_then(Value::as_object) {
        for (id, asset) in assets {
            if let Some(Value::String(source)) = asset.get("sourcePath") {
                let path = PathBuf::from(source.to_string_lossy());
                resolver
                    .paths
                    .entry(id.to_string_lossy())
                    .or_insert_with(|| {
                        if path.is_absolute() {
                            path
                        } else {
                            project_dir.join(path)
                        }
                    });
            }
        }
    }
    Ok(Project {
        document: Arc::new(Document::new(Some(project), 1)),
        composition,
        size,
        rate,
        frames,
        assets: Arc::new(resolver),
        _media: media,
    })
}
struct Demo {
    project: Project,
    host: EngineHost,
    fonts: MainThreadFonts,
    frame: i64,
    focus: FocusHandle,
    last: u64,
    _timer: gpui::Task<()>,
}
impl Demo {
    fn new(project: Project, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fonts = MainThreadFonts::new(Arc::new(|| {})).expect("main-thread font service");
        let host = EngineHost::start(fonts.broker());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let updates = host.updates();
        cx.spawn(async move |this, cx| {
            while updates.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        // Font service runs on main; worker requests never block AppKit. A short
        // timer also refreshes the completion-derived latency in the title.
        let timer =
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(8))
                        .await;
                    if this
                        .update(cx, |this: &mut Self, cx| {
                            this.fonts.pump();
                            if let Some(frame) = this.host.latest().filter(|f| {
                                f.display_latency().is_some() && f.request_id != this.last
                            }) {
                                this.last = frame.request_id;
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        let demo = Self {
            project,
            host,
            fonts,
            frame: 0,
            focus,
            last: 0,
            _timer: timer,
        };
        demo.request();
        demo
    }
    fn request(&self) {
        self.host
            .request(RenderRequest {
                document: self.project.document.clone(),
                composition: self.project.composition.clone(),
                time: Ticks::new(self.project.rate.frame_start(self.frame).unwrap()).unwrap(),
                size: self.project.size,
                assets: self.project.assets.clone(),
            })
            .unwrap();
    }
}
impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.host.latest();
        if let Some(frame) = &frame {
            let latency = frame
                .display_latency()
                .map(|v| format!("{:.1} ms", v.as_secs_f64() * 1000.0))
                .unwrap_or_else(|| "presenting".into());
            window.set_window_title(&format!(
                "Powermove frames · {} / {} · {}",
                self.frame,
                self.project.frames - 1,
                latency
            ));
        }
        let message = self.host.last_error().unwrap_or_else(|| {
            format!(
                "← → Scrub · Shift: 10 frames    Frame {} / {}    {} × {}",
                self.frame,
                self.project.frames - 1,
                self.project.size.width,
                self.project.size.height
            )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(gpui::rgb(0x111114))
            .text_color(gpui::rgb(0xeeeeee))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                let delta = match event.keystroke.key.as_str() {
                    "left" => -1,
                    "right" => 1,
                    _ => return,
                } * if event.keystroke.modifiers.shift {
                    10
                } else {
                    1
                };
                this.frame = (this.frame + delta).clamp(0, this.project.frames - 1);
                this.request();
                cx.notify();
            }))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .when_some(frame, |el, frame| {
                        el.child(
                            frame_element(frame, Viewport::default())
                                .absolute()
                                .size_full(),
                        )
                    })
                    // Ordinary GPUI overlay drawn above the RGB element.
                    .child(
                        div()
                            .absolute()
                            .top_3()
                            .left_3()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(gpui::rgba(0x000000b0))
                            .text_sm()
                            .child("Native engine · BGRA IOSurface"),
                    ),
            )
            .child(div().p_3().text_sm().child(message))
    }
}
fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/frames.pmv"));
    // File work is completed before entering the UI; the integrated app should
    // load on its document worker and hand the immutable snapshot to this view.
    let project = load(&path).expect("open fixture with read_container");
    gpui_platform::application().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1000.0), px(650.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Powermove frames".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Demo::new(project, window, cx)),
        )
        .unwrap();
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}
