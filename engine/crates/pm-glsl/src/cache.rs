//! Translation cache (section 11, step 6) and isolation for untrusted
//! programs (section 11: "Store GLSL is untrusted input: the pipeline is
//! fuzzed and resource-limited").
//!
//! Keys hash the fragment source, its line offset, the pipeline revision,
//! the glslang and naga versions, and a caller context (the wgpu back end and
//! driver). [`Prepared`] programs are backend-independent SPIR-V plus
//! reflection, so a disk entry survives across processes; naga's parse and
//! validation (step 5, about 0.03 ms) runs on every load, and the Metal
//! compiler's own cache (keyed by its driver) makes the pipeline itself cheap
//! after the first build.
//!
//! Translation runs on its own thread with a 1 GiB stack (a 100,000-deep
//! swizzle chain overflows the default 8 MiB, S3.md "Resource limits") and a
//! deadline. A program that misses the deadline fails with
//! [`FailureStage::Deadline`]; its thread is left to finish on its own
//! (glslang and naga cannot be interrupted) and its result is dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::pipeline::{self, Failure, FailureStage, PIPELINE_REVISION, Prepared};

/// Default translation deadline. The slowest valid program measured in S3
/// (a 256 KiB program at the statement budget) translated in 5.2 s cold,
/// including Metal; every corpus program takes under 65 ms.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(20);

/// Stack for the translation thread.
pub const TRANSLATION_STACK: usize = 1 << 30;

/// Entries kept in memory before the oldest are dropped.
const MEMORY_ENTRIES: usize = 512;

/// A cache key: SHA-256 over everything that decides the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(pub [u8; 32]);

impl Key {
    pub fn new(fragment: &str, line_offset: u32, context: &str) -> Key {
        Key::at_revision(fragment, line_offset, context, PIPELINE_REVISION)
    }

    /// The key a pipeline at `revision` gives the program (entries an
    /// earlier engine build wrote are under their own revision's keys).
    pub fn at_revision(fragment: &str, line_offset: u32, context: &str, revision: u32) -> Key {
        let mut h = Sha256::new();
        h.update(b"pm-glsl\0");
        h.update(revision.to_le_bytes());
        h.update(b"glslang 0.8.1; naga 30\0");
        h.update(context.as_bytes());
        h.update([0]);
        h.update(line_offset.to_le_bytes());
        h.update(fragment.as_bytes());
        Key(h.finalize().into())
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Runs [`pipeline::prepare`] on a large-stack thread within `deadline`.
pub fn prepare_isolated(
    fragment: &str,
    line_offset: u32,
    deadline: Duration,
) -> Result<Prepared, Failure> {
    let fragment = fragment.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("pm-glsl translate".into())
        .stack_size(TRANSLATION_STACK)
        .spawn(move || {
            let result = std::panic::catch_unwind(|| pipeline::prepare(&fragment, line_offset))
                .unwrap_or_else(|_| Err(Failure::new(FailureStage::Panic, "translation panicked")));
            let _ = tx.send(result);
        });
    if let Err(e) = spawned {
        return Err(Failure::new(
            FailureStage::Panic,
            format!("could not start the translation thread: {e}"),
        ));
    }
    match rx.recv_timeout(deadline) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(Failure::new(
            FailureStage::Deadline,
            format!(
                "translation did not finish within {} s",
                deadline.as_secs_f64()
            ),
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(Failure::new(
            FailureStage::Panic,
            "the translation thread ended without a result",
        )),
    }
}

type Entry = Arc<Result<Prepared, Failure>>;

/// Entries with their last use, and the use counter.
type Memory = (HashMap<Key, (Entry, u64)>, u64);

/// The translation cache: memory, then disk, then the pipeline.
pub struct Cache {
    context: String,
    dir: Option<PathBuf>,
    deadline: Duration,
    memory: Mutex<Memory>,
}

impl Cache {
    /// `context` names the back end and driver (`AdapterInfo`); `dir` keeps
    /// prepared programs across processes.
    pub fn new(context: impl Into<String>, dir: Option<PathBuf>) -> Cache {
        Cache {
            context: context.into(),
            dir,
            deadline: DEFAULT_DEADLINE,
            memory: Mutex::new((HashMap::new(), 0)),
        }
    }

    pub fn set_dir(&mut self, dir: Option<PathBuf>) {
        self.dir = dir;
    }

    pub fn set_deadline(&mut self, deadline: Duration) {
        self.deadline = deadline;
    }

    pub fn key(&self, fragment: &str, line_offset: u32) -> Key {
        Key::new(fragment, line_offset, &self.context)
    }

    /// The prepared program, translating it if no cache has it. Failures are
    /// remembered in memory only (a pipeline fix must be able to retry them
    /// in the next process).
    pub fn prepare(&self, fragment: &str, line_offset: u32) -> Entry {
        let key = self.key(fragment, line_offset);
        if let Some(hit) = self.memory_get(key) {
            return hit;
        }
        let entry: Entry = Arc::new(match self.disk_get(key) {
            Some(prepared) => Ok(prepared),
            None => {
                let result = prepare_isolated(fragment, line_offset, self.deadline);
                if let Ok(prepared) = &result {
                    self.disk_put(key, prepared);
                }
                result
            }
        });
        self.memory_put(key, entry.clone());
        entry
    }

    fn memory_get(&self, key: Key) -> Option<Entry> {
        let mut guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        let (map, tick) = &mut *guard;
        *tick += 1;
        let now = *tick;
        map.get_mut(&key).map(|(entry, used)| {
            *used = now;
            entry.clone()
        })
    }

    fn memory_put(&self, key: Key, entry: Entry) {
        let mut guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        let (map, tick) = &mut *guard;
        *tick += 1;
        map.insert(key, (entry, *tick));
        if map.len() > MEMORY_ENTRIES
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| *k)
        {
            map.remove(&oldest);
        }
    }

    fn path(&self, key: Key) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(format!("{}.json", key.hex())))
    }

