# RFC 03: The `pm-gpu` interop trait

Status: draft, version 0. Module: `pm_protocol::interop`. Gates Phase 2;
freezes when Phase 2 starts. Owner: GPU and render (`pm-gpu`), with Media
(decoder import) and Engine and platform (surface export through
`pm-bridge`).

## Purpose

One trait owns every place pixels cross between the GPU device and
something else (plan section 8, "Device"): decoder output import, shared
surface export to the viewer, encoder input, and Blender surface import.
`pm-gpu` implements it on wgpu (Metal first; DX12 and Vulkan later); tests
implement it on the CPU. The serializable descriptors are what crosses
processes; native objects (`CVPixelBuffer`, `IOSurfaceRef`, NT handles,
`AVFrame`) stay behind associated types and never appear on the wire.

## Types

| Type | Meaning |
|---|---|
| `GpuInterop` (trait) | `capabilities`, `import_decoded`, `export_surface`, `encoder_input`, `import_blender_surface`, with associated `Texture`, `NativeFrame`, `NativeSurface`, `Error` |
| `EncoderInput<T, F>` | The planes the encoder-input shader writes and the native frame then handed to the encoder |
| `SurfaceFormat` | `bgra8Unorm`, `rgba16Float`, `rgba32Float`, `rgba16Unorm` (`y416`), `r8Unorm`, `rg8Unorm`, `r16Unorm`, `rg16Unorm` |
| `DecodedLayout` | VideoToolbox biplanar 4:2:0 and 4:2:2 at 8 and 10 bits (`420v`/`420f`, `422v`/`422f`, `x420`/`xf20`, `x422`), `y416` AYCbCr 4:4:4:4, FFmpeg planar LSB-aligned n-bit with optional alpha, packed RGBA from image readers |
| `PlaneSpec` | Per-plane format and width and height divisors; `DecodedLayout::planes` lists them in import order |
| `DecodedFrameDesc` | Size, layout, `ColorDescription` (contract 4), clean aperture |
| `AdapterIdentity` | Metal `registryID` (decimal text, it is a u64), DXGI LUID, Vulkan UUID, or software; used to match Chromium's GPU process |
| `PlatformSurface` | An IOSurface or NT handle sent once through `pm-bridge` and referred to by the bridge's index, or shared-memory CPU frames behind a capability handle (fallback B, the hidden host's Chromium viewer) |
| `SurfaceDescriptor`, `SurfacePool` | The viewer pool: one generation, 4 to 32 same-size, same-format surfaces, each imported once with `importSharedTexture` |
| `SurfaceRequest`, `EncoderInputDesc`, `BlenderSurfaceDesc` | Inputs to `export_surface`, `encoder_input`, `import_blender_surface` |
| `InteropCapabilities` | Adapter, zero-copy decode, shared surfaces, `rgba32float` filterable and blendable, ProRes hardware, maximum texture size |

## Invariants

Trait contract (documented on `GpuInterop`, exercised by the CPU test
implementation in `interop.rs`):

1. `import_decoded` returns one texture per `DecodedLayout::planes` entry, in
   order, and never copies pixels when `zero_copy_decode` is true.
   Biplanar buffers import as per-plane `R8`/`RG8` or `R16Unorm`/`RG16Unorm`
   textures through `CVMetalTextureCache`; `y416` imports as one
   `RGBA16Unorm` texture with an AYCbCr swizzle (section 13).
2. Exported surfaces are created once per pool generation and reused; a
   surface is not written while the viewer holds it (contract 2).
3. `encoder_input` returns planes whose layout matches `desc.layout` and a
   native frame whose color attachments match `desc.color`.
4. `import_blender_surface` premultiplies on input: Blender surfaces are
   straight alpha (sections 9 and 17).

Checked by `Validate`:

5. `DecodedFrameDesc`: the layout's range (`420v` vs `420f`) matches the
   color description's range; subsampling and bit depth agree; planar
   samples are 8 to 16 bits; packed RGBA uses an RGBA format; a clean
   aperture is non-empty and inside the frame. (Section 13: the requested
   format's range matches the bitstream and is verified against buffer
   attachments.)
6. `SurfacePool`: 4 to 32 surfaces, `pool_index` equal to position, sizes
   1 to 16384, every surface the same size and format, and viewer formats
   only (`bgra8Unorm` or `rgba16Float`).

Normalization constants for each layout are part of contract 4
(`color::normalization`), tested against the section 13 numbers.

## Versioning

Version 0 until Phase 2 starts. The trait is a Rust interface inside engine
processes; only the descriptors and `SurfacePool` cross processes. Adding a
decoded layout (for example ProRes RAW after cutover) is a new variant and a
version bump after the freeze. The Windows port adds implementations, not
trait methods; if it needs a method, that is a bump.

## What Phase 0 must confirm

- **S1:** surface export and import-once with `importSharedTexture`, the
  bridge index scheme in `PlatformSurface`, and the fallbacks (C′ remote
  layer, B shared memory). If S1 lands on C′, a `PlatformSurface` variant is
  added before the freeze.
- **S2:** which VideoToolbox formats the reference machine delivers for
  ProRes 422, 4444, XQ, 10-bit HEVC and H.264; `rgba32float` filter and
  blend support (`InteropCapabilities`); Metal precise math for
  color-critical shaders; the encoder-input buffer formats AVAssetWriter
  accepts with color attachments.
- **S6 (decode):** zero-copy import of the corpus into wgpu for every
  layout listed, including FFmpeg's planar LSB formats (DNxHR HQX and 444,
  H.264 4:2:2 10-bit, VP9 profile 2) and odd sizes (the CPU implementation
  rounds plane sizes up; S6 confirms Metal does the same).
- **S7:** 16-bit straight-alpha Blender surfaces into the job engine through
  `import_blender_surface`.
- **Adapter matching:** that Chromium's GPU process and the engine resolve
  the same `registryID` on the reference machine.

## Open questions

1. Whether `import_blender_surface` should take a mapped file (capability
   handle) instead of a `&[u16]` slice, avoiding one copy of a 512 MiB
   8192² surface. Likely yes; S7 measures.
2. Whether P010-style MSB-aligned FFmpeg output (`p010le`) needs its own
   layout or reuses `Biplanar420_10`. They share normalization; S6 decides.
3. Whether encoder input needs a 4:4:4 Y′CbCr layout for ProRes 4444 at
   12 bits (`y416` input to VideoToolbox) or always goes through RGB. S2
   decides, together with the pipeline transparency bar.

## Implementation notes (production bridge, Phase 2)

- The export half is `pm_gpu::bridge_out`: `ExportedSurface::create` takes
  this contract's `SurfaceRequest` and returns the IOSurface and the wgpu
  texture over it; `SurfacePool::descriptor` is this contract's
  `SurfacePool`. It is not yet a `GpuInterop` impl: `decode_in` (media) is
  the import half, and one impl over a shared device type waits for
  `encoder_input` and `import_blender_surface`.
- `pm-gpu`'s `decode_in` is behind a default `decode-in` feature; the
  interactive engine's presenter builds without it so the engine addon does
  not link FFmpeg before media reaches the engine.
- The viewer display transform rounds `rgbaf16` results to f16
  (`quantizeToF16`) before the render-target write: Apple GPUs truncate on
  those writes, which after the display-p3 conversion cost up to 0.97 CV10
  on a grey ramp.
- Electron's `importSharedTexture` keeps no reference to the
  `IOSurfaceRef` in main (retain count 1 with an import alive); the GPU
  process holds its own.
