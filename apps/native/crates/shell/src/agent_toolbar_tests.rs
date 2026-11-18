use super::AgentStatus;
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
