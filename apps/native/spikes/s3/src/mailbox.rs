//! Latest tick + seek-priority wake. Never retain an old tick while dropping new
//! ones, and never make input wait for a display-link callback.
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    pub sequence: u64,
    pub timestamp: f64,
    pub target: f64,
    pub callback: f64,
}
#[derive(Default)]
struct State {
    tick: Option<Tick>,
    seek: bool,
}
#[derive(Clone, Default)]
pub struct Clock(Arc<(Mutex<State>, Condvar)>);
pub enum Event {
    Tick(Tick),
    Seek,
    Timeout,
}
impl Clock {
    pub fn tick(&self, tick: Tick) {
        let mut s = self.0.0.lock().unwrap();
        s.tick = Some(tick);
        self.0.1.notify_one();
    }
    pub fn seek(&self) {
        let mut s = self.0.0.lock().unwrap();
        s.seek = true;
        self.0.1.notify_one();
    }
    pub fn next(&self) -> Event {
        self.next_for(Duration::from_millis(100))
    }
    pub fn next_for(&self, timeout: Duration) -> Event {
        let s = self.0.0.lock().unwrap();
        let (mut s, _) = self
            .0
            .1
            .wait_timeout_while(s, timeout, |s| !s.seek && s.tick.is_none())
            .unwrap();
        if s.seek {
            s.seek = false;
            s.tick = None;
            Event::Seek
        } else if let Some(t) = s.tick.take() {
            Event::Tick(t)
        } else {
            Event::Timeout
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn newest_tick_survives_and_seek_has_priority() {
        let c = Clock::default();
        for sequence in 0..100 {
            c.tick(Tick {
                sequence,
                timestamp: 0.,
                target: 0.,
                callback: 0.,
            });
        }
        assert!(matches!(c.next(), Event::Tick(Tick { sequence: 99, .. })));
        c.tick(Tick {
            sequence: 100,
            timestamp: 0.,
            target: 0.,
            callback: 0.,
        });
        c.seek();
        assert!(matches!(c.next(), Event::Seek));
        // The pending pre-seek playback tick was retired with the discontinuity.
        assert!(c.0.0.lock().unwrap().tick.is_none());
    }
    #[test]
    fn seek_wakes_a_waiting_worker_without_a_display_tick() {
        let c = Clock::default();
        let wake = c.clone();
        let t = std::thread::spawn(move || c.next());
        wake.seek();
        assert!(matches!(t.join().unwrap(), Event::Seek));
    }
}
