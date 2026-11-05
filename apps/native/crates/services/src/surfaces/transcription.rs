//! S090–S092. Frozen shared/transcription-pick.ts and transcription/client.ts.
//! Models are measured host publications, never inferred installed capabilities.
use super::{DocumentStamp, Flight, Ticket};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Absent,
    Downloading,
    Ready,
    Error,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub languages: String,
    pub word_timestamps: bool,
    #[serde(default)]
    pub language_codes: Option<Vec<String>>,
    #[serde(default)]
    pub detects_language: Option<bool>,
    pub state: ModelState,
    #[serde(default)]
    pub featured: bool,
    #[serde(default)]
    pub recommended: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub models: Vec<Model>,
    pub active_model_id: Option<String>,
    /// Missing is the frozen older-host contract; native default is false until
    /// a concrete native engine has actually reported availability.
    pub available: Option<bool>,
    pub language: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            models: vec![],
            active_model_id: None,
            available: Some(false),
            language: None,
            extra: BTreeMap::new(),
        }
    }
}
pub fn base_language(language: Option<&str>) -> String {
    language
        .filter(|s| *s != "auto")
        .unwrap_or("")
        .split('-')
        .next()
        .unwrap_or("")
        .to_lowercase()
}
fn spoken_language(active: Option<&Model>, language: Option<&str>) -> String {
    let base = base_language(language);
    if !base.is_empty() {
        return base;
    }
    match active {
        Some(m) if m.languages == "en" || m.detects_language == Some(false) => "en".into(),
        _ => String::new(),
    }
}
impl Status {
    pub fn model_for(&self, words: bool, language: Option<&str>) -> Option<&Model> {
        if self.available == Some(false) {
            return None;
        }
        let active = self
            .models
            .iter()
            .find(|m| Some(&m.id) == self.active_model_id.as_ref() && m.state == ModelState::Ready);
        if !words || active.is_some_and(|m| m.word_timestamps) {
            return active;
        }
        let spoken = spoken_language(active, language.or(self.language.as_deref()));
        self.models
            .iter()
            .filter(|m| m.state == ModelState::Ready && m.word_timestamps)
            .find(|m| {
                if !spoken.is_empty() {
                    m.language_codes
                        .as_ref()
                        .map_or(m.languages == "en" && spoken == "en", |codes| {
                            codes.contains(&spoken)
                        })
                } else if active.is_some_and(|m| m.languages == "multi") {
                    m.languages == "multi" && m.detects_language != Some(false)
                } else {
                    true
                }
            })
    }
    pub fn offered(&self, words: bool) -> Vec<&Model> {
        let fits = |m: &&Model| !words || m.word_timestamps;
        let curated: Vec<_> = self
            .models
            .iter()
            .filter(fits)
            .filter(|m| m.featured || m.state == ModelState::Downloading)
            .collect();
        let curated = if curated.is_empty() {
            self.models.iter().filter(fits).collect()
        } else {
            curated
        };
        if !words {
            return curated;
        }
        let closes = |m: &&Model| {
            if m.state == ModelState::Downloading {
                return true;
            }
            if m.state == ModelState::Ready || !m.word_timestamps {
                return false;
            }
            let mut after = self.clone();
            after
                .models
                .iter_mut()
                .find(|entry| entry.id == m.id)
                .unwrap()
                .state = ModelState::Ready;
            after.model_for(true, None).is_some()
        };
        let offered: Vec<_> = curated.into_iter().filter(closes).collect();
        if offered.is_empty() {
            self.models
                .iter()
                .filter(|m| m.word_timestamps)
                .filter(closes)
                .collect()
        } else {
            offered
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub path: PathBuf,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub language: Option<String>,
    pub word_timestamps: bool,
}
impl Request {
    pub fn valid(&self) -> bool {
        self.path.is_absolute()
            && self.start.is_none_or(|t| t.is_finite() && t >= 0.)
            && self
                .end
                .is_none_or(|t| t.is_finite() && t > self.start.unwrap_or(0.))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub model_id: String,
    pub language: Option<String>,
    pub duration: f64,
    pub timing: Option<String>,
    pub segments: Vec<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JobState {
    Idle,
    Running { progress: f64 },
    CancelRequested,
    Cancelled,
    Complete,
    Failed { code: String },
    Stale,
}
#[derive(Debug)]
pub struct Job {
    pub state: JobState,
    pub transcript: Option<Transcript>,
    flight: Flight,
    stamp: Option<DocumentStamp>,
}
impl Default for Job {
    fn default() -> Self {
        Self {
            state: JobState::Idle,
            transcript: None,
            flight: Flight::default(),
            stamp: None,
        }
    }
}
impl Job {
    pub fn start(&mut self, request: &Request, stamp: DocumentStamp) -> Option<Ticket> {
        if self.flight.current().is_some() || !request.valid() {
            return None;
        }
        self.state = JobState::Running { progress: 0. };
        self.transcript = None;
        self.stamp = Some(stamp);
        Some(self.flight.start())
    }
    pub fn progress(&mut self, ticket: Ticket, value: f64) -> bool {
        if !self.flight.accepts(ticket) || !value.is_finite() || !(0. ..=1.).contains(&value) {
            return false;
        }
        let JobState::Running { progress } = self.state else {
            return false;
        };
        if value < progress {
            return false;
        }
        self.state = JobState::Running { progress: value };
        true
    }
    pub fn cancel(&mut self) -> Option<Ticket> {
        let ticket = self.flight.current()?;
        if !matches!(self.state, JobState::Running { .. }) {
            return None;
        }
        self.state = JobState::CancelRequested;
        Some(ticket)
    }
    /// A resolved partial transcript after Cancel must never become captions.
    pub fn finish(
        &mut self,
        ticket: Ticket,
        current: &DocumentStamp,
        result: Result<Transcript, String>,
    ) -> bool {
        if !self.flight.finish(ticket) {
            return false;
        }
        if self.state == JobState::CancelRequested {
            self.state = JobState::Cancelled;
            return true;
        }
        if self.stamp.as_ref() != Some(current) {
            self.state = JobState::Stale;
            return true;
        }
        match result {
            Ok(transcript) => {
                self.transcript = Some(transcript);
                self.state = JobState::Complete;
            }
            Err(code) => self.state = JobState::Failed { code },
        }
        true
    }
}

/// Worker-only host seam. Missing production engine/download/archive adapters
/// remain explicit; this trait does not mark any model ready by itself.
pub trait TranscriptionHost: Send + 'static {
    fn status(&mut self) -> crate::Result<Status>;
    fn transcribe(&mut self, ticket: Ticket, request: Request) -> crate::Result<Transcript>;
    /// Must use a separate control path from a blocked decoder worker.
    fn cancel(&self, ticket: Ticket) -> crate::Result<()>;
    /// Archive original and partial model bytes before removing the active entry.
    fn archive_model(&mut self, id: &str) -> crate::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn model(
        id: &str,
        languages: &str,
        words: bool,
        codes: Option<Vec<&str>>,
        detects: Option<bool>,
    ) -> Model {
        Model {
            id: id.into(),
            languages: languages.into(),
            word_timestamps: words,
            language_codes: codes.map(|v| v.into_iter().map(String::from).collect()),
            detects_language: detects,
            state: ModelState::Ready,
            featured: false,
            recommended: false,
            extra: BTreeMap::new(),
        }
    }
    fn status() -> Status {
        Status {
            available: Some(true),
            active_model_id: Some("active".into()),
            models: vec![
                model("active", "multi", false, None, Some(true)),
                model("english", "en", true, None, None),
            ],
            ..Status::default()
        }
    }
    fn request() -> Request {
        Request {
            path: std::env::temp_dir().join("synthetic.wav"),
            start: Some(0.),
            end: Some(1.),
            language: None,
            word_timestamps: true,
        }
    }
    fn stamp() -> DocumentStamp {
        DocumentStamp {
            project: "p".into(),
            session: 1,
            revision: 1,
        }
    }
    fn transcript() -> Transcript {
        Transcript {
            model_id: "m".into(),
            language: None,
            duration: 1.,
            timing: Some("word".into()),
            segments: vec![],
        }
    }
    #[test]
    fn unknown_multilingual_speech_never_falls_back_to_english() {
        let mut s = status();
        assert!(s.model_for(true, None).is_none());
        s.models
            .push(model("multi", "multi", true, None, Some(true)));
        assert_eq!(s.model_for(true, None).unwrap().id, "multi");
    }
    #[test]
    fn explicit_language_uses_base_and_exact_language_inventory() {
        let mut s = status();
        s.models.push(model(
            "german",
            "multi",
            true,
            Some(vec!["de"]),
            Some(false),
        ));
        assert_eq!(s.model_for(true, Some("de-AT")).unwrap().id, "german");
        assert!(s.model_for(true, Some("ja")).is_none());
    }
    #[test]
    fn non_detecting_active_auto_means_english() {
        let mut s = status();
        s.models[0].detects_language = Some(false);
        assert_eq!(s.model_for(true, None).unwrap().id, "english");
    }
    #[test]
    fn native_unavailable_is_never_optimistic() {
        let mut s = status();
        s.available = Some(false);
        assert!(s.model_for(false, None).is_none());
        assert_eq!(Status::default().available, Some(false));
    }
    #[test]
    fn active_timed_model_and_missing_active_follow_tag() {
        let mut s = status();
        s.models[0].word_timestamps = true;
        assert_eq!(s.model_for(true, Some("ja")).unwrap().id, "active");
        s.active_model_id = None;
        assert!(s.model_for(false, None).is_none());
        assert_eq!(s.model_for(true, None).unwrap().id, "active");
    }
    #[test]
    fn chooser_excludes_incompatible_curated_and_falls_back() {
        let mut s = status();
        s.models[1].state = ModelState::Absent;
        s.models[1].featured = true;
        s.models
            .push(model("multi", "multi", true, None, Some(true)));
        s.models[2].state = ModelState::Absent;
        assert_eq!(
            s.offered(true)
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["multi"]
        );
    }
    #[test]
    fn future_model_metadata_survives_roundtrip() {
        let raw = json!({"id":"m","languages":"en","wordTimestamps":true,"state":"ready","future":{"x":1}});
        let m: Model = serde_json::from_value(raw).unwrap();
        assert_eq!(serde_json::to_value(m).unwrap()["future"], json!({"x":1}));
    }
    #[test]
    fn cancel_rejects_late_success_and_progress() {
        let mut j = Job::default();
        let t = j.start(&request(), stamp()).unwrap();
        assert!(j.progress(t, 0.5));
        assert!(!j.progress(t, 0.4));
        assert!(!j.progress(t, f64::NAN));
        assert_eq!(j.cancel(), Some(t));
        assert!(!j.progress(t, 1.));
        assert!(j.finish(t, &stamp(), Ok(transcript())));
        assert_eq!(j.state, JobState::Cancelled);
        assert!(j.transcript.is_none());
        assert!(!j.finish(t, &stamp(), Ok(transcript())));
    }
    #[test]
    fn reopen_and_edit_fence_generated_results() {
        for field in ["revision", "session"] {
            let mut j = Job::default();
            let t = j.start(&request(), stamp()).unwrap();
            let mut current = stamp();
            if field == "session" {
                current.session += 1
            } else {
                current.revision += 1
            };
            assert!(j.finish(t, &current, Ok(transcript())));
            assert_eq!(j.state, JobState::Stale);
            assert!(j.transcript.is_none());
        }
    }
    #[test]
    fn requests_require_finite_ordered_source_span() {
        let mut r = request();
        assert!(r.valid());
        r.end = Some(0.);
        assert!(!r.valid());
        r.end = Some(f64::INFINITY);
        assert!(!r.valid());
        r.end = Some(1.);
        r.start = Some(-1.);
        assert!(!r.valid());
    }
}
