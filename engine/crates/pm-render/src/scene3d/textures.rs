//! Textures of the 3D pass: material maps (`new THREE.Texture(bitmap)`:
//! RGBA8, sRGB decode for color and emissive maps, repeat wrapping,
//! generated mipmaps, trilinear filtering), shadow maps (RGBA8 with
//! nearest filtering, clamped), and the rectangular area light tables
//! (`LTC_FLOAT_1` and `LTC_FLOAT_2`: RGBA f32 64 × 64, linear
//! magnification, nearest minification, clamped).
//!
//! Map images decode through `pm-media` and only in the still classes the
//! 2D renderer draws from the native reader (`crate::media::probe_still`
//! with `StillReader::Native`); images with
//! transparency are refused, since the premultiplication Chromium applies
//! to an `ImageBitmap` uploaded to WebGL has not been measured. Both are
//! decided statically ([`Textures::probe`], cached per file version), so
//! the capability check routes such maps before a frame renders. A map's
//! rows upload top row first, as WebGL uploads an `ImageBitmap` (which
//! ignores `UNPACK_FLIP_Y_WEBGL`), so texture coordinate v = 0 samples the
//! image's top row.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// A material map on the GPU.
pub struct MapTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

struct MapEntry {
    texture: Arc<MapTexture>,
    /// The 3D frame generation of the last use ([`Textures::evict`]).
    used: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MapKey {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
    srgb: bool,
}

/// Why a map cannot be drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum MapError {
    /// Outside the classes this pass draws (routed to the legacy engine).
    Unsupported(String),
    Failed(String),
}

const MIP_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let a = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(a * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(a.x, 1.0 - a.y);
    return o;
}
@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return textureSampleLevel(src, smp, in.uv, 0.0);
}
"#;

/// Material maps, shadow-map samplers, and the LTC tables, kept across
/// frames.
pub struct Textures {
    maps: HashMap<MapKey, MapEntry>,
    /// The current 3D frame generation (`Scene3dRenderer::begin_frame`).
    generation: u64,
    pool: pm_media::pool::FramePool,
    mip: Option<(
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::BindGroupLayout,
    )>,
    ltc: Option<(wgpu::TextureView, wgpu::TextureView)>,
    pub repeat_trilinear: Option<wgpu::Sampler>,
    pub clamp_nearest: Option<wgpu::Sampler>,
    pub ltc_sampler: Option<wgpu::Sampler>,
    linear_clamp: Option<wgpu::Sampler>,
}

impl Default for Textures {
    fn default() -> Self {
        Textures {
            maps: HashMap::new(),
            generation: 0,
            pool: pm_media::pool::FramePool::new(16 << 20),
            mip: None,
            ltc: None,
            repeat_trilinear: None,
            clamp_nearest: None,
            ltc_sampler: None,
            linear_clamp: None,
        }
    }
}

fn stamp(path: &Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()))
}

