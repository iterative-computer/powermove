//! Headless pictures of the Project panel on a synthetic project with real
//! media (review captures; never committed). The media folder is made by
//! the reviewer (generated test patterns), named by `PM_PROJECT_MEDIA`:
//! `PM_PROJECT_MEDIA=… PM_PANEL_CAPTURE=… cargo test -p pm-native-app capture_project_panel -- --ignored`
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{publication, rig_project};
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_panel_media::{Availability, RuntimeState};
use std::{collections::BTreeMap, path::Path, sync::Arc};

/// (id, name, kind, relative file, w, h, duration, bytes, folder)
type Fixture = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u32,
    u32,
    f64,
    u64,
    Option<&'static str>,
);
const ASSETS: [Fixture; 9] = [
    (
        "a1",
        "street-wide.mp4",
        "video",
        "Footage/street-wide.mp4",
        1920,
        1080,
        20.,
        2_400_000,
        Some("fFootage"),
    ),
    (
        "a2",
        "moving-color.mp4",
        "video",
        "moving-color.mp4",
        1920,
        1080,
        6.,
        1_200_000,
        None,
    ),
    (
        "a3",
        "orbit-close.mp4",
        "video",
        "Footage/B-roll/orbit-close.mp4",
        1280,
        720,
        10.,
        3_100_000,
        Some("fBroll"),
    ),
    (
        "a4",
        "sfx-typing.wav",
        "audio",
        "SFX/sfx-typing.wav",
        0,
        0,
        32.,
        2_800_000,
        None,
    ),
    (
        "a5",
        "paper-cursor.png",
        "image",
        "paper-cursor.png",
        1200,
        800,
        0.,
        310_000,
        None,
    ),
    (
        "a6",
        "LastClip-1E9AE2ED-EDDD-4115-8C8E-3A1.mov",
        "video",
        "gone/LastClip.mov",
        1920,
        1080,
        5.,
        900_000,
        None,
    ),
    (
        "a7",
        "homepage-dawn-smooth.svg",
        "image",
        "gone/homepage.svg",
        1440,
        900,
        0.,
        12_000,
        None,
    ),
    (
        "a8",
        "homepage.png",
        "image",
        "homepage.png",
        1024,
        1024,
        0.,
        640_000,
        None,
    ),
    (
        "a9",
        "sfx-air-swells.wav",
        "audio",
        "SFX/sfx-air-swells.wav",
        0,
        0,
        31.,
        2_700_000,
        None,
    ),
];

