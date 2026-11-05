//! Killable compiler boundary. Two process slots, no pending queue, and one
//! request per worker: a bad compiler cannot contaminate the next request.
//! IPC is an inherited local socket (no listener, source file, or reader
//! thread). Every I/O uses the same wall deadline. Drop kills and reaps the
//! worker before its admission permit is released.

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::{Failure, FailureStage, Prepared, Program};
use serde::{Deserialize, Serialize};

#[cfg(test)]
thread_local! {
    static TEST_REQUEST_START: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn test_request_start() -> Option<Instant> {
    TEST_REQUEST_START.get()
}

pub const PROCESS_SLOTS: usize = 2;
pub const MAX_FRAME: usize = 64 << 20;
pub const MAX_RESIDENT_BYTES: u64 = 1 << 30;
const WATCH_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Serialize, Deserialize)]
pub enum Request {
    Prepare {
        fragment: String,
        line_offset: u32,
    },
    Finish(Prepared),
    Translate {
        fragment: String,
        line_offset: u32,
    },
    Msl {
        module: naga::Module,
        stage: naga::ShaderStage,
        slots: Vec<Slot>,
    },
    /// Diagnostic writer with naga's default resource mapping.
    DiagnosticMsl(naga::Module),
    Fuzz {
        fragment: String,
        line_offset: u32,
    },
}

#[derive(Serialize, Deserialize)]
pub enum Response {
    Prepared(Box<Prepared>),
    Program(Box<Program>),
    Msl(String, String),
    Fuzz(crate::fuzz::Outcome),
}

/// Explicit Metal slots from the caller's actual pipeline layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub group: u32,
    pub binding: u32,
    pub buffer: Option<u8>,
    pub texture: Option<u8>,
    pub sampler: Option<u8>,
}

fn process_error(message: impl Into<String>) -> Failure {
    Failure::new(FailureStage::CompilerProcess, message)
}

/// Fixed-size admission pool. Clone/share the pool, not one pool per task.
/// The process-wide default is used by Cache and Prepared::finish.
pub struct Pool {
    executable: PathBuf,
    capacity: usize,
    active: AtomicUsize,
    resident_limit: u64,
}

struct Permit<'a>(&'a AtomicUsize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn budget(pid: u32, deadline: Instant, resident_limit: u64) -> Result<Duration, Failure> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            Failure::new(
                FailureStage::Deadline,
                "GLSL compiler deadline exceeded; worker killed and reaped",
            )
        })?;
    #[cfg(target_os = "macos")]
    {
        #[link(name = "proc")]
        unsafe extern "C" {
            fn proc_pidinfo(
                pid: i32,
                flavor: i32,
                arg: u64,
                buffer: *mut std::ffi::c_void,
                size: i32,
            ) -> i32;
        }
        // Darwin proc_taskinfo: six u64 followed by twelve i32 (96 bytes).
        let mut info = [0u64; 12];
        // SAFETY: aligned, initialized buffer exactly matching PROC_PIDTASKINFO_SIZE.
        let n = unsafe { proc_pidinfo(pid as i32, 4, 0, info.as_mut_ptr().cast(), 96) };
        if n == 96 && info[1] > resident_limit {
            return Err(Failure::new(
                FailureStage::CompilerResources,
                format!(
                    "GLSL compiler exceeded {resident_limit} resident bytes; worker killed and reaped"
                ),
            ));
        }
        if n != 96 && std::io::Error::last_os_error().raw_os_error() != Some(3) {
            return Err(Failure::new(
                FailureStage::CompilerResources,
                "could not monitor compiler resident memory; worker killed and reaped",
            ));
        }
        // A just-exited process may already be absent; EOF/status reports it.
    }
    #[cfg(target_os = "linux")]
    {
        let resident = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
            Ok(status) => linux_resident_bytes(&status).or_else(|e| {
                // /proc/status snapshots State and the memory map separately:
                // exit can drop the map after State was read as R/S. Verify
                // statm is entirely zero before accepting this transient.
                match std::fs::read_to_string(format!("/proc/{pid}/statm")) {
                    Ok(statm)
                        if statm.split_whitespace().count() == 7
                            && statm.split_whitespace().all(|n| n == "0") =>
                    {
                        Ok(0)
                    }
                    Err(gone) if gone.kind() == std::io::ErrorKind::NotFound => Ok(0),
                    _ => Err(e),
                }
            }),
            // A reaped worker is absent; IPC reports its exit.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
        .map_err(|e| {
            Failure::new(
                FailureStage::CompilerResources,
                format!("could not monitor compiler resident memory: {e}"),
            )
        })?;
        if resident > resident_limit {
            return Err(Failure::new(
                FailureStage::CompilerResources,
                format!(
                    "GLSL compiler exceeded {resident_limit} resident bytes; worker killed and reaped"
                ),
            ));
        }
    }
    Ok(left.min(WATCH_INTERVAL))
}

