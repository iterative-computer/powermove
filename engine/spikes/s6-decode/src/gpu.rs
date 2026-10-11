//! wgpu (Metal) side: zero-copy import of decoded planes and the normalization
//! pass.
//!
//! VideoToolbox buffers are imported per plane through `CVMetalTextureCache`
//! (y416 as one `Rgba16Unorm` texture). FFmpeg frames in pool memory are imported
//! as linear textures over the slot's `MTLBuffer`. Either way the resulting
//! `MTLTexture` is wrapped with `wgpu_hal::metal::Device::texture_from_raw` and
//! `wgpu::Device::create_texture_from_hal`; the texture's drop callback holds the
//! CoreVideo objects or pool lease until wgpu has finished with it.

use crate::apple::*;
use crate::frame::{AlphaSrc, Decoded, FrameDesc, Kind, Pixels};
use crate::pool::MetalPool;
use anyhow::{Context, Result, bail, ensure};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLBuffer, MTLDevice, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage};
use std::sync::Arc;
use std::time::Instant;

const SHADER: &str = include_str!("normalize.wgsl");

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: String,
    mtl: Retained<ProtocolObject<dyn MTLDevice>>,
    cache: Cf,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    dummy: wgpu::TextureView,
    pub pool: Arc<MetalPool>,
}

pub struct Imported {
    pub textures: Vec<wgpu::Texture>,
    pub desc: FrameDesc,
    /// Every plane's MTLTexture was checked to alias the decoder's output memory:
    /// the CVPixelBuffer's IOSurface (and plane), or the pool MTLBuffer at the
    /// exact address FFmpeg wrote the plane to.
    pub aliased: bool,
}

#[derive(Default, Clone, Copy, Debug, serde::Serialize)]
pub struct GpuTiming {
    pub import_us: f64,
    pub normalize_us: f64,
}

