//! Image sequences read in place, observed off every frame thread.
//!
//! A sequence's asset retains a small manifest that never changes; its
//! pictures are its numbered files, which another application may render
//! again at any time. One worker thread looks at the files of every tracked
//! sequence (every two seconds, or at once when asked), and publishes a
//! snapshot of each sequence's member generation: a digest of each numbered
//! file's length, modification and change times, inode and device (a
//! missing number counts too). Frame threads read the snapshot and never
//! look at the files themselves; listeners hear which sequences changed.
//!
//! The same [`generation`] identifies a sequence's renditions in the app,
//! so a proxy and the render cache follow one view of the files.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use pm_media::image::sequence::Manifest;
use pm_text::file_version::FileVersion;

/// How often tracked sequences are looked at again without being asked
/// (files on network volumes send no change events).
const RECONCILE: Duration = Duration::from_secs(2);

/// One numbered file's identity: its version (`pm_text::file_version`,
/// the identity media files and rendered frames are named by: length,
/// modification and change times, inode, device). `None`: no file.
pub type MemberStamp = Option<FileVersion>;

/// What the watch looks at a file with (a test stands in for a slow volume).
pub type Stat = Arc<dyn Fn(&Path) -> MemberStamp + Send + Sync>;

/// A file's [`MemberStamp`] from the file system.
pub fn member_stamp(path: &Path) -> MemberStamp {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| FileVersion::read(&m))
}

/// The member generation of `sequence`: a digest of every numbered file's
/// version, from the first number to the last.
pub fn generation(sequence: &Manifest) -> u64 {
    digest(&members_with(sequence, &member_stamp))
}

/// Each numbered file and its version (the empty version for a missing
/// number), first to last.
fn members_with(
    sequence: &Manifest,
    stat: &dyn Fn(&Path) -> MemberStamp,
) -> Vec<(PathBuf, FileVersion)> {
    (sequence.first..=sequence.last)
        .map(|n| {
            let file = sequence.file(n);
            let version = stat(&file).unwrap_or_default();
            (file, version)
        })
        .collect()
}

fn digest(members: &[(PathBuf, FileVersion)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    members.hash(&mut hash);
    hash.finish()
}

/// What the watch last published.
#[derive(Default)]
pub struct Snapshot {
    /// Rises whenever a tracked sequence's generation changes.
    pub version: u64,
    /// Each tracked sequence's manifest path and member generation.
    pub generations: HashMap<PathBuf, u64>,
    /// Each tracked sequence's numbered files and their versions: what a
    /// frame of it reads, as keys name it (`crate::render_cache::stamps`).
    pub members: HashMap<PathBuf, Arc<Vec<(PathBuf, FileVersion)>>>,
}

/// Called on the watch's thread with the manifests whose files changed;
/// answers false once it no longer listens.
type Listener = Box<dyn Fn(&[PathBuf]) -> bool + Send>;

#[derive(Default)]
struct State {
    /// Each owner's tracked files (any original: those that are sequence
    /// manifests are watched).
    tracked: BTreeMap<String, BTreeSet<PathBuf>>,
    snapshot: Arc<Snapshot>,
    /// A look is asked for (coalesced: at most one waits).
    asked: bool,
    started: bool,
    listeners: Vec<Listener>,
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
    stat: Stat,
}

/// The sequence watch (cheap to clone; [`global`] is the process's).
#[derive(Clone)]
pub struct SequenceWatch(Arc<Inner>);

/// The process's watch.
pub fn global() -> &'static SequenceWatch {
    static WATCH: OnceLock<SequenceWatch> = OnceLock::new();
    WATCH.get_or_init(|| SequenceWatch::with_stat(Arc::new(member_stamp)))
}

impl SequenceWatch {
    pub fn with_stat(stat: Stat) -> Self {
        Self(Arc::new(Inner {
            state: Mutex::default(),
            wake: Condvar::new(),
            stat,
        }))
    }