#[cfg(target_os = "linux")]
fn linux_resident_bytes(status: &str) -> std::io::Result<u64> {
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            let mut fields = value.split_whitespace();
            if let (Some(kib), Some("kB"), None) = (fields.next(), fields.next(), fields.next())
                && let Some(bytes) = kib.parse::<u64>().ok().and_then(|n| n.checked_mul(1024))
            {
                return Ok(bytes);
            }
            return Err(std::io::Error::other("invalid VmRSS"));
        }
    }
    // Zombies have no address space and no VmRSS. Any other omission is a
    // monitoring failure, not evidence that a live compiler uses zero bytes.
    if status.lines().any(|line| {
        line.strip_prefix("State:")
            .is_some_and(|v| matches!(v.split_whitespace().next(), Some("Z" | "X")))
    }) {
        Ok(0)
    } else {
        Err(std::io::Error::other("missing VmRSS for live compiler"))
    }
}

#[cfg(unix)]
fn wait_for_io(
    host: &UnixStream,
    events: libc::c_short,
    deadline: Instant,
    pid: u32,
    resident_limit: u64,
) -> Result<(), Failure> {
    loop {
        let timeout = budget(pid, deadline, resident_limit)?;
        let millis = timeout.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as i32;
        let mut fd = libc::pollfd {
            fd: host.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: one initialized pollfd, valid for the entire call. The
        // host owns its descriptor until this request finishes.
        let ready = unsafe { libc::poll(&mut fd, 1, millis) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(process_error(format!("compiler IPC poll failed: {error}")));
        }
        if fd.revents & libc::POLLNVAL != 0 {
            return Err(process_error("compiler IPC descriptor is invalid"));
        }
        if ready > 0 {
            // poll rounds up to milliseconds; never admit I/O after the
            // original deadline or without the resident-memory check.
            budget(pid, deadline, resident_limit)?;
            return Ok(());
        }
    }
}

#[cfg(unix)]
fn read_response(
    host: &mut UnixStream,
    data: &mut [u8],
    deadline: Instant,
    pid: u32,
    resident_limit: u64,
) -> Result<(), Failure> {
    let mut remaining = data;
    while !remaining.is_empty() {
        wait_for_io(host, libc::POLLIN, deadline, pid, resident_limit)?;
        match host.read(remaining) {
            Ok(0) => {
                return Err(process_error("compiler exited without a complete response"));
            }
            Ok(n) => remaining = &mut remaining[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => {
                return Err(process_error(format!("compiler IPC read failed: {e}")));
            }
        }
    }
    Ok(())
}

impl Pool {
    pub fn new(executable: PathBuf, capacity: usize) -> Self {
        Self {
            executable,
            capacity,
            active: AtomicUsize::new(0),
            resident_limit: MAX_RESIDENT_BYTES,
        }
    }
    /// Host-selected memory budget; source code cannot change it.
    pub fn with_resident_limit(mut self, bytes: u64) -> Self {
        self.resident_limit = bytes;
        self
    }
    pub fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
    fn admit(&self) -> Result<Permit<'_>, Failure> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.capacity).then_some(n + 1)
            })
            .map(|_| Permit(&self.active))
            .map_err(|_| {
                Failure::new(
                    FailureStage::CompilerBusy,
                    "all GLSL compiler process slots are occupied; retry later",
                )
            })
    }

    #[cfg(unix)]
    pub fn run(&self, request: Request, timeout: Duration) -> Result<Response, Failure> {
        use std::os::fd::OwnedFd;
        // Independent unit tests expecting success must not compete for the
        // deliberately tiny production pool; integration tests exercise real admission.
        #[cfg(test)]
        let _test_guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(test)]
        TEST_REQUEST_START.set(Some(Instant::now()));
        let _permit = self.admit()?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| process_error("invalid compiler deadline"))?;
        let (mut host, child) = UnixStream::pair()
            .map_err(|e| process_error(format!("cannot create compiler IPC socket: {e}")))?;
        // Darwin refuses timeout socket options once the peer has closed,
        // even with a complete response still buffered. Configure the host
        // before spawning, then poll without mutating socket options.
        host.set_nonblocking(true)
            .map_err(|e| process_error(format!("cannot make compiler IPC nonblocking: {e}")))?;
        let input: OwnedFd = child
            .try_clone()
            .map_err(|e| process_error(format!("cannot clone compiler IPC socket: {e}")))?
            .into();
        let output: OwnedFd = child.into();
        let mut worker = Worker(
            Command::new(&self.executable)
                .arg("--glsl-compiler")
                // No inherited credentials or environment-based compiler plugins.
                .env_clear()
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .stdin(Stdio::from(input))
                .stdout(Stdio::from(output))
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| process_error(format!("cannot start pm-glsl-compiler: {e}")))?,
        );
        let io_error = |e: std::io::Error| {
            if Instant::now() >= deadline
                || matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
            {
                Failure::new(
                    FailureStage::Deadline,
                    "GLSL compiler deadline exceeded; worker killed and reaped",
                )
            } else {
                process_error(format!("compiler IPC failed: {e}"))
            }
        };
        let bytes = serde_json::to_vec(&request).map_err(|e| process_error(e.to_string()))?;
        if bytes.len() > MAX_FRAME {
            return Err(process_error("compiler request exceeds IPC budget"));
        }
        let pid = worker.0.id();
        for data in [&(bytes.len() as u64).to_le_bytes()[..], &bytes] {
            let mut remaining = data;
            while !remaining.is_empty() {
                wait_for_io(&host, libc::POLLOUT, deadline, pid, self.resident_limit)?;
                match host.write(remaining) {
                    Ok(0) => return Err(process_error("compiler closed its input")),
                    Ok(n) => remaining = &remaining[n..],
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(e) => return Err(io_error(e)),
                }
            }
        }
        let mut header = [0; 8];
        read_response(&mut host, &mut header, deadline, pid, self.resident_limit)?;
        let length = u64::from_le_bytes(header);
        if length > MAX_FRAME as u64 {
            return Err(process_error("compiler response exceeds IPC budget"));
        }
        let mut bytes = vec![0; length as usize];
        read_response(&mut host, &mut bytes, deadline, pid, self.resident_limit)?;
        let response = serde_json::from_slice::<Result<Response, Failure>>(&bytes)
            .map_err(|e| process_error(format!("invalid compiler response: {e}")))?;
        // Never block on normal exit after a complete response: Drop kills
        // and wait() reaps even if a library's shutdown code hangs.
        let _ = worker.0.try_wait();
        response
    }
}

