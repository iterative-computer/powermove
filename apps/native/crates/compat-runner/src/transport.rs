//! Worker-owned transport for a disposable native project. No JS shadow clock.
use pm_runtime_v8::{Error, Result};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Transport {
    time: f64,
    playing: bool,
    looping: bool,
    origin: f64,
    base: f64,
}
impl Default for Transport {
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
impl Transport {
    pub fn command(&mut self, input: &Value, document: &Value) -> Result<Value> {
        let mut next = self.clone();
        let result = next.apply(input, document)?;
        *self = next;
        Ok(result)
    }
    fn apply(&mut self, input: &Value, document: &Value) -> Result<Value> {
        let old_time = self.time;
        let old_playing = self.playing;
        let fps = document["fps"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.)
            .ok_or(Error::Invalid("transport frame rate"))?;
        let duration = document["dur"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or(Error::Invalid("transport duration"))?;
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
                self.time = if input["raw"] == true {
                    time
                } else {
                    (time * fps).round() / fps
                };
                if self.playing {
                    self.base = self.time;
                    self.origin = now()?;
                }
            }
            Some("play") if !self.playing => {
                // Audio/video decoding is a separate native service. Refuse
                // media playback until that service is attached to this runner.
                if document["assets"].as_array().is_some_and(|v| !v.is_empty()) {
                    return Err(Error::Host("native media transport is not attached".into()));
                }
                self.origin = now()?;
                self.base = self.time;
                self.playing = true;
            }
            Some("play") => (),
            Some("pause") => {
                if self.playing {
                    self.time = (self.time * fps + 1e-7).floor() / fps;
                }
                self.playing = false;
            }
            Some("loop") => {
                self.looping = input["value"]
                    .as_bool()
                    .ok_or(Error::Invalid("transport loop"))?
            }
            Some("tick") if self.playing => {
                let elapsed = ((now()? - self.origin) / 1000.).max(0.);
                self.time = self.base + elapsed;
                let (start, end) = match document["work"].as_array().map(Vec::as_slice) {
                    Some([a, b]) if b.as_f64() > a.as_f64() => {
                        (a.as_f64().unwrap_or(0.), b.as_f64().unwrap_or(duration))
                    }
                    _ => (0., duration),
                };
                if self.time >= end - 1e-6 {
                    if !self.looping || end <= start {
                        self.time = end;
                        self.playing = false;
                    } else {
                        self.time = start + (self.time - start).rem_euclid(end - start);
                    }
                }
            }
            Some("tick") => (),
            _ => return Err(Error::Invalid("transport method")),
        }
        Ok(
            json!({"time":self.time,"playing":self.playing,"loop":self.looping,
            "timeChanged":old_time!=self.time,"playingChanged":old_playing!=self.playing}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_clock_loops_work_range_and_pauses_on_the_displayed_frame() {
        let mut clock = Transport::default();
        let doc = json!({"fps":30,"dur":2,"work":[0.5,1.5],"assets":[]});
        clock
            .command(&json!({"method":"seek","time":1}), &doc)
            .unwrap();
        clock
            .command(&json!({"method":"play","now":1000}), &doc)
            .unwrap();
        assert_eq!(
            clock
                .command(&json!({"method":"tick","now":1750}), &doc)
                .unwrap()["time"],
            0.75
        );
        clock
            .command(&json!({"method":"loop","value":false}), &doc)
            .unwrap();
        assert_eq!(
            clock
                .command(&json!({"method":"tick","now":2000}), &doc)
                .unwrap()["playing"],
            false
        );
    }
    #[test]
    fn invalid_input_and_unattached_media_never_start_playback() {
        let mut clock = Transport::default();
        let doc = json!({"fps":30,"dur":2,"assets":[{}]});
        assert!(
            clock
                .command(&json!({"method":"play","now":1000}), &doc)
                .is_err()
        );
        assert!(
            clock
                .command(&json!({"method":"seek","time":null}), &doc)
                .is_err()
        );
        assert_eq!(
            clock.command(&json!({"method":"state"}), &doc).unwrap()["playing"],
            false
        );
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn invalid_seek_clock_preserves_the_running_transport() {
        let mut clock = Transport::default();
        let doc = json!({"fps":30,"dur":2,"assets":[]});
        clock
            .command(&json!({"method":"play","now":1000}), &doc)
            .unwrap();
        let before = clock.command(&json!({"method":"state"}), &doc).unwrap();
        assert!(
            clock
                .command(&json!({"method":"seek","time":1,"now":null}), &doc)
                .is_err()
        );
        let after = clock.command(&json!({"method":"state"}), &doc).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            clock
                .command(&json!({"method":"tick","now":1250}), &doc)
                .unwrap()["time"],
            0.25
        );
    }
}
