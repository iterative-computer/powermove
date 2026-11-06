//! The realm's view of playback. `PM.time` is a lock and a copy, never a
//! document read; the app's clock owner pushes time here and receives requests.
use pm_kernel::Published;
use pm_runtime_v8::{Error, Result};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Timing facts of the open composition, read from the published tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeBase {
    pub fps: f64,
    pub duration: f64,
    pub work: Option<(f64, f64)>,
}
impl TimeBase {
    pub fn of(published: &Published) -> Self {
        let root = published.snapshot.root();
        let open = published.snapshot.open_comp();
        let comp = root.and_then(|r| r.get("comps")).and_then(|c| match &open {
            Some(id) => c.get(id),
            None => None,
        });
        let field = |key: &str| {
            comp.and_then(|c| c.get(key))
                .or(root.and_then(|r| r.get(key)))
        };
        let number = |key: &str| field(key).and_then(|n| n.to_value().as_f64());
        let work = field("work").map(|w| w.to_value()).and_then(|w| {
            let w = w.as_array()?;
            Some((w.first()?.as_f64()?, w.get(1)?.as_f64()?))
        });
        Self {
            fps: number("fps")
                .filter(|v| v.is_finite() && *v > 0.)
                .unwrap_or(30.),
            duration: number("dur")
                .filter(|v| v.is_finite() && *v >= 0.)
                .unwrap_or(0.),
            work,
        }
    }
}

/// Playback a realm asked for. The app routes these to its one clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransportRequest {
    Seek(f64),
    Play,
    Pause,
}

pub trait TransportOwner: Send + Sync + 'static {
    fn request(&self, request: TransportRequest);
}

#[derive(Clone, Debug)]
struct Clock {
    time: f64,
    playing: bool,
    looping: bool,
    origin: f64,
    base: f64,
}
impl Default for Clock {
    fn default() -> Self {
        Self {
            time: 0.,
            playing: false,
            looping: true,
            origin: 0.,
            base: 0.,
        }
    }
}