fn project(media: &Path) -> String {
    let assets = ASSETS
        .iter()
        .map(|(id, name, kind, file, w, h, dur, size, folder)| {
            let format = Path::new(name)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            let folder = folder.map_or(String::new(), |f| format!(r#","folder":"{f}""#));
            format!(
                r#""{id}":{{"id":"{id}","name":"{name}","kind":"{kind}","w":{w},"h":{h},"dur":{dur},"size":{size},"format":"{format}","sourcePath":{path:?}{folder}}}"#,
                path = media.join(file).to_string_lossy()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let footage = media.join("Footage");
    let broll = footage.join("B-roll");
    let link = |p: &Path| format!(r#","link":{{"path":{:?},"ino":0}}"#, p.to_string_lossy());
    let linked = std::env::var("PM_PROJECT_LINKED").is_ok();
    let folders = format!(
        r#""fFootage":{{"id":"fFootage","name":"Footage","parent":null{}}},"fBroll":{{"id":"fBroll","name":"B-roll","parent":"fFootage"{}}},"fRefs":{{"id":"fRefs","name":"References","parent":null}}"#,
        if linked {
            link(&footage)
        } else {
            String::new()
        },
        if linked { link(&broll) } else { String::new() },
    );
    format!(
        r##"{{"id":"capture","name":"Untitled","compId":"c","w":1920,"h":1080,"fps":24,"dur":74,"bg":"#232323","assets":{{{assets}}},"assetFolders":{{{folders}}},"comps":{{}},"layers":[]}}"##
    )
}

#[test]
#[ignore = "writes PNGs; set PM_PROJECT_MEDIA and PM_PANEL_CAPTURE"]
fn capture_project_panel() {
    let media =
        std::path::PathBuf::from(std::env::var("PM_PROJECT_MEDIA").expect("PM_PROJECT_MEDIA"));
    let dir = std::env::var("PM_PANEL_CAPTURE").expect("PM_PANEL_CAPTURE");
    let tag = std::env::var("PM_CAPTURE_TAG").unwrap_or_else(|_| "capture".into());
    std::fs::create_dir_all(&dir).unwrap();
    // Thumbnails as the app makes them, registered for the view to draw.
    let mut thumbs = crate::media_thumbs::Thumbs::default();
    let cache = Path::new(&dir).join(".thumbs");
    let ffmpeg = std::env::var_os("PM_FFMPEG").map(std::path::PathBuf::from);
    let mut posters = BTreeMap::new();
    for (id, _, kind, file, _, _, dur, _, _) in ASSETS {
        let path = media.join(file);
        if !path.is_file() {
            continue;
        }
        let item = crate::media_thumbs::Item {
            asset: id.into(),
            kind: kind.into(),
            path,
            at: (kind == "video").then(|| (dur * 0.1).min(0.5)),
        };
        let made = crate::media_thumbs::make(&cache, &item, ffmpeg.as_deref());
        if let Some((key, _)) = &made {
            posters.insert(id.to_owned(), key.clone());
        }
        thumbs.done(thumbs.generation(), id.into(), made);
    }
    for (view, w, h) in [("grid", 300., 640.), ("list", 300., 640.)] {
        for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            objc2::rc::autoreleasepool(|_| {
                let tokens = Tokens::new(scheme, Accessibility::default());
                let r = rig_project("assets", &project(&media));
                let published = r.kernel.published(crate::v4_view_tests::PROJECT).unwrap();
                let stamp = pm_panel_media::Catalog::published(&published)
                    .unwrap()
                    .stamp;
                let states = ASSETS
                    .iter()
                    .map(|(id, _, _, file, ..)| {
                        let availability = if media.join(file).is_file() {
                            Availability::Ready
                        } else if file.ends_with(".svg") {
                            Availability::Failed("The picture could not be read".into())
                        } else {
                            Availability::Offline
                        };
                        let state = RuntimeState {
                            availability,
                            poster: posters.get(*id).cloned(),
                            ..Default::default()
                        };
                        (id.to_string(), state)
                    })
                    .collect();
                r.runtime.update_media(
                    Default::default(),
                    pm_panel_media::readiness::ObservedRuntime::new(stamp, states),
                );
                let first = publication(&r);
                let runtime = r.runtime.clone();
                let mut cx = HeadlessAppContext::with_platform(
                    Arc::new(
                        gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                            .with_automatic_optical_sizing(),
                    ),
                    Arc::new(()),
                    || {
                        Some(Box::new(
                            gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                        ))
                    },
                );
                let fill = tokens.colors.panel;
                let surface = std::rc::Rc::new(std::cell::RefCell::new(None));
                let keep = surface.clone();
                let window = cx
                    .open_window(size(px(w), px(h)), |window, cx| {
                        let view = cx.new(|cx| {
                            SurfaceView::new(runtime, first, tokens, Natives::default(), window, cx)
                        });
                        *keep.borrow_mut() = Some(view.clone());
                        cx.new(|_| Frame { view, fill })
                    })
                    .unwrap();
                let surface = surface.borrow().clone().unwrap();
                for i in 0..120 {
                    // The panel asks for its saved list/grid choice.
                    for request in r.runtime.take_platform_requests() {
                        let answer = match request.operation.as_str() {
                            "project.view" => Ok(serde_json::json!(view)),
                            _ => Ok(serde_json::Value::Null),
                        };
                        r.runtime.platform_reply(request.id, answer);
                    }
                    // The panel's later surfaces (its width's columns, media
                    // states) reach the view as the app delivers them.
                    for p in r.runtime.take_publications() {
                        cx.update(|cx| surface.update(cx, |v, cx| v.publish(p, cx)));
                    }
                    cx.run_until_parked();
                    // A frame lays the list out, which reports its width
                    // (the grid's columns) and visible rows.
                    if i % 8 == 7 {
                        let _ = cx.capture_screenshot(window.into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                let image = cx.capture_screenshot(window.into()).unwrap();
                image
                    .save(format!("{dir}/{tag}-{view}-{name}.png"))
                    .unwrap();
                drop(r);
            });
        }
    }
}

struct Frame {
    view: gpui::Entity<SurfaceView>,
    fill: u32,
}
impl gpui::Render for Frame {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div()
            .size_full()
            .bg(gpui::rgb(self.fill))
            .font_family(pm_design::TEXT_FONT)
            .child(self.view.clone())
    }
}
