//! The `pm-gpu` interop trait (contract 3, section 8 "Device").
//!
//! One trait owns every place pixels cross between the GPU device and
//! something else: decoder output import, shared-surface export to the
//! viewer, encoder input, and Blender surface import. `pm-gpu` implements it
//! on wgpu (Metal first); tests implement it on the CPU. The serializable
//! descriptors here are what crosses processes; native objects
//! (`CVPixelBuffer`, `IOSurfaceRef`, NT handles) stay behind associated types.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::color::{ColorDescription, Subsampling};
use crate::common::{ContentHash, PixelRect, Seq, Size};
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// GPU texture formats that cross the interop boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SurfaceFormat {
    /// 8-bit BGRA, viewer pool option (blue-noise dither in the display transform).
    Bgra8Unorm,
    /// Half float RGBA, viewer pool option and default working storage.
    Rgba16Float,
    /// Float RGBA, 32-bit precision working storage.
    Rgba32Float,
    /// `y416` imported as one texture with an AYCbCr swizzle.
    Rgba16Unorm,
    /// 8-bit luma plane of a biplanar buffer.
    R8Unorm,
    /// 8-bit interleaved chroma plane.
    Rg8Unorm,
    /// 16-bit luma plane (`x420`, `x422`, P010).
    R16Unorm,
    /// 16-bit interleaved chroma plane.
    Rg16Unorm,
}

impl SurfaceFormat {
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            SurfaceFormat::R8Unorm => 1,
            SurfaceFormat::Rg8Unorm | SurfaceFormat::R16Unorm => 2,
            SurfaceFormat::Bgra8Unorm | SurfaceFormat::Rg16Unorm => 4,
            SurfaceFormat::Rgba16Float | SurfaceFormat::Rgba16Unorm => 8,
            SurfaceFormat::Rgba32Float => 16,
        }
    }
}

/// Decoded frame layouts the engine imports without copies (section 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum DecodedLayout {
    /// VideoToolbox 8-bit biplanar 4:2:0 (`420v` limited, `420f` full).
    Biplanar420_8 { full_range: bool },
    /// VideoToolbox 8-bit biplanar 4:2:2 (`422v` limited, `422f` full).
    Biplanar422_8 { full_range: bool },
    /// VideoToolbox 10-bit biplanar 4:2:0, MSB-aligned (`x420` limited, `xf20` full).
    Biplanar420_10 { full_range: bool },
    /// VideoToolbox 10-bit biplanar 4:2:2, MSB-aligned (`x422`).
    Biplanar422_10 { full_range: bool },
    /// VideoToolbox 16-bit AYCbCr 4:4:4:4 (`y416`), ProRes 4444 and XQ.
    Ayuv4444_16,
    /// FFmpeg planar n-bit, LSB-aligned (DNxHR HQX and 444, H.264 4:2:2 10-bit, VP9 profile 2, DPX).
    PlanarLsb {
        bits: u8,
        subsampling: Subsampling,
        alpha: bool,
    },
    /// Packed RGBA from image readers.
    Rgba { format: SurfaceFormat },
}

impl DecodedLayout {
    /// The quantization range the layout's pixel format implies, for
    /// Y′CbCr layouts. VideoToolbox picks `420v`/`420f` (and `x420`/`xf20`)
    /// to match the bitstream; the importer verifies it against buffer
    /// attachments (section 13).
    pub const fn full_range(&self) -> Option<bool> {
        match *self {
            DecodedLayout::Biplanar420_8 { full_range }
            | DecodedLayout::Biplanar422_8 { full_range }
            | DecodedLayout::Biplanar420_10 { full_range }
            | DecodedLayout::Biplanar422_10 { full_range } => Some(full_range),
            DecodedLayout::Ayuv4444_16
            | DecodedLayout::PlanarLsb { .. }
            | DecodedLayout::Rgba { .. } => None,
        }
    }

