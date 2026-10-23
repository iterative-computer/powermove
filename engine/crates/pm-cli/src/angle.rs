//! The Node CLI's on-demand Chromium service. The renderer's existing
//! FallbackHost contract keeps unknown/pinned programs off the unverified
//! native path, and its level 1 SVG images are rasterized by the same
//! Chromium (decision S1), as the hidden host's on-demand Chromium does.
use pm_render::shader_layers::{FallbackFailure, FallbackHost, FallbackJob, FallbackPixels};
use pm_render::vector::svg_rasters::{SvgRasterNeed, SvgRasterSource, SvgRasterizer};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug)]
struct RunnerFailure(FallbackFailure);
impl From<String> for RunnerFailure {
    fn from(message: String) -> Self {
        Self(FallbackFailure {
            reason: "unavailable".into(),
            message,
        })
    }
}
impl From<&str> for RunnerFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

pub struct AngleHost {
    endpoint: SocketAddr,
    token: String,
    child: Option<Mutex<Child>>,
    last_failure: Mutex<Option<FallbackFailure>>,
}
impl AngleHost {
    pub fn from_environment() -> Result<Option<Self>, String> {
        let mut child = None;
        let (endpoint, token) = if let Ok(endpoint) = std::env::var("POWERMOVE_ANGLE_ENDPOINT") {
            (
                endpoint,
                std::env::var("POWERMOVE_ANGLE_TOKEN")
                    .map_err(|_| "ANGLE service token is missing")?,
            )
        } else {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let distribution = executable
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent());
            let helper = std::env::var_os("POWERMOVE_ANGLE_HELPER")
                .map(std::path::PathBuf::from)
                .or_else(|| distribution.map(|p| p.join("native-angle-child.cjs")));
            let Some(helper) = helper.filter(|p| p.is_file()) else {
                return Ok(None);
            };
            let node = std::env::var_os("POWERMOVE_NODE").unwrap_or_else(|| "node".into());
            let mut process = Command::new(node)
                .arg(&helper)
                .arg(helper.parent().ok_or("ANGLE helper has no directory")?)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| format!("angle-runner-unavailable: {e}"))?;
            let stdout = process.stdout.take().ok_or("ANGLE helper has no output")?;
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut line = String::new();
                let result = BufReader::new(stdout)
                    .take(16384)
                    .read_line(&mut line)
                    .map(|_| line);
                let _ = send.send(result);
            });
            let line = match receive.recv_timeout(Duration::from_secs(15)) {
                Ok(Ok(line)) => line,
                _ => {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err("angle-runner-unavailable: helper did not start within 15 s".into());
                }
            };
            let result: Result<serde_json::Value, _> = serde_json::from_str(&line);
            let result = match result {
                Ok(value) => value,
                Err(error) => {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err(format!("angle-runner-unavailable: {error}"));
                }
            };
            let endpoint = result["POWERMOVE_ANGLE_ENDPOINT"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let token = result["POWERMOVE_ANGLE_TOKEN"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            child = Some(Mutex::new(process));
            (endpoint, token)
        };
        let endpoint: SocketAddr = endpoint
            .parse()
            .map_err(|_| "invalid ANGLE loopback endpoint")?;
        if !endpoint.ip().is_loopback() {
            return Err("ANGLE service must be on loopback".into());
        }
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid ANGLE service token".into());
        }
        Ok(Some(Self {
            endpoint,
            token,
            child,
            last_failure: Mutex::new(None),
        }))
    }
    fn request(&self, job: &FallbackJob) -> Result<FallbackPixels, RunnerFailure> {
        let float32 = job.request.color_depth == 32;
        let bins: Vec<_> = job
            .textures
            .iter()
            .enumerate()
            .map(|(i, _)| serde_json::json!({"$bin": i}))
            .collect();
        let textures: Vec<Vec<u8>> = job
            .textures
            .iter()
            .map(|t| {
                if float32 {
                    t.rgba32f
                        .as_ref()
                        .map(|bits| pm_render::shader_layers::fallback::float_bytes(bits))
                        .ok_or_else(|| RunnerFailure::from("missing float32 ANGLE input"))
                } else {
                    Ok(pm_render::shader_layers::fallback::half_bytes(&t.rgba16f))
                }
            })
            .collect::<Result<_, _>>()?;
        let body = frame(
            &serde_json::json!({"request": job.request, "textures": bins}),
            &textures,
        )?;
        let expected = (job.request.width as usize)
            .checked_mul(job.request.height as usize)
            .and_then(|n| n.checked_mul(if float32 { 16 } else { 8 }))
            .ok_or("ANGLE target is too large")?;
        if expected > 512 * 1024 * 1024 {
            return Err("ANGLE target exceeds 512 MB".into());
        }
        let bytes = self.post("/", &body, expected)?;
        decode(
            &bytes,
            &job.request.key,
            job.request.width,
            job.request.height,
            job.request.color_depth,
        )
    }

    /// One authenticated request to the service: the reply's body.
    fn post(&self, path: &str, body: &[u8], expected: usize) -> Result<Vec<u8>, String> {
        let mut stream = TcpStream::connect_timeout(&self.endpoint, Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| e.to_string())?;
        write!(stream, "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", self.endpoint, self.token, body.len()).map_err(|e| e.to_string())?;
        stream.write_all(body).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        stream
            .take((expected + 1024 * 1024 + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let start = bytes
            .windows(4)
            .position(|b| b == b"\r\n\r\n")
            .ok_or("invalid ANGLE HTTP response")?
            + 4;
        if !bytes.starts_with(b"HTTP/1.1 200 ") {
            return Err("ANGLE service refused the request".into());
        }
        bytes.drain(..start);
        Ok(bytes)
    }

    /// A level 1 SVG raster drawn by the service's Chromium with the app's
    /// module (`svg-rasters.ts`: `decodeSvg`, then `SvgRasterCanvas`, today's
    /// `rasterSvgAsset` and `texFor` upload read back): premultiplied RGBA8,
    /// top-down. The page checks the bytes still hash to the planned content.
    fn svg_raster(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String> {
        let key = &need.request.key;
        let failed = |why: String| {
            format!(
                "Chromium could not rasterize the level 1 SVG image {} ({}x{}) of layer {}: {why}",
                need.path.display(),
                key.width,
                key.height,
                need.layer
            )
        };
        let svg = std::fs::read(&need.path).map_err(|e| failed(e.to_string()))?;
        let expected = usize::try_from(key.bytes()).map_err(|_| failed("too large".into()))?;
        if expected > 512 * 1024 * 1024 {
            return Err(failed("the raster exceeds 512 MB".into()));
        }
        let body = frame(
            &serde_json::json!({
                "key": key,
                "svg": { "$bin": 0 },
            }),
            &[svg],
        )
        .map_err(failed)?;
        let reply = self.post("/svg", &body, expected).map_err(failed)?;
        let (header, mut bins) = unframe(&reply).map_err(failed)?;
        if header["ok"] != true {
            return Err(failed(format!(
                "{}: {}",
                header["reason"].as_str().unwrap_or("unavailable"),
                header["message"]
                    .as_str()
                    .unwrap_or("the SVG raster failed")
            )));
        }
        if header["pixels"]["$bin"] != 0 || bins.len() != 1 {
            return Err(failed("invalid SVG pixel slot".into()));
        }
        let pixels = bins.remove(0);
        if pixels.len() != expected {
            return Err(failed(format!(
                "{} bytes for a {}x{} raster",
                pixels.len(),
                key.width,
                key.height
            )));
        }
        Ok(pixels)
    }
}
impl Drop for AngleHost {
    fn drop(&mut self) {
        let Some(child) = &self.child else { return };
        let Ok(mut child) = child.lock() else { return };
        // EOF closes the HTTP service and its browser. A dead parent also
        // closes this pipe, so a force-killed CLI leaves no Chromium child.
        drop(child.stdin.take());
        for _ in 0..250 {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl SvgRasterSource for AngleHost {
    fn fetch(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String> {
        self.svg_raster(need)
    }
}

/// The named error of a level 1 SVG image with no Chromium to draw it.
pub const SVG_CHROMIUM_UNAVAILABLE: &str = "svg-chromium-unavailable";

/// Level 1 SVG rasters when this pm-cli has no Chromium helper at all: every
/// frame that draws one fails with [`SVG_CHROMIUM_UNAVAILABLE`]. Exports are
/// atomic, so nothing is written.
struct NoChromium;

impl SvgRasterSource for NoChromium {
    fn fetch(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String> {
        Err(format!(
            "{SVG_CHROMIUM_UNAVAILABLE}: layer {} of {} draws the level 1 SVG image {}. The app \
             rasterizes SVG images with Chromium, and this pm-cli has no Chromium helper \
             (native-angle-child.cjs beside its distribution, POWERMOVE_ANGLE_HELPER, or \
             POWERMOVE_ANGLE_ENDPOINT). Render with `powermove render`, or pass \
             --approximate-svg to draw it with resvg, an approximation of the app's pixels.",
            need.layer,
            need.composition,
            need.path.display()
        ))
    }
}

/// Who draws level 1 SVG images for a pm-cli renderer (decision S1): the
/// helper's on-demand Chromium whenever there is one; with none, resvg only
/// when the user accepted an approximation (`--approximate-svg`, reported by
/// [`crate::render::approximate_svg_notice`]); otherwise a named failure.
pub fn configure_svg(
    renderer: &mut pm_render::Renderer,
    host: Option<&std::sync::Arc<AngleHost>>,
    approximate: bool,
) {
    match host {
        Some(host) => {
            renderer.set_svg_rasterizer(SvgRasterizer::Chromium);
            renderer.set_svg_source(Some(host.clone()));
        }
        None if approximate => {
            renderer.set_svg_rasterizer(SvgRasterizer::Resvg);
            renderer.set_svg_source(None);
        }
        None => {
            renderer.set_svg_rasterizer(SvgRasterizer::Chromium);
            renderer.set_svg_source(Some(std::sync::Arc::new(NoChromium)));
        }
    }
}

/// The service's wire frame (`shared/wire.ts` `encodeFrame`): a big-endian
/// length and the JSON header, then each binary slot with its length.
fn frame(header: &serde_json::Value, bins: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let header = serde_json::to_vec(header).map_err(|e| e.to_string())?;
    let mut body =
        Vec::with_capacity(header.len() + 4 + bins.iter().map(|b| b.len() + 4).sum::<usize>());
    body.extend_from_slice(&(header.len() as u32).to_be_bytes());
    body.extend_from_slice(&header);
    for bin in bins {
        body.extend_from_slice(
            &u32::try_from(bin.len())
                .map_err(|_| "a slot over 4 GB")?
                .to_be_bytes(),
        );
        body.extend_from_slice(bin);
    }
    Ok(body)
}

/// [`frame`]'s inverse (`decodeFrame`), with a 1 MB header limit.
fn unframe(bytes: &[u8]) -> Result<(serde_json::Value, Vec<Vec<u8>>), String> {
    let length =
        u32::from_be_bytes(bytes.get(..4).ok_or("truncated reply")?.try_into().unwrap()) as usize;
    if length > 1024 * 1024 {
        return Err("reply header exceeds 1 MB".into());
    }
    let header: serde_json::Value =
        serde_json::from_slice(bytes.get(4..4 + length).ok_or("truncated reply header")?)
            .map_err(|e| e.to_string())?;
    let mut at = 4 + length;
    let mut bins = Vec::new();
    while at < bytes.len() {
        let size = u32::from_be_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("truncated reply slot table")?
                .try_into()
                .unwrap(),
        ) as usize;
        at += 4;
        bins.push(
            bytes
                .get(at..at + size)
                .ok_or("truncated reply slot")?
                .to_vec(),
        );
        at += size;
    }
    Ok((header, bins))
}

impl FallbackHost for AngleHost {
    fn run(&self, job: &FallbackJob) -> Result<FallbackPixels, FallbackFailure> {
        let result = self.request(job).map_err(|failure| failure.0);
        *self.last_failure.lock().unwrap_or_else(|e| e.into_inner()) =
            result.as_ref().err().cloned();
        result
    }
}

impl AngleHost {
    /// Keep the runner's named cause when the renderer stops making progress
    /// after a transient failure (for example a missing Chromium installation).
    pub fn last_failure(&self) -> Option<FallbackFailure> {
        self.last_failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
fn decode(
    bytes: &[u8],
    _key: &str,
    width: u32,
    height: u32,
    color_depth: u8,
) -> Result<FallbackPixels, RunnerFailure> {
    let length = u32::from_be_bytes(
        bytes
            .get(..4)
            .ok_or("truncated ANGLE reply")?
            .try_into()
            .unwrap(),
    ) as usize;
    if length > 1024 * 1024 {
        return Err("ANGLE reply header exceeds 1 MB".into());
    }
    let header: serde_json::Value =
        serde_json::from_slice(bytes.get(4..4 + length).ok_or("truncated ANGLE header")?)
            .map_err(|e| e.to_string())?;
    if header["ok"] != true {
        return Err(RunnerFailure(FallbackFailure {
            reason: header["reason"].as_str().unwrap_or("unavailable").into(),
            message: header["message"]
                .as_str()
                .unwrap_or("ANGLE runner failed")
                .into(),
        }));
    }
    if header["pixels"]["$bin"] != 0 {
        return Err("invalid ANGLE pixel slot".into());
    }
    let at = 4 + length;
    let size = u32::from_be_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("missing ANGLE pixels")?
            .try_into()
            .unwrap(),
    ) as usize;
    let expected = width as usize * height as usize * if color_depth == 32 { 16 } else { 8 };
    if size != expected || bytes.len() != at + 4 + size {
        return Err("ANGLE pixels do not match the target".into());
    }
    Ok(FallbackPixels {
        width,
        height,
        rgba16f: if color_depth == 32 {
            Vec::new()
        } else {
            pm_render::shader_layers::fallback::halves(&bytes[at + 4..])
        },
        rgba32f: (color_depth == 32)
            .then(|| pm_render::shader_layers::fallback::float_bits(&bytes[at + 4..])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_runner_failure_reason() {
        let header = br#"{"ok":false,"reason":"compile","message":"fixture compile failure"}"#;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
        bytes.extend_from_slice(header);
        let failure = decode(&bytes, "a", 1, 1, 16).unwrap_err().0;
        assert_eq!(failure.reason, "compile");
        assert_eq!(failure.message, "fixture compile failure");
    }
    #[test]
    fn svg_frames_round_trip_the_wire_format() {
        let body = frame(
            &serde_json::json!({"key": {"content": "c", "width": 2, "height": 1}, "svg": {"$bin": 0}}),
            &[b"<svg/>".to_vec()],
        )
        .unwrap();
        let (header, bins) = unframe(&body).unwrap();
        assert_eq!(header["svg"]["$bin"], 0);
        assert_eq!(bins, [b"<svg/>".to_vec()]);
        assert!(unframe(&body[..body.len() - 1]).is_err());
    }
    #[test]
    fn without_a_helper_a_level_1_svg_fails_by_name() {
        let need = SvgRasterNeed {
            request: pm_protocol::svg_raster::SvgRasterRequest {
                key: pm_protocol::svg_raster::SvgRasterKey {
                    content: "0".repeat(64),
                    texture: None,
                    width: 4,
                    height: 4,
                },
                asset: "a".into(),
            },
            path: "/tmp/art.svg".into(),
            composition: "P-root".into(),
            layer: "L1".into(),
        };
        let error = NoChromium.fetch(&need).unwrap_err();
        assert!(
            error.starts_with("svg-chromium-unavailable: layer L1 of P-root"),
            "{error}"
        );
        assert!(error.contains("--approximate-svg"), "{error}");
    }
    #[test]
    fn accepts_exact_binary16_and_rejects_truncation_or_extra_pixels() {
        let header = br#"{"ok":true,"pixels":{"$bin":0}}"#;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(&[0, 60, 0, 0, 0, 0, 0, 60]);
        assert_eq!(
            decode(&bytes, "a", 1, 1, 16).unwrap().rgba16f,
            [0x3c00, 0, 0, 0x3c00]
        );
        assert!(decode(&bytes[..bytes.len() - 1], "a", 1, 1, 16).is_err());
        bytes.push(0);
        assert!(decode(&bytes, "a", 1, 1, 16).is_err());
    }
    #[test]
    fn accepts_exact_binary32_without_half_conversion() {
        let header = br#"{"ok":true,"pixels":{"$bin":0}}"#;
        let bits = [0x3f80_0001u32, 0x3f00_0001, 0x0000_0001, 0xbf80_0001];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&16u32.to_be_bytes());
        for value in bits {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let pixels = decode(&bytes, "a", 1, 1, 32).unwrap();
        assert!(pixels.rgba16f.is_empty());
        assert_eq!(pixels.rgba32f, Some(bits.to_vec()));
        assert!(decode(&bytes[..bytes.len() - 1], "a", 1, 1, 32).is_err());
    }
}

/// Read the same profile descriptor even when pm-cli is invoked directly.
///
/// The desktop's namespace is honoured read only. Its decisions compare
/// native output with Electron's WebGL runner; this CLI's runner is
/// Playwright's Chromium (possibly SwiftShader over ssh or launchd), so a
/// first probe here is decided in memory and never leaves a `.native` or
/// `.runner` marker the desktop would trust (amendment 22).
pub fn configure_probes(renderer: &mut pm_render::Renderer) -> Result<(), String> {
    renderer.read_only_shader_reference();
    if std::env::var_os("POWERMOVE_ENGINE_PARITY_ENV").is_some() {
        return Ok(());
    }
    let directory = if let Some(cache) = std::env::var_os("POWERMOVE_ENGINE_SHADER_CACHE") {
        std::path::PathBuf::from(cache)
    } else {
        let profile = std::env::var_os("POWERMOVE_USER_DATA")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| {
                    std::path::PathBuf::from(home).join("Library/Application Support/Powermove")
                })
            });
        let Some(profile) = profile else {
            return Ok(());
        };
        profile.join("Engine Cache/Shader Cache")
    };
    let bytes = match std::fs::read(directory.join("desktop-environment.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("shader-probe-store-unreadable: {error}")),
    };
    let record: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("shader-probe-store-unreadable: {e}"))?;
    if record["v"] == 1 {
        if let Some(environment) = record["environment"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 16384)
        {
            renderer.configure_shader_reference(environment, Some(directory.join("angle-parity")));
        }
    }
    Ok(())
}