pub fn default_pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let executable = std::env::var_os("POWERMOVE_GLSL_COMPILER")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let exe = std::env::current_exe().unwrap_or_default();
                let dir = exe.parent().unwrap_or(std::path::Path::new("."));
                let dir = if dir
                    .file_name()
                    .is_some_and(|n| n == "deps" || n == "examples")
                {
                    dir.parent().unwrap_or(dir)
                } else {
                    dir
                };
                dir.join("pm-glsl-compiler")
            });
        Pool::new(executable, PROCESS_SLOTS)
    })
}

fn check_source_size(fragment: &str) -> Result<(), Failure> {
    if fragment.len() > crate::limits::MAX_SOURCE_BYTES {
        return Err(Failure::new(
            FailureStage::Limits,
            "source-size: GLSL source exceeds the source byte budget",
        ));
    }
    Ok(())
}

pub fn prepare(fragment: &str, line_offset: u32, timeout: Duration) -> Result<Prepared, Failure> {
    check_source_size(fragment)?;
    match default_pool().run(
        Request::Prepare {
            fragment: fragment.into(),
            line_offset,
        },
        timeout,
    )? {
        Response::Prepared(p) => Ok(*p),
        _ => Err(process_error("unexpected prepare response")),
    }
}
pub fn finish(prepared: &Prepared, timeout: Duration) -> Result<Program, Failure> {
    match default_pool().run(Request::Finish(prepared.clone()), timeout)? {
        Response::Program(p) => Ok(*p),
        _ => Err(process_error("unexpected finish response")),
    }
}
pub fn translate(fragment: &str, line_offset: u32, timeout: Duration) -> Result<Program, Failure> {
    check_source_size(fragment)?;
    match default_pool().run(
        Request::Translate {
            fragment: fragment.into(),
            line_offset,
        },
        timeout,
    )? {
        Response::Program(p) => Ok(*p),
        _ => Err(process_error("unexpected translate response")),
    }
}