    /// The chroma subsampling of the layout.
    pub const fn subsampling(&self) -> Subsampling {
        match *self {
            DecodedLayout::Biplanar420_8 { .. } | DecodedLayout::Biplanar420_10 { .. } => {
                Subsampling::S420
            }
            DecodedLayout::Biplanar422_8 { .. } | DecodedLayout::Biplanar422_10 { .. } => {
                Subsampling::S422
            }
            DecodedLayout::Ayuv4444_16 | DecodedLayout::Rgba { .. } => Subsampling::S444,
            DecodedLayout::PlanarLsb { subsampling, .. } => subsampling,
        }
    }

    /// Bits per sample.
    pub const fn bits(&self) -> u8 {
        match *self {
            DecodedLayout::Biplanar420_8 { .. } | DecodedLayout::Biplanar422_8 { .. } => 8,
            DecodedLayout::Biplanar420_10 { .. } | DecodedLayout::Biplanar422_10 { .. } => 10,
            DecodedLayout::Ayuv4444_16 => 16,
            DecodedLayout::PlanarLsb { bits, .. } => bits,
            DecodedLayout::Rgba { format } => match format {
                SurfaceFormat::Bgra8Unorm | SurfaceFormat::R8Unorm | SurfaceFormat::Rg8Unorm => 8,
                SurfaceFormat::Rgba16Float
                | SurfaceFormat::Rgba16Unorm
                | SurfaceFormat::R16Unorm
                | SurfaceFormat::Rg16Unorm => 16,
                SurfaceFormat::Rgba32Float => 32,
            },
        }
    }
}

/// One plane of a decoded layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaneSpec {
    pub format: SurfaceFormat,
    /// Plane width = frame width / `width_divisor` (rounded up).
    pub width_divisor: u8,
    pub height_divisor: u8,
}

impl DecodedLayout {
    /// The planes the importer creates, in order. Planar layouts import each
    /// plane as one single-channel texture.
    pub fn planes(&self) -> Vec<PlaneSpec> {
        let plane = |format, width_divisor, height_divisor| PlaneSpec {
            format,
            width_divisor,
            height_divisor,
        };
        match *self {
            DecodedLayout::Biplanar420_8 { .. } => vec![
                plane(SurfaceFormat::R8Unorm, 1, 1),
                plane(SurfaceFormat::Rg8Unorm, 2, 2),
            ],
            DecodedLayout::Biplanar422_8 { .. } => vec![
                plane(SurfaceFormat::R8Unorm, 1, 1),
                plane(SurfaceFormat::Rg8Unorm, 2, 1),
            ],
            DecodedLayout::Biplanar420_10 { .. } => vec![
                plane(SurfaceFormat::R16Unorm, 1, 1),
                plane(SurfaceFormat::Rg16Unorm, 2, 2),
            ],
            DecodedLayout::Biplanar422_10 { .. } => vec![
                plane(SurfaceFormat::R16Unorm, 1, 1),
                plane(SurfaceFormat::Rg16Unorm, 2, 1),
            ],
            DecodedLayout::Ayuv4444_16 => vec![plane(SurfaceFormat::Rgba16Unorm, 1, 1)],
            DecodedLayout::PlanarLsb {
                bits,
                subsampling,
                alpha,
            } => {
                let format = if bits <= 8 {
                    SurfaceFormat::R8Unorm
                } else {
                    SurfaceFormat::R16Unorm
                };
                let (w, h) = match subsampling {
                    Subsampling::S444 => (1, 1),
                    Subsampling::S422 => (2, 1),
                    Subsampling::S420 => (2, 2),
                };
                let mut planes = vec![
                    plane(format, 1, 1),
                    plane(format, w, h),
                    plane(format, w, h),
                ];
                if alpha {
                    planes.push(plane(format, 1, 1));
                }
                planes
            }
            DecodedLayout::Rgba { format } => vec![plane(format, 1, 1)],
        }
    }
}

/// A decoded frame as the decoder hands it to the interop layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DecodedFrameDesc {
    pub size: Size,
    pub layout: DecodedLayout,
    pub color: ColorDescription,
    /// Clean aperture inside `size`, when the container declares one.
    pub clean_aperture: Option<PixelRect>,
}

