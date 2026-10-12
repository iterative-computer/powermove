//! The Blender job protocol, snapshot, and surface format (contract 11,
//! sections 8, 9, 14.7, 17).
//!
//! The job engine builds Blender snapshots from pm-eval and pm-geom3d
//! (verified equal to today's `renderSnapshot` output) and asks host
//! services to run them in a supervised, user-installed Blender. Each
//! surface is one layer's pixels for one frame, shutter sample, and nested
//! composition: 16-bit RGBA PNG through AgX with straight alpha, cached by
//! content key and never re-rendered within a job.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{
    AssetId, ByteSize, CompositionId, ContentHash, JobId, LayerId, PixelRect, Size, Ticks,
};
use crate::host::CapabilityHandle;
use crate::priority::PriorityClass;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Oldest supported Blender.
pub const MIN_BLENDER_VERSION: (u16, u16) = (4, 5);

/// Section 3 limits (may be raised, never lowered).
pub mod limits {
    pub const MAX_OUTPUT_DIMENSION: u32 = 8192;
    pub const MAX_NESTED_FRAMES: usize = 4096;
    pub const MAX_ASSETS: usize = 512;
    pub const MAX_QUEUED_JOBS: usize = 16;
    pub const JOB_TIMEOUT_MS: u32 = 10 * 60 * 1000;
    pub const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
    pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
    /// Today's output cap. One 8192² 16-bit RGBA layer is 512 MiB, so the cap
    /// is raised or surfaces stream per layer (section 17); per-surface here.
    pub const MAX_OUTPUT_BYTES_PER_SURFACE: u64 = 8192 * 8192 * 8 + 16 * 1024 * 1024;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BlenderEngine {
    Eevee,
    Cycles,
}

/// `render3d.device` as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RequestedDevice {
    Auto,
    Cpu,
    Gpu,
}

/// The device Blender actually used; `auto` is resolved and recorded per job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ResolvedDevice {
    Cpu,
    Metal { name: String },
    Optix { name: String },
    Cuda { name: String },
    Hip { name: String },
    OneApi { name: String },
}

/// Render settings from the composition's `render3d`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderSettings {
    pub engine: BlenderEngine,
    /// 1 to 4096 (final) or 1 to 512 (preview).
    pub samples: u16,
    pub preview: bool,
    /// 0.1 to 1, preview only.
    pub preview_scale: f64,
    pub denoise: bool,
    pub device: RequestedDevice,
}

/// Worker output settings pinned for every job (section 17). Not
/// configurable and not on the wire: the worker script and the cache key
/// (through `worker_script_hash`) assume them. Today's worker writes 8-bit
/// RGBA PNG through AgX; decision 15 moves it to 16-bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedOutput {
    pub format: &'static str,
    pub color_depth: u8,
    pub color_mode: &'static str,
    pub view_transform: &'static str,
    pub look: &'static str,
    pub display_device: &'static str,
    pub dither_intensity: u8,
    pub gamma: u8,
    /// "Fast compression" (section 17). The exact PNG compression percentage
    /// is pinned when spike S7 measures write cost; `None` until then.
    pub compression_percent: Option<u8>,
    /// PNG chunks the engine ignores when reading surfaces.
    pub ignored_png_chunks: [&'static str; 3],
}

pub const PINNED_OUTPUT: PinnedOutput = PinnedOutput {
    format: "PNG",
    color_depth: 16,
    color_mode: "RGBA",
    view_transform: "AgX",
    look: "None",
    display_device: "sRGB",
    dither_intensity: 0,
    gamma: 1,
    compression_percent: None,
    ignored_png_chunks: ["gAMA", "sRGB", "iCCP"],
};

/// Worker command-line flags (section 19, security). Today's service passes
/// `--background --factory-startup --disable-autoexec --python-exit-code 1
/// --python <script>`; the engine adds `--offline-mode`. The environment is
/// scrubbed and `BLENDER_USER_*` point at scratch directories.
pub const WORKER_FLAGS: [&str; 4] = [
    "--background",
    "--factory-startup",
    "--disable-autoexec",
    "--offline-mode",
];

