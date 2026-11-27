//! The transcribe.cpp C API (v0.3.1, header 57b1af43650f195d), loaded from the
//! library main ships (`@transcribe-cpp/darwin-arm64-metal`). The library's
//! `contract.json` and every struct size are checked before any call, so a
//! drifted library refuses with a reason instead of faulting in native code.
use libloading::{Library, Symbol};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const VERSION: &str = "0.3.1";
pub const HEADER_HASH: &str = "57b1af43650f195d";
pub const SAMPLE_RATE: usize = 16_000;
const ERR_ABORTED: c_int = 13;
const ERR_OUTPUT_TRUNCATED: c_int = 18;
const ERR_OUTPUT_REPETITION: c_int = 19;
const TIMESTAMPS_AUTO: c_int = 1;
const BACKEND_AUTO: c_int = 0;

#[repr(C)]
struct ModelLoadParams {
    struct_size: u64,
    backend: c_int,
    device: *mut c_void,
}
#[repr(C)]
struct SessionParams {
    struct_size: u64,
    n_threads: c_int,
    kv_type: c_int,
    n_ctx: i32,
}
#[repr(C)]
struct RunParams {
    struct_size: u64,
    task: c_int,
    timestamps: c_int,
    pnc: c_int,
    itn: c_int,
    diarize: c_int,
    language: *const c_char,
    target_language: *const c_char,
    keep_special_tags: bool,
    family: *mut c_void,
    spec_k_drafts: i32,
    vocabulary: *mut c_void,
    n_vocabulary: i32,
    prompt: *const c_char,
    prefix: *const c_char,
}
#[repr(C)]
struct Capabilities {
    struct_size: u64,
    native_sample_rate: i32,
    n_languages: c_int,
    languages: *const *const c_char,
    max_timestamp_kind: c_int,
    supports_language_detect: bool,
    supports_translate: bool,
    supports_streaming: bool,
    supports_spec_decode: bool,
    max_audio_ms: i64,
    n_translate_target_languages: c_int,
    translate_target_languages: *const *const c_char,
}
#[repr(C)]
struct Segment {
    struct_size: u64,
    t0_ms: i64,
    t1_ms: i64,
    first_word: c_int,
    n_words: c_int,
    first_token: c_int,
    n_tokens: c_int,
    text: *const c_char,
    speaker_id: i32,
}
#[repr(C)]
struct Word {
    struct_size: u64,
    t0_ms: i64,
    t1_ms: i64,
    seg_index: c_int,
    first_token: c_int,
    n_tokens: c_int,
    text: *const c_char,
}
/// (ABI kind, Rust size) for every struct this binding passes.
fn layouts() -> [(c_int, usize, &'static str); 6] {
    use std::mem::size_of;
    [
        (0, size_of::<ModelLoadParams>(), "model load params"),
        (1, size_of::<SessionParams>(), "session params"),
        (2, size_of::<RunParams>(), "run params"),
        (4, size_of::<Capabilities>(), "capabilities"),
        (6, size_of::<Segment>(), "segment"),
        (7, size_of::<Word>(), "word"),
    ]
}

/// One segment of a run: its text, times and the words it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSegment {
    pub text: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RunWord {
    pub text: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub segment: i32,
}
/// What one run heard (main's `EngineResult`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunResult {
    pub text: String,
    pub language: String,
    pub segments: Vec<RunSegment>,
    pub words: Vec<RunWord>,
}
#[derive(Debug)]
pub enum Error {
    /// The runtime can't load or doesn't match this binding.
    Runtime(String),
    /// The library refused a call (its own status text).
    Call(String),
    /// Cancelled through the run's abort flag.
    Cancelled,
    /// The decoder stopped repeating or running long; this is what it heard.
    Partial(Box<RunResult>),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runtime(s) | Self::Call(s) => f.write_str(s),
            Self::Cancelled => f.write_str("Transcription was cancelled."),
            Self::Partial(_) => f.write_str("The transcription stopped early."),
        }
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