    /// Sets `owner`'s tracked files (an empty set stops tracking them); new
    /// ones are looked at now.
    pub fn track(&self, owner: &str, files: impl IntoIterator<Item = PathBuf>) {
        let files: BTreeSet<PathBuf> = files.into_iter().collect();
        let mut state = self.0.state.lock().unwrap();
        let known = state
            .snapshot
            .generations
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let new = files.iter().any(|f| !known.contains(f));
        if files.is_empty() {
            state.tracked.remove(owner);
        } else {
            state.tracked.insert(owner.to_owned(), files);
        }
        if !state.started {
            state.started = true;
            let inner = Arc::downgrade(&self.0);
            let _ = std::thread::Builder::new()
                .name("pm-sequence-watch".into())
                .spawn(move || run(inner));
        }
        if new {
            state.asked = true;
            self.0.wake.notify_all();
        }
    }

    /// Asks for a look now (a folder event, or a test); coalesced.
    pub fn rescan(&self) {
        self.0.state.lock().unwrap().asked = true;
        self.0.wake.notify_all();
    }

    /// The last published snapshot. Never waits for a look.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.0.state.lock().unwrap().snapshot.clone()
    }

    /// The last published version.
    pub fn version(&self) -> u64 {
        self.0.state.lock().unwrap().snapshot.version
    }

    /// Hears the manifests whose files changed, on the watch's thread.
    pub fn listen(&self, listener: impl Fn(&[PathBuf]) -> bool + Send + 'static) {
        self.0
            .state
            .lock()
            .unwrap()
            .listeners
            .push(Box::new(listener));
    }
}

/// The watch's thread: looks, publishes, tells, waits.
fn run(inner: std::sync::Weak<Inner>) {
    // A manifest is read once per state of its own file.
    let mut manifests: HashMap<PathBuf, (MemberStamp, Option<Manifest>)> = HashMap::new();
    loop {
        let Some(watch) = inner.upgrade() else {
            return;
        };
        let files: BTreeSet<PathBuf> = {
            let mut state = watch.state.lock().unwrap();
            state.asked = false;
            state.tracked.values().flatten().cloned().collect()
        };
        let mut generations = HashMap::new();
        let mut members = HashMap::new();
        for file in &files {
            let stamp = (watch.stat)(file);
            let entry = manifests.entry(file.clone()).or_insert((None, None));
            if entry.0 != stamp || stamp.is_none() {
                *entry = (stamp, Manifest::read(file).ok().flatten());
            }
            if let Some(sequence) = &entry.1 {
                let files = members_with(sequence, &|p| (watch.stat)(p));
                generations.insert(file.clone(), digest(&files));
                members.insert(file.clone(), Arc::new(files));
            }
        }
        manifests.retain(|f, _| files.contains(f));
        let mut state = watch.state.lock().unwrap();
        let before = &state.snapshot.generations;
        let changed: Vec<PathBuf> = generations
            .iter()
            .filter(|(f, g)| before.get(*f).is_some_and(|b| b != *g))
            .map(|(f, _)| f.clone())
            .collect();
        let grew = generations.keys().any(|f| !before.contains_key(f));
        if !changed.is_empty() || grew || generations.len() != before.len() {
            state.snapshot = Arc::new(Snapshot {
                version: state.snapshot.version + u64::from(!changed.is_empty() || grew),
                generations,
                members,
            });
        }
        if !changed.is_empty() {
            let listeners = std::mem::take(&mut state.listeners);
            drop(state);
            let kept: Vec<Listener> = listeners.into_iter().filter(|l| l(&changed)).collect();
            state = watch.state.lock().unwrap();
            let added = std::mem::take(&mut state.listeners);
            state.listeners = kept;
            state.listeners.extend(added);
        }
        if !state.asked {
            let (next, _) = watch.wake.wait_timeout(state, RECONCILE).unwrap();
            drop(next);
        }
    }
}