impl Validate for DecodedFrameDesc {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::GpuInterop;
        ensure(!self.size.is_empty(), C, || {
            "decoded frames have a size".into()
        })?;
        if let DecodedLayout::PlanarLsb { bits, .. } = self.layout {
            ensure((8..=16).contains(&bits), C, || {
                format!("planar samples are 8 to 16 bits, got {bits}")
            })?;
        }
        if let DecodedLayout::Rgba { format } = self.layout {
            ensure(
                matches!(
                    format,
                    SurfaceFormat::Bgra8Unorm
                        | SurfaceFormat::Rgba16Unorm
                        | SurfaceFormat::Rgba16Float
                        | SurfaceFormat::Rgba32Float
                ),
                C,
                || format!("{format:?} is not a packed RGBA format"),
            )?;
        }
        if let Some(full) = self.layout.full_range() {
            ensure(
                full == (self.color.range == crate::color::Range::Full),
                C,
                || {
                    "the pixel format's range must match the color description (420v vs 420f)"
                        .into()
                },
            )?;
        }
        ensure(
            self.layout.subsampling() == self.color.subsampling,
            C,
            || "layout and color description disagree on subsampling".into(),
        )?;
        ensure(
            self.layout.bits() == self.color.bit_depth
                || matches!(self.layout, DecodedLayout::Rgba { .. }),
            C,
            || "layout and color description disagree on bit depth".into(),
        )?;
        if let Some(aperture) = &self.clean_aperture {
            ensure(
                !aperture.is_empty() && self.size.contains(aperture),
                C,
                || "clean aperture must lie inside the frame".into(),
            )?;
        }
        Ok(())
    }
}

/// Identifies the GPU so the engine and Chromium's GPU process use the same
/// adapter (Metal `registryID`, Windows LUID).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum AdapterIdentity {
    /// Metal `MTLDevice.registryID`, as decimal text (it is a u64).
    Metal { registry_id: String, name: String },
    /// DXGI adapter LUID, as `high:low` hex text.
    Dxgi { luid: String, name: String },
    /// Vulkan device UUID, hex text (Linux CLI).
    Vulkan { uuid: String, name: String },
    /// CPU implementation (tests, software fallback).
    Software,
}

/// The platform object behind an exported surface, as it crosses processes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum PlatformSurface {
    /// An IOSurface sent once as a Mach port right through `pm-bridge`; the
    /// message carries the index the bridge assigned, never a raw port.
    IoSurface { bridge_index: u32 },
    /// An NT handle duplicated into main with `DuplicateHandle` (Windows).
    NtHandle { bridge_index: u32 },
    /// Shared-memory CPU frames (fallback B, and the hidden host's Chromium viewer).
    SharedMemory {
        handle: crate::host::CapabilityHandle,
        stride: u32,
    },
}

/// A surface in the viewer pool, sent once and imported once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SurfaceDescriptor {
    pub pool_index: u32,
    pub size: Size,
    pub format: SurfaceFormat,
    pub platform: PlatformSurface,
}

/// What a surface pool carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PoolRole {
    Color,
    DepthId,
}

/// The pool announced to the host when it is created or resized. Every
/// surface is imported once with `importSharedTexture`; frames then name a
/// `pool_index` only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SurfacePool {
    /// Increases when the pool is replaced; frames from older generations are stale.
    pub generation: Seq,
    pub surfaces: Vec<SurfaceDescriptor>,
    /// What the pool's surfaces carry. Absent (older peers) or `color`:
    /// viewer frames. `depthId`: the depth and object-id surfaces of frames
    /// whose viewer asked for them (`ViewerState::depth_id`; plan section 8,
    /// the Phase 3c render transport). A depth pool never replaces the
    /// color pool.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub role: Option<PoolRole>,
}

/// Fewest surfaces in a pool: ring + jitter buffer + 2 (section 18), with a
/// ring and jitter buffer of at least one frame each.
pub const MIN_POOL_SURFACES: usize = 4;
/// Most surfaces in a pool.
pub const MAX_POOL_SURFACES: usize = 32;

