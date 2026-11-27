//! Source-monitor semantics (tag source-preview.ts:172–291). Host descriptors
//! confirm media availability; the model never invents a playable source.
use crate::{Error, finite};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Image,
    Video,
    Audio,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SourceAsset {
    pub id: String,
    pub kind: SourceKind,
    pub available: bool,
    pub duration: Option<f64>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceState {
    /// Incremented on replacement/close; observations from old loads are fenced.
    pub epoch: u64,
    pub asset: Option<SourceAsset>,
    pub time: f64,
    pub playing: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SourceCommand {
    Show(String),
    Toggle(String),
    Close,
    PlayPause,
    SeekRatio(f64),
    SeekSeconds(f64),
    StepSeconds(f64),
    Start,
    End,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SourceEffect {
    Load(String),
    Release(String),
    PauseComposition,
    Play,
    Pause,
    Seek(f64),
}
#[derive(Clone, Debug, PartialEq)]
pub struct SourcePlan {
    pub next: SourceState,
    pub effects: Vec<SourceEffect>,
}
impl SourceState {
    pub fn active_id(&self) -> Option<&str> {
        self.asset.as_ref().map(|a| a.id.as_str())
    }
    pub fn temporal(&self) -> bool {
        self.asset
            .as_ref()
            .is_some_and(|a| a.kind != SourceKind::Image)
    }
    /// `resolve` is a data-only media catalog lookup. Effect execution is a
    /// worker job. Publish `next` only after successful host acceptance; late
    /// observations must carry the owning document/source generation.
    pub fn plan(
        &self,
        command: SourceCommand,
        resolve: impl FnOnce(&str) -> Option<SourceAsset>,
    ) -> Result<SourcePlan, Error> {
        let mut next = self.clone();
        let mut effects = vec![];
        match command {
            SourceCommand::Show(ref id) | SourceCommand::Toggle(ref id) => {
                if id.is_empty() {
                    return Err(Error::Invalid("source id"));
                }
                if self.active_id() != Some(id.as_str()) {
                    let asset = resolve(id).ok_or(Error::Invalid("source asset"))?;
                    if asset.id != *id || !asset.available {
                        return Err(Error::Invalid("source unavailable"));
                    }
                    if let Some(duration) = asset.duration
                        && finite(duration)? <= 0.0
                    {
                        return Err(Error::Invalid("source duration"));
                    }
                    if let Some(old) = self.active_id() {
                        effects.push(SourceEffect::Release(old.into()));
                    }
                    effects.extend([
                        SourceEffect::Load(id.into()),
                        SourceEffect::PauseComposition,
                    ]);
                    next = Self {
                        epoch: self
                            .epoch
                            .checked_add(1)
                            .ok_or(Error::Invalid("source epoch"))?,
                        asset: Some(asset),
                        time: 0.0,
                        playing: false,
                    };
                }
                if matches!(command, SourceCommand::Toggle(_)) {
                    next.toggle_play(&mut effects);
                }
            }
            SourceCommand::Close => {
                if let Some(id) = self.active_id() {
                    effects.push(SourceEffect::Release(id.into()));
                }
                next = Self {
                    epoch: self
                        .epoch
                        .checked_add(1)
                        .ok_or(Error::Invalid("source epoch"))?,
                    ..Self::default()
                };
            }
            SourceCommand::PlayPause => next.toggle_play(&mut effects),
            _ if !self.temporal() => {}
            _ => {
                let Some(duration) = self.asset.as_ref().and_then(|a| a.duration) else {
                    return Err(Error::Invalid("unknown source duration"));
                };
                let time = match command {
                    SourceCommand::SeekRatio(ratio) => finite(ratio)?.clamp(0.0, 1.0) * duration,
                    SourceCommand::SeekSeconds(time) => finite(time)?.clamp(0.0, duration),
                    SourceCommand::StepSeconds(delta) => {
                        (self.time + finite(delta)?).clamp(0.0, duration)
                    }
                    SourceCommand::Start => 0.0,
                    SourceCommand::End => duration,
                    _ => unreachable!(),
                };
                next.time = time;
                effects.push(SourceEffect::Seek(time));
            }
        }
        Ok(SourcePlan { next, effects })
    }
    fn toggle_play(&mut self, effects: &mut Vec<SourceEffect>) {
        if !self.temporal() {
            return;
        }
        if self.playing {
            self.playing = false;
            effects.push(SourceEffect::Pause);
        } else {
            if let Some(duration) = self.asset.as_ref().and_then(|a| a.duration)
                && self.time >= duration - 0.01
            {
                self.time = 0.0;
                effects.push(SourceEffect::Seek(0.0));
            }
            self.playing = true;
            effects.push(SourceEffect::Play);
        }
    }
    /// Playback is driven by observed worker time, never wall-clock simulation.
    pub fn observe(
        &mut self,
        id: &str,
        epoch: u64,
        time: f64,
        duration: Option<f64>,
        playing: bool,
    ) -> Result<(), Error> {
        if epoch != self.epoch {
            return Err(Error::Stale);
        }
        let mut next = self.clone();
        let asset = next
            .asset
            .as_mut()
            .filter(|a| a.id == id)
            .ok_or(Error::Stale)?;
        if finite(time)? < 0.0 || duration.is_some_and(|d| !d.is_finite() || d <= 0.0) {
            return Err(Error::Invalid("source observation"));
        }
        asset.duration = duration;
        next.time = duration.map_or(time, |d| time.min(d));
        next.playing = playing && asset.kind != SourceKind::Image;
        *self = next;
        Ok(())
    }
}