/// The snapshot encoding: today's `renderSnapshot` JSON, version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SnapshotEncoding {
    RenderSnapshotJsonV1,
}

/// A snapshot stored in a cache file, addressed by its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRef {
    pub hash: ContentHash,
    pub encoding: SnapshotEncoding,
    pub handle: CapabilityHandle,
    pub bytes: ByteSize,
}

/// An asset the snapshot references (textures, source `.blend` files).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderAsset {
    pub asset: AssetId,
    pub hash: ContentHash,
    pub handle: CapabilityHandle,
}

/// Motion-blur shutter sample `index` of `count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShutterSample {
    pub index: u16,
    pub count: u16,
}

/// One surface the job needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SurfaceRequest {
    pub layer: LayerId,
    /// Outermost composition first; length 1 for top-level layers. Depth ≤ 8.
    pub composition_path: Vec<CompositionId>,
    /// The time the snapshot was evaluated at (shutter offset included).
    pub time: Ticks,
    pub shutter: Option<ShutterSample>,
    pub size: Size,
    /// The surface's cache key (see [`SurfaceKey`]).
    pub key: ContentHash,
}

/// Run a Blender render job (job engine → host services).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderJobRequest {
    pub job: JobId,
    pub priority: PriorityClass,
    pub snapshot: SnapshotRef,
    pub worker_script_hash: ContentHash,
    pub assets: Vec<BlenderAsset>,
    pub settings: BlenderSettings,
    pub surfaces: Vec<SurfaceRequest>,
    /// Batch a frame's layers in one worker invocation (only when S7 shows equal surfaces).
    pub batch_layers: bool,
    /// Where surfaces are written (a cache handle for Blender surfaces).
    pub output: CapabilityHandle,
}

impl Validate for BlenderJobRequest {
    fn validate(&self) -> Result<(), Violation> {
        use limits::*;
        const C: ContractId = ContractId::Blender;
        ensure(self.job.is_valid(), C, || "invalid job id".into())?;
        ensure(
            !self.surfaces.is_empty() && self.surfaces.len() <= MAX_NESTED_FRAMES,
            C,
            || {
                format!(
                    "a job renders 1 to {MAX_NESTED_FRAMES} surfaces, got {}",
                    self.surfaces.len()
                )
            },
        )?;
        ensure(self.assets.len() <= MAX_ASSETS, C, || {
            format!("at most {MAX_ASSETS} assets")
        })?;
        ensure(self.snapshot.bytes.get() <= MAX_SNAPSHOT_BYTES, C, || {
            "snapshot exceeds 64 MB".into()
        })?;
        let max_samples = if self.settings.preview { 512 } else { 4096 };
        ensure(
            (1..=max_samples).contains(&self.settings.samples),
            C,
            || format!("samples must be 1 to {max_samples}"),
        )?;
        ensure(
            !self.settings.preview || (0.1..=1.0).contains(&self.settings.preview_scale),
            C,
            || "preview scale must be 0.1 to 1".into(),
        )?;
        for surface in &self.surfaces {
            ensure(
                !surface.size.is_empty()
                    && surface.size.width <= MAX_OUTPUT_DIMENSION
                    && surface.size.height <= MAX_OUTPUT_DIMENSION,
                C,
                || {
                    format!(
                        "surface size {:?} outside 1..{MAX_OUTPUT_DIMENSION}",
                        surface.size
                    )
                },
            )?;
            ensure((1..=8).contains(&surface.composition_path.len()), C, || {
                "nesting depth is 1 to 8".into()
            })?;
            if let Some(shutter) = surface.shutter {
                ensure(
                    shutter.count > 0 && shutter.index < shutter.count,
                    C,
                    || "shutter index must be below its count".into(),
                )?;
            }
        }
        Ok(())
    }
}

/// A model operation run by Blender through host services (`api.scene3d`
/// model tools, `edit_3d` model jobs). The result returns as
/// [`crate::patch::EngineFact::BlenderResult`] and is committed as one
/// undoable edit, dropped if the document moved past `revision` meanwhile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum BlenderModelOperation {
    /// Generate a model from a recipe: the document's `ModelRecipe` JSON as
    /// UTF-8 text, validated by the kernel's schema (`shared/blender.ts`).
    Generate { recipe: String },
    /// Import an object from a `.blend` source asset (external textures packed
    /// or reported at import).
    Import {
        source: AssetId,
        object: Option<String>,
    },
}