pub struct Normalized {
    pub luma: Vec<[f32; 2]>,
    pub chroma: Vec<[f32; 2]>,
    pub timing: GpuTiming,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    kind: u32,
    alpha: u32,
    sx: u32,
    sy: u32,
    code_max: f32,
    shift: f32,
    y_off: f32,
    y_div: f32,
    c_off: f32,
    c_div: f32,
    a_code_max: f32,
    a_shift: f32,
    a_off: f32,
    a_div: f32,
    _pad: [f32; 2],
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s6"),
            required_features: wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
            ..Default::default()
        }))?;
        let mtl = unsafe {
            let hal = device.as_hal::<wgpu_hal::api::Metal>().context("not a Metal device")?;
            hal.raw_device().clone()
        };
        let mut cache: CVMetalTextureCacheRef = std::ptr::null_mut();
        let st = unsafe {
            CVMetalTextureCacheCreate(
                kCFAllocatorDefault,
                std::ptr::null(),
                Retained::as_ptr(&mtl) as *mut std::ffi::c_void,
                std::ptr::null(),
                &mut cache,
            )
        };
        ensure!(st == 0, "CVMetalTextureCacheCreate: {st}");
        let cache = unsafe { Cf::from_create(cache as CFTypeRef) }.unwrap();

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("normalize"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |b| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let out = |b| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("normalize"),
            entries: &[
                tex(0),
                tex(1),
                tex(2),
                tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                out(5),
                out(6),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("normalize"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("normalize"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let info = adapter.get_info();
        let pool = MetalPool::new(mtl.clone());
        Ok(Gpu {
            device,
            queue,
            adapter: format!("{} ({:?})", info.name, info.backend),
            mtl,
            cache,
            pipeline,
            layout,
            dummy,
            pool,
        })
    }

    pub fn metal_device(&self) -> &ProtocolObject<dyn MTLDevice> {
        &self.mtl
    }

    fn wrap(
        &self,
        raw: Retained<ProtocolObject<dyn MTLTexture>>,
        format: wgpu::TextureFormat,
        w: u32,
        h: u32,
        keep: Box<dyn FnOnce() + Send + Sync>,
    ) -> wgpu::Texture {
        unsafe {
            let hal = wgpu_hal::metal::Device::texture_from_raw(
                raw,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu_hal::CopyExtent { width: w, height: h, depth: 1 },
                Some(keep),
            );
            self.device.create_texture_from_hal::<wgpu_hal::api::Metal>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("decoded plane"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::RESOURCE,
            )
        }
    }

    /// Imports every plane of a decoded frame as a wgpu texture over the
    /// decoder's own memory.
    pub fn import(&self, d: &Decoded) -> Result<Imported> {
        let desc = d.desc()?;
        let mut textures = Vec::with_capacity(desc.planes.len());
        let mut aliased = true;
        match &d.px {
            Pixels::Vt(pb) => {
                for (i, p) in desc.planes.iter().enumerate() {
                    let mut t: CVMetalTextureRef = std::ptr::null_mut();
                    let st = unsafe {
                        CVMetalTextureCacheCreateTextureFromImage(
                            kCFAllocatorDefault,
                            self.cache.as_mut_ptr(),
                            pb.as_mut_ptr(),
                            std::ptr::null(),
                            p.format.metal().0,
                            p.width,
                            p.height,
                            i,
                            &mut t,
                        )
                    };
                    ensure!(st == 0 && !t.is_null(), "CVMetalTextureCacheCreateTextureFromImage plane {i}: {st}");
                    let cvt = unsafe { Cf::from_create(t as CFTypeRef) }.unwrap();
                    let raw = unsafe {
                        let mt = CVMetalTextureGetTexture(cvt.as_mut_ptr());
                        Retained::retain(mt as *mut ProtocolObject<dyn MTLTexture>).context("CVMetalTextureGetTexture")?
                    };
                    // The texture aliases the IOSurface: same memory the decoder wrote.
                    let surface = unsafe { CVPixelBufferGetIOSurface(pb.as_mut_ptr()) } as *const std::ffi::c_void;
                    let tex_surface = raw.iosurface().map(|s| Retained::as_ptr(&s) as *const std::ffi::c_void);
                    let plane_ok = desc.planes.len() == 1 || raw.iosurfacePlane() == i;
                    aliased &= !surface.is_null() && tex_surface == Some(surface) && plane_ok;
                    let keep_pb = pb.clone();
                    let keep = Box::new(move || {
                        drop(cvt);
                        drop(keep_pb);
                    });
                    textures.push(self.wrap(raw, p.format.wgpu(), p.width as u32, p.height as u32, keep));
                }
            }
            Pixels::Ff(fr) => {
                for (p, pl) in desc.planes.iter().zip(&fr.layout.planes) {
                    let raw = unsafe {
                        let td = MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                            p.format.metal(),
                            p.width,
                            p.height,
                            false,
                        );
                        td.setStorageMode(MTLStorageMode::Shared);
                        td.setUsage(MTLTextureUsage::ShaderRead);
                        let buf: &ProtocolObject<dyn MTLBuffer> = fr.slot.buffer();
                        let align = self.mtl.minimumLinearTextureAlignmentForPixelFormat(p.format.metal());
                        if pl.offset % align != 0 || pl.stride % align != 0 {
                            bail!("plane offset {} / stride {} not {}-aligned", pl.offset, pl.stride, align);
                        }
                        buf.newTextureWithDescriptor_offset_bytesPerRow(&td, pl.offset, pl.stride)
                            .context("newTextureWithDescriptor:offset:bytesPerRow:")?
                    };
                    let plane_ptr = unsafe { (*fr.frame).data[textures.len()] } as usize;
                    let tex_ptr = raw
                        .buffer()
                        .map(|b| b.contents().as_ptr() as usize + raw.bufferOffset())
                        .unwrap_or(0);
                    // For a copied frame the texture aliases the pool copy, not FFmpeg's buffer.
                    aliased &= !fr.copied && tex_ptr == plane_ptr;
                    let lease = fr.slot.clone();
                    let keep = Box::new(move || drop(lease));
                    textures.push(self.wrap(raw, p.format.wgpu(), p.width as u32, p.height as u32, keep));
                }
            }
        }
        Ok(Imported { textures, desc, aliased })
    }

    /// Runs the normalization pass and reads the result back (verification).
    pub fn normalize(&self, imp: &Imported, import_us: f64) -> Result<Normalized> {
        let d = &imp.desc;
        let n = d.norm;
        let params = Params {
            kind: d.kind as u32,
            alpha: match d.alpha {
                AlphaSrc::None => 0,
                AlphaSrc::Plane(2) => 1,
                AlphaSrc::Plane(3) => 3,
                AlphaSrc::Plane(_) => bail!("alpha plane index"),
                AlphaSrc::Packed => 2,
            },
            sx: d.log2_cw,
            sy: d.log2_ch,
            code_max: n.code_max,
            shift: n.shift,
            y_off: n.y_off,
            y_div: n.y_div,
            c_off: n.c_off,
            c_div: n.c_div,
            a_code_max: n.a_code_max,
            a_shift: n.a_shift,
            a_off: n.a_off,
            a_div: n.a_div,
            _pad: [0.0; 2],
        };
        let t0 = Instant::now();
        let (w, h) = (d.width as u32, d.height as u32);
        let (cw, ch) = ((w + (1 << d.log2_cw) - 1) >> d.log2_cw, (h + (1 << d.log2_ch) - 1) >> d.log2_ch);
        let mk_out = |w, h| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("normalized"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let out_y = mk_out(w, h);
        let out_c = mk_out(cw, ch);
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&ubuf, 0, unsafe {
            std::slice::from_raw_parts(&params as *const Params as *const u8, std::mem::size_of::<Params>())
        });
        let views: Vec<wgpu::TextureView> = imp.textures.iter().map(|t| t.create_view(&Default::default())).collect();
        let v = |i: usize| views.get(i).unwrap_or(&self.dummy);
        let oy = out_y.create_view(&Default::default());
        let oc = out_c.create_view(&Default::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v(0)) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(v(1)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(v(2)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(v(3)) },
                wgpu::BindGroupEntry { binding: 4, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&oy) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&oc) },
            ],
        });
        let row = |w: u32| (w * 16).div_ceil(256) * 256;
        let rb_y = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rb y"),
            size: (row(w) * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let rb_c = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rb c"),
            size: (row(cw) * ch) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        let copy = |enc: &mut wgpu::CommandEncoder, t: &wgpu::Texture, b: &wgpu::Buffer, w: u32, h: u32| {
            enc.copy_texture_to_buffer(
                t.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: b,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row(w)), rows_per_image: Some(h) },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        };
        copy(&mut enc, &out_y, &rb_y, w, h);
        copy(&mut enc, &out_c, &rb_c, cw, ch);
        let idx = self.queue.submit([enc.finish()]);
        self.device
            .poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None })
            .map_err(|e| anyhow::anyhow!("poll: {e:?}"))?;
        let normalize_us = t0.elapsed().as_secs_f64() * 1e6;
        let read = |b: &wgpu::Buffer, w: u32, h: u32, pick: fn(&[f32]) -> [f32; 2]| -> Result<Vec<[f32; 2]>> {
            let s = b.slice(..);
            s.map_async(wgpu::MapMode::Read, |_| {});
            self.device
                .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
                .map_err(|e| anyhow::anyhow!("poll: {e:?}"))?;
            let data = s.get_mapped_range().map_err(|e| anyhow::anyhow!("map: {e:?}"))?;
            let mut out = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                let r = &data[(y * row(w)) as usize..][..(w * 16) as usize];
                let f: &[f32] = unsafe { std::slice::from_raw_parts(r.as_ptr() as *const f32, (w * 4) as usize) };
                for x in 0..w as usize {
                    out.push(pick(&f[x * 4..x * 4 + 4]));
                }
            }
            drop(data);
            b.unmap();
            Ok(out)
        };
        let luma = read(&rb_y, w, h, |p| [p[0], p[1]])?;
        let chroma = read(&rb_c, cw, ch, |p| [p[0], p[1]])?;
        Ok(Normalized { luma, chroma, timing: GpuTiming { import_us, normalize_us } })
    }

    pub fn import_and_normalize(&self, d: &Decoded) -> Result<(Imported, Normalized)> {
        let t0 = Instant::now();
        let imp = self.import(d)?;
        let import_us = t0.elapsed().as_secs_f64() * 1e6;
        let n = self.normalize(&imp, import_us)?;
        Ok((imp, n))
    }

    pub fn flush_cache(&self) {
        unsafe { CVMetalTextureCacheFlush(self.cache.as_mut_ptr(), 0) }
    }
}

