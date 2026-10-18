//! The job engine's decision 19 fallback runner (plan section 11, contract
//! 14; amendment 21): the job engine has no WebGL, so programs only the
//! runner can draw (effects, transitions, shader and extension layers) are
//! run by the host in a window's runner.
//!
//! For each request the backend writes the textures the program samples
//! into the session's scratch cache area (`fallback-<key>-<i>.rgba16f`:
//! binary16 RGBA, little-endian, rows in GL order), sends
//! `shaderFallback` naming them and the output file, waits for the host's
//! answer, and reads the output back. The host answers `ack` once the
//! output is written, or an error: `shaderCompile` (WebGL refuses the
//! program: skipped, as today's exporter skips it), `watchdogTimeout` (the
//! runner's draw passed its deadline: the job stops), anything else (no
//! window to run it: the job fails naming the program). Files are removed
//! once read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_protocol::common::SessionId;
use pm_protocol::envelope::{Request, Response};
use pm_protocol::error::ErrorCode;
use pm_protocol::host::{
    CacheKind, CapabilityHandle, CapabilityKind, HandleAccess, MintHandle, ResolveHandle,
};
use pm_protocol::shader_fallback::HostShaderFallback;
use pm_render::shader_layers::{FallbackFailure, FallbackHost, FallbackJob, FallbackPixels};

use super::blender::HostPort;

/// How long one request may take in the host (a window's runner compiles
/// with a 10 s deadline and draws each pass under its own 10 s deadline;
/// the rest is transfer).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// A job's fallback runner, through the host.
pub(crate) struct HostFallback<P: HostPort + ?Sized> {
    port: Arc<P>,
    session: SessionId,
    cancel: Arc<AtomicBool>,
    area: Mutex<Option<(PathBuf, CapabilityHandle)>>,
}

impl<P: HostPort + ?Sized> HostFallback<P> {
    pub(crate) fn new(port: Arc<P>, session: SessionId, cancel: Arc<AtomicBool>) -> Self {
        HostFallback {
            port,
            session,
            cancel,
            area: Mutex::new(None),
        }
    }

    fn unavailable(message: impl Into<String>) -> FallbackFailure {
        FallbackFailure {
            reason: "unavailable".into(),
            message: message.into(),
        }
    }

    /// The scratch cache area and a handle to it for the session.
    fn area(&self) -> Result<(PathBuf, CapabilityHandle), FallbackFailure> {
        let mut held = self.area.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = held.as_ref() {
            return Ok(a.clone());
        }
        let grant = match self.port.call(
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::Cache {
                    cache: CacheKind::Scratch,
                },
                session: self.session,
            }),
            Some(self.session),
        ) {
            Ok(Response::Handle(g)) => g,
            Ok(_) => return Err(Self::unavailable("the scratch cache: unexpected answer")),
            Err(e) => {
                return Err(Self::unavailable(format!(
                    "the scratch cache: {}",
                    e.message
                )));
            }
        };
        let resolved = match self.port.call(
            Request::ResolveHandle(ResolveHandle {
                handle: grant.handle.clone(),
                session: self.session,
            }),
            Some(self.session),
        ) {
            Ok(Response::HandleResolved(r)) if r.access == HandleAccess::ReadWrite => r,
            Ok(_) => return Err(Self::unavailable("the scratch cache is not writable")),
            Err(e) => {
                return Err(Self::unavailable(format!(
                    "the scratch cache: {}",
                    e.message
                )));
            }
        };
        let area = (PathBuf::from(resolved.path), grant.handle);
        *held = Some(area.clone());
        Ok(area)
    }
}

