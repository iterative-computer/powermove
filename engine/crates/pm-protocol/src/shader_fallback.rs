//! The shader fallback runner's messages (contract 10's extension for
//! decision 19; plan section 11: "a shader that fails translation on a
//! user's machine renders through a minimal GLSL runner in the renderer (one
//! WebGL2 context, the contract below, no compositor), uploaded to the engine
//! like the `raster` override").
//!
//! The interactive engine asks the viewer's renderer to draw programs the
//! native pipeline cannot translate ([`ShaderFallbackMessage::Requests`]);
//! the renderer answers each with its pixels in row bands that fit a message
//! ([`ShaderFallbackMessage::Pixels`]) or with why it could not
//! ([`ShaderFallbackMessage::Failed`]). Session-bound, both directions on
//! the viewer's port.
//!
//! Effects and transitions read textures (`u_tex`, `u_orig`, `u_backdrop`,
//! `u_from`, `u_to`): a request names how many it samples
//! ([`ShaderFallbackRequest::textures`], each the target's size), which
//! unit samples which ([`ShaderFallbackUnit`]), and the calls of every pass
//! after the first (an effect's ping-pong passes). The engine sends their
//! pixels after the request ([`ShaderFallbackMessage::Input`] bands); the
//! renderer runs the request once every band has arrived.
//!
//! The job engine has no viewer: it asks the host ([`HostShaderFallback`],
//! contract 14), which runs the request in a window's runner. The textures
//! and the result travel as files in the scratch cache area.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::Bytes;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Largest target edge (the compositor's 4,096 px cap on shader layers).
pub const MAX_EDGE: u32 = 4096;
/// Largest pixel band in one message (binary16 RGBA bytes).
pub const MAX_BAND_BYTES: usize = 8 * 1024 * 1024;
/// Largest wrapped program (`definitions::limits::WRAPPED_SOURCE_BYTES`).
pub const MAX_SOURCE_BYTES: usize = 512 * 1024;
/// Most uniform calls in one request.
pub const MAX_CALLS: usize = 1024;
/// Most requests in one message.
pub const MAX_REQUESTS: usize = 64;
/// Most textures one request samples (an effect's input and backdrop, a
/// transition's two frames).
pub const MAX_TEXTURES: u32 = 4;
/// Most passes of one request (section 11: 1 to 8).
pub const MAX_PASSES: usize = 8;
/// Texture units a request may sample.
pub const MAX_UNITS: u32 = 16;

/// One `glUniform*` call in issue order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct ShaderFallbackCall {
    /// `1f`, `2f`, `3f`, `4f`, `1i`, or `m3` (column-major).
    #[serde(rename = "fn")]
    #[cfg_attr(feature = "ts", ts(rename = "fn"))]
    pub function: String,
    pub name: String,
    pub v: Vec<f64>,
    /// Applied to this uniform instead when `name` is not active in the
    /// linked program (`setParam`'s positional alias `u_p<i>` or `u_c<i>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub alias: Option<String>,
}

/// Which texture a unit samples: texture `texture` of the request, or from
/// the second pass on (`chain`) the previous pass's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShaderFallbackUnit {
    pub unit: u32,
    pub texture: u32,
    #[serde(default)]
    pub chain: bool,
}

/// What the runner draws for one layer at one frame: the compositor's vertex
/// shader, the wrapped fragment program, the target size, and the uniform
/// calls, into an RGBA16F target cleared to transparent with blending off.
/// Textures (when `textures` > 0) are RGBA16F of the target's size, sampled
/// linear with clamp-to-edge; units not named sample nothing (0, 0, 0, 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShaderFallbackRequest {
    /// SHA-256 (hex) of everything that decides the pixels.
    pub key: String,
    pub layer: String,
    pub vertex: String,
    pub fragment: String,
    pub width: u32,
    pub height: u32,
    pub calls: Vec<ShaderFallbackCall>,
    /// Where the native pipeline failed (`pm_glsl::FailureStage::name`).
    pub stage: String,
    pub message: String,
    /// Textures the request samples (their pixels follow as `Input` bands,
    /// or as files for the host).
    #[serde(default)]
    pub textures: u32,
    #[serde(default)]
    pub units: Vec<ShaderFallbackUnit>,
    /// Calls of the passes after the first, in order (each into a new
    /// target; a `chain` unit samples the previous one).
    #[serde(default)]
    pub more_passes: Vec<Vec<ShaderFallbackCall>>,
}

/// Rows `row .. row + rows` of texture `texture` of a request: binary16
/// RGBA, little-endian, rows in GL order (row 0 at the bottom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShaderFallbackInput {
    pub key: String,
    pub texture: u32,
    pub width: u32,
    pub height: u32,
    pub row: u32,
    pub rows: u32,
    pub rgba16f: Bytes,
}