/// The downstream naga validator and MSL writer use the same bounded pool.
/// Native device pipeline creation still belongs to the device owner.
pub fn msl(
    module: &naga::Module,
    stage: naga::ShaderStage,
    slots: Vec<Slot>,
    timeout: Duration,
) -> Result<(String, String), Failure> {
    match default_pool().run(
        Request::Msl {
            module: module.clone(),
            stage,
            slots,
        },
        timeout,
    )? {
        Response::Msl(source, entry) => Ok((source, entry)),
        _ => Err(process_error("unexpected MSL response")),
    }
}

pub(crate) fn diagnostic_msl(module: &naga::Module) -> Result<String, Failure> {
    match default_pool().run(
        Request::DiagnosticMsl(module.clone()),
        crate::cache::DEFAULT_DEADLINE,
    )? {
        Response::Msl(source, _) => Ok(source),
        _ => Err(process_error("unexpected diagnostic MSL response")),
    }
}

fn msl_local(
    module: &naga::Module,
    stage: naga::ShaderStage,
    slots: &[Slot],
) -> Result<Response, Failure> {
    use naga::back::msl;
    let info = crate::pipeline::naga_validate(module)
        .map_err(|e| Failure::new(FailureStage::NagaValidate, e))?;
    let (entry_index, ep) = module
        .entry_points
        .iter()
        .enumerate()
        .find(|(_, e)| e.stage == stage)
        .ok_or_else(|| Failure::new(FailureStage::NagaMsl, "missing MSL entry point"))?;
    validate_slots(module, info.get_entry_point(entry_index), slots)?;
    let resources = slots
        .iter()
        .map(|s| {
            (
                naga::ResourceBinding {
                    group: s.group,
                    binding: s.binding,
                },
                msl::BindTarget {
                    buffer: s.buffer,
                    texture: s.texture,
                    sampler: s.sampler.map(msl::BindSamplerTarget::Resource),
                    external_texture: None,
                    mutable: false,
                },
            )
        })
        .collect();
    let mut per_entry_point_map = msl::EntryPointResourceMap::default();
    per_entry_point_map.insert(
        ep.name.clone(),
        msl::EntryPointResources {
            resources,
            immediates_buffer: None,
            sizes_buffer: None,
        },
    );
    let options = msl::Options {
        lang_version: (3, 2),
        per_entry_point_map,
        fake_missing_bindings: false,
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            index: naga::proc::BoundsCheckPolicy::Restrict,
            buffer: naga::proc::BoundsCheckPolicy::Restrict,
            image_load: naga::proc::BoundsCheckPolicy::Restrict,
            binding_array: naga::proc::BoundsCheckPolicy::Restrict,
        },
        force_loop_bounding: true,
        emit_int_div_checks: true,
        ..Default::default()
    };
    let pipeline = msl::PipelineOptions {
        entry_point: Some((stage, ep.name.clone())),
        ..Default::default()
    };
    let (source, translated) = msl::write_string(module, &info, &options, &pipeline)
        .map_err(|e| Failure::new(FailureStage::NagaMsl, format!("MSL: {e}")))?;
    let entry = translated
        .entry_point_names
        .into_iter()
        .next()
        .ok_or_else(|| process_error("MSL emitted no entry point"))?
        .map_err(|e| Failure::new(FailureStage::NagaMsl, format!("MSL entry: {e}")))?;
    Ok(Response::Msl(
        format!("#pragma METAL fp math_mode(fast)\n{source}"),
        entry,
    ))
}