/// Exact-match statistics between GPU output and the CPU reference.
#[derive(Default, Clone, Copy, Debug, serde::Serialize)]
pub struct Compare {
    pub values: u64,
    pub exact: u64,
    pub max_abs: f32,
    pub max_ulps: u32,
}

impl Compare {
    pub fn add(&mut self, gpu: &[[f32; 2]], cpu: &[[f32; 2]]) {
        assert_eq!(gpu.len(), cpu.len());
        for (g, c) in gpu.iter().zip(cpu) {
            for k in 0..2 {
                self.values += 1;
                if g[k].to_bits() == c[k].to_bits() {
                    self.exact += 1;
                } else {
                    self.max_abs = self.max_abs.max((g[k] - c[k]).abs());
                    self.max_ulps = self.max_ulps.max(ulps(g[k], c[k]));
                }
            }
        }
    }
    pub fn merge(&mut self, o: &Compare) {
        self.values += o.values;
        self.exact += o.exact;
        self.max_abs = self.max_abs.max(o.max_abs);
        self.max_ulps = self.max_ulps.max(o.max_ulps);
    }
}

fn ulps(a: f32, b: f32) -> u32 {
    let k = |x: f32| {
        let i = x.to_bits() as i32;
        if i < 0 { i32::MIN.wrapping_sub(i) } else { i }
    };
    (k(a) as i64 - k(b) as i64).unsigned_abs().min(u32::MAX as u64) as u32
}

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Biplanar => "biplanar",
        Kind::Planar => "planar",
        Kind::PackedAycbcr => "packed AY'CbCr",
    }
}
