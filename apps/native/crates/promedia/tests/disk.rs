use pm_promedia::{cache::*, storage::*};
use std::{collections::BTreeMap, fs};
fn key() -> RenderKey {
    RenderKey {
        graph: [1; 32],
        media: [2; 32],
        ticks: 1001,
        width: 1920,
        height: 1080,
        format: "rgba16f-linear-bt709-premul".into(),
        renderer_version: "test-v1".into(),
    }
}
fn settings(root: &std::path::Path, quota: u64) -> StorageSettings {
    StorageSettings {
        locations: [
            StorageClass::Originals,
            StorageClass::Proxies,
            StorageClass::Optimized,
            StorageClass::Renders,
        ]
        .into_iter()
        .map(|c| {
            (
                c,
                Location {
                    directory: root.join(format!("{c:?}")),
                    quota_bytes: quota,
                },
            )
        })
        .collect(),
        archive: root.join("archive"),
    }
}
#[test]
fn cache_roundtrip_reopen_idempotence_and_keep_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let cache = RenderCache::open(dir.path(), 1024).unwrap();
    let k = key();
    assert_eq!(cache.state(&k, false).unwrap(), BarState::Missing);
    assert_eq!(cache.state(&k, true).unwrap(), BarState::Rendering);
    cache.put(&k, b"frame").unwrap();
    cache.put(&k, b"frame").unwrap();
    assert!(cache.put(&k, b"different").is_err());
    assert_eq!(cache.get(&k).unwrap().unwrap(), b"frame");
    cache.keep_rendered(&k, true).unwrap();
    drop(cache);
    let cache = RenderCache::open(dir.path(), 1024).unwrap();
    assert_eq!(cache.state(&k, false).unwrap(), BarState::Kept);
    cache.keep_rendered(&k, false).unwrap();
    assert_eq!(cache.state(&k, false).unwrap(), BarState::Rendered);
}
#[test]
fn every_semantic_key_input_invalidates_cache_identity() {
    let k = key();
    let mut hashes = std::collections::BTreeSet::from([k.digest().unwrap()]);
    for n in 0..7 {
        let mut other = k.clone();
        match n {
            0 => other.graph[0] += 1,
            1 => other.media[0] += 1,
            2 => other.ticks += 1,
            3 => other.width += 1,
            4 => other.height += 1,
            5 => other.format.push('x'),
            _ => other.renderer_version.push('x'),
        };
        assert!(hashes.insert(other.digest().unwrap()));
    }
    let mut bad = k;
    bad.width = 0;
    assert!(bad.digest().is_err());
}
/// The limit removes the least recently used frame: one played since it
/// was written stays, an older unplayed one goes.
#[test]
fn cache_quota_evicts_the_least_recently_used_frame() {
    let dir = tempfile::tempdir().unwrap();
    let c = RenderCache::open(dir.path(), 106).unwrap();
    let a = key();
    let mut b = a.clone();
    b.ticks += 1;
    let mut d = a.clone();
    d.ticks += 2;
    let pause = || std::thread::sleep(std::time::Duration::from_millis(20));
    c.put(&a, b"12345").unwrap();
    pause();
    c.put(&b, b"12345").unwrap();
    pause();
    c.touch(&a).unwrap();
    pause();
    c.put(&d, b"12345").unwrap();
    assert!(c.get(&a).unwrap().is_some(), "played: kept");
    assert!(c.get(&b).unwrap().is_none(), "least recently used: removed");
    assert!(c.get(&d).unwrap().is_some());
}
#[test]
fn cache_quota_evicts_only_unkept_derived_frames() {
    let dir = tempfile::tempdir().unwrap();
    let c = RenderCache::open(dir.path(), 106).unwrap();
    let a = key();
    let mut b = a.clone();
    b.ticks += 1;
    let mut d = a.clone();
    d.ticks += 2;
    c.put(&a, b"12345").unwrap();
    c.keep_rendered(&a, true).unwrap();
    c.put(&b, b"12345").unwrap();
    c.put(&d, b"12345").unwrap();
    assert!(c.get(&a).unwrap().is_some());
    assert!(c.get(&b).unwrap().is_none());
    assert!(c.get(&d).unwrap().is_some());
    c.keep_rendered(&d, true).unwrap();
    assert!(c.trim_to(0).is_err());
    assert!(c.get(&a).unwrap().is_some());
    assert!(c.get(&d).unwrap().is_some());
}
#[test]
fn cache_refuses_oversized_entry_and_missing_keep() {
    let dir = tempfile::tempdir().unwrap();
    let c = RenderCache::open(dir.path(), 50).unwrap();
    assert!(c.put(&key(), b"long frame").is_err());
    assert!(c.keep_rendered(&key(), true).is_err());
}
/// A damaged entry is told from a read that failed, is never returned, and
/// is no bytes at all to a writer: the frame rendered again replaces it
/// (its Keep Rendered mark stays). A sound entry is never overwritten by
/// other bytes.
#[test]
fn cache_detects_corruption_and_replaces_a_damaged_entry() {
    let dir = tempfile::tempdir().unwrap();
    let c = RenderCache::open(dir.path(), 1024).unwrap();
    let k = key();
    c.put(&k, b"frame").unwrap();
    c.keep_rendered(&k, true).unwrap();
    assert!(c.put(&k, b"other").is_err(), "a sound entry stays");
    assert_eq!(c.get(&k).unwrap().as_deref(), Some(&b"frame"[..]));
    let path = dir.path().join(format!("{}.frame", k.digest().unwrap()));
    let sound = fs::read(&path).unwrap();
    // A flipped payload byte, a cut payload, and a file cut inside its header.
    for damage in 0..3 {
        let mut bytes = sound.clone();
        match damage {
            0 => bytes[48] ^= 1,
            1 => bytes.truncate(50),
            _ => bytes.truncate(10),
        }
        fs::write(&path, &bytes).unwrap();
        let error = c.get(&k).unwrap_err();
        assert!(corrupt(&error), "{damage}: {error}");
        c.put(&k, b"frame").unwrap();
        assert_eq!(c.get(&k).unwrap().as_deref(), Some(&b"frame"[..]));
        assert_eq!(c.presence(&k).unwrap(), Some(true), "still kept");
    }
    // A read that failed for another reason is not corruption.
    assert!(!corrupt(&std::io::Error::from(
        std::io::ErrorKind::PermissionDenied
    )));
    fs::write(&path, b"damaged").unwrap();
    c.discard(&k).unwrap();
    assert!(!path.exists());
    assert_eq!(c.presence(&k).unwrap(), None);
    c.discard(&k).unwrap();
}
#[test]
fn cache_rejects_truncated_headers_lengths_and_magic() {
    for mode in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let c = RenderCache::open(dir.path(), 1024).unwrap();
        let k = key();
        c.put(&k, b"frame").unwrap();
        let p = dir.path().join(format!("{}.frame", k.digest().unwrap()));
        let mut b = fs::read(&p).unwrap();
        match mode {
            0 => b.truncate(10),
            1 => b[0] = 0,
            _ => b[40] = 255,
        };
        fs::write(&p, b).unwrap();
        assert!(c.get(&k).is_err());
    }
}
#[test]
fn foreign_directory_and_partial_output_are_never_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("foreign");
    fs::create_dir(&p).unwrap();
    fs::write(p.join("precious"), b"keep").unwrap();
    assert!(RenderCache::open(&p, 1024).is_err());
    assert_eq!(fs::read(p.join("precious")).unwrap(), b"keep");
    let root = dir.path().join("cache");
    let c = RenderCache::open(&root, 1024).unwrap();
    fs::write(root.join(".partial-interrupted"), b"unfinished").unwrap();
    c.trim_to(0).unwrap();
    assert_eq!(
        fs::read(root.join(".partial-interrupted")).unwrap(),
        b"unfinished"
    );
    assert!(c.get(&key()).unwrap().is_none());
}
#[test]
fn bar_model_merges_only_adjacent_equal_ranges() {
    let b = render_bar(&[
        (0, 10, BarState::Rendered),
        (10, 20, BarState::Rendered),
        (20, 30, BarState::Rendering),
        (40, 50, BarState::Rendered),
    ])
    .unwrap();
    assert_eq!(b.len(), 3);
    assert_eq!(b[0].end_tick, 20);
    for ranges in [
        vec![(0, 0, BarState::Missing)],
        vec![(-1, 1, BarState::Missing)],
        vec![(0, 10, BarState::Missing), (9, 20, BarState::Kept)],
    ] {
        assert!(render_bar(&ranges).is_err());
    }
}
#[test]
fn storage_settings_validate_locations_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), 10);
    s.validate().unwrap();
    assert_eq!(
        serde_json::from_slice::<StorageSettings>(&serde_json::to_vec(&s).unwrap()).unwrap(),
        s
    );
    let mut overlap = s.clone();
    overlap.archive = overlap.locations[&StorageClass::Originals]
        .directory
        .clone();
    assert!(overlap.validate().is_err());
    let mut relative = s.clone();
    relative.archive = "relative".into();
    assert!(relative.validate().is_err());
    let mut missing = s;
    missing.locations = BTreeMap::new();
    assert!(missing.validate().is_err());
}
#[test]
fn storage_quota_never_prunes_to_make_room() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(settings(dir.path(), 5)).unwrap();
    let p = store
        .put(StorageClass::Originals, "a.mov", b"12345")
        .unwrap();
    assert!(store.put(StorageClass::Originals, "b.mov", b"x").is_err());
    assert_eq!(fs::read(&p).unwrap(), b"12345");
    assert_eq!(
        store.usage(StorageClass::Originals).unwrap().active_bytes,
        5
    );
}
#[test]
fn purge_archives_original_bytes_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), 100);
    let store = Storage::open(s.clone()).unwrap();
    let original = store
        .put(StorageClass::Originals, "source.mov", b"original bytes")
        .unwrap();
    let archived = store
        .purge(StorageClass::Originals, "source.mov", false)
        .unwrap();
    assert!(!original.exists());
    assert_eq!(fs::read(&archived).unwrap(), b"original bytes");
    drop(store);
    let store = Storage::open(s).unwrap();
    assert_eq!(
        store.usage(StorageClass::Originals).unwrap().active_bytes,
        0
    );
    assert_eq!(fs::read(archived).unwrap(), b"original bytes");
}
#[test]
fn purge_preserves_history_and_proxy_only_references() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(settings(dir.path(), 100)).unwrap();
    for class in [
        StorageClass::Originals,
        StorageClass::Proxies,
        StorageClass::Optimized,
    ] {
        let p = store.put(class, "owned.mov", b"only copy").unwrap();
        assert!(store.purge(class, "owned.mov", true).is_err());
        assert_eq!(fs::read(p).unwrap(), b"only copy");
    }
}
#[test]
fn repeated_purge_keeps_every_version_without_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(settings(dir.path(), 100)).unwrap();
    store.put(StorageClass::Originals, "a", b"v1").unwrap();
    let a = store.purge(StorageClass::Originals, "a", false).unwrap();
    store.put(StorageClass::Originals, "a", b"v2").unwrap();
    let b = store.purge(StorageClass::Originals, "a", false).unwrap();
    assert_ne!(a, b);
    assert_eq!(fs::read(a).unwrap(), b"v1");
    assert_eq!(fs::read(b).unwrap(), b"v2");
}
#[test]
fn invalid_object_paths_and_existing_names_never_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(settings(dir.path(), 100)).unwrap();
    for name in ["", "..", "../escape", "a/b", ".hidden", "/absolute", "a\\b"] {
        assert!(
            store.put(StorageClass::Originals, name, b"x").is_err(),
            "{name}"
        );
    }
    let p = store.put(StorageClass::Originals, "a", b"first").unwrap();
    assert!(store.put(StorageClass::Originals, "a", b"second").is_err());
    assert_eq!(fs::read(p).unwrap(), b"first");
}
#[cfg(unix)]
#[test]
fn symlink_roots_payloads_and_markers_refuse() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = d.path().join("link");
    symlink(&target, &link).unwrap();
    assert!(RenderCache::open(&link, 1000).is_err());
    let root = d.path().join("cache");
    let c = RenderCache::open(&root, 1000).unwrap();
    let payload = d.path().join("original");
    fs::write(&payload, b"precious").unwrap();
    symlink(
        &payload,
        root.join(format!("{}.frame", key().digest().unwrap())),
    )
    .unwrap();
    assert!(c.get(&key()).is_err());
    assert!(c.trim_to(0).is_err());
    assert_eq!(fs::read(&payload).unwrap(), b"precious");
    let pins = d.path().join("pins");
    let pinned = RenderCache::open(&pins, 1000).unwrap();
    pinned.put(&key(), b"render").unwrap();
    symlink(
        &payload,
        pins.join(format!("{}.keep", key().digest().unwrap())),
    )
    .unwrap();
    assert!(pinned.state(&key(), false).is_err());
    assert!(pinned.trim_to(0).is_err());
    assert_eq!(fs::read(&payload).unwrap(), b"precious");
    let s = settings(&d.path().join("storage"), 100);
    let store = Storage::open(s).unwrap();
    let managed = store.path(StorageClass::Originals, "a").unwrap();
    symlink(&payload, &managed).unwrap();
    assert!(store.purge(StorageClass::Originals, "a", false).is_err());
    assert_eq!(fs::read(payload).unwrap(), b"precious");
}