/// Job engine → host (contract 14): run `request` in a window's runner.
/// `area` is a `cache {scratch}` handle of the job's session; `textures`
/// (one file per texture) and `output` name files in it: raw binary16 RGBA,
/// little-endian, rows in GL order, `width × height × 8` bytes. The host
/// answers `ack` once `output` is written, or an error: `shaderCompile`
/// (WebGL refuses the program), `watchdogTimeout` (its draw passed the
/// runner's deadline), `unsupported` (no runner is available).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HostShaderFallback {
    pub request: ShaderFallbackRequest,
    pub area: crate::host::CapabilityHandle,
    pub textures: Vec<String>,
    pub output: String,
}

/// A file name inside a cache area: letters, digits, `-`, `_`, `.`, not
/// starting with `.`, at most 128 bytes.
pub fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn request_ok(r: &ShaderFallbackRequest) -> Result<(), Violation> {
    let c = ContractId::Definitions;
    ensure(key_ok(&r.key), c, || "invalid fallback key".into())?;
    ensure(size_ok(r.width, r.height), c, || {
        format!("fallback target {}×{}", r.width, r.height)
    })?;
    ensure(
        r.vertex.len() <= MAX_SOURCE_BYTES && r.fragment.len() <= MAX_SOURCE_BYTES,
        c,
        || "fallback program too large".into(),
    )?;
    ensure(r.textures <= MAX_TEXTURES, c, || {
        format!("{} fallback textures", r.textures)
    })?;
    ensure(r.more_passes.len() < MAX_PASSES, c, || {
        format!("{} fallback passes", r.more_passes.len() + 1)
    })?;
    ensure(r.units.len() as u32 <= MAX_UNITS, c, || {
        "too many units".into()
    })?;
    for u in &r.units {
        ensure(u.unit < MAX_UNITS && u.texture < r.textures, c, || {
            format!("unit {} samples texture {}", u.unit, u.texture)
        })?;
    }
    for calls in std::iter::once(&r.calls).chain(&r.more_passes) {
        ensure(calls.len() <= MAX_CALLS, c, || {
            format!("{} uniform calls", calls.len())
        })?;
        for call in calls {
            let n = match call.function.as_str() {
                "1f" | "1i" => 1,
                "2f" => 2,
                "3f" => 3,
                "4f" => 4,
                "m3" => 9,
                _ => 0,
            };
            ensure(n > 0 && call.v.len() == n, c, || {
                format!("uniform call {} {}", call.function, call.name)
            })?;
        }
    }
    Ok(())
}

impl Validate for HostShaderFallback {
    fn validate(&self) -> Result<(), Violation> {
        let c = ContractId::HostServices;
        request_ok(&self.request)?;
        ensure(
            self.textures.len() as u32 == self.request.textures,
            c,
            || {
                format!(
                    "{} texture files for {} textures",
                    self.textures.len(),
                    self.request.textures
                )
            },
        )?;
        ensure(
            self.textures.iter().all(|n| file_name_ok(n)) && file_name_ok(&self.output),
            c,
            || "fallback file names must be plain names in the area".into(),
        )
    }
}

/// Rows `row .. row + rows` of a request's result: binary16 RGBA,
/// little-endian, rows in GL order (row 0 at the bottom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShaderFallbackPixels {
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub row: u32,
    pub rows: u32,
    pub rgba16f: Bytes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "fallback"
)]
pub enum ShaderFallbackMessage {
    /// Interactive engine → renderer: programs to draw.
    Requests {
        requests: Vec<ShaderFallbackRequest>,
    },
    /// Renderer → interactive engine: one band of a request's pixels.
    Pixels(ShaderFallbackPixels),
    /// Interactive engine → renderer: one band of a texture a request
    /// samples (sent after the request).
    Input(ShaderFallbackInput),
    /// Renderer → interactive engine: the runner could not draw a request
    /// (`compile`, `deadline`, `contextLost`, `unavailable`, `size`, or
    /// `watchdog`: a draw passed the runner's deadline).
    Failed {
        key: String,
        reason: String,
        message: String,
    },
}

