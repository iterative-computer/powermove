use super::AgentStatus;
use objc2_app_kit::NSAttributedStringNSStringDrawing;

#[test]
fn pm546_chip_hugs_content_and_only_truncates_at_rest_when_capped() {
    use super::{Accessibility, Scheme, Tokens, coedit_presence};

    let tokens = Tokens::new(Scheme::Light, Accessibility::default());
    let mut status = AgentStatus {
        working: true,
        ..Default::default()
    };
    assert_eq!(coedit_presence::layout(&status, &tokens, false).width, 120.);
    let phrase = "Checking the newest edit in the timeline";
    status.progress = Some(phrase.into());
    let snug = coedit_presence::layout(&status, &tokens, false);
    assert_eq!(snug.title.string().to_string(), phrase);
    assert!(snug.width > 120. && snug.width < 300.);
    assert_eq!(
        snug.width - snug.text_x - snug.title.size().width.ceil(),
        8.
    );

    status.progress = Some(format!("{phrase} and checking the result in the canvas"));
    let capped = coedit_presence::layout(&status, &tokens, false);
    assert_eq!(capped.width, 300.);
    let text = capped.title.string().to_string();
    let prefix = text.strip_suffix('…').expect("ellipsis");
    assert!(status.progress.as_ref().unwrap()[prefix.len()..].starts_with(' '));
}

#[test]
fn pm546_hover_refits_in_place_with_badge_clear_of_actions() {
    use super::{Accessibility, Scheme, Tokens, coedit_presence};

    let tokens = Tokens::new(Scheme::Light, Accessibility::default());
    let status = AgentStatus {
        working: true,
        additional_runs: 1,
        progress: Some("Checking the newest edit in the timeline".into()),
        ..Default::default()
    };
    let normal = coedit_presence::layout(&status, &tokens, false);
    let hover = coedit_presence::layout(&status, &tokens, true);
    assert_eq!(normal.width, hover.width);
    assert!(normal.actions.is_none());
    assert!(hover.title.string().to_string().ends_with('…'));
    let (badge, count) = hover.badge.unwrap();
    assert_eq!(count.string().to_string(), "+1");
    assert_eq!(badge.size.width, count.size().width.ceil() + 8.);
    let actions = hover.actions.unwrap();
    assert!(badge.origin.x + badge.size.width < actions.origin.x);
    assert_eq!(actions.origin.x + actions.size.width, hover.width - 8.);

    let short = AgentStatus {
        progress: Some("Checking".into()),
        ..status
    };
    let hovered_short = coedit_presence::layout(&short, &tokens, true);
    let (badge, _) = hovered_short.badge.unwrap();
    assert!(badge.origin.x + badge.size.width < hovered_short.actions.unwrap().origin.x);
}

#[test]
fn pm546_done_keeps_the_compact_undo_next_to_the_label() {
    use super::{Accessibility, Scheme, Tokens, coedit_presence};

    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let done = coedit_presence::layout(&AgentStatus::default(), &tokens, true);
    assert_eq!(done.title.string().to_string(), "Done");
    assert!(done.actions.is_none());
    let (button, undo) = done.undo.unwrap();
    assert_eq!(undo.string().to_string(), "Undo");
    assert_eq!(
        button.origin.x - done.text_x - done.title.size().width.ceil(),
        10.
    );
    assert_eq!(button.size.width - undo.size().width.ceil(), 10.);
    assert!((120. ..=124.).contains(&done.width));
}

/// Compositor-independent 2x foregrounds, for placing onto real toolbar chrome.
#[test]
#[ignore = "writes PNGs; set PM_COEDIT_CHIP_CAPTURE"]
fn pm546_capture_coedit_chip_without_windows() {
    use super::*;
    use crate::agent_launcher::Tone;
    use objc2_foundation::{NSAffineTransform, NSDictionary};

    let dir = std::path::PathBuf::from(
        std::env::var("PM_COEDIT_CHIP_CAPTURE").expect("capture directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    for (mode, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        let appearance = NSAppearance::appearanceNamed(unsafe {
            if scheme == Scheme::Light {
                NSAppearanceNameAqua
            } else {
                NSAppearanceNameDarkAqua
            }
        })
        .unwrap();
        let draw = block2::StackBlock::new(|| {
            let tokens = Tokens::new(scheme, Accessibility::default());
            let running = AgentStatus {
                working: true,
                progress: Some("I’ll find the wavy dither effect in the library and check how the composition uses it.".into()),
                ..Default::default()
            };
            for (state, status, hover, undo_hot) in [
                ("idle", None, false, false),
                ("running", Some(running.clone()), false, false),
                ("hover", Some(running.clone()), true, false),
                (
                    "paused",
                    Some(AgentStatus {
                        paused: true,
                        ..running.clone()
                    }),
                    false,
                    false,
                ),
                (
                    "waiting",
                    Some(AgentStatus {
                        held: true,
                        ..running.clone()
                    }),
                    false,
                    false,
                ),
                ("done-undo", Some(AgentStatus::default()), false, false),
                ("undo-hover", Some(AgentStatus::default()), false, true),
                (
                    "two-runs",
                    Some(AgentStatus {
                        progress: Some("Checking the newest timeline edit".into()),
                        additional_runs: 1,
                        ..running
                    }),
                    false,
                    false,
                ),
            ] {
                let t = tokens.clone();
                let draw = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
                    let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
                    let bezel = |r: NSRect, color| {
                        let radius = r.size.height / 2.;
                        native_color(color).setFill();
                        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, radius, radius)
                            .fill();
                    };
                    NSGraphicsContext::saveGraphicsState_class();
                    let scale = NSAffineTransform::transform();
                    scale.scaleBy(2.);
                    scale.concat();
                    if let Some(status) = &status {
                        let chip = coedit_presence::layout(status, &t, hover);
                        NSGraphicsContext::saveGraphicsState_class();
                        let offset = NSAffineTransform::transform();
                        offset.translateXBy_yBy(300. - chip.width, 20.);
                        offset.concat();
                        chip.draw_fixture(status, &t, undo_hot);
                        NSGraphicsContext::restoreGraphicsState_class();
                    }
                    bezel(rect(308., 24., 32., 28.), t.colors.field);
                    launcher_native::image(Tone::Idle, &t).drawInRect(rect(312., 26., 24., 24.));
                    bezel(
                        rect(348., 20., 86., coedit_presence::HEIGHT),
                        t.colors.accent,
                    );
                    let export = toolbar_symbol("square.and.arrow.up", "Export", &t);
                    let draw = block2::RcBlock::new(move |r: NSRect| -> objc2::runtime::Bool {
                        export.drawInRect(r);
                        NSColor::whiteColor().setFill();
                        NSRectFillUsingOperation(r, NSCompositingOperation::SourceAtop);
                        objc2::runtime::Bool::YES
                    });
                    NSImage::imageWithSize_flipped_drawingHandler(
                        NSSize::new(16., 18.),
                        false,
                        &draw,
                    )
                    .drawInRect(rect(358., 29., 16., 18.));
                    let label = coedit_presence::title("Export", 13., &NSColor::whiteColor());
                    label.drawAtPoint(NSPoint::new(379., 38. - label.size().height / 2.));
                    NSGraphicsContext::restoreGraphicsState_class();
                    objc2::runtime::Bool::YES
                });
                let image = NSImage::imageWithSize_flipped_drawingHandler(
                    NSSize::new(880., 128.),
                    false,
                    &draw,
                );
                let data = image.TIFFRepresentation().unwrap();
                let rep = NSBitmapImageRep::imageRepWithData(&data).unwrap();
                assert_eq!((rep.pixelsWide(), rep.pixelsHigh()), (880, 128));
                let png = unsafe {
                    rep.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }
                .unwrap();
                std::fs::write(dir.join(format!("{mode}-{state}.png")), png.to_vec()).unwrap();
            }
        });
        appearance.performAsCurrentDrawingAppearance(&draw);
    }
}

