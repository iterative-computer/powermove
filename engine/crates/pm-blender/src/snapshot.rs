//! The Blender snapshot (contract 11, `SnapshotEncoding::RenderSnapshotJsonV1`).
//!
//! A snapshot is what today's `renderSnapshot` (`core/scene3d/rendering.ts`)
//! hands the worker for one composition at one time and size: the evaluated
//! geometry and world matrices of every visible 3D object layer, their
//! materials, the lights, the camera, the depth sets (`blocks`), the layers
//! that draw the background (`backgroundFor`), the environment, and the
//! render settings, plus the asset files the materials reference. The
//! worker never owns the project or its timeline.
//!
//! The JSON tree is kept as `pm_model::Value` so a snapshot compares
//! directly with the corpus capture of today's output ([`crate::compare`]).
//! Its canonical bytes (`pm_model::json::to_canonical_string`: sorted keys,
//! `JSON.stringify` numbers) are what the cache file holds and what
//! [`Snapshot::hash`] addresses.

use std::path::PathBuf;
use std::sync::Arc;

use pm_model::Value;
use pm_model::json::{Map, to_canonical_string};

/// One Blender frame: a composition at an evaluation time and output size,
/// as `blenderFrameRequests` lists it (one entry per frame, shutter sample,
/// and nested composition).
#[derive(Debug, Clone, PartialEq)]
pub struct BlenderFrame {
    /// Composition ids, outermost first (length 1 for the root; at most 8
    /// levels below it, as today's walk allows).
    pub composition_path: Vec<String>,
    /// Evaluation time in the composition's own seconds, shutter offset
    /// included (`t + ((i + 0.5) / n − 0.5) × shutter / fps`).
    pub time: f64,
    /// The composition's output size at this nesting level (before the
    /// preview scale).
    pub width: u32,
    pub height: u32,
    /// Shutter sample `index` of `count` (`None` for the frame's own time).
    pub shutter: Option<(u16, u16)>,
    /// A paused rendered preview (preview samples and scale).
    pub preview: bool,
}

impl BlenderFrame {
    /// The composition this frame renders.
    pub fn composition(&self) -> &str {
        self.composition_path.last().map_or("", String::as_str)
    }

    /// The renderer's lookup key of this frame: composition, time bits, the
    /// size it is drawn at, and the preview flag (today's `frameKey` without
    /// the project id).
    pub fn lookup(&self) -> FrameKey {
        FrameKey {
            composition: self.composition().to_owned(),
            time_bits: self.time.to_bits(),
            width: self.width,
            height: self.height,
            preview: self.preview,
        }
    }
}

/// Identifies the surfaces of one Blender frame inside a render
/// (`frameKey(PM, comp, time, preview, width, height)` today).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameKey {
    pub composition: String,
    pub time_bits: u64,
    pub width: u32,
    pub height: u32,
    pub preview: bool,
}

/// Where an asset's bytes are.
#[derive(Debug, Clone, PartialEq)]
pub enum AssetBytes {
    /// The asset's own file (PNG, JPEG, and WebP images, `.blend` sources).
    File(PathBuf),
    /// Bytes made for the job (an image converted to PNG).
    Bytes(Arc<Vec<u8>>),
}

impl AssetBytes {
    pub fn read(&self) -> std::io::Result<Arc<Vec<u8>>> {
        match self {
            AssetBytes::File(p) => std::fs::read(p).map(Arc::new),
            AssetBytes::Bytes(b) => Ok(b.clone()),
        }
    }
}

/// An asset the snapshot references: today's `{id, name, data}`.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotAsset {
    /// The id materials name (an asset id, or `texture_<n>` for a model's
    /// own textures).
    pub id: String,
    /// `<id>.<png|jpg|webp>` for images, the asset's name otherwise; the
    /// worker loads it by this extension.
    pub name: String,
    pub bytes: AssetBytes,
    /// SHA-256 of the bytes (the corpus records asset bytes by SHA-256).
    pub sha256: [u8; 32],
}

impl SnapshotAsset {
    /// An asset whose bytes are read now to hash them.
    pub fn new(id: String, name: String, bytes: AssetBytes) -> std::io::Result<SnapshotAsset> {
        use sha2::{Digest, Sha256};
        let data = bytes.read()?;
        let sha256 = Sha256::digest(data.as_slice()).into();
        Ok(SnapshotAsset {
            id,
            name,
            bytes,
            sha256,
        })
    }

    /// The asset's content hash for the surface key (its SHA-256).
    pub fn hash(&self) -> pm_protocol::common::ContentHash {
        pm_protocol::common::ContentHash(self.sha256)
    }
}