impl Validate for SurfacePool {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::GpuInterop;
        ensure(
            (MIN_POOL_SURFACES..=MAX_POOL_SURFACES).contains(&self.surfaces.len()),
            C,
            || {
                format!(
                    "a pool has {MIN_POOL_SURFACES} to {MAX_POOL_SURFACES} surfaces, got {}",
                    self.surfaces.len()
                )
            },
        )?;
        for (index, surface) in self.surfaces.iter().enumerate() {
            ensure(surface.pool_index as usize == index, C, || {
                format!("surface {index} has pool index {}", surface.pool_index)
            })?;
            ensure(
                !surface.size.is_empty()
                    && surface.size.width <= 16384
                    && surface.size.height <= 16384,
                C,
                || format!("surface {index} size {:?} outside 1..16384", surface.size),
            )?;
            ensure(
                matches!(
                    surface.format,
                    SurfaceFormat::Bgra8Unorm | SurfaceFormat::Rgba16Float
                ),
                C,
                || {
                    format!(
                        "viewer surfaces are bgra8 or rgba16float, got {:?}",
                        surface.format
                    )
                },
            )?;
        }
        let first = &self.surfaces[0];
        ensure(
            self.surfaces
                .iter()
                .all(|s| s.size == first.size && s.format == first.format),
            C,
            || "every surface in a pool has the same size and format".into(),
        )
    }
}

/// Ask the interop layer for an exportable surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SurfaceRequest {
    pub size: Size,
    pub format: SurfaceFormat,
}

/// What an encoder session wants as input.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EncoderInputDesc {
    pub size: Size,
    pub layout: DecodedLayout,
    pub color: ColorDescription,
}

/// A Blender surface as stored by the job engine (16-bit RGBA PNG, straight alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderSurfaceDesc {
    pub size: Size,
    pub crop: PixelRect,
    pub key: ContentHash,
}

/// What an interop implementation supports on this machine. Spike S2 fills
/// `rgba32float_filterable` and `rgba32float_blendable` per reference machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct InteropCapabilities {
    pub adapter: AdapterIdentity,
    pub zero_copy_decode: bool,
    pub shared_surfaces: bool,
    pub rgba32float_filterable: bool,
    pub rgba32float_blendable: bool,
    pub prores_hardware: bool,
    pub max_texture_dimension: u32,
}

/// An encoder input frame: GPU views of its planes (one per
/// [`DecodedLayout::planes`] entry) for the encoder-input shader to write,
/// and the native frame handed to the encoder afterwards.
#[derive(Debug)]
pub struct EncoderInput<T, F> {
    pub planes: Vec<T>,
    pub frame: F,
}

/// The interop trait. Implemented by `pm-gpu` (wgpu) and by test doubles.
///
/// Invariants:
/// - `import_decoded` never copies pixels when `zero_copy_decode` is true;
///   it returns one texture per [`DecodedLayout::planes`] entry, in order.
/// - Exported surfaces are created once per pool and reused; a surface is not
///   written while the viewer holds it (release per frame, contract 2).
/// - `encoder_input` returns a native frame the encoder consumes with color
///   attachments matching `desc.color`.
/// - `import_blender_surface` premultiplies on input (Blender surfaces are straight).
pub trait GpuInterop {
    type Texture;
    /// A decoder's native frame (`CVPixelBuffer`, `AVFrame`, a CPU buffer).
    type NativeFrame;
    /// A shareable surface (`IOSurfaceRef`, NT handle, shared memory).
    type NativeSurface;
    type Error: std::error::Error;

    fn capabilities(&self) -> InteropCapabilities;
    fn import_decoded(
        &mut self,
        desc: &DecodedFrameDesc,
        frame: &Self::NativeFrame,
    ) -> Result<Vec<Self::Texture>, Self::Error>;
    fn export_surface(
        &mut self,
        request: &SurfaceRequest,
    ) -> Result<(Self::Texture, Self::NativeSurface), Self::Error>;
    fn encoder_input(
        &mut self,
        desc: &EncoderInputDesc,
    ) -> Result<EncoderInput<Self::Texture, Self::NativeFrame>, Self::Error>;
    fn import_blender_surface(
        &mut self,
        desc: &BlenderSurfaceDesc,
        rgba16_straight: &[u16],
    ) -> Result<Self::Texture, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorDescription;

