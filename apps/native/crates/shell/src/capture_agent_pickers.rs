//! Companion to the app's `capture_agent_states`: overlays the real native
//! menu, rendered without an AppKit window, over the TestPlatform PNG.
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use image::imageops::{FilterType, overlay, resize};
    use pm_design::{Accessibility, Scheme, Tokens};
    use pm_shell::layout::Rect;
    use std::{fs, path::PathBuf};

    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let states = args.next().context("fixture directory")?;
    let captures = args.next().context("capture directory")?;
    for state in ["model", "effort", "access", "long-model", "model-search"] {
        let request = states.join("requests").join(format!("{state}.json"));
        if !request.is_file() {
            continue;
        }
        let request: serde_json::Value = serde_json::from_slice(&fs::read(request)?)?;
        let baseline = request["picker"].is_null();
        for (theme, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            objc2::rc::autoreleasepool(|_| -> anyhow::Result<()> {
                let name = format!("agent-{state}-{theme}");
                let mut panel = image::open(captures.join(format!("{name}.png")))?.into_rgba8();
                let tokens = Tokens::new(scheme, Accessibility::default());
                let owner = Rect {
                    w: tokens.metrics.agent_width,
                    h: tokens.metrics.agent_height,
                    ..Default::default()
                };
                let anchor: Rect = if baseline {
                    Rect {
                        x: 12.,
                        y: 8.,
                        w: 100.,
                        h: tokens.metrics.control,
                    }
                } else {
                    let frame: Rect = serde_json::from_slice(&fs::read(
                        captures.join(format!("{name}-anchor.json")),
                    )?)?;
                    Rect {
                        y: owner.h - frame.y - frame.h,
                        ..frame
                    }
                };
                let query = if state == "model-search" {
                    "sonnet"
                } else {
                    ""
                };
                let (png, at, clipped) =
                    pm_shell::capture_agent_picker(&tokens, &request, owner, anchor, query)
                        .context("paint detached AppKit menu")?;
                let attached = at.x <= anchor.x
                    && at.x + at.w >= anchor.x
                    && at.y <= anchor.y + anchor.h + 0.5
                    && at.y + at.h >= anchor.y + anchor.h
                    && at.y + at.h <= owner.h + 0.5;
                if baseline {
                    ensure!(!attached, "baseline should reproduce the detached menu");
                    if state == "access" {
                        ensure!(
                            !clipped.is_empty(),
                            "baseline should reproduce the clipped descriptions"
                        );
                    }
                    println!(
                        "{name}: baseline gap = {:.0}pt; {} descriptions clip",
                        at.y - anchor.y - anchor.h,
                        clipped.len()
                    );
                } else {
                    ensure!(attached, "menu {at:?} detached from {anchor:?}");
                    ensure!(clipped.is_empty(), "clipped descriptions: {clipped:?}");
                    println!("{name}: anchor attached, no clipped descriptions");
                }
                let bitmap = image::load_from_memory(&png)?.into_rgba8();
                let scale = panel.width() as f32 / owner.w;
                let dims = ((at.w * scale).round() as u32, (at.h * scale).round() as u32);
                let bitmap = if bitmap.dimensions() == dims {
                    bitmap
                } else {
                    resize(&bitmap, dims.0, dims.1, FilterType::Lanczos3)
                };
                overlay(
                    &mut panel,
                    &bitmap,
                    (at.x * scale).round() as i64,
                    ((owner.h - at.y - at.h) * scale).round() as i64,
                );
                panel.save(captures.join(format!("{name}-menu.png")))?;
                Ok(())
            })?;
        }
    }
    Ok(())
}
