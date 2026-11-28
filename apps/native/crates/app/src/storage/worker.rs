use super::maintenance::{self, Bucket, CATEGORIES, RECEIPT};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const QUIET: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_secs(5);
const INTERVAL: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
struct Requests {
    queued: BTreeSet<Bucket>,
    running: BTreeSet<Bucket>,
    wake: Option<async_channel::Sender<()>>,
}
static REQUESTS: Mutex<Requests> = Mutex::new(Requests {
    queued: BTreeSet::new(),
    running: BTreeSet::new(),
    wake: None,
});

pub(super) fn request(bucket: Bucket) {
    let mut requests = REQUESTS.lock().unwrap();
    requests.queued.insert(bucket);
    if let Some(wake) = &requests.wake {
        let _ = wake.try_send(());
    }
}

pub(super) fn status(bucket: Bucket) -> Option<&'static str> {
    let requests = REQUESTS.lock().unwrap();
    if requests.running.contains(&bucket) {
        Some("Cleaning…")
    } else if requests.queued.contains(&bucket) {
        Some("Waiting for Idle")
    } else {
        None
    }
}

fn exports_clear() -> bool {
    !pm_engine_service::jobs::exports_running()
        && !crate::export_coordinator::holding_export_inputs()
}

fn idle() -> bool {
    exports_clear() && pm_engine_service::jobs::app_quiet_for(QUIET)
}

pub(super) fn first_reclaimed(profile: &Path) -> Option<u64> {
    let receipt = profile.join(RECEIPT);
    if !fs::symlink_metadata(&receipt).ok()?.is_file() {
        return None;
    }
    fs::read_to_string(receipt).ok()?.trim().parse().ok()
}

pub(super) fn record_first(profile: &Path, bytes: u64) -> std::io::Result<()> {
    let mut receipt = tempfile::NamedTempFile::new_in(profile)?;
    writeln!(receipt, "{bytes}")?;
    receipt.as_file().sync_all()?;
    receipt.persist(profile.join(RECEIPT))?;
    fs::File::open(profile)?.sync_all()
}

/// The receipt uses the same disk-block measurement as Settings › Storage.
pub(super) fn reclaim(
    profile: &Path,
    measure: bool,
    cleanup: impl FnOnce() -> anyhow::Result<u64>,
) -> anyhow::Result<u64> {
    let before = measure.then(|| super::used(profile));
    let removed = cleanup()?;
    Ok(before.map_or(removed, |before| {
        before.saturating_sub(super::used(profile))
    }))
}

