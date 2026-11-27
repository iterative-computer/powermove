//! Host-monotonic-clock deadlines. The main thread schedules effects without JS.
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct OverlayVisibility {
    idle: Duration,
    last_activity: Duration,
}
impl OverlayVisibility {
    pub fn new(table: &super::DesignTokenTable, now: Duration) -> Result<Self, String> {
        let idle = table
            .metric("motion.overlay_idle")
            .ok_or("missing motion.overlay_idle")?;
        let idle = Duration::try_from_secs_f64(idle).map_err(|_| "invalid motion.overlay_idle")?;
        Ok(Self {
            idle,
            last_activity: now,
        })
    }
    pub fn activity(&mut self, now: Duration) {
        self.last_activity = now;
    }
    pub fn visible(
        &mut self,
        now: Duration,
        hovered: bool,
        menu_open: bool,
        focus_within: bool,
    ) -> bool {
        if hovered || menu_open || focus_within {
            self.activity(now);
            return true;
        }
        now.saturating_sub(self.last_activity) < self.idle
    }
}
/// One viewer banner slot. A replacement restarts its deadline; an action pins it.
#[derive(Clone, Debug, Default)]
pub struct BannerSlot {
    pub id: Option<crate::Id>,
    expires: Option<Duration>,
}
impl BannerSlot {
    pub fn show(&mut self, id: crate::Id, now: Duration, has_action: bool) {
        self.id = Some(id);
        self.expires = if has_action {
            None
        } else {
            now.checked_add(Duration::from_secs(4))
        };
    }
    pub fn tick(&mut self, now: Duration) -> Option<crate::Id> {
        if self.expires.is_some_and(|end| now >= end) {
            return self.dismiss();
        }
        None
    }
    pub fn dismiss(&mut self) -> Option<crate::Id> {
        self.expires = None;
        self.id.take()
    }
}