    fn disk_get(&self, key: Key) -> Option<Prepared> {
        let bytes = std::fs::read(self.path(key)?).ok()?;
        // A damaged or foreign entry is a miss.
        serde_json::from_slice::<DiskEntry>(&bytes)
            .ok()
            .filter(|e| e.key == key.hex() && e.revision == PIPELINE_REVISION)
            .map(|e| e.prepared)
    }

    fn disk_put(&self, key: Key, prepared: &Prepared) {
        let Some(path) = self.path(key) else { return };
        let entry = DiskEntry {
            key: key.hex(),
            revision: PIPELINE_REVISION,
            prepared: prepared.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&entry) else {
            return;
        };
        // Written atomically: a reader never sees half an entry.
        let _ = write_atomic(&path, &bytes);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DiskEntry {
    key: String,
    revision: u32,
    prepared: Prepared,
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("entry"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_uv); }\n";

    #[test]
    fn keys_cover_source_offset_and_context() {
        let a = Key::new(SRC, 5, "metal");
        assert_eq!(a, Key::new(SRC, 5, "metal"));
        assert_ne!(a, Key::new(SRC, 4, "metal"));
        assert_ne!(a, Key::new(SRC, 5, "vulkan"));
        assert_ne!(
            a,
            Key::new(&SRC.replace("v_uv)", "v_uv * 2.0)"), 5, "metal")
        );
    }

    #[test]
    fn disk_entries_survive_a_new_cache_and_damage_is_a_miss() {
        let dir = std::env::temp_dir().join(format!("pm-glsl-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Cache::new("test", Some(dir.clone()));
        let a = first.prepare(SRC, 5);
        assert!(a.is_ok());
        let second = Cache::new("test", Some(dir.clone()));
        let key = second.key(SRC, 5);
        assert_eq!(second.disk_get(key).as_ref(), a.as_ref().as_ref().ok());
        std::fs::write(dir.join(format!("{}.json", key.hex())), b"{broken").unwrap();
        assert!(second.disk_get(key).is_none());
        // A miss translates again and repairs the entry.
        assert!(second.prepare(SRC, 5).is_ok());
        assert!(second.disk_get(key).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What `Cache::disk_put` writes, under any revision.
    fn plant(dir: &Path, key: Key, revision: u32, prepared: &Prepared) {
        let entry = DiskEntry {
            key: key.hex(),
            revision,
            prepared: prepared.clone(),
        };
        write_atomic(
            &dir.join(format!("{}.json", key.hex())),
            &serde_json::to_vec(&entry).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn an_entry_written_under_an_earlier_revision_is_not_reused() {
        // A machine that ran an earlier engine build keeps its prepared
        // programs on disk (POWERMOVE_ENGINE_SHADER_CACHE). Revision 1's
        // `round` was `roundEven` (ties to even); a program that calls it
        // must be translated again, not loaded from revision 1's entry.
        const ROUND: &str = "#version 300 es\nprecision highp float;\nuniform float u_x;\nout vec4 o;\nvoid main(){ o = vec4(round(u_x)); }\n";
        let dir = std::env::temp_dir().join(format!("pm-glsl-cache-rev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let current = pipeline::prepare(ROUND, 3).unwrap();
        // What the earlier revision kept: another program's output, so a
        // reuse is visible.
        let stale = pipeline::prepare(SRC, 3).unwrap();
        assert_ne!(stale, current);
        for revision in 1..PIPELINE_REVISION {
            let old = Key::at_revision(ROUND, 3, "test", revision);
            assert_ne!(old, Key::new(ROUND, 3, "test"), "revision {revision}");
            // Under the key that revision gave the program...
            plant(&dir, old, revision, &stale);
        }
        // ...and, as a damaged or hand-copied entry would be, under the
        // current key with an earlier revision recorded.
        let cache = Cache::new("test", Some(dir.clone()));
        plant(&dir, cache.key(ROUND, 3), PIPELINE_REVISION - 1, &stale);
        assert!(cache.disk_get(cache.key(ROUND, 3)).is_none());
        let entry = cache.prepare(ROUND, 3);
        assert_eq!(entry.as_ref().as_ref().ok(), Some(&current));
        // The fresh translation replaced the stale entry.
        let again = Cache::new("test", Some(dir.clone()));
        assert_eq!(again.disk_get(again.key(ROUND, 3)).as_ref(), Some(&current));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Programs over every stage and polyfill: what the disk cache keeps.
    const EMITTED_PROGRAMS: &[&str] = &[
        SRC,
        "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform float u_x;\nuniform vec4 u_v;\nout vec4 o;\nvoid main(){ o = vec4(round(u_x), round(v_uv), radians(u_x)) + degrees(u_v) + round(u_v) + vec4(radians(v_uv), degrees(v_uv)); }\n",
        "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform float u_x;\nout vec4 o;\nvoid main(){ float i; vec2 j; o = vec4(normalize(u_x), faceforward(u_x, v_uv.x, v_uv.y), reflect(u_x, v_uv.x), refract(u_x, v_uv.y, 0.5)) + vec4(modf(u_x, i), modf(v_uv, j), i); }\n",
        "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform mat3 u_m;\nout vec4 o;\nvoid main(){ mat3 m = inverse(u_m) * outerProduct(vec3(v_uv, 1.0), vec3(1.0, v_uv)); mat2 n = inverse(mat2(m)); o = vec4(m[0] + vec3(n[1], 0.0), tanh(v_uv.x)) + vec4(pow(v_uv.x, 2.2)); }\n",
        "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\nuniform sampler2D u_backdrop;\nuniform float u_p0;\nout vec4 o;\nvoid main(){ vec4 a = texture(u_tex, v_uv); vec4 b = texture(u_backdrop, v_uv + vec2(u_p0)); o = mix(a, b, smoothstep(0.0, 1.0, u_p0)); }\n",
    ];

    /// The revision and the SHA-256 of what [`EMITTED_PROGRAMS`] prepare to.
    const EMITTED: (u32, &str) = (
        2,
        "981e3be32ecc0f78f4c9bbf902a177afe0859b0fa0b84f3cdaae2c072583aa28",
    );

    #[test]
    fn emitted_programs_match_the_pipeline_revision() {
        // A change to what the pipeline emits (a stage, a polyfill, glslang
        // or naga) changes the prepared programs the disk cache keeps across
        // engine builds; reusing one written before the change brings back
        // the old output. Bump `PIPELINE_REVISION` (and the versions in
        // `Key::at_revision` when glslang or naga changed), then record the
        // new hash here.
        let mut h = Sha256::new();
        for (i, src) in EMITTED_PROGRAMS.iter().enumerate() {
            let prepared = pipeline::prepare(src, 3)
                .unwrap_or_else(|f| panic!("program {i} does not prepare: {f}"));
            h.update(serde_json::to_vec(&prepared).unwrap());
        }
        let hash: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert!(
            EMITTED == (PIPELINE_REVISION, hash.as_str()),
            "prepared output is {hash} at PIPELINE_REVISION {PIPELINE_REVISION}; EMITTED records {EMITTED:?}. \
             If the output changed, bump PIPELINE_REVISION and set EMITTED to (the new revision, {hash:?})."
        );
    }

    #[test]
    fn failures_are_cached_in_memory_only() {
        let dir = std::env::temp_dir().join(format!("pm-glsl-cache-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new("test", Some(dir.clone()));
        let bad =
            "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){ o = nope; }\n";
        let a = cache.prepare(bad, 3);
        assert!(a.is_err());
        assert!(Arc::ptr_eq(&a, &cache.prepare(bad, 3)));
        assert!(
            !dir.join(format!("{}.json", cache.key(bad, 3).hex()))
                .exists()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deadline_stops_waiting_for_a_slow_translation() {
        // 2^17 doubling: under the token budget but slow in glslang.
        let mut macros = String::from("#define A0 1.0\n");
        for i in 1..=17 {
            macros.push_str(&format!("#define A{i} (A{p} + A{p})\n", p = i - 1));
        }
        let source = format!(
            "#version 300 es\nprecision highp float;\nout vec4 o;\n{macros}void main(){{ o = vec4(A17); }}\n"
        );
        let start = std::time::Instant::now();
        let r = prepare_isolated(&source, 3, Duration::from_millis(1));
        assert!(start.elapsed() < Duration::from_secs(2));
        // Either it finished in a millisecond (never on any machine measured)
        // or the deadline answered.
        if let Err(f) = r {
            assert!(
                matches!(f.stage, FailureStage::Deadline | FailureStage::Limits),
                "{f}"
            );
        }
    }
}
