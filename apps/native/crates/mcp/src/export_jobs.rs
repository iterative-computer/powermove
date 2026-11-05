//! Resumable exports belong to a native document owner, not a requesting run.
//! Waiting is on MCP workers after releasing the kernel-host mutex.
use crate::{
    Error, Result,
    services::{ExportChild, FrozenProject, VideoExportRequest, VideoOptions},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Progress {
    pub frames: u64,
    pub total: Option<u64>,
    pub encoder: Option<String>,
    pub seconds: f64,
}
struct Job {
    snapshot: FrozenProject,
    child: Arc<dyn ExportChild>,
    started: Instant,
    result: Mutex<Option<Result<Value>>>,
    done: Condvar,
    reported: AtomicBool,
}
pub struct ExportJobs {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    pub wait: Duration,
    pub limit: Duration,
}
impl Default for ExportJobs {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            wait: Duration::from_secs(20 * 60),
            limit: Duration::from_secs(12 * 60 * 60),
        }
    }
}
pub struct Pending {
    pub id: String,
    job: Arc<Job>,
    wait: Duration,
    limit: Duration,
}
impl ExportJobs {
    pub fn with_limits(wait: Duration, limit: Duration) -> Self {
        Self {
            wait,
            limit,
            ..Default::default()
        }
    }

    pub fn prepare(
        &self,
        snapshot: FrozenProject,
        args: &Value,
        child: Arc<dyn ExportChild>,
        directory: std::path::PathBuf,
    ) -> Result<Pending> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| Error::unavailable("native export job coordinator"))?;
        jobs.retain(|_, job| {
            !job.reported.load(Ordering::SeqCst) || job.result.lock().is_ok_and(|r| r.is_none())
        });
        let (id, job) = if let Some(id) = args.get("jobId") {
            let Some(id) = id
                .as_str()
                .filter(|_| args.as_object().is_some_and(|o| o.len() == 1))
            else {
                return Err(Error::invalid(
                    "export_video with a jobId takes no other arguments.",
                ));
            };
            let job = jobs
                .get(id)
                .filter(|j| {
                    !j.reported.load(Ordering::SeqCst)
                        && j.snapshot.document_id == snapshot.document_id
                        && j.snapshot.project_id == snapshot.project_id
                })
                .cloned()
                .ok_or_else(unknown)?;
            (id.to_owned(), job)
        } else {
            if let Some((id, _)) = jobs
                .iter()
                .find(|(_, j)| j.result.lock().is_ok_and(|r| r.is_none()))
            {
                return Err(Error::invalid(format!(
                    "An export is already running. Call export_video with {{\"jobId\":\"{id}\"}} to wait for it."
                )));
            }
            crate::catalog::validate("export_video", args)?;
            let options = VideoOptions::parse(args)?;
            let job = Arc::new(Job {
                snapshot: snapshot.clone(),
                child: child.clone(),
                started: Instant::now(),
                result: Mutex::new(None),
                done: Condvar::new(),
                reported: AtomicBool::new(false),
            });
            let worker = job.clone();
            let request = VideoExportRequest {
                snapshot,
                options,
                directory,
            };
            std::thread::Builder::new()
                .name("pm-mcp-export".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::services::run_video(child.as_ref(), &request)
                    }))
                    .unwrap_or_else(|_| {
                        Err(Error::unavailable(
                            "native export worker stopped before reporting its outcome",
                        ))
                    });
                    if let Ok(mut state) = worker.result.lock() {
                        *state = Some(result);
                        worker.done.notify_all();
                    }
                })?;
            let id = pm_kernel::new_id("export");
            jobs.insert(id.clone(), job.clone());
            (id, job)
        };
        Ok(Pending {
            id,
            job,
            wait: self.wait,
            limit: self.limit,
        })
    }
    pub fn consume(&self, id: &str) -> Result<bool> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| Error::unavailable("native export job coordinator"))?;
        let Some(job) = jobs.get(id) else {
            return Ok(false);
        };
        let first = !job.reported.swap(true, Ordering::SeqCst);
        let complete = job
            .result
            .lock()
            .map_err(|_| Error::unavailable("native export result"))?
            .is_some();
        if complete {
            jobs.remove(id);
        }
        Ok(first)
    }
    /// Worker-only drain. A deadline refusal never claims that a child stopped.
    pub fn drain(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| Error::unavailable("native export coordinator"))?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for job in jobs {
            let state = job
                .result
                .lock()
                .map_err(|_| Error::unavailable("native export result"))?;
            let (state, _) = job
                .done
                .wait_timeout_while(
                    state,
                    deadline.saturating_duration_since(Instant::now()),
                    |r| r.is_none(),
                )
                .map_err(|_| Error::unavailable("native export result"))?;
            if state.is_none() {
                return Err(Error::unavailable(
                    "native export worker is still running; outcome unknown",
                ));
            }
        }
        Ok(())
    }
}
impl Pending {
    pub fn wait(&self) -> Result<(Value, bool)> {
        let state = self
            .job
            .result
            .lock()
            .map_err(|_| Error::unavailable("native export result"))?;
        let left = self.limit.saturating_sub(self.job.started.elapsed());
        let (state, _) = self
            .job
            .done
            .wait_timeout_while(state, self.wait.min(left), |s| s.is_none())
            .map_err(|_| Error::unavailable("native export result"))?;
        if let Some(result) = state.as_ref() {
            return result.clone().map(|v| (v, true));
        }
        if self.job.started.elapsed() >= self.limit {
            return Err(Error::invalid(
                "Powermove timed out while running __mcp_export_video. Inspect get_workspace_state or capture_panel before retrying; an edit may already have completed.",
            ));
        }
        let mut value = json!({"status":"running","jobId":self.id,"next":format!("The export continues in Powermove. Call export_video with {{\"jobId\":\"{}\"}} to keep waiting for its file.",self.id)});
        if let Some(progress) = self.job.child.video_progress(&self.job.snapshot) {
            let mut fields = json!({"frames":progress.frames});
            if let Some(total) = progress.total.filter(|n| *n > 0) {
                fields["totalFrames"] = json!(total);
                let done = progress.frames as f64 / total as f64;
                if done > 0.0 && done < 1.0 && progress.seconds.is_finite() {
                    fields["secondsRemaining"] =
                        json!((progress.seconds * (1.0 - done) / done).round() as i64);
                }
            }
            if let Some(encoder) = progress.encoder.filter(|s| !s.is_empty()) {
                fields["encoder"] = json!(encoder)
            }
            value["progress"] = fields;
        }
        Ok((value, false))
    }
}
fn unknown() -> Error {
    Error::invalid("Unknown export job. A finished job is reported once; check the export folder.")
}