/// A caller supplies the real pipeline layout; the SPIR-V-derived module
/// decides each resource's kind. Never silently overwrite a duplicate or
/// invent a missing Metal slot. Extra slots for other stages are allowed.
fn validate_slots(
    module: &naga::Module,
    used: &naga::valid::FunctionInfo,
    slots: &[Slot],
) -> Result<(), Failure> {
    use std::collections::HashSet;
    let mut bindings = HashSet::new();
    let mut metal = HashSet::new();
    for s in slots {
        if !bindings.insert((s.group, s.binding)) {
            return Err(Failure::new(
                FailureStage::Bind,
                "duplicate resource binding in MSL layout",
            ));
        }
        for (kind, slot) in [(0, s.buffer), (1, s.texture), (2, s.sampler)] {
            if let Some(slot) = slot
                && !metal.insert((kind, slot))
            {
                return Err(Failure::new(
                    FailureStage::Bind,
                    "aliased Metal slots in MSL layout",
                ));
            }
        }
    }
    for (handle, global) in module.global_variables.iter() {
        if used[handle].is_empty() {
            continue;
        }
        let Some(binding) = &global.binding else {
            continue;
        };
        let slot = slots
            .iter()
            .find(|s| s.group == binding.group && s.binding == binding.binding);
        let valid = slot.is_some_and(|s| match global.space {
            naga::AddressSpace::Uniform | naga::AddressSpace::Storage { .. } => {
                s.buffer.is_some() && s.texture.is_none() && s.sampler.is_none()
            }
            naga::AddressSpace::Handle => match module.types[global.ty].inner {
                naga::TypeInner::Image { .. } => {
                    s.texture.is_some() && s.buffer.is_none() && s.sampler.is_none()
                }
                naga::TypeInner::Sampler { .. } => {
                    s.sampler.is_some() && s.buffer.is_none() && s.texture.is_none()
                }
                _ => false,
            },
            _ => false,
        });
        if !valid {
            return Err(Failure::new(
                FailureStage::Bind,
                format!(
                    "missing or incompatible MSL resource slot at set {} binding {}",
                    binding.group, binding.binding,
                ),
            ));
        }
    }
    Ok(())
}

