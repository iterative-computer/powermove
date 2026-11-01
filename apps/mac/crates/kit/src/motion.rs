//! Shared motion parameters. Pass the system Reduce Motion preference to optional transitions.
use std::time::Duration;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageDirection {
    Back,
    Sideways,
    Deeper,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageMotion {
    pub x: f32,
    pub duration: Duration,
}
pub fn page_fly(direction: PageDirection, reduced: bool) -> PageMotion {
    if reduced || direction == PageDirection::Sideways {
        PageMotion {
            x: 0.,
            duration: Duration::ZERO,
        }
    } else {
        PageMotion {
            x: if direction == PageDirection::Back {
                -24.
            } else {
                24.
            },
            duration: Duration::from_millis(200),
        }
    }
}
pub fn page_settle(reduced: bool) -> Duration {
    Duration::from_millis(if reduced { 0 } else { 180 })
}
/// Native controls apply actions immediately; legacy menu blink is intentionally zero ms.
pub const MENU_BLINK: Duration = Duration::ZERO;
#[derive(Clone, Debug, Default)]
pub struct NavGlide {
    pub selected: Option<usize>,
    pub hovered: Option<usize>,
}
impl NavGlide {
    pub fn active(&self) -> Option<usize> {
        self.hovered.or(self.selected)
    }
    pub fn leave(&mut self) {
        self.hovered = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reduce_motion() {
        assert_eq!(page_fly(PageDirection::Back, true).duration, Duration::ZERO);
        assert_eq!(page_fly(PageDirection::Back, false).x, -24.);
        assert_eq!(page_fly(PageDirection::Sideways, false).x, 0.);
        assert_eq!(page_settle(true), Duration::ZERO);
    }
    #[test]
    fn nav_settles_to_selection() {
        let mut n = NavGlide {
            selected: Some(2),
            hovered: Some(4),
        };
        assert_eq!(n.active(), Some(4));
        n.leave();
        assert_eq!(n.active(), Some(2));
    }
}