/// Tracks `manifest` for a test and waits until the global watch has
/// published the generation its files have now.
#[cfg(test)]
pub(crate) fn settle(owner: &str, manifest: &Path) {
    let sequence = Manifest::read(manifest).unwrap().unwrap();
    global().track(owner, [manifest.to_path_buf()]);
    let want = generation(&sequence);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while global().snapshot().generations.get(manifest) != Some(&want) {
        global().rescan();
        assert!(
            std::time::Instant::now() < deadline,
            "the watch never published"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(dir: &Path) -> (PathBuf, Manifest) {
        let sequence = Manifest {
            powermove_image_sequence: 1,
            dir: dir.to_owned(),
            prefix: "shot.".into(),
            suffix: ".png".into(),
            padding: 4,
            first: 1,
            last: 3,
            rate: [24, 1],
        };
        let path = dir.join("retained");
        std::fs::write(&path, sequence.to_bytes()).unwrap();
        (path, sequence)
    }

    /// A frame rewritten with the same length and its old modification
    /// time restored is another generation (its change time moved); a
    /// frame replaced by a renamed file is another (its inode), and so are
    /// a removed frame and a filled hole.
    #[test]
    fn every_kind_of_rewrite_is_another_generation() {
        let dir = tempfile::tempdir().unwrap();
        let (_, sequence) = manifest(dir.path());
        let frame = |n: i64| sequence.file(n);
        std::fs::write(frame(1), [1u8; 32]).unwrap();
        std::fs::write(frame(2), [2u8; 32]).unwrap();
        let mut seen = vec![generation(&sequence)];
        let mut changed = |what: &str, change: &dyn Fn()| {
            std::thread::sleep(Duration::from_millis(20));
            change();
            let now = generation(&sequence);
            assert!(!seen.contains(&now), "{what}");
            seen.push(now);
        };
        changed("same length, mtime restored", &|| {
            let modified = std::fs::metadata(frame(1)).unwrap().modified().unwrap();
            std::fs::write(frame(1), [9u8; 32]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(frame(1))
                .unwrap()
                .set_modified(modified)
                .unwrap();
        });
        changed("renamed over", &|| {
            std::fs::write(dir.path().join("t"), [7u8; 32]).unwrap();
            std::fs::rename(dir.path().join("t"), frame(2)).unwrap();
        });
        changed("removed", &|| std::fs::remove_file(frame(1)).unwrap());
        changed("hole filled", &|| {
            std::fs::write(frame(3), [3u8; 32]).unwrap()
        });
        assert_eq!(
            generation(&sequence),
            *seen.last().unwrap(),
            "unchanged files"
        );
    }

    /// The watch publishes generations and tells listeners which sequence
    /// changed; reading a snapshot never waits for a look, however slow
    /// the volume (a look blocked on its files).
    #[test]
    fn a_blocked_look_never_delays_a_reader() {
        let dir = tempfile::tempdir().unwrap();
        let (path, sequence) = manifest(dir.path());
        for n in 1..=3 {
            std::fs::write(sequence.file(n), [n as u8; 8]).unwrap();
        }
        let blocked = Arc::new((Mutex::new(false), Condvar::new()));
        let gate = blocked.clone();
        let watch = SequenceWatch::with_stat(Arc::new(move |p| {
            let (lock, wake) = &*gate;
            let mut held = lock.lock().unwrap();
            while *held {
                held = wake.wait(held).unwrap();
            }
            member_stamp(p)
        }));
        let (tx, rx) = std::sync::mpsc::channel();
        watch.listen(move |changed| tx.send(changed.to_vec()).is_ok());
        watch.track("test", [path.clone()]);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !watch.snapshot().generations.contains_key(&path) {
            assert!(std::time::Instant::now() < deadline, "never published");
            std::thread::sleep(Duration::from_millis(5));
        }
        let first = watch.snapshot().generations[&path];
        // The volume stops answering while a frame is rewritten.
        *blocked.0.lock().unwrap() = true;
        std::fs::write(sequence.file(2), [9u8; 8]).unwrap();
        watch.rescan();
        std::thread::sleep(Duration::from_millis(50));
        let started = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(watch.snapshot().generations[&path], first);
        }
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "a reader waited"
        );
        *blocked.0.lock().unwrap() = false;
        blocked.1.notify_all();
        let changed = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(changed, vec![path.clone()]);
        assert_ne!(watch.snapshot().generations[&path], first);
        assert_eq!(watch.snapshot().generations[&path], generation(&sequence));
    }
}
