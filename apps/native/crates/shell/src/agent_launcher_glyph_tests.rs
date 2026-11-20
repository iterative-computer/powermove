use super::*;
use objc2::AnyThread;
use objc2_app_kit::{NSAffineTransformNSAppKitAdditions, NSDeviceRGBColorSpace};
use objc2_foundation::{NSAffineTransform, NSDictionary};

const SIDE: f64 = 24.;

fn raster(image: &NSImage, scale: u32) -> image::RgbaImage {
    let side = SIDE as u32 * scale;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), side as isize, side as isize,
            8, 4, true, false, NSDeviceRGBColorSpace, (side * 4) as isize, 32,
        )
    }
    .unwrap();
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).unwrap();
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let transform = NSAffineTransform::transform();
    transform.scaleBy(scale.into());
    transform.concat();
    NSColor::clearColor().setFill();
    NSRectFillUsingOperation(
        NSRect::new(NSPoint::ZERO, NSSize::new(SIDE, SIDE)),
        NSCompositingOperation::Copy,
    );
    let size = image.size();
    image.drawInRect(NSRect::new(
        NSPoint::new((SIDE - size.width) / 2., (SIDE - size.height) / 2.),
        size,
    ));
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .unwrap();
    image::load_from_memory(&png.to_vec()).unwrap().to_rgba8()
}

fn ink_bounds(image: &image::RgbaImage) -> (u32, u32) {
    let (mut left, mut top, mut right, mut bottom) = (image.width(), image.height(), 0, 0);
    for (x, y, _) in image.enumerate_pixels().filter(|(_, _, p)| p[3] >= 128) {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    assert!(
        left <= right && top <= bottom,
        "toolbar glyph has no visible ink"
    );
    (right - left + 1, bottom - top + 1)
}

#[test]
fn launcher_paints_toolbar_sized_sparkles() {
    let output = std::env::var_os("PM_AGENT_ICON_CAPTURE").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir).unwrap();
    }
    let idle = Launcher::default();
    let mut working = idle.clone();
    working.update(
        Snapshot {
            tasks: vec![crate::agent_launcher::Task {
                id: "run".into(),
                running: true,
                ..Default::default()
            }],
            ..Default::default()
        },
        1000,
    );
    let mut ended = working.clone();
    ended.update(Snapshot::default(), 2000);

    let mut measurements = Vec::new();
    for (theme, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        let t = Tokens::new(scheme, Accessibility::default());
        let reference = toolbar_symbol("sparkles", "Agent", &t);
        for (state, model, presence) in [
            ("idle", &idle, false),
            ("running", &working, true),
            ("finished", &ended, true),
            // Both terminal outcomes drop the run from the launcher snapshot.
            ("failed", &ended, false),
        ] {
            let glyph = image(model.toolbar_tone(presence), &t);
            // Reuse the same image as a window would when it moves between displays.
            for (pass, scale) in [1, 2, 1].into_iter().enumerate() {
                let painted = raster(&glyph, scale);
                let expected = ink_bounds(&raster(&reference, scale));
                let actual = ink_bounds(&painted);
                if let Some(dir) = &output {
                    painted
                        .save(dir.join(format!("{theme}-{state}-{scale}x.png")))
                        .unwrap();
                    if state == "idle" {
                        for (name, symbol) in [
                            ("sparkles", "sparkles"),
                            ("grid", "square.grid.2x2"),
                            ("gear", "gearshape"),
                        ] {
                            raster(&toolbar_symbol(symbol, name, &t), scale)
                                .save(dir.join(format!("{theme}-{name}-{scale}x.png")))
                                .unwrap();
                        }
                    }
                }
                measurements.push((theme, state, pass, scale, actual, expected));
            }
        }
    }
    for (theme, state, pass, scale, actual, expected) in measurements {
        assert!(
            actual.0.abs_diff(expected.0) <= 1 && actual.1.abs_diff(expected.1) <= 1,
            "{theme} {state} at {scale}x (pass {pass}): painted {actual:?} px, toolbar reference {expected:?} px"
        );
    }
}
