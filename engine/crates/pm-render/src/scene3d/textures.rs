//! Textures of the 3D pass: material maps (`new THREE.Texture(bitmap)`:
//! sRGB decode for color and emissive maps, repeat wrapping, generated
//! mipmaps, trilinear filtering), shadow maps (RGBA8 with nearest
//! filtering, clamped), and the rectangular area light tables
//! (`LTC_FLOAT_1` and `LTC_FLOAT_2`: RGBA f32 64 × 64, linear
//! magnification, nearest minification, clamped).
//!
//! Map images are the stills image layers draw: the same native-depth
//! samples converted by the same source pass (`MediaCache::image`), in
//! every class image layers draw, oriented, top row first (texture
//! coordinate v = 0 samples the image's top row). three.js samples a map as
//! straight alpha, so the converted premultiplied values are divided by
//! alpha into the map (`MAP_STORE_WGSL`): color maps in linear light,
//! stored as sRGB-encoded RGBA8 (the hardware decodes on sampling), data
//! maps as their encoded values; a still deeper than 8 bits or HDR keeps
//! half float. Mipmaps are box filters, in linear light for color maps.
//! Probes are decided once per file version ([`Textures::probe`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// A still outside the drawn classes: the same feature an image layer
    /// of the file reports (`crate::media::probe_image`), as both read the
    /// same samples.
    Unsupported(crate::capability::Feature),
    Failed(String),
}