/// The loaded runtime. Keep one per process; models borrow it.
pub struct Runtime {
    lib: Library,
    dir: PathBuf,
}
fn reinstall(problem: impl std::fmt::Display) -> Error {
    Error::Runtime(format!(
        "Transcription can’t start: {problem}. Reinstall Powermove."
    ))
}
/// '0.3.1' from '0.3.1.post2' or '0.3.1-beta' (the binding's own rule).
fn base(version: &str) -> &str {
    let version = version.trim();
    let end = version
        .char_indices()
        .scan(false, |dot, (i, c)| {
            let keep = c.is_ascii_digit() || c == '.' && !*dot;
            *dot = c == '.';
            keep.then_some(i + c.len_utf8())
        })
        .last()
        .unwrap_or(0);
    version[..end].trim_end_matches('.')
}
impl Runtime {
    /// Load `dir/libtranscribe.dylib` after checking `dir/contract.json`
    /// (main's `verifyRuntime`) and initialise its backends from `dir`.
    pub fn load(dir: &Path) -> Result<Self> {
        let contract: serde_json::Value = std::fs::read(dir.join("contract.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| {
                Error::Runtime(
                    "The transcription runtime is incomplete. Reinstall Powermove.".into(),
                )
            })?;
        let hash = contract["header_hash"].as_str().unwrap_or("unknown");
        if hash != HEADER_HASH {
            return Err(reinstall(format!(
                "the transcription runtime was built for header {hash}, but its binding expects {HEADER_HASH}"
            )));
        }
        if let Some(version) = contract["version"].as_str()
            && base(version) != base(VERSION)
        {
            return Err(reinstall(format!(
                "the transcription runtime is version {version}, but its binding is {VERSION}"
            )));
        }
        // SAFETY: loading the library runs its initialisers; it is the pinned
        // build whose contract was checked above.
        let lib = unsafe { Library::new(dir.join("libtranscribe.dylib")) }
            .map_err(|e| reinstall(format!("its library didn’t load ({e})")))?;
        let runtime = Self {
            lib,
            dir: dir.to_owned(),
        };
        let size: Symbol<unsafe extern "C" fn(c_int) -> u64> =
            runtime.symbol("transcribe_abi_struct_size")?;
        for (kind, ours, name) in layouts() {
            // SAFETY: a pure query of the library's struct table.
            let theirs = unsafe { size(kind) };
            if theirs != ours as u64 {
                return Err(reinstall(format!(
                    "its {name} are {theirs} bytes, but this binding passes {ours}"
                )));
            }
        }
        let path = CString::new(runtime.dir.to_string_lossy().as_bytes())
            .map_err(|_| reinstall("its folder name has a NUL byte"))?;
        let init: Symbol<unsafe extern "C" fn(*const c_char) -> c_int> =
            runtime.symbol("transcribe_init_backends")?;
        // SAFETY: a NUL-terminated folder path, read during the call.
        let status = unsafe { init(path.as_ptr()) };
        if status != 0 {
            let fallback: Symbol<unsafe extern "C" fn() -> c_int> =
                runtime.symbol("transcribe_init_backends_default")?;
            // SAFETY: no arguments.
            runtime.check(unsafe { fallback() }, "starting the transcription backends")?;
        }
        Ok(runtime)
    }
    fn symbol<T>(&self, name: &str) -> Result<Symbol<'_, T>> {
        // SAFETY: each caller names the C signature it was generated with.
        unsafe { self.lib.get(name.as_bytes()) }
            .map_err(|_| reinstall(format!("its library lacks {name}")))
    }
    fn status_text(&self, status: c_int) -> String {
        let Ok(text) =
            self.symbol::<unsafe extern "C" fn(c_int) -> *const c_char>("transcribe_status_string")
        else {
            return format!("status {status}");
        };
        // SAFETY: the library returns a static NUL-terminated string.
        unsafe { owned(text(status)) }
    }
    fn check(&self, status: c_int, doing: &str) -> Result<()> {
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Call(format!(
                "{doing}: {}",
                self.status_text(status)
            )))
        }
    }
    /// Load one GGUF model (main's `TranscribeModel.load(file, {backend:'auto'})`)
    /// with a session of `threads` threads.
    pub fn model(&self, file: &Path, threads: usize) -> Result<Model<'_>> {
        let path = CString::new(file.to_string_lossy().as_bytes())
            .map_err(|_| Error::Call("The model path has a NUL byte.".into()))?;
        let init: Symbol<unsafe extern "C" fn(*mut ModelLoadParams)> =
            self.symbol("transcribe_model_load_params_init")?;
        let load: Symbol<
            unsafe extern "C" fn(*const c_char, *const ModelLoadParams, *mut *mut c_void) -> c_int,
        > = self.symbol("transcribe_model_load_file")?;
        let mut params = ModelLoadParams {
            struct_size: 0,
            backend: 0,
            device: std::ptr::null_mut(),
        };
        let mut handle = std::ptr::null_mut();
        // SAFETY: the init fills our checked-size struct; load reads it and the path.
        unsafe {
            init(&mut params);
            params.backend = BACKEND_AUTO;
            self.check(
                load(path.as_ptr(), &params, &mut handle),
                "loading the model",
            )?;
        }
        let mut model = Model {
            runtime: self,
            handle,
            session: std::ptr::null_mut(),
            languages: vec![],
        };
        model.languages = model.capabilities()?;
        let init: Symbol<unsafe extern "C" fn(*mut SessionParams)> =
            self.symbol("transcribe_session_params_init")?;
        let open: Symbol<
            unsafe extern "C" fn(*mut c_void, *const SessionParams, *mut *mut c_void) -> c_int,
        > = self.symbol("transcribe_session_init")?;
        let mut params = SessionParams {
            struct_size: 0,
            n_threads: 0,
            kv_type: 0,
            n_ctx: 0,
        };
        // SAFETY: as above; the model handle is live.
        unsafe {
            init(&mut params);
            params.n_threads = threads.clamp(1, 64) as c_int;
            self.check(
                open(handle, &params, &mut model.session),
                "starting a session",
            )?;
        }
        Ok(model)
    }
}
/// A loaded model and its session. Dropping frees both.
pub struct Model<'a> {
    runtime: &'a Runtime,
    handle: *mut c_void,
    session: *mut c_void,
    /// The model's own language tags (Nemotron says de-DE where we say de).
    pub languages: Vec<String>,
}
// SAFETY: transcribe.cpp handles are used from one thread at a time; `Model`
// is moved to the transcription worker, never shared.
unsafe impl Send for Model<'_> {}
/// SAFETY: `p` is null or a NUL-terminated string the library keeps alive.
unsafe fn owned(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
extern "C" fn abort_requested(flag: *mut c_void) -> bool {
    // SAFETY: `flag` is the AtomicBool `run` registered and outlives the run.
    unsafe { &*(flag as *const AtomicBool) }.load(Ordering::Relaxed)
}
impl Model<'_> {
    fn capabilities(&self) -> Result<Vec<String>> {
        let rt = self.runtime;
        let init: Symbol<unsafe extern "C" fn(*mut Capabilities)> =
            rt.symbol("transcribe_capabilities_init")?;
        let get: Symbol<unsafe extern "C" fn(*mut c_void, *mut Capabilities) -> c_int> =
            rt.symbol("transcribe_model_get_capabilities")?;
        // SAFETY: zeroed then initialised by the library; read back after the call.
        unsafe {
            let mut caps: Capabilities = std::mem::zeroed();
            init(&mut caps);
            rt.check(get(self.handle, &mut caps), "reading the model's languages")?;
            Ok((0..caps.n_languages.max(0) as usize)
                .map(|i| owned(*caps.languages.add(i)))
                .collect())
        }
    }
    /// The model's tag for a language: exact, else its first regional form.
    pub fn language(&self, language: Option<&str>) -> Option<String> {
        let lower = language?.to_lowercase();
        self.languages
            .iter()
            .find(|t| t.to_lowercase() == lower)
            .or_else(|| {
                self.languages
                    .iter()
                    .find(|t| t.to_lowercase().starts_with(&format!("{lower}-")))
            })
            .cloned()
    }
    /// One run over 16 kHz mono samples with automatic timestamps (main's
    /// `session.run(samples, {timestamps:'auto', language})`). `cancel` aborts.
    pub fn run(
        &mut self,
        samples: &[f32],
        language: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<RunResult> {
        let rt = self.runtime;
        let init: Symbol<unsafe extern "C" fn(*mut RunParams)> =
            rt.symbol("transcribe_run_params_init")?;
        let run: Symbol<
            unsafe extern "C" fn(*mut c_void, *const f32, c_int, *const RunParams) -> c_int,
        > = rt.symbol("transcribe_run")?;
        let set_abort: Symbol<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void)> =
            rt.symbol("transcribe_set_abort_callback")?;
        let language = language
            .map(|l| CString::new(l).map_err(|_| Error::Call("Invalid language tag.".into())))
            .transpose()?;
        // SAFETY: the params come from the library's init with our checked size;
        // samples and the language outlive the call; the abort flag outlives it
        // and is unregistered before returning.
        let status = unsafe {
            let mut params: RunParams = std::mem::zeroed();
            init(&mut params);
            params.timestamps = TIMESTAMPS_AUTO;
            if let Some(language) = &language {
                params.language = language.as_ptr();
            }
            set_abort(
                self.session,
                abort_requested as *mut c_void,
                cancel as *const AtomicBool as *mut c_void,
            );
            let status = run(
                self.session,
                samples.as_ptr(),
                samples.len().min(i32::MAX as usize) as c_int,
                &params,
            );
            set_abort(self.session, std::ptr::null_mut(), std::ptr::null_mut());
            status
        };
        if cancel.load(Ordering::Relaxed) || status == ERR_ABORTED {
            return Err(Error::Cancelled);
        }
        if status == ERR_OUTPUT_TRUNCATED || status == ERR_OUTPUT_REPETITION {
            return Err(Error::Partial(Box::new(self.result()?)));
        }
        rt.check(status, "transcribing")?;
        self.result()
    }
    fn result(&self) -> Result<RunResult> {
        let rt = self.runtime;
        let text: Symbol<unsafe extern "C" fn(*mut c_void) -> *const c_char> =
            rt.symbol("transcribe_full_text")?;
        let detected: Symbol<unsafe extern "C" fn(*mut c_void) -> *const c_char> =
            rt.symbol("transcribe_detected_language")?;
        let count: Symbol<unsafe extern "C" fn(*mut c_void) -> c_int> =
            rt.symbol("transcribe_n_segments")?;
        let words: Symbol<unsafe extern "C" fn(*mut c_void) -> c_int> =
            rt.symbol("transcribe_n_words")?;
        let segment_init: Symbol<unsafe extern "C" fn(*mut Segment)> =
            rt.symbol("transcribe_segment_init")?;
        let segment: Symbol<unsafe extern "C" fn(*mut c_void, c_int, *mut Segment) -> c_int> =
            rt.symbol("transcribe_get_segment")?;
        let word_init: Symbol<unsafe extern "C" fn(*mut Word)> =
            rt.symbol("transcribe_word_init")?;
        let word: Symbol<unsafe extern "C" fn(*mut c_void, c_int, *mut Word) -> c_int> =
            rt.symbol("transcribe_get_word")?;
        // SAFETY: accessors on the live session's last result; structs are
        // initialised by the library and strings copied before the next call.
        unsafe {
            let mut out = RunResult {
                text: owned(text(self.session)),
                language: owned(detected(self.session)),
                ..Default::default()
            };
            for i in 0..count(self.session).max(0) {
                let mut s: Segment = std::mem::zeroed();
                segment_init(&mut s);
                rt.check(segment(self.session, i, &mut s), "reading a segment")?;
                out.segments.push(RunSegment {
                    text: owned(s.text),
                    t0_ms: s.t0_ms,
                    t1_ms: s.t1_ms,
                });
            }
            for i in 0..words(self.session).max(0) {
                let mut w: Word = std::mem::zeroed();
                word_init(&mut w);
                rt.check(word(self.session, i, &mut w), "reading a word")?;
                out.words.push(RunWord {
                    text: owned(w.text),
                    t0_ms: w.t0_ms,
                    t1_ms: w.t1_ms,
                    segment: w.seg_index,
                });
            }
            Ok(out)
        }
    }
}
impl Drop for Model<'_> {
    fn drop(&mut self) {
        // SAFETY: handles this value owns, freed once, session first.
        unsafe {
            if !self.session.is_null()
                && let Ok(free) = self
                    .runtime
                    .symbol::<unsafe extern "C" fn(*mut c_void)>("transcribe_session_free")
            {
                free(self.session);
            }
            if !self.handle.is_null()
                && let Ok(free) = self
                    .runtime
                    .symbol::<unsafe extern "C" fn(*mut c_void)>("transcribe_model_free")
            {
                free(self.handle);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_compare_by_their_numeric_base() {
        assert_eq!(base("0.3.1.post2"), "0.3.1");
        assert_eq!(base("0.3.1-beta"), "0.3.1");
        assert_eq!(base(" 0.3.1 "), "0.3.1");
    }
    #[test]
    fn a_missing_or_drifted_runtime_refuses_with_a_reason() {
        let dir = std::env::temp_dir().join(format!("pm-transcribe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = Runtime::load(&dir).err().unwrap().to_string();
        assert_eq!(
            err,
            "The transcription runtime is incomplete. Reinstall Powermove."
        );
        std::fs::write(
            dir.join("contract.json"),
            r#"{"version":"0.3.1","header_hash":"0000"}"#,
        )
        .unwrap();
        let err = Runtime::load(&dir).err().unwrap().to_string();
        assert!(
            err.contains("built for header 0000, but its binding expects 57b1af43650f195d"),
            "{err}"
        );
        std::fs::write(
            dir.join("contract.json"),
            r#"{"version":"0.4.0","header_hash":"57b1af43650f195d"}"#,
        )
        .unwrap();
        let err = Runtime::load(&dir).err().unwrap().to_string();
        assert!(
            err.contains("version 0.4.0, but its binding is 0.3.1"),
            "{err}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