/// Worker protocol. Only the helper calls this. Raw compiler calls stay in
/// its large-stack thread; panic, abort, timeout and OOM cannot escape it.
pub fn worker_main() -> Result<(), Box<dyn std::error::Error>> {
    resource_limits()?;
    let mut header = [0; 8];
    std::io::stdin().read_exact(&mut header)?;
    let length = u64::from_le_bytes(header);
    if length > MAX_FRAME as u64 {
        return Err("compiler request exceeds IPC budget".into());
    }
    let mut bytes = vec![0; length as usize];
    std::io::stdin().read_exact(&mut bytes)?;
    let task = std::thread::Builder::new()
        .stack_size(crate::cache::TRANSLATION_STACK)
        .spawn(move || -> Result<Response, Failure> {
            let request: Request =
                serde_json::from_slice(&bytes).map_err(|e| process_error(e.to_string()))?;
            match request {
                Request::Fuzz {
                    fragment,
                    line_offset,
                } => Ok(Response::Fuzz(crate::fuzz::outcome(&fragment, line_offset))),
                Request::DiagnosticMsl(module) => {
                    crate::pipeline::msl_local(&module).map(|s| Response::Msl(s, String::new()))
                }
                Request::Msl {
                    module,
                    stage,
                    slots,
                } => msl_local(&module, stage, &slots),
                Request::Prepare {
                    fragment,
                    line_offset,
                } => crate::pipeline::prepare_local(&fragment, line_offset)
                    .map(|p| Response::Prepared(Box::new(p))),
                Request::Finish(prepared) => prepared
                    .finish_local()
                    .map(|p| Response::Program(Box::new(p))),
                Request::Translate {
                    fragment,
                    line_offset,
                } => crate::pipeline::translate_local(&fragment, line_offset)
                    .map(|p| Response::Program(Box::new(p))),
            }
        })?;
    let result = task
        .join()
        .unwrap_or_else(|_| Err(Failure::new(FailureStage::Panic, "compiler panicked")));
    let bytes = serde_json::to_vec(&result)?;
    if bytes.len() > MAX_FRAME {
        return Err("compiler response exceeds IPC budget".into());
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&(bytes.len() as u64).to_le_bytes())?;
    out.write_all(&bytes)?;
    out.flush()?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn resource_limits() -> std::io::Result<()> {
    #[repr(C)]
    struct Limit {
        current: u64,
        maximum: u64,
    }
    unsafe extern "C" {
        fn setrlimit(resource: i32, limit: *const Limit) -> i32;
    }
    // Darwin/Linux resource.h: CPU=0, CORE=4. Darwin rejects finite AS/DATA
    // hard limits; the parent samples resident memory during IPC instead.
    // No core dumps containing private GLSL.
    for (resource, size) in [(0, 30), (4, 0)] {
        let limit = Limit {
            current: size,
            maximum: size,
        };
        // SAFETY: native Darwin/Linux rlimit layout and a valid immutable pointer.
        if unsafe { setrlimit(resource, &limit) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn resource_limits() -> std::io::Result<()> {
    Err(std::io::Error::other(
        "GLSL compiler OS resource limits require macOS or Linux",
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn resident_monitor_fails_closed_except_for_exited_workers() {
        assert_eq!(
            linux_resident_bytes("State:\tR (running)\nVmRSS:\t123 kB\n").unwrap(),
            123 * 1024
        );
        assert_eq!(linux_resident_bytes("State:\tZ (zombie)\n").unwrap(), 0);
        for status in [
            "",
            "State:\tR (running)\n",
            "VmRSS: 2 MB",
            "VmRSS: 18446744073709551615 kB",
        ] {
            assert!(linux_resident_bytes(status).is_err());
        }
    }
}

#[cfg(all(test, unix))]
mod ipc_tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_complete_response_is_read_after_the_worker_socket_closes() {
        let (mut host, mut child) = UnixStream::pair().unwrap();
        host.set_nonblocking(true).unwrap();
        child.write_all(b"complete").unwrap();
        drop(child);
        let mut bytes = [0; 8];
        read_response(
            &mut host,
            &mut bytes,
            Instant::now() + Duration::from_secs(20),
            std::process::id(),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(&bytes, b"complete");
    }

    #[test]
    fn a_closed_worker_socket_never_completes_an_incomplete_response() {
        for response in [&b""[..], &b"short"[..]] {
            let (mut host, mut child) = UnixStream::pair().unwrap();
            host.set_nonblocking(true).unwrap();
            child.write_all(response).unwrap();
            drop(child);
            let failure = read_response(
                &mut host,
                &mut [0; 8],
                Instant::now() + Duration::from_secs(20),
                std::process::id(),
                u64::MAX,
            )
            .unwrap_err();
            assert_eq!(failure.stage, FailureStage::CompilerProcess);
            assert!(failure.message.contains("without a complete response"));
        }
    }
}