pub(crate) fn start(profile: PathBuf, cx: &mut gpui::App) {
    crate::media::use_idle_retention();
    let (wake_tx, wake_rx) = async_channel::bounded(1);
    REQUESTS.lock().unwrap().wake = Some(wake_tx.clone());
    let bg = cx.background_executor().clone();
    let tick_bg = bg.clone();
    bg.spawn(async move {
        loop {
            tick_bg.timer(POLL).await;
            if wake_tx.is_closed() {
                break;
            }
            let _ = wake_tx.try_send(());
        }
    })
    .detach();
    let mut last = None::<Instant>;
    let mut initial_freed = 0u64;
    cx.spawn(async move |cx| {
        while wake_rx.recv().await.is_ok() {
            let has_manual = !REQUESTS.lock().unwrap().queued.is_empty();
            let automatic = idle() && last.is_none_or(|at| at.elapsed() >= INTERVAL);
            let manual_ready = has_manual && exports_clear();
            if !manual_ready && !automatic {
                continue;
            }
            let snapshot = cx.update(|cx| {
                let editors = crate::entry::editors(cx);
                if editors.iter().any(|editor| {
                    let editor = editor.read(cx);
                    editor.opening || editor.session.is_some() && !editor.media_resolved
                }) {
                    return None;
                }
                let pinned = editors
                    .iter()
                    .map(|editor| editor.read(cx))
                    .flat_map(|editor| {
                        editor
                            .media
                            .originals
                            .values()
                            .chain(editor.media.proxies.values())
                            .chain(editor.media.optimized.values())
                            .cloned()
                            .chain(editor.path.iter().cloned())
                    })
                    .collect();
                Some(pinned)
            });
            let Some(pinned) = snapshot else {
                continue;
            };
            let (buckets, manual_cache) = {
                let mut requests = REQUESTS.lock().unwrap();
                let manual_buckets = if manual_ready {
                    std::mem::take(&mut requests.queued)
                } else {
                    BTreeSet::new()
                };
                let manual_cache = manual_buckets.contains(&Bucket::Cache);
                let mut buckets = manual_buckets;
                if automatic {
                    buckets.extend(CATEGORIES.iter().map(|category| category.bucket));
                }
                requests.running.clone_from(&buckets);
                (buckets, manual_cache)
            };
            if buckets.is_empty() {
                continue;
            }
            // Show "Cleaning…" immediately when the user clicked Clean Up Now.
            if manual_ready {
                cx.update(|cx| {
                    if let Some(editor) = crate::entry::editors(cx)
                        .into_iter()
                        .find(|editor| editor.read(cx).builtins.windows.contains_key("settings"))
                    {
                        editor.update(cx, |controller, cx| controller.update_builtins(cx));
                    }
                });
            }
            let result = cx
                .background_executor()
                .spawn({
                    let profile = profile.clone();
                    let buckets = buckets.clone();
                    async move {
                        let first = first_reclaimed(&profile).is_none();
                        // An explicit "Clean Up Now" on Media Cache clears all
                        // unpinned regenerable cache; background sweeps keep up
                        // to the configured cache limit.
                        let limit = if manual_cache {
                            0
                        } else {
                            crate::pro_media::stored_cache_limit(&profile)
                        };
                        let keep_going = || {
                            if manual_ready {
                                exports_clear()
                            } else {
                                idle()
                            }
                        };
                        let freed = reclaim(&profile, first, || {
                            maintenance::clean(&profile, &buckets, limit, &pinned, &keep_going)
                        })?;
                        let complete = keep_going();
                        let cumulative = initial_freed.saturating_add(freed);
                        if first && automatic && complete {
                            record_first(&profile, cumulative)?;
                        }
                        Ok::<_, anyhow::Error>((freed, first, complete))
                    }
                })
                .await;
            let refresh_cache = buckets.contains(&Bucket::Cache)
                && result.as_ref().is_ok_and(|(freed, _, _)| *freed > 0);
            {
                let mut requests = REQUESTS.lock().unwrap();
                requests.running.clear();
                if matches!(&result, Ok((_, _, false))) {
                    requests.queued.extend(buckets);
                }
            }
            match result {
                Ok((freed, first, complete)) => {
                    if first {
                        initial_freed = initial_freed.saturating_add(freed);
                    }
                    if complete && automatic {
                        last = Some(Instant::now());
                    }
                    if first && automatic && complete && initial_freed > 0 {
                        crate::error_log::storage_cleanup(initial_freed);
                    }
                    if freed > 0 {
                        crate::diag::line(serde_json::json!({
                            "storageCleanup": true,
                            "freedBytes": freed,
                            "initial": first,
                        }));
                    }
                }
                Err(error) => {
                    eprintln!("Storage cleanup: {error}");
                    if automatic {
                        last = Some(Instant::now());
                    }
                }
            }
            cx.update(|cx| {
                let editors = crate::entry::editors(cx);
                if refresh_cache {
                    for editor in &editors {
                        editor.read(cx).pro_media.refresh_after_cleanup();
                    }
                }
                if let Some(editor) = editors
                    .into_iter()
                    .find(|editor| editor.read(cx).builtins.windows.contains_key("settings"))
                {
                    editor.update(cx, |controller, cx| controller.refresh_storage(cx));
                }
            });
        }
    })
    .detach();
}
