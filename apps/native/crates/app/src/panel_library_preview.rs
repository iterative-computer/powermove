//! Library-owned, bounded raster covers. One headless renderer at a time;
//! no native window, no recurring timer and no process-global image cache.
use super::panel_library::{Origin, PanelCard, panel_size};
use crate::v4_view::{Natives, SurfaceView};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// The stack a cover's render gets: the main thread's, where every panel
/// is drawn in its window. A caller's thread may be a GCD worker (512 KiB),
/// which a panel's render recursion overflowed (PM-669).
const COVER_STACK: usize = 8 * 1024 * 1024;

pub(super) fn capture(
    kernel: pm_kernel::KernelHandle,
    project: Option<String>,
    cards: Vec<PanelCard>,
    live: HashMap<String, Publication>,
    tokens: pm_design::Tokens,
    dir: PathBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> HashMap<String, Vec<u8>> {
    std::thread::Builder::new()
        .name("panel-library-covers".into())
        .stack_size(COVER_STACK)
        .spawn(move || capture_covers(kernel, project, cards, live, tokens, dir, cancel))
        .ok()
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default()
}

fn capture_covers(
    kernel: pm_kernel::KernelHandle,
    project: Option<String>,
    cards: Vec<PanelCard>,
    mut live: HashMap<String, Publication>,
    tokens: pm_design::Tokens,
    dir: PathBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> HashMap<String, Vec<u8>> {
    let mut images = HashMap::new();
    let document = project
        .as_deref()
        .and_then(|id| kernel.published(id))
        .and_then(|p| p.snapshot.document.to_value())
        .map(|value| pm_model::json::to_canonical_string(&value).into_bytes())
        .unwrap_or_default();
    let version = cache_stamp(
        Builtins::cover_version(),
        &format!("{:?}", tokens),
        &document,
    );
    let _ = std::fs::create_dir_all(&dir);
    // Open-panel captures stay in memory. Cold covers use the profile cache.
    for card in cards.into_iter().take(64) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if card.origin != Origin::BuiltIn && !live.contains_key(&card.id) {
            continue;
        }
        let path = dir.join(format!("{}.png", blake3::hash(card.id.as_bytes()).to_hex()));
        let stamp_path = path.with_extension("stamp");
        let publication = live.remove(&card.id);
        if publication.is_none()
            && std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(version.as_str())
        {
            if std::fs::metadata(&path).is_ok_and(|m| m.len() <= 4 * 1024 * 1024) {
                if let Ok(bytes) = std::fs::read(&path) {
                    images.insert(card.id, bytes);
                    continue;
                }
            }
        }
        let result = objc2::rc::autoreleasepool(|_| -> Option<Vec<u8>> {
            let mut runtime = match project.as_deref() {
                Some(project) => Builtins::new(kernel.clone(), project, || {}).ok()?,
                None => Builtins::application(kernel.clone(), || {}, None).ok()?,
            };
            let ticket = runtime.mount(&card.id).ok()?;
            let deadline = Instant::now() + Duration::from_secs(2);
            while ticket.try_take().is_none() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) || Instant::now() >= deadline {
                    runtime.close();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let first = publication.clone().or_else(|| {
                runtime
                    .take_publications()
                    .into_iter()
                    .find(|p| p.panel == card.id)
            })?;
            let runtime = Arc::new(runtime);
            let (w, h) = panel_size(&card.id);
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
            let window = cx
                .open_window(size(px(w.min(960.)), px(h.min(720.))), |window, cx| {
                    let view = cx.new(|cx| {
                        SurfaceView::new(
                            runtime.clone(),
                            first,
                            tokens.clone(),
                            Natives::default(),
                            window,
                            cx,
                        )
                    });
                    cx.new(|_| Frame {
                        view,
                        fill: tokens.colors.panel,
                    })
                })
                .ok()?;
            // Publication pumping is essential: the first frame can be only a
            // mount shell. Draw the accepted surface after its follow-up arrives.
            let settle_deadline = Instant::now() + Duration::from_millis(500);
            for tick in 0..64 {
                cx.run_until_parked();
                if publication.is_none() {
                    for p in runtime.take_publications() {
                        if p.panel == card.id {
                            let _ = window.update(&mut cx, |frame, _, cx| {
                                frame.view.update(cx, |v, cx| v.publish(p, cx))
                            });
                        }
                    }
                }
                let ready = window
                    .update(&mut cx, |frame, _, cx| !frame.view.read(cx).compiling())
                    .unwrap_or(false);
                if (tick >= 3 && ready) || Instant::now() >= settle_deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).ok()?;
            // 2x the minimum cover scale; only one full panel raster is alive.
            let (nw, nh) = ((w.min(960.) * 0.8) as u32, (h.min(720.) * 0.8) as u32);
            let mut pixels = Vec::with_capacity((nw * nh * 4) as usize);
            for y in 0..nh {
                for x in 0..nw {
                    pixels.extend_from_slice(
                        &image
                            .get_pixel(x * image.width() / nw, y * image.height() / nh)
                            .0,
                    );
                }
            }
            runtime.close();
            crate::mcp_services::png(&pixels, nw, nh).ok()
        });
        if let Some(png) = result {
            if publication.is_none() {
                // One replaceable file per panel; code/theme/document changes
                // invalidate its stamp rather than accumulating old versions.
                if std::fs::write(&path, &png).is_ok() {
                    let _ = std::fs::write(&stamp_path, &version);
                }
            }
            images.insert(card.id, png);
        }
    }
    images
}
struct Frame {
    view: Entity<SurfaceView>,
    fill: u32,
}
impl Render for Frame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(self.fill))
            .font_family(pm_design::TEXT_FONT)
            .child(self.view.clone())
    }
}