/// Raw binary16 bytes of pixels (the file format).
fn bytes(pixels: &FallbackPixels) -> Vec<u8> {
    pixels
        .rgba16f
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

/// The host's error as a runner failure.
pub(crate) fn failure_of(code: ErrorCode, message: String) -> FallbackFailure {
    FallbackFailure {
        reason: match code {
            ErrorCode::ShaderCompile => "compile",
            ErrorCode::WatchdogTimeout => "watchdog",
            _ => "unavailable",
        }
        .into(),
        message,
    }
}

impl<P: HostPort + ?Sized> FallbackHost for HostFallback<P> {
    fn run(&self, job: &FallbackJob) -> Result<FallbackPixels, FallbackFailure> {
        let (dir, handle) = self.area()?;
        let r = &job.request;
        let textures: Vec<String> = (0..job.textures.len())
            .map(|i| format!("fallback-{}-{i}.rgba16f", r.key))
            .collect();
        let output = format!("fallback-{}-out.rgba16f", r.key);
        let cleanup = || {
            for name in textures.iter().chain(std::iter::once(&output)) {
                let _ = std::fs::remove_file(dir.join(name));
            }
        };
        for (name, pixels) in textures.iter().zip(&job.textures) {
            if let Err(e) = std::fs::write(dir.join(name), bytes(pixels)) {
                cleanup();
                return Err(Self::unavailable(format!(
                    "a texture could not be written ({e})"
                )));
            }
        }
        let answer = self.port.call_until(
            Request::ShaderFallback(HostShaderFallback {
                request: r.clone(),
                area: handle,
                textures: textures.clone(),
                output: output.clone(),
            }),
            Some(self.session),
            Instant::now() + REQUEST_TIMEOUT,
            &self.cancel,
        );
        let result = match answer {
            None if self.cancel.load(Ordering::Relaxed) => {
                Err(Self::unavailable("the job was cancelled"))
            }
            None => Err(Self::unavailable(format!(
                "the host did not answer within {} s",
                REQUEST_TIMEOUT.as_secs()
            ))),
            Some(Err(e)) => Err(failure_of(e.code, e.message)),
            Some(Ok(Response::Ack)) => match std::fs::read(dir.join(&output)) {
                Ok(data) if data.len() as u64 == u64::from(r.width) * u64::from(r.height) * 8 => {
                    Ok(FallbackPixels {
                        width: r.width,
                        height: r.height,
                        rgba16f: data
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect(),
                    })
                }
                Ok(data) => Err(FallbackFailure {
                    reason: "size".into(),
                    message: format!(
                        "the runner wrote {} bytes for a {}×{} target",
                        data.len(),
                        r.width,
                        r.height
                    ),
                }),
                Err(e) => Err(Self::unavailable(format!(
                    "the result could not be read ({e})"
                ))),
            },
            Some(Ok(_)) => Err(Self::unavailable("the host answered something else")),
        };
        cleanup();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::JobId;
    use pm_protocol::error::{EngineError, ErrorScope};
    use pm_protocol::host::HandleResolved;

    /// A host that runs requests by echoing the first texture (halved).
    struct Echo {
        dir: PathBuf,
        answer: Mutex<Option<EngineError>>,
        seen: Mutex<Vec<HostShaderFallback>>,
    }

    impl HostPort for Echo {
        fn call(&self, request: Request, _: Option<SessionId>) -> Result<Response, EngineError> {
            match request {
                Request::MintHandle(_) => Ok(Response::Handle(pm_protocol::host::HandleGrant {
                    handle: CapabilityHandle::from_random_bytes([7; 16]),
                    kind: CapabilityKind::Cache {
                        cache: CacheKind::Scratch,
                    },
                    content_hash: None,
                })),
                Request::ResolveHandle(r) => Ok(Response::HandleResolved(HandleResolved {
                    handle: r.handle,
                    kind: CapabilityKind::Cache {
                        cache: CacheKind::Scratch,
                    },
                    path: self.dir.to_string_lossy().into_owned(),
                    access: HandleAccess::ReadWrite,
                })),
                other => panic!("unexpected {other:?}"),
            }
        }

        fn call_until(
            &self,
            request: Request,
            _: Option<SessionId>,
            _: Instant,
            _: &AtomicBool,
        ) -> Option<Result<Response, EngineError>> {
            let Request::ShaderFallback(f) = request else {
                panic!()
            };
            self.seen.lock().unwrap().push(f.clone());
            if let Some(e) = self.answer.lock().unwrap().clone() {
                return Some(Err(e));
            }
            let input = std::fs::read(self.dir.join(&f.textures[0])).unwrap();
            std::fs::write(self.dir.join(&f.output), input).unwrap();
            Some(Ok(Response::Ack))
        }

        fn begin_blender_job(&self, _: &JobId) {}

        fn take_blender_surfaces(&self, _: &JobId) -> Vec<pm_protocol::blender::BlenderSurface> {
            vec![]
        }
    }

    fn job() -> FallbackJob {
        let pixels = FallbackPixels {
            width: 2,
            height: 1,
            rgba16f: vec![0x3c00, 0, 0, 0x3c00, 0, 0x3c00, 0, 0x3c00],
        };
        let request = pm_render::shader_layers::FallbackRequest {
            key: "ab".repeat(32),
            layer: "L".into(),
            vertex: "#version 300 es\nvoid main(){}".into(),
            fragment: "#version 300 es\nvoid main(){}".into(),
            width: 2,
            height: 1,
            calls: vec![],
            stage: "bind".into(),
            message: "sampler `u` (Cube)".into(),
            textures: 1,
            units: vec![pm_render::shader_layers::FallbackUnit {
                unit: 0,
                texture: 0,
                chain: true,
            }],
            more_passes: vec![],
        };
        FallbackJob {
            request,
            textures: vec![Arc::new(pixels)],
        }
    }

    #[test]
    fn requests_travel_as_files_in_the_scratch_area_and_are_cleaned_up() {
        let dir = std::env::temp_dir().join(format!("pm-fallback-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let echo = Arc::new(Echo {
            dir: dir.clone(),
            answer: Mutex::new(None),
            seen: Mutex::new(vec![]),
        });
        let host = HostFallback::new(
            echo.clone(),
            SessionId::new(3).unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let job = job();
        let out = host.run(&job).unwrap();
        assert_eq!(out, *job.textures[0]);
        let seen = echo.seen.lock().unwrap();
        assert_eq!(
            seen[0].textures,
            vec![format!("fallback-{}-0.rgba16f", job.request.key)]
        );
        assert!(pm_protocol::validate::Validate::validate(&seen[0]).is_ok());
        drop(seen);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "files removed");
        // The host's errors become runner failures.
        for (code, reason) in [
            (ErrorCode::ShaderCompile, "compile"),
            (ErrorCode::WatchdogTimeout, "watchdog"),
            (ErrorCode::Unsupported, "unavailable"),
        ] {
            *echo.answer.lock().unwrap() = Some(EngineError::new(
                code,
                ErrorScope::Session {
                    session: SessionId::new(3).unwrap(),
                },
                "why",
            ));
            let f = host.run(&job).unwrap_err();
            assert_eq!(f.reason, reason);
            assert_eq!(f.message, "why");
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