/// Renderer → host: run a model operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderModelJob {
    pub job: JobId,
    pub document: crate::common::DocumentId,
    /// The revision the operation was requested at.
    pub revision: crate::common::Revision,
    pub operation: BlenderModelOperation,
    pub assets: Vec<BlenderAsset>,
}

/// Longest recipe text (mesh recipes carry up to 300k numbers).
pub const MAX_RECIPE_BYTES: usize = 8 * 1024 * 1024;

impl Validate for BlenderModelJob {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Blender;
        ensure(self.job.is_valid() && self.document.is_valid(), C, || {
            "invalid job or document id".into()
        })?;
        ensure(self.assets.len() <= limits::MAX_ASSETS, C, || {
            format!("at most {} assets", limits::MAX_ASSETS)
        })?;
        match &self.operation {
            BlenderModelOperation::Generate { recipe } => ensure(
                !recipe.is_empty() && recipe.len() <= MAX_RECIPE_BYTES,
                C,
                || "recipe must be 1 byte to 8 MiB of JSON".into(),
            ),
            BlenderModelOperation::Import { source, object } => {
                ensure(source.is_valid(), C, || "invalid source asset".into())?;
                ensure(
                    object
                        .as_ref()
                        .is_none_or(|o| !o.is_empty() && o.chars().count() <= 160),
                    C,
                    || "object names are 1 to 160 characters".into(),
                )?;
                ensure(self.assets.iter().any(|a| &a.asset == source), C, || {
                    "the source .blend must be among the job's assets".into()
                })
            }
        }
    }
}

/// One file a model operation produced (GLB, textures), in the host's scratch cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderOutputFile {
    pub name: String,
    pub hash: ContentHash,
    pub handle: CapabilityHandle,
    pub bytes: ByteSize,
}

/// A finished surface (host → job engine, one event per surface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderSurface {
    pub job: JobId,
    /// Index into the request's `surfaces`.
    pub request_index: u32,
    pub key: ContentHash,
    /// The full render size.
    pub size: Size,
    /// The region with content, inside `size`; pixels outside are transparent.
    pub crop: PixelRect,
    /// 16-bit RGBA PNG, AgX, straight alpha, in the output cache.
    pub handle: CapabilityHandle,
    pub bytes: ByteSize,
    pub device: ResolvedDevice,
}

impl Validate for BlenderSurface {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Blender;
        ensure(self.size.contains(&self.crop), C, || {
            "crop must lie inside the surface".into()
        })?;
        ensure(
            self.bytes.get() <= limits::MAX_OUTPUT_BYTES_PER_SURFACE,
            C,
            || "surface exceeds the per-surface output cap".into(),
        )
    }
}

/// Job progress (host → job engine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderProgress {
    pub job: JobId,
    pub completed: u32,
    pub total: u32,
}

/// Blender availability, as host services report it. Not committed to the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderStatus {
    pub available: bool,
    /// `major.minor.patch`.
    pub version: Option<String>,
    pub error: Option<String>,
}

impl BlenderStatus {
    /// Whether the version meets [`MIN_BLENDER_VERSION`].
    pub fn supported(&self) -> bool {
        let Some(version) = &self.version else {
            return false;
        };
        let mut parts = version.split('.').map(|p| p.parse::<u16>());
        match (parts.next(), parts.next()) {
            (Some(Ok(major)), Some(Ok(minor))) => {
                self.available && (major, minor) >= MIN_BLENDER_VERSION
            }
            _ => false,
        }
    }
}

/// Every input of a surface's cache key (section 14.7). The job engine hashes
/// [`SurfaceKey::key_bytes`] with BLAKE3 to get [`SurfaceRequest::key`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SurfaceKey {
    pub snapshot_hash: ContentHash,
    pub worker_script_hash: ContentHash,
    /// Sorted.
    pub asset_hashes: Vec<ContentHash>,
    pub blender_version: String,
    pub settings: BlenderSettings,
    pub device: ResolvedDevice,
    pub preview: bool,
    /// Hash of the evaluated camera (view matrix and projection).
    pub camera: ContentHash,
    pub size: Size,
    pub layer: LayerId,
    pub shutter: Option<ShutterSample>,
}