#[test]
fn reducing_quota_does_not_make_kept_frames_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let c = RenderCache::open(dir.path(), 1024).unwrap();
    c.put(&key(), b"large kept frame").unwrap();
    c.keep_rendered(&key(), true).unwrap();
    drop(c);
    let c = RenderCache::open(dir.path(), 1).unwrap();
    assert_eq!(c.state(&key(), false).unwrap(), BarState::Kept);
    assert!(c.trim_to(1).is_err());
    assert_eq!(c.get(&key()).unwrap().unwrap(), b"large kept frame");
}
#[test]
fn storage_and_render_cache_compose_without_eviction_of_originals() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(settings(dir.path(), 1024)).unwrap();
    let original = store
        .put(StorageClass::Originals, "precious.frame", b"source bytes")
        .unwrap();
    let cache = store.render_cache().unwrap();
    cache.put(&key(), b"render").unwrap();
    assert_eq!(store.usage(StorageClass::Renders).unwrap().active_bytes, 54);
    cache.trim_to(0).unwrap();
    assert_eq!(store.usage(StorageClass::Renders).unwrap().active_bytes, 0);
    assert_eq!(fs::read(original).unwrap(), b"source bytes");
}
#[cfg(unix)]
#[test]
fn failed_archive_publication_leaves_original_intact() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), 100);
    let store = Storage::open(s.clone()).unwrap();
    let p = store.put(StorageClass::Originals, "a", b"keep").unwrap();
    fs::rename(&s.archive, dir.path().join("archive-old")).unwrap();
    symlink(dir.path().join("archive-old"), &s.archive).unwrap();
    assert!(store.purge(StorageClass::Originals, "a", false).is_err());
    assert_eq!(fs::read(p).unwrap(), b"keep");
}

