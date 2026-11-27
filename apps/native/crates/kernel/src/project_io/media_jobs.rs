//! Conversion consumer bookkeeping from main c7f46a056. The media worker owns
//! execution; this id-driven model coalesces only unchanged source identities,
//! gives every caller its own output, and cancels when its final consumer exits.
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceIdentity {
    /// Versioned conversion recipe/options, independent of source identity.
    pub recipe: String,
    pub path: PathBuf,
    pub size: u64,
    pub modified_ns: i128,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consumer {
    pub id: u64,
    pub owner: u64,
    pub output: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: u64,
    pub source: SourceIdentity,
    pub consumers: Vec<Consumer>,
}
#[derive(Debug, Default)]
pub struct ConversionQueue {
    jobs: BTreeMap<u64, Job>,
    serial: u64,
}
impl ConversionQueue {
    /// Returns (worker job id, new job). Admission must verify the identity
    /// before calling; a changed/moved source is a different job. When a source
    /// is unavailable, retain supplied owned bytes rather than rejecting open.
    pub fn attach(
        &mut self,
        source: SourceIdentity,
        consumer: Consumer,
    ) -> Result<(u64, bool), &'static str> {
        if self
            .jobs
            .values()
            .flat_map(|j| &j.consumers)
            .any(|c| c.id == consumer.id || c.output == consumer.output)
        {
            return Err("conversion consumer or output already exists");
        }
        if let Some(job) = self.jobs.values_mut().find(|j| j.source == source) {
            job.consumers.push(consumer);
            return Ok((job.id, false));
        }
        self.serial += 1;
        let id = self.serial;
        self.jobs.insert(
            id,
            Job {
                id,
                source,
                consumers: vec![consumer],
            },
        );
        Ok((id, true))
    }
    pub fn job(&self, id: u64) -> Option<&Job> {
        self.jobs.get(&id)
    }
    /// Job ids whose worker must now be cancelled. Other consumers keep their
    /// output capabilities and can complete normally.
    pub fn owner_exited(&mut self, owner: u64) -> Vec<u64> {
        let mut cancel = Vec::new();
        for (id, job) in &mut self.jobs {
            job.consumers.retain(|c| c.owner != owner);
            if job.consumers.is_empty() {
                cancel.push(*id);
            }
        }
        for id in &cancel {
            self.jobs.remove(id);
        }
        cancel
    }
    pub fn detach(&mut self, consumer: u64) -> Option<u64> {
        let job = self
            .jobs
            .values_mut()
            .find(|j| j.consumers.iter().any(|c| c.id == consumer))?;
        job.consumers.retain(|c| c.id != consumer);
        if !job.consumers.is_empty() {
            return None;
        }
        let id = job.id;
        self.jobs.remove(&id);
        Some(id)
    }
    /// The worker publishes separate verified copies for these outputs. This
    /// method never reassigns or deletes any consumer's existing bytes.
    pub fn complete(&mut self, job: u64) -> Option<Vec<Consumer>> {
        self.jobs.remove(&job).map(|j| j.consumers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> SourceIdentity {
        SourceIdentity {
            recipe: "playback-v1".into(),
            path: "synthetic.mov".into(),
            size: 7,
            modified_ns: 1,
            sha256: "synthetic-full-digest".into(),
        }
    }
    fn consumer(id: u64, owner: u64) -> Consumer {
        Consumer {
            id,
            owner,
            output: format!("output-{id}.mov").into(),
        }
    }
    #[test]
    fn unchanged_conversion_joins_but_every_consumer_has_its_own_output() {
        let mut queue = ConversionQueue::default();
        let (id, fresh) = queue.attach(source(), consumer(1, 1)).unwrap();
        assert!(fresh);
        assert_eq!(queue.attach(source(), consumer(2, 2)).unwrap(), (id, false));
        let outputs = queue.complete(id).unwrap();
        assert_eq!(outputs.len(), 2);
        assert_ne!(outputs[0].output, outputs[1].output);
    }
    #[test]
    fn changed_source_and_last_consumer_or_owner_exit_preserve_other_jobs() {
        let mut queue = ConversionQueue::default();
        let (first, _) = queue.attach(source(), consumer(1, 1)).unwrap();
        queue.attach(source(), consumer(2, 2)).unwrap();
        let mut changed = source();
        changed.modified_ns += 1;
        let (second, fresh) = queue.attach(changed, consumer(3, 1)).unwrap();
        assert!(fresh);
        assert_ne!(first, second);
        assert_eq!(queue.owner_exited(1), vec![second]);
        assert_eq!(queue.job(first).unwrap().consumers.len(), 1);
        assert_eq!(queue.detach(2), Some(first));
        assert!(queue.job(first).is_none());
        assert!(queue.complete(second).is_none());
    }
}
