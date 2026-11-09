//! The agent's speech tools (today's `transcribe_media` and `generate_captions`
//! bodies): transcription jobs on this Mac that outlive one tool call. A call
//! waits a while for its job, then answers with progress; calling again with
//! the same arguments (or the caption job's id) keeps waiting. Transcripts
//! are kept for paging; caption results become one `add_captions` command
//! that the session applies inside the run's own transaction.
use crate::{
    captions_generate::{self, Outcome},
    transcription::{self, Failure, MODEL_MISSING, Transcript},
};
use pm_captions::generate::ClipTiming;
use pm_kernel::{KernelHandle, Published};
use pm_mcp::{Error, Result, services::FrozenProject};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

const TRANSCRIBE_WAIT: Duration = Duration::from_secs(80);
const CAPTIONS_WAIT: Duration = Duration::from_secs(60);
const PAGE_CHARS: usize = 12_000;

/// One background job and its eventual answer.
struct Job<T> {
    cancel: Arc<AtomicBool>,
    progress: AtomicU64,
    answer: Mutex<Option<std::result::Result<T, Failure>>>,
    ready: Condvar,
    owner: u32,
}
impl<T: Clone + Send + 'static> Job<T> {
    fn start(
        owner: u32,
        work: impl FnOnce(&Arc<Self>) -> std::result::Result<T, Failure> + Send + 'static,
    ) -> Arc<Self> {
        let job = Arc::new(Self {
            cancel: Arc::default(),
            progress: AtomicU64::new(0f64.to_bits()),
            answer: Mutex::new(None),
            ready: Condvar::new(),
            owner,
        });
        let running = job.clone();
        std::thread::Builder::new()
            .name("pm-agent-speech".into())
            .spawn(move || {
                let answer = work(&running);
                *running.answer.lock().unwrap_or_else(|e| e.into_inner()) = Some(answer);
                running.ready.notify_all();
            })
            .expect("speech job thread");
        job
    }
    fn report(&self, progress: f64) {
        self.progress
            .store(progress.clamp(0., 1.).to_bits(), Ordering::Relaxed);
    }
    fn progress(&self) -> f64 {
        (f64::from_bits(self.progress.load(Ordering::Relaxed)) * 100.).round() / 100.
    }
    fn wait(&self, limit: Duration) -> Option<std::result::Result<T, Failure>> {
        let answer = self.answer.lock().unwrap_or_else(|e| e.into_inner());
        let (answer, _) = self
            .ready
            .wait_timeout_while(answer, limit, |a| a.is_none())
            .unwrap_or_else(|e| e.into_inner());
        answer.clone()
    }
}
fn model_required(what: &str) -> Error {
    Error {
        code: MODEL_MISSING.into(),
        message: format!(
            "No speech model that can {what} is downloaded yet. Tell the user to download one in Settings › Transcription, and call again once the download finishes. Do not retry before then."
        ),
        backing: None,
    }
}
fn failed(failure: Failure, what: &str) -> Error {
    match failure {
        Failure::ModelMissing(_) => model_required(what),
        Failure::Cancelled => Error::invalid("The transcription was cancelled."),
        Failure::Failed(message) => Error::invalid(message),
    }
}
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.);
    format!("{}:{:05.2}", (s / 60.).floor() as u64, s % 60.)
}
fn round(v: f64) -> f64 {
    (v * 100.).round() / 100.
}
/// Source seconds to the first composition time the clip shows them.
fn to_composition(clip: &ClipTiming, start: f64, end: f64) -> Option<(f64, f64)> {
    let word = pm_captions::CaptionWord {
        text: "x".into(),
        start,
        end,
    };
    pm_captions::generate::map_words_to_composition(&[word], clip)
        .first()
        .map(|w| (round(w.start), round(w.end)))
}
/// Today's transcript page: timestamped lines, or segments with their words.
pub fn page(
    transcript: &Transcript,
    clip: Option<&ClipTiming>,
    args: &Value,
    header: Value,
) -> Value {
    let map = |start: f64, end: f64| match clip {
        Some(clip) => to_composition(clip, start, end),
        None => Some((round(start), round(end))),
    };
    let segments: Vec<_> = transcript
        .segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .filter_map(|s| Some((s, map(s.start, s.end)?)))
        .collect();
    let cursor = args["cursor"].as_u64().unwrap_or(0) as usize;
    let mut out = header;
    out["timeBase"] = json!(if clip.is_some() {
        "composition"
    } else {
        "source"
    });
    out["modelId"] = json!(transcript.model_id);
    out["language"] = json!(transcript.language);
    out["segmentCount"] = json!(segments.len());
    out["cursor"] = json!(cursor);
    let words = args["format"] == "words";
    if words && transcript.timing != "word" {
        out["wordTiming"] = json!("estimated");
        out["wordTimingNote"] = json!(
            "The speech model in use does not time each word: word times are spread over each phrase. Cut on segment times, or ask the user to download a word-timed model in Settings › Transcription for exact word times."
        );
    }
    if segments.is_empty() {
        out[if words { "segments" } else { "text" }] = if words { json!([]) } else { json!("") };
        out["note"] = json!("No speech detected.");
        return out;
    }
    let mut used = 0;
    let mut index = cursor;
    if words {
        let mut page = vec![];
        while let Some((segment, (start, end))) = segments.get(index) {
            let words: Vec<Value> = segment
                .words
                .iter()
                .filter_map(|w| {
                    let (a, b) = map(w.start, w.end)?;
                    Some(json!([w.text.trim(), a, b]))
                })
                .collect();
            let entry = json!({"start":start,"end":end,"text":segment.text.trim(),"words":words});
            let size = entry.to_string().len();
            if !page.is_empty() && used + size > PAGE_CHARS {
                break;
            }
            page.push(entry);
            used += size;
            index += 1;
        }
        out["wordFormat"] = json!("[text, start, end]");
        out["segments"] = json!(page);
    } else {
        let mut lines: Vec<String> = vec![];
        while let Some((segment, (start, end))) = segments.get(index) {
            let line = format!(
                "[{}–{}] {}",
                clock(*start),
                clock(*end),
                segment.text.trim()
            );
            if !lines.is_empty() && used + line.len() + 1 > PAGE_CHARS {
                break;
            }
            used += line.len() + 1;
            lines.push(line);
            index += 1;
        }
        out["text"] = json!(lines.join("\n"));
    }
    if index < segments.len() {
        out["nextCursor"] = json!(index);
    }
    out
}