/// The mip chain length of a `w` × `h` texture.
pub fn mip_levels(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

impl Textures {
    /// Creates the samplers once.
    pub fn samplers(&mut self, device: &wgpu::Device) {
        if self.repeat_trilinear.is_some() {
            return;
        }
        self.repeat_trilinear = Some(device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3d map"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        }));
        self.clamp_nearest = Some(device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3d shadow map"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        }));
        self.ltc_sampler = Some(device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3d ltc"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        }));
        self.linear_clamp = Some(device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3d mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }));
    }

    /// `ltc_1` and `ltc_2` (requires `FLOAT32_FILTERABLE`).
    pub fn ltc(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(wgpu::TextureView, wgpu::TextureView), String> {
        if !device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
        {
            return Err("rectangular area lights need filterable 32-bit float textures".into());
        }
        if let Some(t) = &self.ltc {
            return Ok(t.clone());
        }
        let make = |data: &[u8], label: &str| {
            let t = wgpu::util::DeviceExt::create_texture_with_data(
                device,
                queue,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 64,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                data,
            );
            t.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let t = (
            make(include_bytes!("ltc/ltc_1.f32"), "ltc_1"),
            make(include_bytes!("ltc/ltc_2.f32"), "ltc_2"),
        );
        self.ltc = Some(t.clone());
        Ok(t)
    }

    fn mip_pipelines(&mut self, device: &wgpu::Device) {
        if self.mip.is_some() {
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3d mips"),
            source: wgpu::ShaderSource::Wgsl(MIP_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3d mips"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d mips"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("3d mips"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        self.mip = Some((
            make(wgpu::TextureFormat::Rgba8Unorm),
            make(wgpu::TextureFormat::Rgba8UnormSrgb),
            layout,
        ));
    }

    /// The GPU texture of a map image (mipmapped), decoded once per file
    /// version.
    pub fn map(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        srgb: bool,
    ) -> Result<Arc<MapTexture>, MapError> {
        let (len, modified) = stamp(path)
            .ok_or_else(|| MapError::Failed(format!("{} is offline", path.display())))?;
        let key = MapKey {
            path: path.to_path_buf(),
            len,
            modified,
            srgb,
        };
        if let Some(e) = self.maps.get_mut(&key) {
            e.used = self.generation;
            return Ok(e.texture.clone());
        }
        let (w, h, rgba) = decode(path, &self.pool)?;
        let texture = self.upload_rgba(device, queue, encoder, w, h, &rgba, srgb, true);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let t = Arc::new(MapTexture { texture, view });
        self.maps
            .retain(|k, _| k.path != key.path || k.srgb != key.srgb);
        self.maps.insert(
            key,
            MapEntry {
                texture: t.clone(),
                used: self.generation,
            },
        );
        Ok(t)
    }

    /// An RGBA8 texture from tight rows (top row first), with
    /// `generateMipmap`'s chain when `mips` (box filters, in linear light
    /// for sRGB textures).
    #[allow(clippy::too_many_arguments)]
    pub fn upload_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        w: u32,
        h: u32,
        rgba: &[u8],
        srgb: bool,
        mips: bool,
    ) -> wgpu::Texture {
        self.samplers(device);
        self.mip_pipelines(device);
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let levels = if mips { mip_levels(w, h) } else { 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3d map"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        // `generateMipmap`: each level a box filter of the one above, in
        // linear light for sRGB textures (the views decode and encode).
        let (unorm, srgb_pipe, layout) = self.mip.as_ref().expect("mip pipelines");
        let pipeline = if srgb { srgb_pipe } else { unorm };
        let sampler = self.linear_clamp.as_ref().expect("samplers");
        for level in 1..levels {
            let view = |l: u32| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: l,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            };
            let (src, dst) = (view(level - 1), view(level));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3d mip"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        texture
    }

    /// Starts 3D frame `generation`: maps no 3D frame used since
    /// generation `keep_from` are released (today's runtime disposes the
    /// textures its scene no longer references on every sync).
    pub(crate) fn evict(&mut self, generation: u64, keep_from: u64) {
        self.generation = generation;
        self.maps.retain(|_, e| e.used >= keep_from);
    }

    /// Map textures on the GPU, and their bytes (mip chains included).
    pub fn resident(&self) -> (usize, u64) {
        let bytes = self
            .maps
            .values()
            .map(|e| {
                let t = &e.texture.texture;
                (0..t.mip_level_count())
                    .map(|l| {
                        let (w, h) = ((t.width() >> l).max(1), (t.height() >> l).max(1));
                        u64::from(w) * u64::from(h) * 4
                    })
                    .sum::<u64>()
            })
            .sum();
        (self.maps.len(), bytes)
    }

    /// Whether `path` is a map image this pass draws: readable, in the
    /// still classes the 2D renderer draws, and opaque (every alpha 255).
    /// Decided once per file version (path, size, modification time), so
    /// the capability check and the scene build can ask at every frame.
    pub fn probe(path: &Path) -> Result<(), MapError> {
        type Probes = Mutex<HashMap<MapKey, Result<(), MapError>>>;
        static CACHE: OnceLock<Probes> = OnceLock::new();
        let (len, modified) = stamp(path)
            .ok_or_else(|| MapError::Failed(format!("{} is offline", path.display())))?;
        let key = MapKey {
            path: path.to_path_buf(),
            len,
            modified,
            srgb: false,
        };
        let cache = CACHE.get_or_init(Default::default);
        if let Some(r) = cache.lock().unwrap().get(&key) {
            return r.clone();
        }
        let result = decode(path, &pm_media::pool::FramePool::new(0)).map(|_| ());
        let mut c = cache.lock().unwrap();
        // One answer per path: a new version replaces the old one.
        c.retain(|k, _| k.path != key.path);
        if c.len() >= 4096 {
            c.clear();
        }
        c.insert(key, result.clone());
        result
    }
}

/// Decodes a map image to tight RGBA8 rows, top row first.
pub fn decode(
    path: &Path,
    pool: &pm_media::pool::FramePool,
) -> Result<(u32, u32, Vec<u8>), MapError> {
    // Maps upload the native reader's samples, so they are probed in its
    // classes, not in those of Chromium's compatibility bitmap.
    match crate::media::probe_still(path, crate::media::StillReader::Native) {
        Ok(()) => {}
        Err(crate::media::ProbeIssue::Unsupported(f)) => {
            return Err(MapError::Unsupported(format!("3D texture map image ({f})")));
        }
        Err(crate::media::ProbeIssue::Unreadable(m)) => return Err(MapError::Failed(m)),
    }
    // Maps upload stored samples: a PNG gamma Chromium or ImageIO would
    // apply is not implemented for them.
    if pm_media::image::png_gamma::png_gamma(path)
        .map_err(|e| MapError::Failed(format!("{}: {e}", path.display())))?
        .is_some()
    {
        return Err(MapError::Unsupported(
            "3D texture map image (PNG gamma (gAMA))".into(),
        ));
    }
    let still = pm_media::image::read(path, pool)
        .map_err(|e| MapError::Failed(format!("{}: {e}", path.display())))?;
    let f = &still.frame.format;
    if f.planes.len() != 1
        || f.planes[0].format != pm_media::frame::PlaneFormat::Rgba8
        || f.rgba_premultiplied
        || still.orientation != 1
    {
        return Err(MapError::Unsupported(format!(
            "3D texture map image layout {}",
            f.label
        )));
    }
    let (w, h) = (f.width, f.height);
    let mut out = vec![0u8; w * h * 4];
    still
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            for y in 0..h {
                // SAFETY: the plane holds `height` rows of `stride` bytes.
                let row = unsafe { std::slice::from_raw_parts(p.ptr.add(y * p.stride), w * 4) };
                out[y * w * 4..(y + 1) * w * 4].copy_from_slice(row);
            }
        })
        .map_err(|e| MapError::Failed(e.to_string()))?;
    if out.chunks_exact(4).any(|px| px[3] != 255) {
        return Err(MapError::Unsupported(
            "3D texture maps with transparency".into(),
        ));
    }
    Ok((w as u32, h as u32, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_lengths() {
        assert_eq!(mip_levels(1, 1), 1);
        assert_eq!(mip_levels(256, 256), 9);
        assert_eq!(mip_levels(300, 17), 9);
    }
}
