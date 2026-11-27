//! Speech to text on this Mac (S090–S092): the profile's model store (shared
//! with Settings › Transcription), the transcribe.cpp runtime in the bundle,
//! and one worker thread that keeps the last model loaded and runs one job at
//! a time, as main's transcription process does. Media audio is decoded with
//! FFmpeg and resampled to 16 kHz mono on that thread. Everything here blocks:
//! call [`transcribe`] off the main thread.
use pm_transcribe::{
    pipeline::{self, Audio, Recognizer, Segment, Timing},
    runtime::{Model, RunResult, Runtime},
    store::{Store, catalog},
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

/// Opens the profile's model store once per process and hands it to Settings.
pub fn install(profile: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = profile.join("models").join("transcription");
        match Store::open(&root, catalog(), Arc::new(pm_builtins::speech::changed)) {
            Ok(store) => pm_builtins::speech::install(store),
            Err(e) => eprintln!("[transcription] models unavailable: {e}"),
        }
    });
}
pub fn store() -> Option<&'static Store> {
    pm_builtins::speech::store()
}
/// The runtime folder: the bundle's `Resources/transcribe`, else
/// `PM_TRANSCRIBE_RUNTIME` (development).
fn runtime_dir() -> Option<PathBuf> {
    let bundled = std::env::current_exe()
        .ok()?
        .parent()?
        .parent()
        .map(|c| c.join("Resources").join("transcribe"))
        .filter(|d| d.join("contract.json").is_file());
    bundled.or_else(|| {
        std::env::var_os("PM_TRANSCRIBE_RUNTIME")
            .map(PathBuf::from)
            .filter(|d| d.join("contract.json").is_file())
    })
}

#[derive(Clone, Debug)]
pub struct Request {
    pub path: PathBuf,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub language: Option<String>,
    pub word_timestamps: bool,
}
/// Main's `Transcript`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub duration: f64,
    pub segments: Vec<Segment>,
    pub timing: String,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// No model can serve the request (main's `transcription-model-missing`).
    ModelMissing(String),
    Cancelled,
    Failed(String),
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelMissing(m) | Self::Failed(m) => f.write_str(m),
            Self::Cancelled => f.write_str("Transcription was cancelled."),
        }
    }
}
pub const MODEL_MISSING: &str = "transcription-model-missing";

/// The tag handed to the model, or `None` to let it detect (main's
/// `decodeLanguage`).
fn decode_language(model: &pm_transcribe::store::Model, requested: Option<&str>) -> Option<String> {
    if model.languages == "en" {
        return None;
    }
    let base = requested
        .filter(|r| *r != "auto")
        .map(|r| r.split('-').next().unwrap_or("").to_lowercase())
        .unwrap_or_default();
    if !base.is_empty() && model.language_codes.contains(&base) {
        return Some(base);
    }
    if model.detects_language {
        return None;
    }
    Some(if model.language_codes.iter().any(|c| c == "en") {
        "en".into()
    } else {
        model.language_codes.first().cloned().unwrap_or_default()
    })
}
/// The language a transcript is labelled with (main's `transcriptLanguage`,
/// then the model's own detection when it lists that language).
fn label_language(
    model: &pm_transcribe::store::Model,
    requested: Option<&str>,
    setting: &str,
    detected: Option<&str>,
) -> Option<String> {
    if model.languages == "en" {
        return Some("en".into());
    }
    let supported = |tag: &str| {
        tag != "auto"
            && model
                .language_codes
                .contains(&tag.split('-').next().unwrap_or("").to_lowercase())
    };
    if let Some(r) = requested.filter(|r| supported(r)) {
        return Some(r.into());
    }
    if supported(setting) {
        return Some(setting.into());
    }
    let base = detected?.split(['-', '_']).next()?.to_lowercase();
    model.language_codes.contains(&base).then_some(base)
}