    /// A CPU implementation: proves the trait is implementable without a GPU
    /// and checks the plane rules.
    struct Cpu;

    #[derive(Debug, PartialEq)]
    struct CpuTexture {
        width: u32,
        height: u32,
        format: SurfaceFormat,
        data: Vec<u8>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct CpuError(String);

    impl GpuInterop for Cpu {
        type Texture = CpuTexture;
        type NativeFrame = Vec<Vec<u8>>;
        type NativeSurface = usize;
        type Error = CpuError;

        fn capabilities(&self) -> InteropCapabilities {
            InteropCapabilities {
                adapter: AdapterIdentity::Software,
                zero_copy_decode: false,
                shared_surfaces: false,
                rgba32float_filterable: true,
                rgba32float_blendable: true,
                prores_hardware: false,
                max_texture_dimension: 16384,
            }
        }

        fn import_decoded(
            &mut self,
            desc: &DecodedFrameDesc,
            frame: &Self::NativeFrame,
        ) -> Result<Vec<CpuTexture>, CpuError> {
            let planes = desc.layout.planes();
            if planes.len() != frame.len() {
                return Err(CpuError(format!(
                    "{} planes expected, got {}",
                    planes.len(),
                    frame.len()
                )));
            }
            planes
                .iter()
                .zip(frame)
                .map(|(plane, bytes)| {
                    let width = desc.size.width.div_ceil(plane.width_divisor as u32);
                    let height = desc.size.height.div_ceil(plane.height_divisor as u32);
                    let expected = (width * height * plane.format.bytes_per_pixel()) as usize;
                    if bytes.len() != expected {
                        return Err(CpuError(format!(
                            "plane needs {expected} bytes, got {}",
                            bytes.len()
                        )));
                    }
                    Ok(CpuTexture {
                        width,
                        height,
                        format: plane.format,
                        data: bytes.clone(),
                    })
                })
                .collect()
        }

        fn export_surface(
            &mut self,
            request: &SurfaceRequest,
        ) -> Result<(CpuTexture, usize), CpuError> {
            let len = (request.size.width * request.size.height * request.format.bytes_per_pixel())
                as usize;
            Ok((
                CpuTexture {
                    width: request.size.width,
                    height: request.size.height,
                    format: request.format,
                    data: vec![0; len],
                },
                0,
            ))
        }

        fn encoder_input(
            &mut self,
            desc: &EncoderInputDesc,
        ) -> Result<EncoderInput<CpuTexture, Self::NativeFrame>, CpuError> {
            let textures: Vec<CpuTexture> = desc
                .layout
                .planes()
                .iter()
                .map(|p| {
                    let width = desc.size.width.div_ceil(p.width_divisor as u32);
                    let height = desc.size.height.div_ceil(p.height_divisor as u32);
                    CpuTexture {
                        width,
                        height,
                        format: p.format,
                        data: vec![0; (width * height * p.format.bytes_per_pixel()) as usize],
                    }
                })
                .collect();
            let frame = textures.iter().map(|t| t.data.clone()).collect();
            Ok(EncoderInput {
                planes: textures,
                frame,
            })
        }

        fn import_blender_surface(
            &mut self,
            desc: &BlenderSurfaceDesc,
            rgba: &[u16],
        ) -> Result<CpuTexture, CpuError> {
            if rgba.len() != (desc.size.width * desc.size.height * 4) as usize {
                return Err(CpuError("surface size mismatch".into()));
            }
            // Premultiply on input, in f32, stored as half-float-sized texels.
            let mut data = Vec::with_capacity(rgba.len() * 2);
            for px in rgba.chunks_exact(4) {
                let a = px[3] as f32 / 65535.0;
                for &c in &px[..3] {
                    let v = (c as f32 / 65535.0) * a;
                    data.extend_from_slice(&((v * 65535.0).round() as u16).to_le_bytes());
                }
                data.extend_from_slice(&px[3].to_le_bytes());
            }
            Ok(CpuTexture {
                width: desc.size.width,
                height: desc.size.height,
                format: SurfaceFormat::Rgba16Unorm,
                data,
            })
        }
    }

