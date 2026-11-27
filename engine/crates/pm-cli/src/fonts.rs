//! The CLI's text fonts (`docs/engine/text.md`, "Fonts on render threads").
//!
//! `pm-cli` renders on its main thread, so the CoreText registry answers
//! the renderer's layout directly (`pm_text::served::DirectServer`) with
//! the full fonts it reads. The renderer's font cache seeds model the
//! editor renderer whose exports the CLI reproduces: its UI text laid out
//! at `--ui-scale` (default 2, a Retina display; amendment 19).

use std::sync::Arc;

/// The device scale factor the CLI assumes for the editor's UI text.
pub const DEFAULT_UI_SCALE: f32 = 2.0;

/// Gives `renderer` the registry's fonts with the editor renderer's seeds
/// at `ui_scale`. False when the registry cannot start (off the main
/// thread, or a platform without one); text and captions then stay
/// unsupported.
pub fn install(renderer: &mut pm_render::Renderer, ui_scale: f32) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Some(registry) = pm_text::registry::CoreTextRegistry::try_new() else {
            return false;
        };
        let server = Arc::new(pm_text::served::DirectServer::new(registry, |r| {
            r.generation()
        }));
        let seeds = pm_text::compat::app_ui_font_cache_seeds(&[ui_scale]);
        renderer.set_text_fonts(Some(Box::new(pm_render::text::served::ServedFonts::new(
            server,
            Arc::new(pm_render::text::served::FixedSeeds(seeds)),
        ))));
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (renderer, ui_scale);
        false
    }
}

/// Parses `--ui-scale`.
pub fn parse_ui_scale(v: &str) -> Result<f32, String> {
    match v.parse::<f32>() {
        Ok(s) if s.is_finite() && (0.5..=4.0).contains(&s) => Ok(s),
        _ => Err(format!(
            "ui scale {v:?} (a device scale factor from 0.5 to 4)"
        )),
    }
}