/// Divides a converted, premultiplied working texture by alpha: the
/// straight texels three.js samples.
const MAP_STORE_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(a * 2.0 - 1.0, 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(pos.xy), 0);
    return vec4<f32>(select(vec3<f32>(0.0), p.rgb / p.a, p.a > 0.0), p.a);
}
"#;

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
    /// Maps of a file version that did not load (`resolve_maps` drops or
    /// refuses them).
    failed: HashMap<MapKey, MapError>,
    /// The current 3D frame generation (`Scene3dRenderer::begin_frame`).
    generation: u64,
    /// The mip pass by map format, and its layout.
    mip: Option<(
        HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
        wgpu::BindGroupLayout,
    )>,
    /// The straight-alpha store pass by map format, and its layout.
    store: Option<(
        HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
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
            failed: HashMap::new(),
            generation: 0,
            mip: None,
            store: None,
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

    /// A one-texture fullscreen pass of `wgsl` (`vs` and `fs`) into
    /// `format`.
    fn pass_pipeline(
        device: &wgpu::Device,
        wgsl: &str,
        format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3d map pass"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d map pass"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3d map pass"),
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
    }

    fn texture_layout(
        device: &wgpu::Device,
        filterable: bool,
        sampler: bool,
    ) -> wgpu::BindGroupLayout {
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }];
        if sampler {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3d map pass"),
            entries: &entries,
        })
    }

    /// Draws one fullscreen pass of `pipeline` into `dst`.
    fn draw_pass(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
        dst: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d map pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
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
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// `generateMipmap`: each level of `texture` a box filter of the one
    /// above, in linear light for sRGB formats (the views decode and
    /// encode) and float formats holding linear light.
    fn mips(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) {
        self.samplers(device);
        let format = texture.format();
        let (pipelines, layout) = self
            .mip
            .get_or_insert_with(|| (HashMap::new(), Self::texture_layout(device, true, true)));
        let pipeline = pipelines
            .entry(format)
            .or_insert_with(|| Self::pass_pipeline(device, MIP_WGSL, format, layout));
        let sampler = self.linear_clamp.as_ref().expect("samplers");
        for level in 1..texture.mip_level_count() {
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
            Self::draw_pass(encoder, pipeline, &bind, &dst);
        }
    }

    fn key(path: &Path, srgb: bool) -> Option<MapKey> {
        let (len, modified) = stamp(path)?;
        Some(MapKey {
            path: path.to_path_buf(),
            len,
            modified,
            srgb,
        })
    }

    fn insert(&mut self, key: MapKey, texture: wgpu::Texture) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.maps
            .retain(|k, _| k.path != key.path || k.srgb != key.srgb);
        self.failed
            .retain(|k, _| k.path != key.path || k.srgb != key.srgb);
        self.maps.insert(
            key,
            MapEntry {
                texture: Arc::new(MapTexture { texture, view }),
                used: self.generation,
            },
        );
    }

    /// The map of `path` from its converted still (`MediaCache::image`:
    /// premultiplied, in linear light for an sRGB map, else in the display
    /// encoding): straight texels in sRGB RGBA8, RGBA8 or (`deep`) half
    /// float, with its mip chain.
    #[allow(clippy::too_many_arguments)]
    pub fn put_still(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        srgb: bool,
        still: &wgpu::Texture,
        deep: bool,
    ) {
        let Some(key) = Self::key(path, srgb) else {
            return;
        };
        let format = match (deep, srgb) {
            (true, _) => wgpu::TextureFormat::Rgba16Float,
            (false, true) => wgpu::TextureFormat::Rgba8UnormSrgb,
            (false, false) => wgpu::TextureFormat::Rgba8Unorm,
        };
        let (w, h) = (still.width(), still.height());
        let texture = Self::map_texture(device, w, h, format);
        let (pipelines, layout) = self
            .store
            .get_or_insert_with(|| (HashMap::new(), Self::texture_layout(device, false, false)));
        let pipeline = pipelines
            .entry(format)
            .or_insert_with(|| Self::pass_pipeline(device, MAP_STORE_WGSL, format, layout));
        let src = still.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3d map store"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&src),
            }],
        });
        let dst = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        Self::draw_pass(encoder, pipeline, &bind, &dst);
        self.mips(device, encoder, &texture);
        self.insert(key, texture);
    }

    /// Records that the map of `path`'s current version did not load.
    pub fn put_failure(&mut self, path: &Path, srgb: bool, error: MapError) {
        if let Some(key) = Self::key(path, srgb) {
            self.failed
                .retain(|k, _| k.path != key.path || k.srgb != key.srgb);
            self.failed.insert(key, error);
        }
    }

    fn map_texture(
        device: &wgpu::Device,
        w: u32,
        h: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3d map"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_levels(w, h),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                // Read back by the map tests.
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// The GPU texture of a map image (mipmapped), prepared for this file
    /// version by `Scene3dRenderer::prepare_image_maps` (or an SVG map).
    pub fn map(&mut self, path: &Path, srgb: bool) -> Result<Arc<MapTexture>, MapError> {
        let key = Self::key(path, srgb)
            .ok_or_else(|| MapError::Failed(format!("{} is offline", path.display())))?;
        if let Some(e) = self.maps.get_mut(&key) {
            e.used = self.generation;
            return Ok(e.texture.clone());
        }
        Err(self
            .failed
            .get(&key)
            .cloned()
            .unwrap_or_else(|| MapError::Failed(format!("{} was not loaded", path.display()))))
    }

    /// Whether the map of `path`'s current version is loaded or failed.
    pub fn has_result(&self, path: &Path, srgb: bool) -> bool {
        Self::key(path, srgb)
            .is_some_and(|k| self.maps.contains_key(&k) || self.failed.contains_key(&k))
    }

    /// Whether the map of `path`'s current version is cached.
    pub fn has_map(&mut self, path: &Path, srgb: bool) -> bool {
        let Some((len, modified)) = stamp(path) else {
            return false;
        };
        let key = MapKey {
            path: path.to_path_buf(),
            len,
            modified,
            srgb,
        };
        match self.maps.get_mut(&key) {
            Some(e) => {
                e.used = self.generation;
                true
            }
            None => false,
        }
    }

    /// Installs an SVG map's pixels into the ordinary map cache. The key
    /// includes the source file version, as it does for decoded stills.
    #[allow(clippy::too_many_arguments)]
    pub fn put_svg(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        srgb: bool,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), MapError> {
        let key =
            Self::key(path, srgb).ok_or_else(|| MapError::Failed("SVG map is offline".into()))?;
        if let Some(e) = self.maps.get_mut(&key) {
            e.used = self.generation;
            return Ok(());
        }
        let texture = self.upload_rgba(device, queue, encoder, width, height, rgba, srgb, true);
        self.insert(key, texture);
        Ok(())
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
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let texture = if mips {
            Self::map_texture(device, w, h, format)
        } else {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3d map"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
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
        self.mips(device, encoder, &texture);
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
                let texel = u64::from(t.format().block_copy_size(None).unwrap_or(4));
                (0..t.mip_level_count())
                    .map(|l| {
                        let (w, h) = ((t.width() >> l).max(1), (t.height() >> l).max(1));
                        u64::from(w) * u64::from(h) * texel
                    })
                    .sum::<u64>()
            })
            .sum();
        (self.maps.len(), bytes)
    }

    /// Whether `path` is a map image this pass draws: the verdict of an
    /// image layer of the file ([`crate::media::probe_image`], decided once
    /// per file version), so the capability check and the scene build can
    /// ask at every frame.
    pub fn probe(path: &Path) -> Result<(), MapError> {
        crate::media::probe_image(path).map_err(|e| match e {
            crate::media::ProbeIssue::Unsupported(f) => MapError::Unsupported(f),
            crate::media::ProbeIssue::Unreadable(m) => MapError::Failed(m),
        })
    }
}

/// A map's converted still: `MediaCache::image` in linear light for an sRGB
/// map, else in the display encoding, at full float precision (the map
/// keeps its own texture, so the converted one is released), and whether
/// the still is deeper than 8 bits.
pub fn still_map(
    media: &mut crate::media::MediaCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &crate::pipelines::Pipelines,
    encoder: &mut wgpu::CommandEncoder,
    path: &Path,
    srgb: bool,
) -> Result<(wgpu::Texture, bool), MapError> {
    use pm_protocol::color::{Precision, WorkingEncoding};
    let working = if srgb {
        WorkingEncoding::LinearLight
    } else {
        WorkingEncoding::Standard
    };
    let target = media
        .image(
            device,
            queue,
            pipelines,
            encoder,
            path,
            working,
            Precision::Float32,
            None,
        )
        .map_err(|e| match e {
            crate::media::MediaFailure::Unsupported(f) => MapError::Unsupported(f),
            crate::media::MediaFailure::Failed(m) => MapError::Failed(m),
        })?;
    media.release_image(path, Precision::Float32, working);
    Ok((target.texture, crate::media::deep_still(path)))
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
