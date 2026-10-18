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
}

/// What the runner draws for one layer at one frame: the compositor's vertex
/// shader, the wrapped fragment program, the target size, and the uniform
/// calls, into an RGBA16F target cleared to transparent with blending off.
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
    /// Renderer → interactive engine: the runner could not draw a request
    /// (`compile`, `deadline`, `contextLost`, `unavailable`, `size`).
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
                for r in requests {
                    ensure(key_ok(&r.key), c, || "invalid fallback key".into())?;
                    ensure(size_ok(r.width, r.height), c, || {
                        format!("fallback target {}×{}", r.width, r.height)
                    })?;
                    ensure(
                        r.vertex.len() <= MAX_SOURCE_BYTES && r.fragment.len() <= MAX_SOURCE_BYTES,
                        c,
                        || "fallback program too large".into(),
                    )?;
                    ensure(r.calls.len() <= MAX_CALLS, c, || {
                        format!("{} uniform calls", r.calls.len())
                    })?;
                    for call in &r.calls {
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
        };
        let call = |f: &str, n: usize| ShaderFallbackCall {
            function: f.into(),
            name: "x".into(),
            v: vec![0.0; n],
        };
        let ok = ShaderFallbackMessage::Requests {
            requests: vec![request(
                8,
                vec![call("2f", 2), call("m3", 9), call("1i", 1)],
            )],
        };
        assert!(ok.validate().is_ok());
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
