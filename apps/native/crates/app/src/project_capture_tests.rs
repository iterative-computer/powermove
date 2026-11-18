//! Headless pictures of the Project panel on a synthetic project with real
//! media (review captures; never committed). The media folder is made by
//! the reviewer (generated test patterns), named by `PM_PROJECT_MEDIA`:
//! `PM_PROJECT_MEDIA=… PM_PANEL_CAPTURE=… cargo test -p pm-native-app --bin powermove-native capture_project_panel -- --ignored`
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{publication, rig_project};
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_builtins::Builtins;
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
    let source = std::env::var("PM_PROJECT_CAPTURE_JSON").ok()
        .map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_else(|| project(&media));
    let data: serde_json::Value = serde_json::from_str(&source).unwrap();
    let assets: Vec<(String, String, std::path::PathBuf, f64)> = data["assets"].as_object().unwrap()
        .iter().map(|(id, a)| (id.clone(), a["kind"].as_str().unwrap().to_owned(),
            a["sourcePath"].as_str().unwrap().into(), a["dur"].as_f64().unwrap_or(0.))).collect();
    let loading = std::env::var_os("PM_CAPTURE_LOADING").is_some();
    // Thumbnails as the app makes them, registered for the view to draw.
    let mut thumbs = crate::media_thumbs::Thumbs::default();
    let cache = Path::new(&dir).join(".thumbs");
    let ffmpeg = std::env::var_os("PM_FFMPEG").map(std::path::PathBuf::from);
    let mut posters = BTreeMap::new();
    for (id, kind, path, dur) in &assets {
        let path = path.clone();
        if !path.is_file() {
            continue;
        }
        let item = crate::media_thumbs::Item {
            asset: id.clone(),
            kind: kind.clone(),
            path,
            at: (kind == "video").then(|| (dur * 0.1).min(0.5)),
        };
        let made = crate::media_thumbs::make(&cache, &item, ffmpeg.as_deref());
        if let Some((key, _)) = &made {
            posters.insert(id.to_owned(), key.clone());
        }
        thumbs.done(thumbs.generation(), id.clone(), made);
    }
    if media.join("moving-color.mp4").is_file() {
        let key = posters.get("a2").expect("the real video fixture must generate a poster");
        assert!(crate::media_thumbs::resolve(key).is_some(), "the native renderer can resolve the video poster");
    }
    for (view, w, h) in [("grid", 300., 640.), ("list", 300., 640.)] {
        for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            if std::env::var("PM_CAPTURE_ONLY").ok().is_some_and(|v| v != format!("{view}-{name}")) { continue; }
            objc2::rc::autoreleasepool(|_| {
                let tokens = Tokens::new(scheme, Accessibility::default());
                let r = rig_project("assets", &source);
                let published = r.kernel.published(crate::v4_view_tests::PROJECT).unwrap();
                let stamp = pm_panel_media::Catalog::published(&published)
                    .unwrap()
                    .stamp;
                let states = |ready: bool| -> BTreeMap<String, RuntimeState> {
                    assets.iter().map(|(id, _, path, _)| {
                        let availability = if path.is_file() {
                            if ready { Availability::Ready } else { Availability::Loading }
                        } else if path.extension().is_some_and(|v| v == "svg") {
                            Availability::Failed("The picture could not be read".into())
                        } else { Availability::Offline };
                        (id.clone(), RuntimeState { availability,
                            poster: ready.then(|| posters.get(id).cloned()).flatten(),
                            ..Default::default() })
                    }).collect()
                };
                r.runtime.update_media(Default::default(),
                    pm_panel_media::readiness::ObservedRuntime::new(stamp.clone(), states(!loading)));
                if std::env::var("PM_PROJECT_LINKED").is_ok() {
                    let state = match std::env::var("PM_PROJECT_SYNC").as_deref() {
                        Ok("missing") => pm_panel_media::FolderSync::NotFound,
                        Ok("syncing") => pm_panel_media::FolderSync::Syncing,
                        _ => pm_panel_media::FolderSync::Synced,
                    };
                    r.runtime.update_folder_sync(std::collections::BTreeMap::from([("fFootage".into(), state.clone()), ("fBroll".into(), state)]));
                }
                publish_runtime(&r.runtime);
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
                if let Ok(scale) = std::env::var("PM_CAPTURE_SCALE") {
                    cx.simulate_scale_change(window.into(), scale.parse().expect("PM_CAPTURE_SCALE"));
                }
                let surface = surface.borrow().clone().unwrap();
                for i in 0..240 {
                    if i == 80 {
                        if let Ok(delta) = std::env::var("PM_CAPTURE_WHEEL") {
                            let position = gpui::point(px(100.), px(100.));
                            cx.simulate_event(window.into(), gpui::MouseMoveEvent { position, ..Default::default() });
                            cx.simulate_event(window.into(), gpui::ScrollWheelEvent { position,
                                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta.parse::<f32>().unwrap()))),
                                ..Default::default() });
                        }
                    }
                    if i == 144 && loading {
                        r.runtime.update_media(Default::default(),
                            pm_panel_media::readiness::ObservedRuntime::new(stamp.clone(), states(true)));
                        publish_runtime(&r.runtime);
                    }
                    // Synthetic pointer delivery in the headless renderer only.
                    // No operating-system window, focus or cursor is involved.
                    if i == 120 {
                        if let Ok(target) = std::env::var("PM_PROJECT_CAPTURE_SELECT") {
                            cx.update(|cx| surface.update(cx, |v, cx| v.review_send(&target, "pointerdown", cx)));
                        }
                    }
                    if i == 160 {
                        if let Ok(target) = std::env::var("PM_CAPTURE_HOVER") {
                            cx.update(|cx| surface.update(cx, |v, cx| v.review_send(&target, "pointermove", cx)));
                        }
                        if let Ok(name) = std::env::var("PM_CAPTURE_RENAME_NAME") {
                            cx.update(|cx| surface.update(cx, |v, cx| {
                                let id = v.id_of_text(&name).expect("rename capture name is visible");
                                v.send(id, "doubleclick", Default::default(), cx);
                            }));
                        }
                    }
                    if i == 200 {
                        if let Ok(target) = std::env::var("PM_CAPTURE_RENAME_COMMIT") {
                            cx.update(|cx| surface.update(cx, |v, cx| v.review_send(&target, "change", cx)));
                        }
                    }
                    if i == 210 && std::env::var_os("PM_CAPTURE_RENAME_COMMIT").is_some() {
                        // A rename advances the document revision. Match the app's
                        // next observation publication so its posters remain ready.
                        let next = r.kernel.published(crate::v4_view_tests::PROJECT).unwrap();
                        let stamp = pm_panel_media::Catalog::published(&next).unwrap().stamp;
                        r.runtime.update_media(Default::default(),
                            pm_panel_media::readiness::ObservedRuntime::new(stamp, states(true)));
                        publish_runtime(&r.runtime);
                    }
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
                        let frame = cx.capture_screenshot(window.into()).unwrap();
                        if std::env::var_os("PM_CAPTURE_FRAMES").is_some() {
                            frame.save(format!("{dir}/{tag}-{view}-{name}-frame-{i:03}.png")).unwrap();
                        }
                    }
                    if std::env::var_os("PM_CAPTURE_RENAME_NAME").is_some() {
                        let stage = match i { 151 => Some("before"), 191 => Some("during"), 239 => Some("after"), _ => None };
                        if let Some(stage) = stage {
                            if let Ok(commit) = std::env::var("PM_CAPTURE_RENAME_COMMIT") {
                                let (field, value) = commit.split_once('=').expect("rename field=value");
                                if stage == "during" {
                                    assert!(cx.update(|cx| surface.read(cx).semantic_ids().iter().any(|id| id == field)),
                                        "name double-click must publish the inline field before capturing");
                                } else if stage == "after" {
                                    let value: String = serde_json::from_str(value).expect("rename JSON string");
                                    assert!(cx.update(|cx| surface.read(cx).id_of_text(&value).is_some()),
                                        "committing the field must publish the renamed text before capturing");
                                }
                            }
                            let image = cx.capture_screenshot(window.into()).unwrap();
                            image.save(format!("{dir}/{tag}-{stage}-{view}-{name}.png")).unwrap();
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                // The headless harness has no display-link invalidation loop.
                // Redraw after asynchronous image decoding has settled.
                window.update(&mut cx, |_, window, _| window.refresh()).unwrap();
                cx.run_until_parked();
                let image = cx.capture_screenshot(window.into()).unwrap();
                image
                    .save(format!("{dir}/{tag}-{view}-{name}.png"))
                    .unwrap();
                if std::env::var_os("PM_CAPTURE_VERIFY_GRID").is_some() {
                    assert_eq!(view, "grid");
                    let published = r.kernel.published(crate::v4_view_tests::PROJECT).unwrap();
                    let catalog = pm_panel_media::Catalog::published(&published).unwrap();
                    let observed = pm_panel_media::readiness::ObservedRuntime::new(catalog.stamp.clone(), states(true));
                    let mut model = pm_panel_media::MediaPanel::default();
                    model.refresh(&catalog, &observed);
                    let page = model.page(&catalog, &observed, 0, 4096).unwrap();
                    assert_eq!(page.rows.len(), page.total, "audit needs the complete synthetic model");
                    let order: Vec<_> = page.rows.iter().enumerate().map(|(index, row)| {
                        let name = match &row.data {
                            pm_panel_media::RowData::Asset(a, _) => &a.name,
                            pm_panel_media::RowData::Composition(c) => &c.name,
                            pm_panel_media::RowData::Folder(f) => &f.name,
                        };
                        (serde_json::to_string(&row.id).unwrap(), name.to_string(), index)
                    }).collect();
                    let report = audit_project_grid(&mut cx, window.into(), &surface, &order);
                    std::fs::write(format!("{dir}/{tag}-{view}-{name}-grid-order.json"),
                        serde_json::to_string_pretty(&report).unwrap()).unwrap();
                    println!("PROJECT_GRID_ORDER {}", report);
                }
                drop(r);
            });
        }
    }
}

