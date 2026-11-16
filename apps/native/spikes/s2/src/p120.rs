//! P-120 contract: geometry and AppKit synchronous paints remain transactional.
//! Compositor-paced stable frames use a supplied drawable without main-thread waits.
#[cfg(test)]
mod tests {
    use gpui_macos::composition::{SlotChange, probe};
    use std::{cell::Cell, rc::Rc};

    fn host(key: usize) -> (probe::Registration, Rc<Cell<SlotChange>>, Rc<Cell<u32>>) {
        let next = Rc::new(Cell::new(SlotChange::Unchanged));
        let calls = Rc::new(Cell::new(0));
        let (n, c) = (next.clone(), calls.clone());
        let reg = probe::register(
            key,
            Rc::new(move || {
                c.set(c.get() + 1);
                n.get()
            }),
        );
        (reg, next, calls)
    }

    #[test]
    fn no_host_no_transaction_unless_appkit_forces_it() {
        assert_eq!(probe::frame(0x10, false), None);
        assert!(!probe::unregistered_frame(false));
        assert!(probe::unregistered_frame(true));
        // No composition host: upstream behaviour even for continuous frames.
        assert!(!probe::unregistered_frame(false));
    }

    #[test]
    fn transaction_only_when_geometry_moved_then_resets() {
        let (_reg, next, calls) = host(0x20);
        // First publication places every slot, even if the host says unchanged.
        assert_eq!(probe::frame(0x20, false), Some(true));
        // Idle and GPUI-only frames present asynchronously.
        assert_eq!(probe::frame(0x20, false), Some(false));
        assert_eq!(probe::frame(0x20, false), Some(false));
        next.set(SlotChange::Moved);
        assert_eq!(probe::frame(0x20, false), Some(true));
        assert_eq!(probe::frame(0x20, false), Some(true), "still moving");
        // The very next unchanged frame goes back to the async present.
        next.set(SlotChange::Unchanged);
        assert_eq!(probe::frame(0x20, false), Some(false));
        assert_eq!(calls.get(), 6, "host runs on every frame, moved or not");
    }

    #[test]
    fn re_registration_is_fresh_and_drop_unregisters() {
        let (reg, next, calls) = host(0x30);
        assert_eq!(probe::frame(0x30, false), Some(true));
        assert_eq!(probe::frame(0x30, false), Some(false));
        drop(reg);
        assert_eq!(probe::frame(0x30, false), None);
        let (_reg, _next, _calls) = host(0x30);
        assert_eq!(probe::frame(0x30, false), Some(true));
        let _ = (next, calls);
    }

    #[test]
    fn forced_synchronous_paint_wins_over_unchanged_geometry() {
        let (_reg, _next, _calls) = host(0x40);
        let _ = probe::frame(0x40, false);
        assert_eq!(probe::frame(0x40, false), Some(false));
        // Live resize (`displayLayer:`) and tab activation stay transactional.
        assert_eq!(probe::frame(0x40, true), Some(true));
        assert_eq!(probe::frame(0x40, false), Some(false));
    }

    #[test]
    fn untracked_hosts_keep_the_conservative_transaction_every_frame() {
        let _reg = probe::register_untracked(0x50, Rc::new(|| {}));
        for _ in 0..3 {
            assert_eq!(probe::frame(0x50, false), Some(true));
        }
    }

    #[test]
    fn continuous_frames_stay_transactional_isolated_frames_do_not() {
        let (_reg, _next, _calls) = host(0x60);
        let _ = probe::frame(0x60, false);
        // Back-to-back frames: async presents would starve the 3-drawable pool.
        assert_eq!(probe::frame_after(0x60, false, true), Some(true));
        // An isolated frame (hover, caret) presents without blocking.
        assert_eq!(probe::frame_after(0x60, false, false), Some(false));
        assert_eq!(
            gpui_macos::composition::CONTINUOUS_FRAME,
            std::time::Duration::from_micros(33_334)
        );
    }

    #[test]
    fn compositor_pacing_keeps_geometry_atomic_without_continuous_waits() {
        let (_reg, next, _) = host(0x70);
        assert_eq!(probe::paced_frame(0x70, false), Some(true), "fresh slots");
        for _ in 0..4 {
            assert_eq!(probe::paced_frame(0x70, false), Some(false));
        }
        next.set(SlotChange::Moved);
        assert_eq!(probe::paced_frame(0x70, false), Some(true));
        next.set(SlotChange::Unchanged);
        assert_eq!(probe::paced_frame(0x70, true), Some(true), "AppKit paint");
        assert_eq!(probe::paced_frame(0x70, false), Some(false), "resets");
    }

    #[test]
    fn compositor_pacing_does_not_make_untracked_geometry_async() {
        let _reg = probe::register_untracked(0x80, Rc::new(|| {}));
        for _ in 0..4 {
            assert_eq!(probe::paced_frame(0x80, false), Some(true));
        }
    }

    #[test]
    fn deadline_admission_reserves_half_a_refresh_and_rejects_stale_updates() {
        let budget = 1. / 240.;
        assert!(probe::has_frame_budget(10., 10. + 1. / 120., budget));
        assert!(!probe::has_frame_budget(10., 10. + budget, budget));
        assert!(!probe::has_frame_budget(10., 9., budget));
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!probe::has_frame_budget(invalid, 11., budget));
            assert!(!probe::has_frame_budget(10., invalid, budget));
            assert!(!probe::has_frame_budget(10., 11., invalid));
        }
        assert!(!probe::has_frame_budget(10., 11., 0.));
        assert!(!probe::has_frame_budget(10., 11., -budget));
    }

    #[test]
    fn slot_change_combines() {
        use SlotChange::*;
        assert_eq!(Unchanged.or(Unchanged), Unchanged);
        assert_eq!(Unchanged.or(Moved), Moved);
        assert_eq!(Moved.or(Unchanged), Moved);
        assert_eq!(SlotChange::from(true), Moved);
        assert!(!SlotChange::from(false).moved());
    }
}
