use super::{AgentStatus, COEDIT_LIGHT_SIZE, COEDIT_TITLE_WIDTH};

// The actual unbordered AppKit button has a 2 pt gap after its image.
// Native code reads titleRectForBounds from the cell on the main thread.
const TITLE_INSET: f64 = COEDIT_LIGHT_SIZE + 2.;
const TITLE_BUDGET: f64 = COEDIT_TITLE_WIDTH - TITLE_INSET;

#[test]
fn pm546_chip_fits_whole_words_and_count_at_the_native_width() {
    use super::*;

    let font = NSFont::systemFontOfSize(13.);
    let mut status = AgentStatus {
        working: true,
        progress: Some("I’ll find the wavy dither effect in the library".into()),
        ..Default::default()
    };
    for (count, expected) in [
        (0, "• I’ll find the wavy dither…"),
        (1, "• I’ll find the wavy dither…  +1"),
    ] {
        status.additional_runs = count;
        let title = coedit_presence_title(&status, &font, TITLE_BUDGET);
        assert_eq!(title.string().to_string(), expected);
        assert!(title.size().width <= TITLE_BUDGET);
    }
}

/// Toolbar contents drawn with AppKit at their native dimensions, without a
/// window or the system's compositor-owned glass.
#[test]
#[ignore = "writes PNGs; set PM_COEDIT_CHIP_CAPTURE"]
fn pm546_capture_coedit_chip_without_windows() {
    use super::*;
    use crate::agent_launcher::Tone;
    use objc2_foundation::NSDictionary;

    let dir = std::path::PathBuf::from(
        std::env::var("PM_COEDIT_CHIP_CAPTURE").expect("capture directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
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
                progress: Some("I’ll find the wavy dither effect in the library".into()),
                ..Default::default()
            };
            for (state, status, hover) in [
                ("running", running.clone(), false),
                ("hover", running.clone(), true),
                ("done-undo", AgentStatus::default(), false),
                (
                    "two-runs",
                    AgentStatus {
                        progress: Some("Checking the newest edit in the timeline".into()),
                        additional_runs: 1,
                        ..running
                    },
                    false,
                ),
            ] {
                let tokens = tokens.clone();
                let draw = block2::RcBlock::new(move |rect: NSRect| -> objc2::runtime::Bool {
                    native_color(tokens.colors.panel).setFill();
                    NSRectFill(rect);
                    let chip = NSRect::new(
                        NSPoint::new(8., 8.),
                        NSSize::new(COEDIT_CHIP_WIDTH, COEDIT_CHIP_HEIGHT),
                    );
                    native_color(tokens.colors.field).setFill();
                    let radius = tokens.metrics.capsule_radius as f64;
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(chip, radius, radius)
                        .fill();
                    let font = NSFont::systemFontOfSize(13.);
                    let title = coedit_presence_title(&status, &font, TITLE_BUDGET);
                    let text_y = chip.origin.y + (COEDIT_CHIP_HEIGHT - title.size().height) / 2.;
                    title.drawAtPoint(NSPoint::new(
                        chip.origin.x + 4. + TITLE_INSET + (TITLE_BUDGET - title.size().width) / 2.,
                        text_y,
                    ));
                    presence_light(&status, 0., &tokens).drawInRect(NSRect::new(
                        NSPoint::new(
                            chip.origin.x + 4.,
                            chip.origin.y + (COEDIT_CHIP_HEIGHT - COEDIT_LIGHT_SIZE) / 2.,
                        ),
                        NSSize::new(COEDIT_LIGHT_SIZE, COEDIT_LIGHT_SIZE),
                    ));
                    if hover {
                        let color = native_color(tokens.colors.text);
                        let attrs = NSDictionary::from_slices(
                            &[unsafe { NSFontAttributeName }, unsafe {
                                NSForegroundColorAttributeName
                            }],
                            &[&*font as &AnyObject, &*color as &AnyObject],
                        );
                        for (glyph, x) in [("Ⅱ", 200.), ("■", 224.)] {
                            let title = NSString::from_str(glyph);
                            unsafe {
                                let width = title.sizeWithAttributes(Some(&attrs)).width;
                                title.drawAtPoint_withAttributes(
                                    NSPoint::new(chip.origin.x + x + (22. - width) / 2., text_y),
                                    Some(&attrs),
                                );
                            }
                        }
                    }
                    launcher_native::image(Tone::Idle, &tokens)
                        .drawInRect(NSRect::new(NSPoint::new(269., 8.), NSSize::new(24., 24.)));
                    objc2::runtime::Bool::YES
                });
                let image = NSImage::imageWithSize_flipped_drawingHandler(
                    NSSize::new(302., 40.),
                    false,
                    &draw,
                );
                let data = image.TIFFRepresentation().unwrap();
                let rep = NSBitmapImageRep::imageRepWithData(&data).unwrap();
                let png = unsafe {
                    rep.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }
                .unwrap();
                std::fs::write(dir.join(format!("after-{name}-{state}.png")), png.to_vec())
                    .unwrap();
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
