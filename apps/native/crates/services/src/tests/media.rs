use super::test_storage;
use crate::error::Result;
use crate::service_bail as bail;
use crate::{media, storage};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn request(asset: &str) -> media::ImportRequest {
    media::ImportRequest {
        project: "project".into(),
        asset: asset.into(),
        source: PathBuf::from("/synthetic/media.mov"),
        provenance: media::Provenance::ProxyOnly,
    }
}
fn probe() -> media::Probe {
    media::Probe {
        content_sha256: storage::digest(b"synthetic"),
        bytes: 9,
        duration_ticks: 1001,
        ticks_per_second: 24000,
        width: 1920,
        height: 1080,
        audio_channels: 2,
    }
}
#[derive(Default)]
struct MediaDouble {
    admitted: Vec<(String, media::Provenance)>,
    fail: bool,
}
impl media::MediaHook for MediaDouble {
    fn probe(&mut self, _: &Path) -> Result<media::Probe> {
        Ok(probe())
    }
    fn admit(&mut self, job: &str, req: &media::ImportRequest, _: &media::Probe) -> Result<()> {
        if self.fail {
            bail!("kernel import refused")
        }
        self.admitted.push((job.into(), req.provenance.clone()));
        Ok(())
    }
}
#[test]
fn import_fifo_and_idempotent_job_ids() {
    let mut queue = media::ImportQueue::default();
    queue.enqueue("z", request("asset1")).unwrap();
    queue.enqueue("a", request("asset2")).unwrap();
    queue.enqueue("z", request("asset1")).unwrap();
    assert!(queue.enqueue("z", request("other")).is_err());
    assert_eq!(queue.jobs().count(), 2);
    assert_eq!(queue.start_next().unwrap().unwrap().id, "z");
    assert_eq!(queue.start_next().unwrap().unwrap().id, "a");
    assert!(queue.start_next().unwrap().is_none());
}
#[test]
fn cancelled_and_retried_import_rejects_stale_completion() {
    let mut queue = media::ImportQueue::default();
    queue.enqueue("job", request("asset")).unwrap();
    let old = queue.start_next().unwrap().unwrap();
    queue.cancel("job").unwrap();
    queue.retry("job").unwrap();
    let new = queue.start_next().unwrap().unwrap();
    let mut host = MediaDouble::default();
    assert!(queue.finish(old, Ok(probe()), &mut host).is_err());
    assert!(host.admitted.is_empty());
    queue.finish(new, Ok(probe()), &mut host).unwrap();
    assert_eq!(host.admitted[0].1, media::Provenance::ProxyOnly);
    assert!(queue.cancel("job").is_err());
}
#[test]
fn failed_probe_or_admission_is_retryable_without_false_success() {
    let mut queue = media::ImportQueue::default();
    queue.enqueue("job", request("asset")).unwrap();
    let ticket = queue.start_next().unwrap().unwrap();
    let mut host = MediaDouble::default();
    assert!(
        queue
            .finish(
                ticket,
                Err(anyhow::anyhow!("probe failed").into()),
                &mut host
            )
            .is_err()
    );
    assert_eq!(queue.get("job").unwrap().state, media::ImportState::Failed);
    queue.retry("job").unwrap();
    let ticket = queue.start_next().unwrap().unwrap();
    host.fail = true;
    assert!(queue.finish(ticket, Ok(probe()), &mut host).is_err());
    assert!(host.admitted.is_empty());
}
#[test]
fn restart_requeues_inflight_and_retains_cancelled_jobs() {
    let (_temp, mut storage) = test_storage();
    let mut queue = media::ImportQueue::default();
    queue.enqueue("running", request("asset1")).unwrap();
    queue.enqueue("cancelled", request("asset2")).unwrap();
    let old = queue.start_next().unwrap().unwrap();
    queue.cancel("cancelled").unwrap();
    queue.persist(&mut storage).unwrap();
    let mut restored = media::ImportQueue::restore(&storage).unwrap();
    assert_eq!(
        restored.get("cancelled").unwrap().state,
        media::ImportState::Cancelled
    );
    let new = restored.start_next().unwrap().unwrap();
    assert!(new.attempt > old.attempt);
    assert!(
        restored
            .finish(old, Ok(probe()), &mut MediaDouble::default())
            .is_err()
    );
}
#[test]
fn invalid_probe_and_relative_source_are_refused() {
    let mut invalid = probe();
    invalid.ticks_per_second = 0;
    assert!(invalid.validate().is_err());
    invalid = probe();
    invalid.content_sha256 = "bad".into();
    assert!(invalid.validate().is_err());
    let mut req = request("asset");
    req.source = "relative".into();
    assert!(media::ImportQueue::default().enqueue("job", req).is_err());
}
fn artifact_key() -> media::ArtifactKey {
    media::ArtifactKey {
        kind: media::ArtifactKind::Thumbnail,
        source_sha256: storage::digest(b"source"),
        provenance: media::Provenance::Original,
        decoder_version: "decoder-1".into(),
        colour_pipeline: "linear-p3-v1".into(),
        time_ticks: 1001,
        ticks_per_second: 24000,
        width: 320,
        height: 180,
        channels: 2,
        sample_rate: 48000,
    }
}
#[test]
fn artifact_roundtrip_and_exact_invalidation() {
    let (_temp, mut storage) = test_storage();
    let cache = media::ArtifactCache::new(1024).unwrap();
    let key = artifact_key();
    cache.put(&mut storage, key.clone(), vec![1, 2, 3]).unwrap();
    assert_eq!(cache.get(&storage, &key).unwrap(), Some(vec![1, 2, 3]));
    let mut alternatives = vec![key.clone(); 7];
    alternatives[0].time_ticks += 1;
    alternatives[1].colour_pipeline = "other".into();
    alternatives[2].provenance = media::Provenance::ProxyOnly;
    alternatives[3].kind = media::ArtifactKind::Waveform;
    alternatives[4].decoder_version = "decoder-2".into();
    alternatives[5].width += 1;
    alternatives[6].source_sha256 = storage::digest(b"changed");
    for key in alternatives {
        assert!(cache.get(&storage, &key).unwrap().is_none());
    }
}
#[test]
fn cache_corruption_and_oversized_payload_are_explicit() {
    let (_temp, mut storage) = test_storage();
    let cache = media::ArtifactCache::new(3).unwrap();
    let key = artifact_key();
    assert!(cache.put(&mut storage, key.clone(), vec![0; 4]).is_err());
    cache.put(&mut storage, key.clone(), vec![1, 2, 3]).unwrap();
    let cache_id = storage::digest(&serde_json::to_vec(&key).unwrap());
    let dir = storage.directory("artifacts", &cache_id).unwrap();
    let entries = storage::generations(&dir).unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&entries[0].1).unwrap()).unwrap();
    value["bytes"] = json!([9, 9, 9]);
    fs::write(&entries[0].1, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(cache.get(&storage, &key).is_err());
}