    #[test]
    fn plane_rules() {
        let x420 = DecodedLayout::Biplanar420_10 { full_range: false };
        assert_eq!(x420.planes().len(), 2);
        assert_eq!(
            x420.planes()[1],
            PlaneSpec {
                format: SurfaceFormat::Rg16Unorm,
                width_divisor: 2,
                height_divisor: 2
            }
        );
        let x422 = DecodedLayout::Biplanar422_10 { full_range: false };
        assert_eq!(x422.planes()[1].height_divisor, 1);
        let dnx = DecodedLayout::PlanarLsb {
            bits: 10,
            subsampling: Subsampling::S422,
            alpha: true,
        };
        assert_eq!(dnx.planes().len(), 4);
        assert_eq!(
            DecodedLayout::Ayuv4444_16.planes(),
            vec![PlaneSpec {
                format: SurfaceFormat::Rgba16Unorm,
                width_divisor: 1,
                height_divisor: 1
            }]
        );
    }

    #[test]
    fn cpu_interop_imports_odd_sized_420() {
        let desc = DecodedFrameDesc {
            size: Size::new(5, 3),
            layout: DecodedLayout::Biplanar420_8 { full_range: false },
            color: ColorDescription::SRGB_RGBA8,
            clean_aperture: None,
        };
        let frame = vec![vec![16u8; 15], vec![128u8; 3 * 2 * 2]];
        let textures = Cpu.import_decoded(&desc, &frame).unwrap();
        assert_eq!((textures[1].width, textures[1].height), (3, 2));
        assert!(Cpu.import_decoded(&desc, &vec![vec![0; 15]]).is_err());
    }

    #[test]
    fn cpu_interop_premultiplies_blender_surfaces() {
        let desc = BlenderSurfaceDesc {
            size: Size::new(1, 1),
            crop: PixelRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            key: ContentHash([0; 32]),
        };
        let texture = Cpu
            .import_blender_surface(&desc, &[65535, 0, 65535, 32768])
            .unwrap();
        let red = u16::from_le_bytes([texture.data[0], texture.data[1]]);
        assert_eq!(red, 32768);
        let caps = Cpu.capabilities();
        assert_eq!(caps.adapter, AdapterIdentity::Software);
        let (surface, _) = Cpu
            .export_surface(&SurfaceRequest {
                size: Size::new(2, 2),
                format: SurfaceFormat::Rgba16Float,
            })
            .unwrap();
        assert_eq!(surface.data.len(), 32);
        let EncoderInput {
            planes,
            frame: native,
        } = Cpu
            .encoder_input(&EncoderInputDesc {
                size: Size::new(4, 4),
                layout: DecodedLayout::Biplanar420_10 { full_range: false },
                color: ColorDescription::SRGB_RGBA8,
            })
            .unwrap();
        assert_eq!(planes.len(), native.len());
    }

    #[test]
    fn pool_validation() {
        let surface = |i| SurfaceDescriptor {
            pool_index: i,
            size: Size::new(3840, 2160),
            format: SurfaceFormat::Rgba16Float,
            platform: PlatformSurface::IoSurface { bridge_index: i },
        };
        let pool = SurfacePool {
            generation: Seq::ZERO,
            surfaces: (0..6).map(surface).collect(),
            role: None,
        };
        assert!(pool.validate().is_ok());
        let mut bad = pool.clone();
        bad.surfaces.swap(0, 1);
        assert!(bad.validate().is_err());
        let mut mixed = pool.clone();
        mixed.surfaces[2].format = SurfaceFormat::Bgra8Unorm;
        assert!(mixed.validate().is_err());
        let small = SurfacePool {
            generation: Seq::ZERO,
            surfaces: (0..2).map(surface).collect(),
            role: None,
        };
        assert!(small.validate().is_err());
    }
}