struct Job {
    request: Request,
    model: pm_transcribe::store::Model,
    file: PathBuf,
    language: Option<String>,
    cancel: Arc<AtomicBool>,
    progress: Box<dyn FnMut(f64) + Send>,
    reply: mpsc::Sender<Result<pipeline::Heard, Failure>>,
}
/// The worker: the runtime once per process, the last model kept loaded.
fn worker() -> &'static Mutex<mpsc::Sender<Job>> {
    static WORKER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("pm-transcription".into())
            .spawn(move || {
                let mut runtime: Option<&'static Runtime> = None;
                let mut loaded: Option<(PathBuf, Model<'static>)> = None;
                while let Ok(mut job) = rx.recv() {
                    let result = run(&mut runtime, &mut loaded, &mut job);
                    let _ = job.reply.send(result);
                }
            })
            .expect("transcription thread");
        Mutex::new(tx)
    })
}
fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .saturating_sub(2)
        .clamp(1, 6)
}
fn run(
    runtime: &mut Option<&'static Runtime>,
    loaded: &mut Option<(PathBuf, Model<'static>)>,
    job: &mut Job,
) -> Result<pipeline::Heard, Failure> {
    if job.cancel.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled);
    }
    if runtime.is_none() {
        let dir = runtime_dir().ok_or_else(|| {
            Failure::Failed(
                "Transcription can’t start: its runtime is missing. Reinstall Powermove.".into(),
            )
        })?;
        let loaded = Runtime::load(&dir).map_err(|e| Failure::Failed(e.to_string()))?;
        *runtime = Some(Box::leak(Box::new(loaded)));
    }
    let runtime = runtime.expect("loaded above");
    if loaded.as_ref().is_none_or(|(file, _)| *file != job.file) {
        *loaded = None;
        let model = runtime.model(&job.file, threads()).map_err(|_| {
            Failure::Failed("The speech model could not be loaded. Delete it in Settings › Transcription and download it again.".into())
        })?;
        *loaded = Some((job.file.clone(), model));
    }
    let model = &mut loaded.as_mut().expect("loaded above").1;
    let tag = model.language(job.language.as_deref());
    let span = pipeline::padded_span(job.request.start, job.request.end);
    let mut audio = Decoded::open(&job.request.path, span.start).map_err(Failure::Failed)?;
    let seconds = audio
        .duration
        .map(|d| span.end.unwrap_or(d).min(d) - span.start)
        .or(span.end.map(|e| e - span.start));
    audio.end = span.end;
    let mut recognizer = Engine {
        model,
        language: tag,
        cancel: job.cancel.clone(),
    };
    pipeline::transcribe(
        &mut recognizer,
        &mut audio,
        span,
        seconds,
        Timing::of(&job.model.timing),
        job.model.lag,
        &job.cancel,
        &mut job.progress,
    )
    .map_err(|e| match e {
        pipeline::Failure::Cancelled => Failure::Cancelled,
        pipeline::Failure::Audio(m) | pipeline::Failure::Model(m) => Failure::Failed(m),
    })
}
struct Engine<'a, 'r> {
    model: &'a mut Model<'r>,
    language: Option<String>,
    cancel: Arc<AtomicBool>,
}
impl Recognizer for Engine<'_, '_> {
    fn run(&mut self, samples: &[f32]) -> Result<RunResult, pm_transcribe::runtime::Error> {
        self.model
            .run(samples, self.language.as_deref(), &self.cancel)
    }
}
/// A media file's best audio stream from `start` seconds, as 16 kHz mono.
struct Decoded {
    decoder: pm_media::audio::AudioDecoder,
    resampler: pipeline::Resampler,
    rate: f64,
    /// Source seconds read so far, from the file's start.
    at: f64,
    end: Option<f64>,
    duration: Option<f64>,
}
impl Decoded {
    fn open(path: &Path, start: f64) -> Result<Self, String> {
        if !path.is_file() {
            return Err("The media file could not be found.".into());
        }
        let mut decoder = pm_media::audio::AudioDecoder::open(path, None).map_err(|e| {
            if e.to_string().contains("no audio stream") {
                "This file has no audio to transcribe.".to_owned()
            } else {
                "This file’s audio could not be read.".to_owned()
            }
        })?;
        let info = decoder.info().clone();
        let rate = f64::from(info.sample_rate.max(1));
        let first = (start * rate).floor() as u64;
        if first > 0 {
            decoder
                .seek(first)
                .map_err(|_| "The audio could not be decoded.".to_owned())?;
        }
        Ok(Self {
            resampler: pipeline::Resampler::new(info.sample_rate),
            rate,
            at: first as f64 / rate,
            end: None,
            duration: info.duration(),
            decoder,
        })
    }
}
impl Audio for Decoded {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let channels = self.decoder.info().channels().max(1);
        let mut block = (self.rate / 2.) as usize;
        if let Some(end) = self.end {
            block = block.min(((end - self.at) * self.rate).max(0.).ceil() as usize);
        }
        let mut planes = vec![Vec::with_capacity(block); channels];
        let read = if block == 0 {
            0
        } else {
            self.decoder
                .read(&mut planes, block)
                .map_err(|_| "The audio could not be decoded.".to_owned())?
        };
        self.at += read as f64 / self.rate;
        self.resampler.push(&planes, out);
        let more = read == block && block > 0;
        if !more {
            self.resampler.finish(out);
        }
        Ok(more)
    }
}

