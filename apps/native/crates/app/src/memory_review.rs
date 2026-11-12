//! Opt-in GUI memory reproduction. A JSON command file is read only when
//! PM_NATIVE_MEMORY_REVIEW names it in a development build. Commands carry
//! increasing `id`s and an `action`: open (with `path`), play, stop, close,
//! collect, pressure, sample, or quit. Numeric diagnostics go to a sibling
//! .samples.jsonl file, outside the project and the normal user profile.
use crate::session::Controller;
use gpui::*;
use pm_protocol::{clock::TransportCommand, common::PlaybackRate};
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf, time::Duration};

pub fn install(
    entity: &Entity<Controller>,
    fonts: std::rc::Rc<std::cell::RefCell<pm_text::host::FontService>>,
    cx: &mut App,
) {
    let Some(path) = std::env::var_os("PM_NATIVE_MEMORY_REVIEW").map(PathBuf::from) else {
        return;
    };
    let weak = entity.downgrade();
    cx.spawn(async move |cx| {
        let mut seen = None;
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            if weak.upgrade().is_none() { break; }
            let command = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let Some(command) = command else { continue };
            let Some(id) = command["id"].as_u64() else { continue };
            if seen.is_some_and(|seen| id <= seen) {
                continue;
            }
            seen = Some(id);
            let result = weak.update(cx, |v, cx| {
                match command["action"].as_str().unwrap_or_default() {
                    "open" => {
                        if let Some(path) = command["path"].as_str() {
                            v.open_file(PathBuf::from(path), cx);
                        }
                    }
                    "play" => {
                        if let Some(viewer) = &v.viewer {
                            let _ = viewer.command(TransportCommand::Play { rate: PlaybackRate::NORMAL });
                        }
                    }
                    "stop" => {
                        if let Some(viewer) = &v.viewer {
                            let _ = viewer.command(TransportCommand::Pause);
                        }
                    }
                    "close" => v.request_close(crate::lifecycle::Intent::Window, cx),
                    "collect" => pm_engine_service::memory::pressure_relief(),
                    "pressure" => {
                        if let Some(runtime) = &v.builtins.runtime {
                            runtime.memory_pressure();
                        }
                        pm_engine_service::memory::pressure_relief();
                    }
                    "quit" => cx.defer(crate::lifecycle::begin_quit),
                    _ => {}
                }
                json!({
                    "id": id,
                    "footprint": pm_engine_service::memory::phys_footprint(),
                    "project": v.session.is_some(),
                    "opening": v.opening,
                    "settling": v.settling.is_some(),
                    "closing": v.close_pending,
                    "document_owners": v.published.as_ref().map(|p| std::sync::Arc::strong_count(&p.snapshot.document)),
                    "clock": v.viewer.as_ref().and_then(|v| v.status().clock),
                    "presented": v.viewer.as_ref().map(|v| v.status().presented),
                    "stale_rejections": v.viewer.as_ref().map(|v| v.status().stale_rejections),
                    "held_frames": v.viewer.as_ref().map(|v| v.status().held_frames),
                    "skipped_targets": v.viewer.as_ref().map(|v| v.status().skipped_targets),
                    "no_drawable": v.viewer.as_ref().map(|v| v.status().no_drawable),
                    "output_pixels": v.main_output.get().map(|o| [o.width, o.height]),
                    "render_scale": v.viewer.as_ref().and_then(|v| v.status().frame).map(|f| f.scale),
                    "media_originals": v.media.originals.len(),
                    "offline_assets": v.offline_media().len(),
                    "fonts": fonts.borrow().memory_usage(),
                    "text_mirrors": crate::text_host::mirror_count(),
                    "leases": pm_engine_service::memory::usage(),
                    "caches": v.engine.memory_usage(),
                })
            });
            let Ok(sample) = result else { break };
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path.with_extension("samples.jsonl")) {
                let _ = writeln!(file, "{sample}");
            }
        }
    }).detach();
}
