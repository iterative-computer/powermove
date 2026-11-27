//! Library-owned, bounded raster covers. One shared headless renderer & font
//! system per capture pass; no native window, no recurring timer, and
//! progressive per-panel streaming so covers appear immediately.
use super::panel_library::{Origin, PanelCard, panel_size};
use crate::v4_view::{Natives, SurfaceView};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

/// The stack a cover's render gets: the main thread's, where every panel
/// is drawn in its window. A caller's thread may be a GCD worker (512 KiB),
/// which a panel's render recursion overflowed (PM-669).
const COVER_STACK: usize = 8 * 1024 * 1024;

/// Shared system font database across cover captures: scanning macOS system
/// fonts takes hundreds of milliseconds, so build it once per process instead
/// of once per panel card.
fn cover_text_system() -> Arc<gpui_parley::ParleyTextSystem> {
    static TEXT: OnceLock<Arc<gpui_parley::ParleyTextSystem>> = OnceLock::new();
    TEXT.get_or_init(|| {
        Arc::new(
            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                .with_automatic_optical_sizing(),
        )
    })
    .clone()
}

pub(super) fn capture(
    kernel: pm_kernel::KernelHandle,
    project: Option<String>,
    cards: Vec<PanelCard>,
    live: HashMap<String, Publication>,
    tokens: pm_design::Tokens,
    dir: PathBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> HashMap<String, Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    capture_streaming(
        kernel,
        project,
        cards,
        live,
        tokens,
        dir,
        cancel,
        move |id, png| {
            let _ = tx.send((id, png));
        },
    );
    rx.into_iter().collect()
}

/// Captures covers for `cards`, invoking `on_image` as soon as each cover is
/// available (disk hits first in milliseconds, then newly rendered cold covers
/// sharing one `Builtins` runtime and one `HeadlessAppContext`).
#[allow(clippy::too_many_arguments)]
pub(super) fn capture_streaming(
    kernel: pm_kernel::KernelHandle,
    project: Option<String>,
    cards: Vec<PanelCard>,
    live: HashMap<String, Publication>,
    tokens: pm_design::Tokens,
    dir: PathBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    on_image: impl FnMut(String, Vec<u8>) + Send + 'static,
) {
    let _ = std::thread::Builder::new()
        .name("panel-library-covers".into())
        .stack_size(COVER_STACK)
        .spawn(move || {
            capture_covers(kernel, project, cards, live, tokens, dir, cancel, on_image)
        })
        .ok()
        .and_then(|thread| thread.join().ok());
}