/// Transcribes `request` on the worker, blocking until it ends. `progress`
/// runs on the worker with 0…1.
pub fn transcribe(
    request: Request,
    cancel: Arc<AtomicBool>,
    progress: impl FnMut(f64) + Send + 'static,
) -> Result<Transcript, Failure> {
    if !request.path.is_absolute() {
        return Err(Failure::Failed(
            "Transcription needs an absolute path to a media file.".into(),
        ));
    }
    let store = store()
        .ok_or_else(|| Failure::ModelMissing("Transcription isn’t available here.".into()))?;
    let Some((model, dir)) = store.model_for(request.word_timestamps) else {
        let message = if request.word_timestamps && store.active().is_some() {
            "Captions need a speech model that times each word in the spoken language."
        } else {
            "No transcription model is installed. Download one in Settings › Transcription."
        };
        return Err(Failure::ModelMissing(message.into()));
    };
    let setting = store.language();
    let requested = request.language.clone();
    let language = decode_language(&model, requested.as_deref().or(Some(&setting)));
    let file = dir.join(&model.files[0].name);
    let (reply, answer) = mpsc::channel();
    worker()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(Job {
            request,
            model: model.clone(),
            file,
            language,
            cancel,
            progress: Box::new(progress),
            reply,
        })
        .map_err(|_| Failure::Failed("The transcription process stopped. Try again.".into()))?;
    let heard = answer
        .recv()
        .map_err(|_| Failure::Failed("The transcription process stopped. Try again.".into()))??;
    Ok(Transcript {
        language: label_language(
            &model,
            requested.as_deref(),
            &setting,
            heard.language.as_deref(),
        ),
        model_id: model.id.clone(),
        duration: heard.duration,
        segments: heard.segments,
        timing: model.timing.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(id: &str) -> pm_transcribe::store::Model {
        catalog().into_iter().find(|m| m.id == id).unwrap()
    }
    #[test]
    fn decode_and_label_languages_follow_main() {
        let en = model("parakeet-unified-en-0.6b");
        assert_eq!(decode_language(&en, Some("de")), None);
        assert_eq!(
            label_language(&en, Some("de"), "auto", None).as_deref(),
            Some("en")
        );
        let multi = catalog()
            .into_iter()
            .find(|m| m.languages == "multi" && m.detects_language)
            .unwrap();
        assert_eq!(decode_language(&multi, Some("auto")), None);
        let code = multi.language_codes[0].clone();
        assert_eq!(
            decode_language(&multi, Some(&format!("{code}-XX"))),
            Some(code.clone())
        );
        assert_eq!(
            label_language(&multi, None, "auto", Some(&format!("{code}-XX"))),
            Some(code)
        );
        assert_eq!(label_language(&multi, None, "auto", Some("zz")), None);
    }
    #[test]
    fn a_relative_path_is_refused_before_any_model() {
        let request = Request {
            path: "clip.mov".into(),
            start: None,
            end: None,
            language: None,
            word_timestamps: true,
        };
        let refused = transcribe(request, Arc::default(), |_| {});
        assert!(matches!(refused, Err(Failure::Failed(_))));
    }
}