pub(crate) fn assert_project_grid_contiguous(model: &[String], first: usize, end: usize, painted: &[String]) {
    assert!(first <= end && end <= model.len(), "painted range outside model");
    assert_eq!(painted, &model[first..end], "painted cards must be the contiguous model slice at this scroll offset");
}

#[test]
#[should_panic(expected = "painted cards must be the contiguous model slice")]
fn project_grid_contiguity_rejects_a_skipped_card() {
    assert_project_grid_contiguous(&["a".into(), "b".into(), "c".into()], 0, 3, &["a".into(), "c".into()]);
}

#[test]
#[should_panic(expected = "painted cards must be the contiguous model slice")]
fn project_grid_contiguity_rejects_a_card_at_the_wrong_position() {
    assert_project_grid_contiguous(&["a".into(), "b".into(), "c".into()], 1, 3, &["c".into(), "b".into()]);
}

fn audit_project_grid(
    cx: &mut HeadlessAppContext,
    window: gpui::AnyWindowHandle,
    surface: &gpui::Entity<SurfaceView>,
    model: &[(String, String, usize)],
) -> serde_json::Value {
    let prepared = cx.update(|cx| surface.read(cx).prepared.clone());
    let lists: Vec<_> = prepared.nodes.values().filter(|n| n.virtual_window.is_some()).collect();
    assert_eq!(lists.len(), 1);
    let list = lists[0];
    let geometry = list.virtual_window.unwrap();
    let (offset, viewport, max_offset) = cx.update(|cx| surface.read(cx).test_scroll_state(list.id).unwrap());
    let columns = model.len().div_ceil(geometry.count);
    assert_eq!(columns, 2, "this audit reproduces the two-column fixture");
    let scroll = -f32::from(offset.y);
    let first = ((f64::from(scroll) / geometry.item_size).floor() as usize).min(geometry.count);
    let end = (((f64::from(scroll) + f64::from(f32::from(viewport.size.height))) / geometry.item_size).ceil() as usize).min(geometry.count);
    let mut rows = Vec::new();
    for (index, id) in list.children.iter().enumerate() {
        let row = &prepared.nodes[id];
        let Some(bounds) = cx.debug_bounds(window, &format!("n{id}")).unwrap() else { continue };
        if bounds.bottom() <= viewport.top() || bounds.top() >= viewport.bottom() { continue; }
        let logical = geometry.range.0 + index;
        let mut cards = Vec::new();
        for child in &row.children {
            let card = &prepared.nodes[child];
            let Some(semantic) = card.semantic.as_deref() else { continue };
            let Some(item) = semantic.strip_prefix("media:") else { continue };
            let bounds = cx.debug_bounds(window, semantic).unwrap().expect("published card must be painted");
            assert!(bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom());
            let expected = model.iter().find(|(id, _, _)| id == item).expect("painted card must belong to the model");
            let mut pending = vec![*child];
            let mut text = Vec::new();
            while let Some(id) = pending.pop() {
                let n = &prepared.nodes[&id];
                if let Some(t) = &n.text { text.push(t.to_string()); }
                pending.extend(n.children.iter().copied());
            }
            assert!(text.contains(&expected.1), "card {item} paints the wrong name: {text:?}");
            cards.push((f32::from(bounds.left()), serde_json::json!({
                "id": item, "name": expected.1, "model_index": expected.2,
                "bounds": [f32::from(bounds.left()), f32::from(bounds.top()), f32::from(bounds.size.width), f32::from(bounds.size.height)],
                "painted_text": text,
            })));
        }
        cards.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(cards.len(), (model.len() - logical * columns).min(columns), "painted row must have every expected column");
        for (column, (_, card)) in cards.iter().enumerate() {
            assert_eq!(card["model_index"].as_u64().unwrap() as usize, logical * columns + column, "card must occupy its model row and column");
        }
        rows.push((f32::from(bounds.top()), logical, cards.into_iter().map(|(_, c)| c).collect::<Vec<_>>()));
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(rows.iter().map(|(_, index, _)| *index).collect::<Vec<_>>(), (first..end).collect::<Vec<_>>(), "painted row indices must cover the viewport without gaps");
    let painted: Vec<_> = rows.iter().flat_map(|(_, _, cards)| cards.iter().map(|c| c["id"].as_str().unwrap().to_owned())).collect();
    let ids: Vec<_> = model.iter().map(|(id, _, _)| id.clone()).collect();
    assert_project_grid_contiguous(&ids, first * columns, (end * columns).min(model.len()), &painted);
    serde_json::json!({
        "gpui_offset": [f32::from(offset.x), f32::from(offset.y)], "scroll_offset": scroll,
        "max_offset": [f32::from(max_offset.x), f32::from(max_offset.y)],
        "viewport": [f32::from(viewport.left()), f32::from(viewport.top()), f32::from(viewport.size.width), f32::from(viewport.size.height)],
        "row_height": geometry.item_size, "columns": columns, "published_range": geometry.range,
        "first_painted_row": first, "last_painted_row": end - 1, "contiguous": true,
        "model_order": model.iter().map(|(id, name, index)| serde_json::json!({"id":id,"name":name,"index":index,"row":index / columns,"column":index % columns})).collect::<Vec<_>>(),
        "painted_rows": rows.into_iter().map(|(y, index, cards)| serde_json::json!({"row":index,"y":y,"cards":cards})).collect::<Vec<_>>(),
    })
}

// Match the app's media refresh: updating observations alone does not ask
// the panel to project them. Wait only for these in-memory evaluation jobs.
fn publish_runtime(runtime: &Builtins) {
    for (_, ticket) in runtime.publish() {
        let mut ticket = ticket.expect("capture publication queued");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Some(result) = ticket.try_take() {
                result.expect("capture publication evaluated");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "capture publication timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
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