fn cache_stamp(bundle: &str, theme: &str, document: &[u8]) -> String {
    let mut hash = blake3::Hasher::new();
    for part in [bundle.as_bytes(), theme.as_bytes(), document] {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.finalize().to_hex().to_string()
}
#[cfg(test)]
mod tests {
    use super::{bounded, cache_stamp};
    /// A flat cover is no picture; anything drawn on it is.
    #[test]
    fn a_flat_cover_is_blank() {
        let flat = vec![250u8; 40 * 30 * 4];
        let png = crate::mcp_services::png(&flat, 40, 30).unwrap();
        assert!(super::blank(&png));
        let mut drawn = flat.clone();
        for p in drawn.chunks_exact_mut(4).take(400) {
            p[0] = 0;
        }
        let png = crate::mcp_services::png(&drawn, 40, 30).unwrap();
        assert!(!super::blank(&png));
    }
    /// Covers keep their panel's proportions: a tall panel stays tall.
    #[test]
    fn covers_scale_evenly() {
        assert_eq!(bounded(600, 1400), (206, 480));
        assert_eq!(bounded(1920, 720), (768, 288));
        assert_eq!(bounded(300, 200), (300, 200));
    }
    #[test]
    fn library_preview_cache_invalidates_on_panel_code_theme_or_document_change() {
        let first = cache_stamp("panel-v1", "dark", b"project-v1");
        assert_eq!(first, cache_stamp("panel-v1", "dark", b"project-v1"));
        assert_ne!(first, cache_stamp("panel-v2", "dark", b"project-v1"));
        assert_ne!(first, cache_stamp("panel-v1", "light", b"project-v1"));
        assert_ne!(first, cache_stamp("panel-v1", "dark", b"project-v2"));
    }
}

/// Whether a cover is one flat colour almost everywhere (an empty panel's
/// ground): 99 % of its sampled pixels within a step of its first.
pub(super) fn blank(png: &[u8]) -> bool {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    let Some(size) = reader.output_buffer_size() else {
        return false;
    };
    let mut buffer = vec![0; size];
    let Ok(info) = reader.next_frame(&mut buffer) else {
        return false;
    };
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return false,
    };
    let pixels: Vec<&[u8]> = buffer[..info.buffer_size()]
        .chunks_exact(channels)
        .step_by(7)
        .collect();
    let Some(first) = pixels.first() else {
        return true;
    };
    let near = |p: &&[u8]| (0..3).all(|i| p[i].abs_diff(first[i]) <= 6);
    pixels.iter().filter(|p| near(p)).count() * 100 >= pixels.len() * 99
}
/// A picture `w`×`h` scaled evenly to fit the cover raster's budget
/// (768×480 at most): its proportions kept.
fn bounded(w: u32, h: u32) -> (u32, u32) {
    let scale = (768. / w.max(1) as f32).min(480. / h.max(1) as f32).min(1.);
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}
/// Decode a WebKit snapshot to the same bounded raster budget as native covers.
pub(super) fn fit_web(id: &str, bytes: Vec<u8>) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let size = reader.output_buffer_size()?;
    if size > 64 * 1024 * 1024 {
        return None;
    }
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).ok()?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return None,
    };
    let _ = id;
    let (nw, nh) = bounded(info.width, info.height);
    let mut rgba = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        for x in 0..nw {
            let i =
                (((y * info.height / nh) * info.width + x * info.width / nw) as usize) * channels;
            rgba.extend_from_slice(&buffer[i..i + 3]);
            rgba.push(if channels == 4 { buffer[i + 3] } else { 255 });
        }
    }
    crate::mcp_services::png(&rgba, nw, nh).ok()
}

pub(super) fn crop_drawn(
    id: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
    scale: f32,
    rect: pm_shell::layout::Rect,
) -> Option<Vec<u8>> {
    let x = (rect.x.max(0.) * scale) as u32;
    let y = (rect.y.max(0.) * scale) as u32;
    let w = ((rect.w.min(960.) * scale) as u32).min(width.saturating_sub(x));
    let h = ((rect.h.min(720.) * scale) as u32).min(height.saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    let _ = id;
    // The panel's own shape, scaled down evenly (never squeezed into
    // another panel's proportions).
    let (nw, nh) = bounded(w, h);
    let mut cover = Vec::with_capacity((nw * nh * 4) as usize);
    for dy in 0..nh {
        for dx in 0..nw {
            let i = (((y + dy * h / nh) * width + x + dx * w / nw) * 4) as usize;
            cover.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    crate::mcp_services::png(&cover, nw, nh).ok()
}