/// Glyph only, system bezel not shown. No NSApplication or NSWindow.
#[test]
#[ignore = "writes glyph PNGs; set PM_AGENT_GLYPH_CAPTURE"]
fn capture_agent_toolbar_glyphs_without_windows() {
    use super::*;
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext,
    };
    use objc2_foundation::NSDictionary;
    let dir = std::path::PathBuf::from(
        std::env::var("PM_AGENT_GLYPH_CAPTURE").expect("capture directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    for (name, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        objc2::rc::autoreleasepool(|_| {
            let appearance = NSAppearance::appearanceNamed(unsafe {
                if scheme == Scheme::Light {
                    NSAppearanceNameAqua
                } else {
                    NSAppearanceNameDarkAqua
                }
            })
            .unwrap();
            let draw = block2::StackBlock::new(|| {
                let tokens = Tokens::new(scheme, Default::default());
                for (state, status) in [
                    ("idle", AgentStatus::default()),
                    (
                        "working",
                        AgentStatus {
                            working: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "unread",
                        AgentStatus {
                            unread: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "error",
                        AgentStatus {
                            failed: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "attention",
                        AgentStatus {
                            attention: true,
                            ..Default::default()
                        },
                    ),
                ] {
                    let rep = unsafe {
                        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                                NSBitmapImageRep::alloc(), std::ptr::null_mut(), 46, 46, 8, 4, true, false,
                                NSDeviceRGBColorSpace, 46 * 4, 32,
                            ).unwrap()
                    };
                    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).unwrap();
                    NSGraphicsContext::saveGraphicsState_class();
                    NSGraphicsContext::setCurrentContext(Some(&ctx));
                    NSColor::clearColor().set();
                    NSRectFillUsingOperation(
                        NSRect::new(NSPoint::ZERO, NSSize::new(46., 46.)),
                        NSCompositingOperation::Copy,
                    );
                    agent_toolbar_image(&status, 0., &tokens)
                        .drawInRect(NSRect::new(NSPoint::ZERO, NSSize::new(46., 46.)));
                    ctx.flushGraphics();
                    NSGraphicsContext::restoreGraphicsState_class();
                    let png = unsafe {
                        rep.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    }
                    .unwrap();
                    std::fs::write(
                        dir.join(format!("agent-{state}-{name}-glyph-only.png")),
                        png.to_vec(),
                    )
                    .unwrap();
                }
            });
            appearance.performAsCurrentDrawingAppearance(&draw);
        });
    }
}
#[test]
fn hidden_run_notices_follow_attention_error_unread_priority() {
    let mut status = AgentStatus {
        unread: true,
        ..Default::default()
    };
    assert_eq!(status.tooltip(), "Agent finished · unread reply");
    status.failed = true;
    assert_eq!(status.tooltip(), "Agent run failed");
    status.attention = true;
    assert_eq!(status.tooltip(), "Agent needs your answer");
    status = AgentStatus::default();
    assert_eq!(status.tooltip(), "Ask the Agent");
}
#[test]
fn the_agent_item_says_what_the_agent_is_doing_and_attention_wins() {
    assert_eq!(AgentStatus::default().tooltip(), "Ask the Agent");
    let working = AgentStatus {
        working: true,
        elapsed: Some("1:04".into()),
        ..Default::default()
    };
    assert_eq!(working.tooltip(), "Agent is working · 1:04");
    let asking = AgentStatus {
        attention: true,
        ..working
    };
    assert_eq!(asking.tooltip(), "Agent needs your answer");
}
