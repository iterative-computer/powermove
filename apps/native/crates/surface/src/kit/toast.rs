//! Timing state shared by action toasts. Hovering pauses the remaining lifetime.
use std::time::{Duration, Instant};

pub struct ActionToast<A> {
    pub message: String,
    pub action: A,
    remaining: Duration,
    updated: Instant,
    hovered: bool,
}
impl<A> ActionToast<A> {
    pub fn new(message: String, action: A, now: Instant) -> Self {
        Self { message, action, remaining: Duration::from_secs(6), updated: now, hovered: false }
    }
    pub fn update(&mut self, now: Instant, hovered: bool) -> bool {
        if !self.hovered {
            self.remaining = self.remaining.saturating_sub(now.saturating_duration_since(self.updated));
        }
        self.updated = now;
        self.hovered = hovered;
        self.remaining.is_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expires_after_six_seconds() {
        let now = Instant::now();
        let mut toast = ActionToast::new("Deleted Clip".into(), (), now);
        assert!(!toast.update(now + Duration::from_secs(5), false));
        assert!(toast.update(now + Duration::from_secs(6), false));
    }
    #[test]
    fn hover_preserves_remaining_time() {
        let now = Instant::now();
        let mut toast = ActionToast::new("Deleted Clip".into(), (), now);
        assert!(!toast.update(now + Duration::from_secs(2), true));
        assert!(!toast.update(now + Duration::from_secs(20), false));
        assert!(!toast.update(now + Duration::from_secs(23), false));
        assert!(toast.update(now + Duration::from_secs(24), false));
    }
}