/// A Blender snapshot of one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub frame: BlenderFrame,
    /// The `snapshot` object today's worker receives.
    pub json: Value,
    pub assets: Vec<SnapshotAsset>,
}

impl Snapshot {
    /// The snapshot's canonical bytes (`renderSnapshotJsonV1`).
    pub fn encode(&self) -> Vec<u8> {
        to_canonical_string(&self.json).into_bytes()
    }

    /// BLAKE3 of [`Snapshot::encode`] (contract 11's `SnapshotRef.hash`).
    pub fn hash(&self) -> pm_protocol::common::ContentHash {
        pm_protocol::common::ContentHash(*blake3::hash(&self.encode()).as_bytes())
    }

    fn map(&self) -> Option<&Map> {
        self.json.as_object()
    }

    /// The object layers the snapshot renders, in snapshot order.
    pub fn layers(&self) -> Vec<String> {
        self.map()
            .and_then(|m| m.get("objects"))
            .and_then(Value::as_array)
            .map(|objects| {
                objects
                    .iter()
                    .filter_map(|o| o.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The render size (`width`, `height`: the frame size times the preview
    /// scale, rounded, at least 2).
    pub fn size(&self) -> (u32, u32) {
        let n = |k: &str| {
            self.map()
                .and_then(|m| m.get(k))
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as u32
        };
        (n("width"), n("height"))
    }

    /// BLAKE3 of the canonical camera object and render size: the surface
    /// key's evaluated camera (today's `cameraKey`, which hashes
    /// `matrixWorld` and the projection).
    pub fn camera_hash(&self) -> pm_protocol::common::ContentHash {
        let camera = self
            .map()
            .and_then(|m| m.get("camera"))
            .cloned()
            .unwrap_or(Value::Null);
        let (w, h) = self.size();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"pm-blender camera 1");
        hasher.update(to_canonical_string(&camera).as_bytes());
        hasher.update(&w.to_le_bytes());
        hasher.update(&h.to_le_bytes());
        pm_protocol::common::ContentHash(*hasher.finalize().as_bytes())
    }

    /// The settings object (`render3d` with the frame's samples).
    pub fn settings(&self) -> Option<&Map> {
        self.map()
            .and_then(|m| m.get("settings"))
            .and_then(Value::as_object)
    }
}

/// `render3DSchema.parse(comp.render3d || {})` (`shared/blender.ts`): the
/// composition's Blender settings with every default, or why the stored
/// value does not parse (today the parse throws and the frame fails).
#[derive(Debug, Clone, PartialEq)]
pub struct Render3dSettings {
    pub enabled: bool,
    pub cycles: bool,
    pub samples: u16,
    pub preview_samples: u16,
    pub preview_scale: f64,
    pub denoise: bool,
    pub device: pm_protocol::blender::RequestedDevice,
}

impl Default for Render3dSettings {
    fn default() -> Self {
        Render3dSettings {
            enabled: false,
            cycles: false,
            samples: 64,
            preview_samples: 16,
            preview_scale: 0.5,
            denoise: true,
            device: pm_protocol::blender::RequestedDevice::Auto,
        }
    }
}

impl Render3dSettings {
    /// Parses `render3d` with the schema's rules (strict object, integer
    /// sample ranges, preview scale 0.1 to 1). `None` or `undefined` gives
    /// the defaults.
    pub fn parse(value: Option<&Value>) -> Result<Render3dSettings, String> {
        let mut s = Render3dSettings::default();
        let Some(value) = value else {
            return Ok(s);
        };
        let Value::Object(map) = value else {
            return Err("render3d must be an object".into());
        };
        for (key, v) in map.iter() {
            let key = key.to_string_lossy();
            let int = |lo: f64, hi: f64| -> Result<u16, String> {
                match v {
                    Value::Number(n) if n.fract() == 0.0 && *n >= lo && *n <= hi => Ok(*n as u16),
                    _ => Err(format!(
                        "render3d.{key} must be an integer from {lo} to {hi}"
                    )),
                }
            };
            let boolean = || -> Result<bool, String> {
                v.as_bool()
                    .ok_or_else(|| format!("render3d.{key} must be a boolean"))
            };
            match key.as_str() {
                "enabled" => s.enabled = boolean()?,
                "denoise" => s.denoise = boolean()?,
                "engine" => {
                    s.cycles = match v.as_str() {
                        Some("eevee") => false,
                        Some("cycles") => true,
                        _ => return Err("render3d.engine must be eevee or cycles".into()),
                    }
                }
                "samples" => s.samples = int(1.0, 4096.0)?,
                "previewSamples" => s.preview_samples = int(1.0, 512.0)?,
                "previewScale" => match v {
                    Value::Number(n) if (0.1..=1.0).contains(n) => s.preview_scale = *n,
                    _ => return Err("render3d.previewScale must be 0.1 to 1".into()),
                },
                "device" => {
                    use pm_protocol::blender::RequestedDevice;
                    s.device = match v.as_str() {
                        Some("auto") => RequestedDevice::Auto,
                        Some("cpu") => RequestedDevice::Cpu,
                        Some("gpu") => RequestedDevice::Gpu,
                        _ => return Err("render3d.device must be auto, cpu, or gpu".into()),
                    }
                }
                other => return Err(format!("Unrecognized key in render3d: \"{other}\"")),
            }
        }
        Ok(s)
    }

    /// The schema's output object, in its key order.
    pub fn to_json(&self) -> Value {
        use pm_protocol::blender::RequestedDevice;
        let mut m = Map::new();
        m.insert("enabled", Value::Bool(self.enabled));
        m.insert(
            "engine",
            Value::String((if self.cycles { "cycles" } else { "eevee" }).into()),
        );
        m.insert("samples", Value::Number(f64::from(self.samples)));
        m.insert(
            "previewSamples",
            Value::Number(f64::from(self.preview_samples)),
        );
        m.insert("previewScale", Value::Number(self.preview_scale));
        m.insert("denoise", Value::Bool(self.denoise));
        m.insert(
            "device",
            Value::String(
                match self.device {
                    RequestedDevice::Auto => "auto",
                    RequestedDevice::Cpu => "cpu",
                    RequestedDevice::Gpu => "gpu",
                }
                .into(),
            ),
        );
        Value::Object(m)
    }

    /// The contract 11 settings of a frame (`preview` picks the preview
    /// samples and scale; Material Preview forces EEVEE).
    pub fn for_frame(
        &self,
        preview: bool,
        force_eevee: bool,
    ) -> pm_protocol::blender::BlenderSettings {
        use pm_protocol::blender::{BlenderEngine, BlenderSettings};
        BlenderSettings {
            engine: if self.cycles && !force_eevee {
                BlenderEngine::Cycles
            } else {
                BlenderEngine::Eevee
            },
            samples: if preview {
                self.preview_samples
            } else {
                self.samples
            },
            preview,
            preview_scale: self.preview_scale,
            denoise: self.denoise,
            device: self.device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Value {
        pm_model::json::parse(s).unwrap()
    }

    #[test]
    fn settings_parse_like_the_schema() {
        let s = Render3dSettings::parse(None).unwrap();
        assert_eq!(s, Render3dSettings::default());
        let s = Render3dSettings::parse(Some(&parse(
            r#"{"enabled":true,"engine":"cycles","samples":32,"device":"cpu"}"#,
        )))
        .unwrap();
        assert!(s.enabled && s.cycles);
        assert_eq!(s.samples, 32);
        assert_eq!(
            pm_model::json::to_canonical_string(&s.to_json()),
            r#"{"denoise":true,"device":"cpu","enabled":true,"engine":"cycles","previewSamples":16,"previewScale":0.5,"samples":32}"#
        );
        for bad in [
            r#"{"samples":0}"#,
            r#"{"samples":1.5}"#,
            r#"{"previewScale":2}"#,
            r#"{"engine":"workbench"}"#,
            r#"{"extra":1}"#,
            r#"[]"#,
        ] {
            assert!(Render3dSettings::parse(Some(&parse(bad))).is_err(), "{bad}");
        }
    }

    #[test]
    fn snapshots_hash_their_canonical_bytes() {
        let json = parse(
            r#"{"width":640,"height":360,"objects":[{"id":"L1","meshes":[]},{"id":"L2","meshes":[]}],"camera":{"fov":45}}"#,
        );
        let s = Snapshot {
            frame: BlenderFrame {
                composition_path: vec!["root".into()],
                time: 0.5,
                width: 640,
                height: 360,
                shutter: None,
                preview: false,
            },
            json,
            assets: vec![],
        };
        assert_eq!(s.layers(), ["L1", "L2"]);
        assert_eq!(s.size(), (640, 360));
        let mut other = s.clone();
        assert_eq!(s.hash(), other.hash());
        assert_eq!(s.camera_hash(), other.camera_hash());
        other.json = parse(
            r#"{"width":640,"height":360,"objects":[{"id":"L1","meshes":[]},{"id":"L2","meshes":[]}],"camera":{"fov":46}}"#,
        );
        assert_ne!(s.hash(), other.hash());
        assert_ne!(s.camera_hash(), other.camera_hash());
        assert_eq!(s.frame.lookup().time_bits, 0.5f64.to_bits());
    }
}
