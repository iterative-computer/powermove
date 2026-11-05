//! Agent export-job call lifetime, separate from the external MCP connection.
//! The engine owns actual encoding/cancellation and supplies measured progress.
use crate::{error, session::Owner};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};
pub const CALL_WAIT_MS: u64 = 20 * 60 * 1000;
pub const JOB_LIMIT_MS: u64 = 12 * 60 * 60 * 1000;
#[derive(Clone, Debug)]
pub struct Progress {
    pub frames: u64,
    pub total: Option<u64>,
    pub encoder: Option<String>,
    pub seconds: f64,
}
pub enum Reply {
    Pending(Value),
    Terminal(Result<Value, String>),
}
struct Job {
    owner: Owner,
    started: u64,
    result: Option<Result<Value, String>>,
    progress: Option<Progress>,
}
#[derive(Default)]
pub struct Jobs {
    jobs: BTreeMap<String, Job>,
}
impl Jobs {
    pub fn begin(&mut self, owner: Owner, id: String, now: u64) -> io::Result<()> {
        if !crate::valid_id(&id, 200) || self.jobs.contains_key(&id) {
            return Err(error("Duplicate or invalid export job"));
        }
        if let Some((id, _)) = self
            .jobs
            .iter()
            .find(|(_, j)| j.owner == owner && j.result.is_none())
        {
            return Err(error(format!(
                "An export is already running. Call export_video with {{\"jobId\":\"{id}\"}} to wait for it."
            )));
        }
        if self.jobs.len() >= 128 {
            return Err(error(
                "Archive acknowledged export jobs before starting more exports.",
            ));
        }
        self.jobs.insert(
            id,
            Job {
                owner,
                started: now,
                result: None,
                progress: None,
            },
        );
        Ok(())
    }
    pub fn progress(&mut self, id: &str, progress: Progress) -> io::Result<()> {
        if !progress.seconds.is_finite()
            || progress.seconds < 0.
            || progress.total.is_some_and(|t| progress.frames > t)
        {
            return Err(error("Invalid measured export progress"));
        }
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| error("Unknown export job"))?;
        if job.result.is_none() {
            job.progress = Some(progress);
        }
        Ok(())
    }
    pub fn finish(&mut self, id: &str, result: Result<Value, String>) -> io::Result<()> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| error("Unknown export job"))?;
        if job.result.is_none() {
            job.result = Some(result);
        }
        Ok(())
    }
    /// A connection disconnect does not call this: the export outlives a caller.
    pub fn owner_lost(&mut self, owner: &Owner) {
        for job in self
            .jobs
            .values_mut()
            .filter(|j| j.owner == *owner && j.result.is_none())
        {
            job.result=Some(Err("The native document host closed while exporting. The output outcome is unknown; inspect it before retrying.".into()));
        }
    }
    pub fn poll(&mut self, owner: &Owner, id: &str, now: u64) -> io::Result<Reply> {
        let job = self
            .jobs
            .get_mut(id)
            .filter(|j| j.owner == *owner)
            .ok_or_else(|| {
                error(
                    "Unknown export job. A finished job is reported once; check the export folder.",
                )
            })?;
        if now.saturating_sub(job.started) >= JOB_LIMIT_MS && job.result.is_none() {
            job.result = Some(Err(
                "The export exceeded its job deadline. Inspect its output before retrying.".into(),
            ));
        }
        if let Some(result) = &job.result {
            return Ok(Reply::Terminal(result.clone()));
        }
        let mut result = json!({"status":"running","jobId":id,"next":format!("The export continues in Powermove. Call export_video with {{\"jobId\":\"{id}\"}} to keep waiting for its file.")});
        if let Some(progress) = &job.progress {
            let mut value = json!({"frames":progress.frames});
            if let Some(total) = progress.total.filter(|t| *t > 0) {
                value["totalFrames"] = json!(total);
                if progress.frames > 0 && progress.frames < total {
                    value["secondsRemaining"] = json!(
                        (progress.seconds * (total - progress.frames) as f64
                            / progress.frames as f64)
                            .round() as u64
                    );
                }
            }
            if let Some(encoder) = progress.encoder.as_ref().filter(|s| !s.is_empty()) {
                value["encoder"] = json!(encoder);
            }
            result["progress"] = value;
        }
        Ok(Reply::Pending(result))
    }
    /// Durable archive/response-acceptance precedes retirement. Failed delivery
    /// can poll the same terminal result again instead of losing a finished file.
    pub fn acknowledge(
        &mut self,
        owner: &Owner,
        id: &str,
        archive: impl FnOnce(&Result<Value, String>) -> io::Result<()>,
    ) -> io::Result<()> {
        let job = self
            .jobs
            .get(id)
            .filter(|j| j.owner == *owner)
            .ok_or_else(|| error("Unknown export job"))?;
        let result = job
            .result
            .as_ref()
            .ok_or_else(|| error("Export is still running"))?;
        archive(result)?;
        self.jobs.remove(id);
        Ok(())
    }
}