fn key_ok(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

fn size_ok(width: u32, height: u32) -> bool {
    (1..=MAX_EDGE).contains(&width) && (1..=MAX_EDGE).contains(&height)
}

impl Validate for ShaderFallbackMessage {
    fn validate(&self) -> Result<(), Violation> {
        let c = ContractId::Definitions;
        match self {
            ShaderFallbackMessage::Requests { requests } => {
                ensure(requests.len() <= MAX_REQUESTS, c, || {
                    format!("{} fallback requests in one message", requests.len())
                })?;
                requests.iter().try_for_each(request_ok)
            }
            ShaderFallbackMessage::Input(p) => {
                ensure(key_ok(&p.key), c, || "invalid fallback key".into())?;
                ensure(p.texture < MAX_TEXTURES, c, || "invalid texture".into())?;
                ensure(size_ok(p.width, p.height), c, || {
                    format!("fallback texture {}×{}", p.width, p.height)
                })?;
                ensure(
                    p.rows >= 1 && u64::from(p.row) + u64::from(p.rows) <= u64::from(p.height),
                    c,
                    || format!("rows {} + {} of {}", p.row, p.rows, p.height),
                )?;
                let expected = u64::from(p.width) * u64::from(p.rows) * 8;
                ensure(
                    p.rgba16f.0.len() as u64 == expected && p.rgba16f.0.len() <= MAX_BAND_BYTES,
                    c,
                    || format!("{} texture bytes for {} rows", p.rgba16f.0.len(), p.rows),
                )
            }
            ShaderFallbackMessage::Pixels(p) => {
                ensure(key_ok(&p.key), c, || "invalid fallback key".into())?;
                ensure(size_ok(p.width, p.height), c, || {
                    format!("fallback pixels {}×{}", p.width, p.height)
                })?;
                ensure(
                    p.rows >= 1 && u64::from(p.row) + u64::from(p.rows) <= u64::from(p.height),
                    c,
                    || format!("rows {} + {} of {}", p.row, p.rows, p.height),
                )?;
                let expected = u64::from(p.width) * u64::from(p.rows) * 8;
                ensure(
                    p.rgba16f.0.len() as u64 == expected && p.rgba16f.0.len() <= MAX_BAND_BYTES,
                    c,
                    || {
                        format!(
                            "{} pixel bytes for {} rows of {}",
                            p.rgba16f.0.len(),
                            p.rows,
                            p.width
                        )
                    },
                )
            }
            ShaderFallbackMessage::Failed { key, .. } => {
                ensure(key_ok(key), c, || "invalid fallback key".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> String {
        "ab".repeat(32)
    }

    #[test]
    fn bands_must_fit_their_rows() {
        let ok = ShaderFallbackMessage::Pixels(ShaderFallbackPixels {
            key: key(),
            width: 4,
            height: 4,
            row: 2,
            rows: 2,
            rgba16f: Bytes(vec![0; 4 * 2 * 8]),
        });
        assert!(ok.validate().is_ok());
        let past = ShaderFallbackMessage::Pixels(ShaderFallbackPixels {
            key: key(),
            width: 4,
            height: 4,
            row: 3,
            rows: 2,
            rgba16f: Bytes(vec![0; 4 * 2 * 8]),
        });
        assert!(past.validate().is_err());
        let short = ShaderFallbackMessage::Pixels(ShaderFallbackPixels {
            key: key(),
            width: 4,
            height: 4,
            row: 0,
            rows: 1,
            rgba16f: Bytes(vec![0; 8]),
        });
        assert!(short.validate().is_err());
    }

    #[test]
    fn requests_are_bounded() {
        let request = |w: u32, calls: Vec<ShaderFallbackCall>| ShaderFallbackRequest {
            key: key(),
            layer: "L1".into(),
            vertex: String::new(),
            fragment: String::new(),
            width: w,
            height: 8,
            calls,
            stage: "bind".into(),
            message: String::new(),
            textures: 0,
            units: vec![],
            more_passes: vec![],
        };
        let call = |f: &str, n: usize| ShaderFallbackCall {
            function: f.into(),
            name: "x".into(),
            v: vec![0.0; n],
            alias: None,
        };
        let ok = ShaderFallbackMessage::Requests {
            requests: vec![request(
                8,
                vec![call("2f", 2), call("m3", 9), call("1i", 1)],
            )],
        };
        assert!(ok.validate().is_ok());
        let mut effect = request(8, vec![call("1i", 1)]);
        effect.textures = 2;
        effect.units = vec![
            ShaderFallbackUnit {
                unit: 0,
                texture: 0,
                chain: true,
            },
            ShaderFallbackUnit {
                unit: 2,
                texture: 1,
                chain: false,
            },
        ];
        effect.more_passes = vec![vec![call("1i", 1)]; 7];
        let ok = ShaderFallbackMessage::Requests {
            requests: vec![effect.clone()],
        };
        assert!(ok.validate().is_ok());
        let mut past = effect.clone();
        past.units[1].texture = 2;
        let mut passes = effect.clone();
        passes.more_passes.push(vec![]);
        for bad in [past, passes] {
            assert!(
                ShaderFallbackMessage::Requests {
                    requests: vec![bad]
                }
                .validate()
                .is_err()
            );
        }
        let host = |names: Vec<&str>| HostShaderFallback {
            request: effect.clone(),
            area: crate::host::CapabilityHandle::from_random_bytes([0xab; 16]),
            textures: names.into_iter().map(String::from).collect(),
            output: "out.rgba16f".into(),
        };
        assert!(
            host(vec!["in-0.rgba16f", "in-1.rgba16f"])
                .validate()
                .is_ok()
        );
        assert!(host(vec!["in-0.rgba16f"]).validate().is_err());
        assert!(host(vec!["../x", "in-1"]).validate().is_err());
        for bad in [
            request(MAX_EDGE + 1, vec![]),
            request(8, vec![call("2f", 3)]),
            request(8, vec![call("5f", 5)]),
        ] {
            assert!(
                ShaderFallbackMessage::Requests {
                    requests: vec![bad]
                }
                .validate()
                .is_err()
            );
        }
    }
}
