//! The first-run stage drawn offscreen with the real engine (no window on
//! screen). Test threads have no font registry, so pictures with text are
//! left to `--stage-capture`, which draws the whole film with the app's fonts.
use crate::stage_capture::*;
use gpui::{px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::stage::{CueSheet, Reel, Stage};

/// The engine draws the room and every reel, the stage paints them, and the
/// first frame is the clear veil (never white).
#[::core::prelude::v1::test]
fn the_engine_draws_every_scene_on_the_stage() {
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens);
    let first = frame_at(&mut cx, window, 0.);
    assert!(
        first.pixels().all(|p| p[3] == 0 || p[0] < 8),
        "the first frame is never light"
    );
    let sheet = CueSheet::default();
    // Inside each reel's screen, past its first beat: the engine's picture.
    // (Cut and Ask carry text, which needs the main thread's font registry:
    // `--stage-capture` draws them.)
    for reel in [Reel::Grade, Reel::Depth] {
        let t = sheet.start_of(pm_shell::stage::Scene::Reel(reel)).unwrap() + 1.8;
        let image = frame_at(&mut cx, window, t);
        let screen = Stage::screen(size(px(ROOM.0), px(ROOM.1)));
        let (cx_, cy_) = (
            f32::from(screen.center().x) / ROOM.0,
            f32::from(screen.center().y) / ROOM.1,
        );
        let lit = (0..9)
            .map(|i| luma(&image, cx_ - 0.15 + 0.0375 * i as f32, cy_))
            .fold(0f32, f32::max);
        assert!(
            lit > 0.08,
            "{reel:?} shows no picture on its screen ({lit})"
        );
    }
    // The room's light is up for the card.
    let card = frame_at(&mut cx, window, sheet.length() + 1.);
    assert!(mean(&card) > 0.01, "the card's room is lit");
}