impl SurfaceKey {
    /// Stable positional encoding, identical on every target. Asset hashes are
    /// sorted first so the key does not depend on reference order.
    pub fn key_bytes(&self) -> Vec<u8> {
        let mut sorted = self.clone();
        sorted.asset_hashes.sort();
        rmp_serde::to_vec(&sorted).expect("surface keys always encode")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::host::CapabilityHandle;

    pub(crate) fn handle(n: u8) -> CapabilityHandle {
        CapabilityHandle::from_random_bytes([n; 16])
    }

    pub(crate) fn job() -> BlenderJobRequest {
        BlenderJobRequest {
            job: "blender-1".into(),
            priority: PriorityClass::UserJob,
            snapshot: SnapshotRef {
                hash: ContentHash([1; 32]),
                encoding: SnapshotEncoding::RenderSnapshotJsonV1,
                handle: handle(1),
                bytes: ByteSize::new(4096).unwrap(),
            },
            worker_script_hash: ContentHash([2; 32]),
            assets: vec![],
            settings: BlenderSettings {
                engine: BlenderEngine::Eevee,
                samples: 64,
                preview: false,
                preview_scale: 0.5,
                denoise: true,
                device: RequestedDevice::Auto,
            },
            surfaces: vec![SurfaceRequest {
                layer: "cube".into(),
                composition_path: vec!["main".into()],
                time: Ticks::ZERO,
                shutter: Some(ShutterSample {
                    index: 0,
                    count: 16,
                }),
                size: Size::new(1920, 1080),
                key: ContentHash([3; 32]),
            }],
            batch_layers: false,
            output: handle(2),
        }
    }

    #[test]
    fn job_limits() {
        assert!(job().validate().is_ok());
        let mut big = job();
        big.surfaces[0].size = Size::new(8193, 10);
        assert!(big.validate().is_err());
        let mut bad = job();
        bad.surfaces[0].shutter = Some(ShutterSample {
            index: 16,
            count: 16,
        });
        assert!(bad.validate().is_err());
        let mut preview = job();
        preview.settings.preview = true;
        preview.settings.samples = 1024;
        assert!(preview.validate().is_err());
    }

    #[test]
    fn version_gate() {
        let status = |v: &str| BlenderStatus {
            available: true,
            version: Some(v.into()),
            error: None,
        };
        assert!(status("4.5.0").supported());
        assert!(status("5.0.1").supported());
        assert!(!status("4.4.3").supported());
        assert!(!status("garbage").supported());
    }

    #[test]
    fn surface_keys_ignore_asset_order() {
        let key = SurfaceKey {
            snapshot_hash: ContentHash([1; 32]),
            worker_script_hash: ContentHash([2; 32]),
            asset_hashes: vec![ContentHash([5; 32]), ContentHash([4; 32])],
            blender_version: "4.5.0".into(),
            settings: job().settings,
            device: ResolvedDevice::Metal {
                name: "Apple M4 Max".into(),
            },
            preview: false,
            camera: ContentHash([9; 32]),
            size: Size::new(1920, 1080),
            layer: "cube".into(),
            shutter: None,
        };
        let mut reordered = key.clone();
        reordered.asset_hashes.reverse();
        assert_eq!(key.key_bytes(), reordered.key_bytes());
        let other = SurfaceKey {
            preview: true,
            ..key.clone()
        };
        assert_ne!(key.key_bytes(), other.key_bytes());
    }

    #[test]
    fn pinned_output_matches_section_17() {
        assert_eq!(PINNED_OUTPUT.view_transform, "AgX");
        assert_eq!(PINNED_OUTPUT.color_depth, 16);
        assert_eq!(PINNED_OUTPUT.dither_intensity, 0);
        assert!(WORKER_FLAGS.contains(&"--disable-autoexec"));
    }
}