#[allow(clippy::too_many_arguments)]
fn capture_covers(
    kernel: pm_kernel::KernelHandle,
    project: Option<String>,
    cards: Vec<PanelCard>,
    mut live: HashMap<String, Publication>,
    tokens: pm_design::Tokens,
    dir: PathBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    mut on_image: impl FnMut(String, Vec<u8>),
) {
    // Key disk cache on panel bundle version + visual theme (scheme & contrast),
    // not the full serialized project document, so opening Panel Library on a
    // large comp never serializes megabytes of JSON or busts covers on every edit.
    let theme_key = format!(
        "{:?}:{}",
        tokens.scheme, tokens.accessibility.increase_contrast
    );
    let version = cache_stamp(Builtins::cover_version(), &theme_key, &[]);
    let _ = std::fs::create_dir_all(&dir);

    // Phase 1: emit every valid disk-cached cover immediately before spinning
    // up any headless renderer or JS runtime.
    let mut pending: Vec<(PanelCard, PathBuf, PathBuf, Option<Publication>)> = Vec::new();
    for card in cards.into_iter().take(64) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if card.origin != Origin::BuiltIn && !live.contains_key(&card.id) {
            continue;
        }
        let path = dir.join(format!("{}.png", blake3::hash(card.id.as_bytes()).to_hex()));
        let stamp_path = path.with_extension("stamp");
        let publication = live.remove(&card.id);
        if publication.is_none()
            && std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(version.as_str())
            && std::fs::metadata(&path).is_ok_and(|m| m.len() <= 4 * 1024 * 1024)
        {
            if let Ok(bytes) = std::fs::read(&path) {
                on_image(card.id, bytes);
                continue;
            }
        }
        pending.push((card, path, stamp_path, publication));
    }

    if pending.is_empty() || cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    // Phase 2: share ONE Builtins runtime, ONE cached ParleyTextSystem, and
    // ONE HeadlessAppContext across all cold panels instead of recreating
    // fonts, Metal, and JS for every card.
    objc2::rc::autoreleasepool(|_| {
        let mut runtime = match project.as_deref() {
            Some(project) => Builtins::new(kernel.clone(), project, || {}).ok(),
            None => Builtins::application(kernel.clone(), || {}, None).ok(),
        };
        let Some(ref mut rt) = runtime else {
            return;
        };
        // Mount all uncached built-in panels upfront on the single shared JS
        // runtime, then wrap in Arc for headless rendering.
        let mut mounted_tickets = Vec::new();
        for (card, _, _, publication) in &pending {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            if publication.is_none()
                && let Ok(ticket) = rt.mount(&card.id)
            {
                mounted_tickets.push((card.id.clone(), ticket));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut initial_pubs: HashMap<String, Publication> = HashMap::new();
        for (_, ticket) in mounted_tickets {
            while ticket.try_take().is_none() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) || Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            for p in rt.take_publications() {
                initial_pubs.insert(p.panel.clone(), p);
            }
        }
        for p in rt.take_publications() {
            initial_pubs.insert(p.panel.clone(), p);
        }
        let runtime = Arc::new(runtime.unwrap());
        let mut cx = HeadlessAppContext::with_platform(
            cover_text_system(),
            Arc::new(()),
            || {
                Some(Box::new(
                    gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                ))
            },
        );

        for (card, path, stamp_path, publication) in pending {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let png = objc2::rc::autoreleasepool(|_| -> Option<Vec<u8>> {
                let first = publication
                    .clone()
                    .or_else(|| initial_pubs.remove(&card.id))
                    .or_else(|| {
                        runtime
                            .take_publications()
                            .into_iter()
                            .rev()
                            .find(|p| p.panel == card.id)
                    })?;
                let (w, h) = panel_size(&card.id);
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
                // Pump until the surface compiles or a short follow-up arrives.
                let settle_deadline = Instant::now() + Duration::from_millis(220);
                for tick in 0..24 {
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
                    if (tick >= 1 && ready) || Instant::now() >= settle_deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
                cx.run_until_parked();
                let image = cx.capture_screenshot(window.into()).ok()?;
                let _ = window.update(&mut cx, |_, window, _| window.remove_window());
                let (nw, nh) = ((w.min(960.) * 0.8) as u32, (h.min(720.) * 0.8) as u32);
                let src_w = image.width();
                let src_h = image.height();
                if src_w == 0 || src_h == 0 || nw == 0 || nh == 0 {
                    return None;
                }
                let raw = image.as_raw();
                let mut pixels = Vec::with_capacity((nw * nh * 4) as usize);
                for y in 0..nh {
                    let sy = ((y as usize) * (src_h as usize) / (nh as usize)) * (src_w as usize);
                    for x in 0..nw {
                        let sx = (x as usize) * (src_w as usize) / (nw as usize);
                        let i = (sy + sx) * 4;
                        pixels.extend_from_slice(&raw[i..i + 4]);
                    }
                }
                crate::mcp_services::png(&pixels, nw, nh).ok()
            });
            if let Some(png) = png {
                if publication.is_none() && std::fs::write(&path, &png).is_ok() {
                    let _ = std::fs::write(&stamp_path, &version);
                }
                on_image(card.id, png);
            }
        }
        runtime.close();
    });
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
    use super::cache_stamp;
    #[test]
    fn library_preview_cache_invalidates_on_panel_code_theme_or_document_change() {
        let first = cache_stamp("panel-v1", "dark", b"project-v1");
        assert_eq!(first, cache_stamp("panel-v1", "dark", b"project-v1"));
        assert_ne!(first, cache_stamp("panel-v2", "dark", b"project-v1"));
        assert_ne!(first, cache_stamp("panel-v1", "light", b"project-v1"));
        assert_ne!(first, cache_stamp("panel-v1", "dark", b"project-v2"));
    }
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
    let (w, h) = panel_size(id);
    let (nw, nh) = ((w.min(960.) * 0.8) as u32, (h.min(720.) * 0.8) as u32);
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
    let (cw, ch) = panel_size(id);
    let (nw, nh) = ((cw * 0.8) as u32, (ch * 0.8) as u32);
    let mut cover = Vec::with_capacity((nw * nh * 4) as usize);
    for dy in 0..nh {
        let sy = ((y + dy * h / nh) as usize) * (width as usize);
        for dx in 0..nw {
            let sx = (x + dx * w / nw) as usize;
            let i = (sy + sx) * 4;
            cover.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    crate::mcp_services::png(&cover, nw, nh).ok()
}
