//! The first-run stage drawn offscreen with the real engine (no window on
//! screen). Test threads have no font registry, so pictures with text are
//! left to `--stage-capture`, which draws the whole film with the app's fonts.
use crate::stage_capture::*;
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::stage::CueSheet;

/// The engine draws the room, the stage paints it, and the first frame is
/// the clear veil (never white).
#[::core::prelude::v1::test]
fn the_engine_draws_every_scene_on_the_stage() {
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens, false, true);
    let first = frame_at(&mut cx, window, 0.);
    assert!(
        first.pixels().all(|p| p[3] == 0 || p[0] < 8),
        "the first frame is never light"
    );
    let sheet = CueSheet::default();
    // The room's light is up for the card.
    let card = frame_at(&mut cx, window, sheet.length() + 1.);
    assert!(mean(&card) > 0.01, "the card's room is lit");
}

fn mean(image: &image::RgbaImage) -> f32 {
    let mut sum = 0.;
    for p in image.pixels() {
        sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
    }
    sum / (image.width() * image.height() * 3) as f32 / 255.
}
