//! Tag engine.ts:280–346; shortcuts.ts:361–382. This is intent/state,
//! not a synthetic media clock. Hosts must bind engine/audio readiness.
use crate::{
    Error, finite,
    state::{boolean, number, put},
};
use pm_kernel::state::UiState;
use pm_model::Value;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub time: f64,
    pub playing: bool,
    pub fps: f64,
    pub duration: f64,
    pub work: (f64, f64),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransportAction {
    Toggle,
    Pause,
    Seek(f64),
    Step(i32),
    Start,
    End,
    WorkIn,
    WorkOut,
}
impl Transport {
    pub fn read(ui: &UiState, fps: f64, duration: f64, work: (f64, f64)) -> Result<Self, Error> {
        if finite(fps)? <= 0.0
            || finite(duration)? <= 0.0
            || finite(work.0)? < 0.0
            || finite(work.1)? > duration
            || work.0 >= work.1
        {
            return Err(Error::Invalid("composition timing"));
        }
        let time = number(&ui.transport, "time", 0.0)?;
        if time < 0.0 {
            return Err(Error::Invalid("time"));
        }
        Ok(Self {
            time,
            playing: boolean(&ui.transport, "playing", false)?,
            fps,
            duration,
            work,
        })
    }
    pub fn apply(self, action: TransportAction, ui: &mut UiState) -> Result<(), Error> {
        let mut time = self.time;
        let mut playing = self.playing;
        match action {
            TransportAction::Toggle if !playing => playing = true,
            TransportAction::Toggle | TransportAction::Pause => {
                if playing {
                    time = (time * self.fps + 1e-7).floor() / self.fps;
                }
                playing = false;
            }
            _ => {
                time = match action {
                    TransportAction::Seek(t) => finite(t)?,
                    TransportAction::Step(frames) => time + f64::from(frames) / self.fps,
                    TransportAction::Start => 0.0,
                    TransportAction::End => self.duration,
                    TransportAction::WorkIn => self.work.0,
                    TransportAction::WorkOut => self.work.1,
                    _ => unreachable!(),
                };
                // Preserve authored tag order: clamp, then project-frame snap.
                time = (time.clamp(0.0, self.duration) * self.fps).round() / self.fps;
            }
        }
        finite(time)?;
        put(&mut ui.transport, "time", Value::Number(time))?;
        put(&mut ui.transport, "playing", Value::Bool(playing))
    }
}