type TranscriptJob = Job<Transcript>;
type CaptionJob = Job<Outcome>;
pub struct AgentSpeech {
    kernel: KernelHandle,
    profile: PathBuf,
    /// By file, span and language: identical calls share one job, and a
    /// finished transcript answers the next page.
    transcripts: Mutex<Vec<(String, Arc<TranscriptJob>)>>,
    captions: Mutex<HashMap<String, Arc<CaptionJob>>>,
}
impl AgentSpeech {
    pub fn new(kernel: KernelHandle, profile: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            kernel,
            profile,
            transcripts: Mutex::default(),
            captions: Mutex::default(),
        })
    }
    fn published(&self, frozen: &FrozenProject) -> Result<Arc<Published>> {
        self.kernel
            .published(&frozen.project_id)
            .filter(|p| p.snapshot.doc_id == frozen.document_id)
            .ok_or_else(|| {
                Error::conflict("The project changed. Read get_project_state before retrying.")
            })
    }
    fn ready(words: bool) -> bool {
        transcription::store()
            .and_then(|s| s.model_for(words))
            .is_some()
    }
}
impl pm_mcp::services::Speech for AgentSpeech {
    fn transcribe(&self, frozen: &FrozenProject, args: &Value) -> Result<Value> {
        if !Self::ready(false) {
            return Err(model_required("transcribe speech"));
        }
        let published = self.published(frozen)?;
        let (_, comp) = captions_generate::open_comp(&published)
            .ok_or_else(|| Error::invalid("Open a composition first."))?;
        let layer = args["layerId"].as_str().filter(|s| !s.is_empty());
        let (asset, clip, name) = match layer {
            Some(id) => {
                let target = captions_generate::targets(&comp, &[id.into()])
                    .pop()
                    .ok_or_else(|| {
                        Error::invalid(
                            "Choose an audio or video layer with media from get_project_state.",
                        )
                    })?;
                if args["assetId"].as_str().is_some_and(|a| a != target.asset) {
                    return Err(Error::invalid(
                        "The selected layer plays a different asset. Pass one or the other.",
                    ));
                }
                let clip = captions_generate::clip_timing(&published, &target)
                    .map_err(|e| failed(e, "transcribe speech"))?;
                (target.asset.clone(), Some(clip), target.name)
            }
            None => {
                let asset = args["assetId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::invalid("Supply assetId or layerId."))?;
                let name = published
                    .snapshot
                    .root()
                    .and_then(|r| r.get("assets")?.get(asset)?.get("name"))
                    .and_then(crate::node_string)
                    .unwrap_or(asset)
                    .to_owned();
                (asset.to_owned(), None, name)
            }
        };
        let path = captions_generate::originals(&published, &self.profile)
            .remove(&asset)
            .ok_or_else(|| Error::invalid("This media's original is offline. Relink it first."))?;
        let (start, end) = match &clip {
            Some(clip) => {
                let [a, b] = clip.visible_source_range();
                (Some(a).filter(|a| *a > 0.), Some(b).filter(|b| *b > a))
            }
            None => (args["start"].as_f64(), args["end"].as_f64()),
        };
        let language = args["language"].as_str().map(str::to_owned);
        let key = format!("{}|{start:?}|{end:?}|{language:?}", path.display());
        let job = {
            let mut jobs = self.transcripts.lock().unwrap_or_else(|e| e.into_inner());
            let found = jobs.iter().find(|(k, _)| *k == key).map(|(_, j)| j.clone());
            found.unwrap_or_else(|| {
                let request = transcription::Request {
                    path,
                    start,
                    end,
                    language,
                    word_timestamps: false,
                };
                let job = Job::start(0, move |job| {
                    let progress = job.clone();
                    transcription::transcribe(request, job.cancel.clone(), move |p| {
                        progress.report(p)
                    })
                });
                jobs.push((key.clone(), job.clone()));
                if jobs.len() > 8 {
                    jobs.remove(0);
                }
                job
            })
        };
        let header = json!({"asset":{"id":asset,"name":name},"layer":layer});
        match job.wait(TRANSCRIBE_WAIT) {
            None => Ok(
                json!({"status":"transcribing","asset":{"id":asset},"progress":job.progress(),
                "note":"Transcription is still running in Powermove. Call transcribe_media again with the same arguments to get the transcript; it keeps going in the meantime."}),
            ),
            Some(Ok(transcript)) => Ok(page(&transcript, clip.as_ref(), args, header)),
            Some(Err(failure)) => {
                self.transcripts
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|(k, _)| *k != key);
                Err(failed(failure, "transcribe speech"))
            }
        }
    }
    fn captions(&self, frozen: &FrozenProject, args: &Value, owner: u32) -> Result<Value> {
        let wait = |id: &str, job: &Arc<CaptionJob>| -> Result<Value> {
            match job.wait(CAPTIONS_WAIT) {
                None => Ok(
                    json!({"jobId":id,"status":"running","progress":job.progress(),
                    "message":"Still transcribing. Call generate_captions with this jobId to keep waiting; do not start another generation."}),
                ),
                Some(answer) => {
                    self.captions
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(id);
                    match answer.map_err(|e| failed(e, "time each word"))? {
                        Outcome::Empty => {
                            Err(Error::invalid("No speech was found in the selected clips."))
                        }
                        Outcome::Captions {
                            cues,
                            count,
                            language,
                            name,
                        } => {
                            let mut command = json!({"type":"add_captions","id":pm_kernel::new_id("L"),
                                "name":name,"from":0,"cues":cues});
                            if let Some(language) = &language {
                                command["language"] = json!(language);
                            }
                            if let Some(style) = args["style"].as_str() {
                                command["style"] = json!({"preset":style});
                            }
                            Ok(
                                json!({"status":"done","command":command,"cues":count,"language":language}),
                            )
                        }
                    }
                }
            }
        };
        if let Some(id) = args["jobId"].as_str().filter(|s| !s.is_empty()) {
            let job = self
                .captions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .filter(|j| j.owner == owner)
                .cloned()
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "Unknown caption job {id}. Start one with generate_captions {{ layerIds }}."
                    ))
                })?;
            return wait(id, &job);
        }
        if let Some((id, _)) = self
            .captions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(_, j)| j.owner == owner)
        {
            return Err(Error::invalid(format!(
                "Captions are already being generated (jobId {id}). Call generate_captions with {{ \"jobId\": \"{id}\" }} to keep waiting."
            )));
        }
        let ids: Vec<String> = args["layerIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let published = self.published(frozen)?;
        let (_, comp) = captions_generate::open_comp(&published)
            .ok_or_else(|| Error::invalid("Open a composition first."))?;
        let targets = captions_generate::targets(&comp, &ids);
        if ids.is_empty() || targets.len() != ids.len() {
            return Err(Error::invalid(
                "generate_captions needs audio or video layer IDs with media, from get_project_state.",
            ));
        }
        if !Self::ready(true) {
            return Err(model_required("time each word"));
        }
        let profile = self.profile.clone();
        let job = Job::start(owner, move |job| {
            let progress = job.clone();
            captions_generate::generate(
                &published,
                &targets,
                &profile,
                &job.cancel,
                Arc::new(move |p| progress.report(p)),
            )
        });
        let id = pm_kernel::new_id("captions");
        self.captions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), job.clone());
        wait(&id, &job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_transcribe::pipeline::{Segment, Word};
    fn transcript() -> Transcript {
        let w = |t: &str, s: f64, e: f64| Word {
            text: t.into(),
            start: s,
            end: e,
            confidence: None,
        };
        Transcript {
            model_id: "m".into(),
            language: Some("en".into()),
            duration: 10.,
            segments: vec![
                Segment {
                    text: "Hello there.".into(),
                    start: 4.,
                    end: 5.,
                    words: vec![w("Hello", 4., 4.4), w("there.", 4.5, 5.)],
                },
                Segment {
                    text: "Later".into(),
                    start: 9.,
                    end: 9.5,
                    words: vec![w("Later", 9., 9.5)],
                },
            ],
            timing: "word".into(),
        }
    }
    #[test]
    fn pages_read_in_source_or_composition_time() {
        let t = transcript();
        let text = page(&t, None, &json!({}), json!({}));
        assert_eq!(text["timeBase"], "source");
        assert_eq!(
            text["text"],
            "[0:04.00–0:05.00] Hello there.\n[0:09.00–0:09.50] Later"
        );
        // A clip from 2 s showing source 3–6 s: the second segment isn't shown.
        let clip = ClipTiming::linear(2., 3., 3., 1.);
        let words = page(&t, Some(&clip), &json!({"format":"words"}), json!({}));
        assert_eq!(words["timeBase"], "composition");
        assert_eq!(words["segmentCount"], 1);
        assert_eq!(words["segments"][0]["start"], 3.);
        assert_eq!(words["segments"][0]["words"][1], json!(["there.", 3.5, 4.]));
        let rest = page(&t, None, &json!({"cursor":1}), json!({}));
        assert_eq!(rest["text"], "[0:09.00–0:09.50] Later");
    }
    #[test]
    fn a_job_answers_waiters_once_done() {
        let job = Job::start(7, |job| {
            job.report(0.5);
            std::thread::sleep(Duration::from_millis(50));
            Ok(3)
        });
        assert_eq!(job.wait(Duration::from_secs(5)).unwrap().unwrap(), 3);
        assert_eq!(job.owner, 7);
        let slow: Arc<Job<u8>> = Job::start(0, |_| {
            std::thread::sleep(Duration::from_millis(300));
            Err(Failure::Cancelled)
        });
        assert!(slow.wait(Duration::from_millis(10)).is_none());
        assert!(matches!(
            slow.wait(Duration::from_secs(5)),
            Some(Err(Failure::Cancelled))
        ));
    }
}