/// With an owner, it mirrors the app's clock; without one, it runs its own.
pub struct Transport {
    clock: Mutex<Clock>,
    owner: Option<Arc<dyn TransportOwner>>,
}
impl Default for Transport {
    fn default() -> Self {
        Self::new(None)
    }
}
impl Transport {
    pub fn new(owner: Option<Arc<dyn TransportOwner>>) -> Self {
        Self {
            clock: Mutex::default(),
            owner,
        }
    }
    pub fn time(&self) -> f64 {
        self.clock.lock().unwrap().time
    }
    pub fn playing(&self) -> bool {
        self.clock.lock().unwrap().playing
    }
    /// The clock owner's latest position. Returns what changed for the realm.
    pub fn publish(&self, time: f64, playing: bool) -> (bool, bool) {
        let mut clock = self.clock.lock().unwrap();
        let moved = (clock.time != time, clock.playing != playing);
        clock.time = time;
        clock.playing = playing;
        moved
    }
    pub fn command(&self, input: &Value, base: TimeBase) -> Result<Value> {
        let mut clock = self.clock.lock().unwrap();
        let mut next = clock.clone();
        let result = Self::apply(&mut next, input, base)?;
        if let Some(owner) = &self.owner {
            match input["method"].as_str() {
                Some("seek") if clock.time != next.time => {
                    owner.request(TransportRequest::Seek(next.time))
                }
                Some("play") if !clock.playing => owner.request(TransportRequest::Play),
                Some("pause") if clock.playing => owner.request(TransportRequest::Pause),
                _ => (),
            }
        }
        *clock = next;
        Ok(result)
    }
    fn apply(clock: &mut Clock, input: &Value, base: TimeBase) -> Result<Value> {
        let old_time = clock.time;
        let old_playing = clock.playing;
        let TimeBase {
            fps,
            duration,
            work,
        } = base;
        let now = || {
            input["now"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.)
                .ok_or(Error::Invalid("transport clock"))
        };
        match input["method"].as_str() {
            Some("state") => (),
            Some("seek") => {
                let time = input["time"]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or(Error::Invalid("transport time"))?
                    .clamp(0., duration);
                clock.time = if input["raw"] == true {
                    time
                } else {
                    (time * fps).round() / fps
                };
                if clock.playing {
                    clock.base = clock.time;
                    clock.origin = now()?;
                }
            }
            Some("play") if !clock.playing => {
                clock.origin = now()?;
                clock.base = clock.time;
                clock.playing = true;
            }
            Some("play") => (),
            Some("pause") => {
                if clock.playing {
                    clock.time = (clock.time * fps + 1e-7).floor() / fps;
                }
                clock.playing = false;
            }
            Some("loop") => {
                clock.looping = input["value"]
                    .as_bool()
                    .ok_or(Error::Invalid("transport loop"))?
            }
            Some("tick") if clock.playing => {
                let elapsed = ((now()? - clock.origin) / 1000.).max(0.);
                clock.time = clock.base + elapsed;
                let (start, end) = match work {
                    Some((a, b)) if b > a => (a, b),
                    _ => (0., duration),
                };
                if clock.time >= end - 1e-6 {
                    if !clock.looping || end <= start {
                        clock.time = end;
                        clock.playing = false;
                    } else {
                        clock.time = start + (clock.time - start).rem_euclid(end - start);
                    }
                }
            }
            Some("tick") => (),
            _ => return Err(Error::Invalid("transport method")),
        }
        Ok(
            json!({"time":clock.time,"playing":clock.playing,"loop":clock.looping,
            "timeChanged":old_time!=clock.time,"playingChanged":old_playing!=clock.playing}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: TimeBase = TimeBase {
        fps: 30.,
        duration: 2.,
        work: Some((0.5, 1.5)),
    };
    #[test]
    fn native_clock_loops_work_range_and_pauses_on_the_displayed_frame() {
        let clock = Transport::default();
        clock
            .command(&json!({"method":"seek","time":1}), BASE)
            .unwrap();
        clock
            .command(&json!({"method":"play","now":1000}), BASE)
            .unwrap();
        assert_eq!(
            clock
                .command(&json!({"method":"tick","now":1750}), BASE)
                .unwrap()["time"],
            0.75
        );
        clock
            .command(&json!({"method":"loop","value":false}), BASE)
            .unwrap();
        assert_eq!(
            clock
                .command(&json!({"method":"tick","now":2000}), BASE)
                .unwrap()["playing"],
            false
        );
    }
    #[test]
    fn invalid_seek_clock_preserves_the_running_transport() {
        let clock = Transport::default();
        clock
            .command(&json!({"method":"play","now":1000}), BASE)
            .unwrap();
        let before = clock.command(&json!({"method":"state"}), BASE).unwrap();
        assert!(
            clock
                .command(&json!({"method":"seek","time":1,"now":null}), BASE)
                .is_err()
        );
        assert_eq!(
            clock.command(&json!({"method":"state"}), BASE).unwrap(),
            before
        );
        assert_eq!(
            clock
                .command(&json!({"method":"tick","now":1250}), BASE)
                .unwrap()["time"],
            0.25
        );
    }
    #[test]
    fn an_owned_clock_routes_requests_and_follows_published_time() {
        struct Owner(std::sync::Arc<Mutex<Vec<TransportRequest>>>);
        impl TransportOwner for Owner {
            fn request(&self, request: TransportRequest) {
                self.0.lock().unwrap().push(request);
            }
        }
        let requests = std::sync::Arc::default();
        let clock = Transport::new(Some(Arc::new(Owner(Arc::clone(&requests)))));
        clock
            .command(&json!({"method":"seek","time":0.51}), BASE)
            .unwrap();
        clock
            .command(&json!({"method":"play","now":1}), BASE)
            .unwrap();
        assert_eq!(
            *requests.lock().unwrap(),
            [TransportRequest::Seek(0.5), TransportRequest::Play]
        );
        assert_eq!(clock.publish(1., true), (true, false));
        assert_eq!(
            clock.command(&json!({"method":"state"}), BASE).unwrap()["time"],
            1.
        );
    }
}
