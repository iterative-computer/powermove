use gpui::Window;
use pm_design::{
    Tokens,
    motion::{Animated, Clock, Spring},
};
use std::time::Instant;

/// v1 page-motion.ts uses a 24 pt, 200 ms cubic-out. Retain its travel and
/// timing with a non-bouncing spring so a new destination can take over.
const PAGE_TRAVEL: f32 = 24.;
const PAGE_SECONDS: f64 = 0.2;
const SETTLE_SECONDS: f64 = 0.18;
const SHIMMER_SECONDS: f32 = 1.6;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Skeleton {
    Discover,
    Catalogue,
    Library,
    Detail,
}

pub(super) struct StoreMotion {
    page: Animated<2>,
    fade: Animated,
    placeholder: Option<Skeleton>,
    phase: f32,
    clock: Clock,
    pub title: pm_shell::motion::RollingLabel,
}

impl Default for StoreMotion {
    fn default() -> Self {
        Self {
            page: Animated::new([0., 1.], Spring::SNAP, 0.005),
            fade: Animated::unit(1.),
            placeholder: None,
            phase: 0.,
            clock: Clock::default(),
            title: Default::default(),
        }
    }
}

impl StoreMotion {
    pub fn navigate(&mut self, direction: f32, reduced: bool) {
        self.placeholder = None;
        self.fade.snap([1.]);
        if reduced || direction == 0. {
            self.page.snap([0., 1.]);
        } else {
            let spring = Spring::settling(PAGE_SECONDS);
            if !self.page.moving() {
                let distance = PAGE_TRAVEL * direction;
                self.page.snap([distance, 0.]);
                // Start with the velocity of v1's cubic-out.
                self.page.velocity = [-distance * 15., 15.];
            }
            self.page.animate_to([0., 1.], spring);
        }
    }

    pub fn frame(&mut self, loading: Option<Skeleton>, t: &Tokens, window: &mut Window) {
        let dt = self.clock.tick(Instant::now());
        let reduced = t.accessibility.reduce_motion;
        if reduced {
            self.page.snap([0., 1.]);
        }
        let mut moving = self.page.step(dt);
        if let Some(kind) = loading {
            if self.placeholder != Some(kind) {
                self.phase = 0.;
            }
            self.placeholder = Some(kind);
            self.fade.snap([0.]);
            if !reduced {
                self.phase = (self.phase + dt / SHIMMER_SECONDS).fract();
                moving = true;
            }
        } else if self.placeholder.is_some() {
            if self.fade.target[0] == 0. {
                let duration = if reduced {
                    t.motion.reduced
                } else {
                    SETTLE_SECONDS
                };
                self.fade.to(1., Spring::settling(duration));
            }
            moving |= self.fade.step(dt);
            if !self.fade.moving() {
                self.placeholder = None;
            }
        }
        if moving {
            window.request_animation_frame();
        } else {
            self.clock.rest();
        }
    }

    pub fn page(&self) -> (f32, f32) {
        (self.page.value[0], self.page.value[1].clamp(0., 1.))
    }

    pub fn fade(&self) -> f32 {
        self.fade.get().clamp(0., 1.)
    }

    pub fn placeholder(&self) -> Option<Skeleton> {
        self.placeholder
    }

    pub fn phase(&self) -> f32 {
        self.phase
    }
}