#[test]
fn render_bar_frames_uses_exact_composition_rate_and_exclusive_end() {
    use pm_time::Rate;
    for rate in [Rate::FPS_24, Rate::FPS_29_97, Rate::new(17, 3).unwrap()] {
        let segments = render_bar(&[
            (0, rate.frame_start(12).unwrap(), BarState::Rendered),
            (
                rate.frame_start(12).unwrap(),
                rate.frame_start(14).unwrap(),
                BarState::Rendering,
            ),
        ])
        .unwrap();
        assert_eq!(
            render_bar_frames(&segments, rate).unwrap(),
            vec![(0, 12, BarState::Rendered), (12, 14, BarState::Rendering)]
        );
    }
}

#[test]
fn render_bar_frames_assigns_each_frame_start_once_even_between_boundaries() {
    let rate = pm_time::Rate::FPS_30;
    let segments = render_bar(&[
        (1, 2, BarState::Missing),
        (2, rate.frame_start(3).unwrap() + 1, BarState::Rendering),
        (
            rate.frame_start(3).unwrap() + 1,
            rate.frame_start(5).unwrap(),
            BarState::Kept,
        ),
    ])
    .unwrap();
    assert_eq!(
        render_bar_frames(&segments, rate).unwrap(),
        vec![(1, 4, BarState::Rendering), (4, 5, BarState::Kept)]
    );
    assert!(
        render_bar_frames(
            &[RenderBar {
                first_tick: 2,
                end_tick: 1,
                state: BarState::Missing
            }],
            rate
        )
        .is_err()
    );
    let max = render_bar(&[(i64::MAX - 1, i64::MAX, BarState::Missing)]).unwrap();
    assert_eq!(
        render_bar_frames(
            &max,
            pm_time::Rate::new(pm_time::TICKS_PER_SECOND, 1).unwrap()
        )
        .unwrap(),
        vec![(i64::MAX - 1, i64::MAX, BarState::Missing)]
    );
}

#[test]
fn graph_edit_invalidates_render_bar_immediately_without_trimming_kept_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let old = key();
    cache.put(&old, b"authored frame").unwrap();
    for kept in [false, true] {
        cache.keep_rendered(&old, kept).unwrap();
        let mut edited = old.clone();
        edited.graph[0] ^= 1;
        assert_eq!(cache.state(&edited, false).unwrap(), BarState::Missing);
        assert_eq!(cache.state(&edited, true).unwrap(), BarState::Rendering);
        assert_eq!(cache.get(&old).unwrap(), Some(b"authored frame".to_vec()));
    }
}
