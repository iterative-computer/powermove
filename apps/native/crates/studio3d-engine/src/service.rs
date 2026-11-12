//! Worker-owned rendering facade. The core lane supplies evaluated scenes by
//! stable ID; a compositor consumes per-layer textures from the shared trace.
use crate::extract::{Extracted, Extractor, SceneInput, SceneRequest, SceneSource};
use crate::gpu::{PresentTarget, Renderer};

pub use pm_render::scene3d::provider::{Denoise, Tier};
pub use pm_render::scene3d::provider::SurfaceHit;

pub struct LayerTexture {
    pub layer: String,
    /// Premultiplied display sRGB, in top-down row order. This is the source
    /// Studio SDR contract, before the compositor's working-space conversion.
    pub texture: wgpu::Texture,
}

/// Synchronous worker stage diagnostics, distinct from display presentation.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTimings {
    pub prepare_ms: f64,
    pub total_ms: f64,
    pub pipeline: crate::gpu::PipelineTimings,
    #[cfg(feature = "oidn")]
    pub oidn: crate::oidn::FilterTimings,
    pub oidn_init_ms: f64,
    pub oidn_reused: bool,
}

pub struct RenderedScene {
    pub request: SceneRequest,
    pub layers: Vec<LayerTexture>,
    pub samples: u32,
    pub adapter: String,
    pub hardware_rt: bool,
    pub timings: FrameTimings,
    pub failures: Vec<pm_render::LayerFailure>,
}

struct PreparedScene {
    quality: Option<crate::extract::FinalQuality>,
    denoise: Denoise,
    live_guided: bool,
    orthographic: bool,
    reset: bool,
    width: u32,
    height: u32,
    outputs: Vec<(String, u32, bool)>,
    key: Vec<u8>,
    resource_start: usize,
    adaptive: crate::environment::AdaptiveSampling,
    layers: Vec<String>,
    failures: Vec<pm_render::LayerFailure>,
}

/// The existing compositor's Fast path. A host must attach it explicitly.
/// A missing path returns an error; it never yields a blank "success".
pub trait FastRenderer: Send {
    fn render(
        &mut self,
        input: &SceneInput,
        request: &SceneRequest,
    ) -> Result<RenderedScene, String>;
}

/// Synchronous worker API. A native engine host must queue calls onto its
/// worker; UI callbacks must never invoke/join these operations.
#[derive(Default)]
pub struct StudioService {
    renderer: Option<Renderer>,
    extractor: Extractor,
    fast: Option<Box<dyn FastRenderer>>,
    previous: Option<(SceneRequest, Vec<u8>)>,
    previous_tier: Option<Tier>,
    geometry_key: Vec<u8>,
    resource_start: usize,
    geometry_uploads: u64,
    scene_updates: u64,
    texture_uploads: u64,
    texture_key: Option<Vec<u8>>,
}

impl StudioService {
    /// A calibration's next cold pass must include a fresh photon bootstrap.
    /// Geometry and pipelines remain retained; ordinary renders reuse the map.
    pub fn reset_caustic_map(&mut self) {
        if let Some(r) = &mut self.renderer {
            r.set_precomputed_caustics(false);
            r.set_precomputed_caustics(true);
        }
    }
    /// Top-left normalized composition coordinates, pinhole camera, world-space
    /// point/normal. Synchronizes the immutable scene without tracing a frame.
    pub fn pick_surface(&mut self, source: &dyn SceneSource, request: SceneRequest, tier: Tier, x:f32, y:f32) -> Result<Option<SurfaceHit>, String> {
        if !x.is_finite() || !y.is_finite() || !(0. ..=1.).contains(&x) || !(0. ..=1.).contains(&y) {
            return Err("Surface coordinates must be finite and in 0..=1".into());
        }
        if tier == Tier::Fast { return Err("Surface picking requires the Studio scene".into()); }
        let prepared = self.prepare_scene(source, request.clone(), tier)?;
        if self.previous.as_ref().is_none_or(|(r,k)| *r != request || *k != prepared.key) || self.previous_tier != Some(tier) {
            // Querying another view must not let its GPU state masquerade as
            // the previous view's progressive accumulation on the next render.
            self.previous = None;
            self.previous_tier = None;
        }
        let Some(hit) = self.renderer.as_mut().unwrap().pick_surface(x,y) else { return Ok(None) };
        let layer = prepared.layers.get(hit.layer as usize).ok_or("Surface pick returned an unknown layer")?.clone();
        Ok(Some(SurfaceHit { layer, instance: hit.instance, point:hit.point, normal:hit.normal }))
    }
    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let rt = device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            && std::env::var_os("STUDIO_NO_RT").is_none();
        Ok(Self {
            renderer: Some(Renderer::with_device(
                device,
                queue,
                "shared engine device".into(),
                rt,
                width,
                height,
            )?),
            ..Self::default()
        })
    }

    pub fn fast_renderer(&mut self, fast: Box<dyn FastRenderer>) {
        self.fast = Some(fast);
    }

    pub fn render(
        &mut self,
        source: &dyn SceneSource,
        request: SceneRequest,
        tier: Tier,
    ) -> Result<RenderedScene, String> {
        self.render_oriented(source, request, tier, false)
    }

    /// Same transport in the compositor's bottom-up FBO row convention.
    pub fn render_bottom_up(
        &mut self,
        source: &dyn SceneSource,
        request: SceneRequest,
        tier: Tier,
    ) -> Result<RenderedScene, String> {
        self.render_oriented(source, request, tier, true)
    }

    fn prepare_scene(&mut self, source: &dyn SceneSource, request: SceneRequest, tier: Tier) -> Result<PreparedScene, String> {
        if request.width == 0 || request.height == 0 {
            return Err("Studio output is empty".into());
        }
        let quality = if matches!(tier, Tier::Final { .. }) {
            source
                .final_quality(&request)?
                .map(|q| q.validate())
                .transpose()?
        } else {
            None
        };
        let denoise = match tier {
            Tier::Final { denoise, .. } => quality.map_or(denoise, |q| {
                if q.denoise {
                    if q.fast_denoise {Denoise::Guided}else{Denoise::Oidn}
                } else {
                    Denoise::None
                }
            }),
            _ => Denoise::Guided,
        };
        // Refuse unsupported optional requests before extraction or GPU work.
        #[cfg(not(feature = "oidn"))]
        if denoise == Denoise::Oidn || matches!(tier, Tier::QualityRefine { .. }) {
            return Err("Studio built without optional OIDN support".into());
        }
        if matches!(
            tier,
            Tier::Final { samples: 0, .. } | Tier::QualityRefine { samples: 0 }
        ) {
            return Err("Studio sample count must be positive".into());
        }
        let input = source.scene(&request)?;
        let failures = source.layer_failures(&request)?;
        let auxiliary = source.auxiliary_layers(&request)?;
        let extracted = self.extractor.extract(&input)?;
        validate(&input, &extracted)?;
        // The MetalFX denoised scaler biases low-albedo, procedurally colored
        // ground dark at Live's half resolution, even with matching guides.
        // Orthographic reference views also retain full pixel coverage and four
        // stratified samples: half-size single rays leave metallic bevels speckled.
        // Our linear temporal/edge-aware filter preserves its exposure. Use
        // that existing GPU path for these scenes; product sets keep MetalFX.
        let live_guided = input.view.orthographic_span.is_some() || extracted.prepared.materials.iter().any(|m| {
            m.policy[3] == 1. && (m.policy[1] as u32 & 16) != 0
        });
        // Compare exact bytes, including resources, instead of trusting a revision
        // or a lossy/partial scene key. No stale frame survives a material edit.
        let (mut key, resource_start) = content(&input, &extracted);
        key.extend_from_slice(format!("{auxiliary:?}").as_bytes());
        let mut outputs: Vec<_> = extracted
            .layers
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index as u32, false))
            .collect();
        let mut auxiliary_ids = std::collections::HashSet::new();
        for layer in auxiliary {
            if layer.id.is_empty() || !auxiliary_ids.insert(layer.id.clone()) {
                return Err("Duplicate or empty Studio output layer".into());
            }
            // An empty procedural layer still owes the compositor a transparent
            // answer. When it has geometry, its ordinary mask already owns it.
            if let Some(output)=outputs.iter_mut().find(|(id,_,_)|*id==layer.id){output.2|=layer.include_void;continue;}
            outputs.push((layer.id, extracted.layers.len() as u32, layer.include_void));
        }
        if outputs.iter().filter(|(_,_,void)|*void).count()>1{return Err("Studio backdrop has multiple output owners".into());}
        let reset = self
            .previous
            .as_ref()
            .is_none_or(|(r, k)| r != &request || *k != key)
            || !matches!(self.previous_tier, Some(Tier::QualityRefine { .. }))
            || !matches!(tier, Tier::QualityRefine { .. });
        let (width, height) = match tier {
            Tier::QualityLive if input.view.orthographic_span.is_none() => (request.width.div_ceil(2), request.height.div_ceil(2)),
            _ => (request.width, request.height),
        };
        if let Some(r) = self.renderer.as_mut() {
            r.resize(width, height)?;
        } else {
            self.renderer = Some(Renderer::new(width, height)?);
        }
        let r = self.renderer.as_mut().unwrap();
        let limit = u64::from(r.device().limits().max_storage_buffer_binding_size);
        for size in [
            extracted.prepared.positions.len() * std::mem::size_of::<[f32; 4]>(),
            extracted.prepared.tris.len() * std::mem::size_of::<crate::scene::GpuTri>(),
            extracted.prepared.nodes.len() * std::mem::size_of::<crate::bvh::WideNode>(),
            extracted.prepared.materials.len() * std::mem::size_of::<crate::scene::GpuMaterial>(),
            extracted.prepared.sdata.len() * std::mem::size_of::<[f32; 4]>(),
        ] {
            if size as u64 > limit {
                return Err("Studio scene exceeds the device storage-buffer limit".into());
            }
        }
        let mut geometry_key = Vec::new();
        for size in [
            extracted.tlas_capacity,
            extracted.prepared.sdata.len(),
            extracted.prepared.materials.len(),
        ] {
            geometry_key.extend_from_slice(&(size as u64).to_le_bytes());
        }
        for data in [
            bytemuck::cast_slice(&extracted.prepared.positions),
            bytemuck::cast_slice(&extracted.prepared.tris),
            bytemuck::cast_slice(&extracted.prepared.nodes[extracted.tlas_capacity..]),
        ] {
            geometry_key.extend_from_slice(&(data.len() as u64).to_le_bytes());
            geometry_key.extend_from_slice(data);
        }
        let geometry_changed = self.geometry_key != geometry_key || self.geometry_uploads == 0;
        let changed = self
            .previous
            .as_ref()
            .is_none_or(|(_, old)| old.get(self.resource_start..) != Some(&key[resource_start..]));
        if changed || geometry_changed {
            let instancing = extracted.prepared.instancing.as_ref().unwrap();
            r.set_instanced_scene(
                &input.scene,
                &extracted.prepared,
                &instancing.geometries,
                &instancing.placements,
                instancing.top_nodes,
                geometry_changed,
            );
            self.geometry_uploads += u64::from(geometry_changed);
            self.scene_updates += 1;
            self.geometry_key = geometry_key;
        }
        let texture_key = texture_key(input.maps.as_ref());
        if self.texture_key.as_ref() != Some(&texture_key) {
            if let Some(maps) = &input.maps {
                let pixels: Vec<_> = maps.layers.iter().flatten().copied().collect();
                r.set_maps(&pixels, maps.width, maps.height, maps.layers.len() as u32);
            } else {
                r.set_screen(&[0; 4], 1, 1);
            }
            self.texture_uploads += 1;
            self.texture_key = Some(texture_key);
        }
        if reset {
            r.set_camera(&input.scene);
            // Ordinary native layers leave uncovered/alpha-tested samples for
            // the compositor. Product sets retain their authored opaque void.
            r.set_void(
                input.view.backdrop_opaque.unwrap_or(input.scene.shadow_catcher)
                    .then(||input.scene.backdrop.to_array()),
            );
            let c = input.scene.camera;
            let (f, right, up) = input
                .view
                .basis
                .map_or_else(|| c.basis(), |b| (b[0], b[1], b[2]));
            let radius = c.aperture_radius(input.scene.meters_per_unit);
            let focus = c.focus.map_or((c.target - c.position).length(), |p| {
                (p - c.position).dot(f)
            });
            let tan = input
                .view
                .orthographic_span
                .unwrap_or_else(|| (c.fov.to_radians() * 0.5).tan());
            let center = (extracted.bounds.0 + extracted.bounds.1) * 0.5;
            let bound = (extracted.bounds.1 - extracted.bounds.0)
                .length()
                .max(0.001)
                * 0.5;
            let ground = input.view.ground.unwrap_or(input.scene.environment.1);
            r.set_view(
                [
                    c.position.to4(radius),
                    f.to4(focus),
                    right.to4(tan * width as f32 / height as f32),
                    up.to4(tan),
                ],
                ground.to4(input.view.floor_y),
                input.view.shadows,
                center.to4(bound),
            );
            r.set_ambient(input.view.ambient.to_array());
            r.set_iris(input.view.iris);
            r.set_projection(input.view.orthographic_span.is_some());
            if let Some([near, far]) = input.view.clip_planes {
                r.set_clip_planes(Some(near), Some(far));
            }
        }
        Ok(PreparedScene {
            quality, denoise, live_guided, reset, width, height, outputs, key, resource_start,
            orthographic: input.view.orthographic_span.is_some(),
            adaptive: input.scene.world.adaptive, layers: extracted.layers,
            failures,
        })
    }

    fn render_oriented(
        &mut self,
        source: &dyn SceneSource,
        request: SceneRequest,
        tier: Tier,
        flip_y: bool,
    ) -> Result<RenderedScene, String> {
        let frame_start = std::time::Instant::now();
        let mut timings = FrameTimings::default();
        if request.width == 0 || request.height == 0 {return Err("Studio output is empty".into())}
        if tier == Tier::Fast {
            let input = source.scene(&request)?;
            return self.fast.as_mut().ok_or("Fast compositor adapter is not attached")?.render(&input, &request);
        }
        let PreparedScene { quality, denoise, live_guided, orthographic, reset, width, height, outputs, key, resource_start, adaptive, failures, .. } =
            self.prepare_scene(source, request.clone(), tier)?;
        let r = self.renderer.as_mut().unwrap();
        r.live = tier == Tier::QualityLive;
        r.sampling_policy=quality.map_or(Default::default(),|q|q.sampling);
        let (mut samples, mut bounces, salt) = match tier {
            Tier::QualityLive => (if orthographic {4} else {1}, 3, 0),
            Tier::QualityRefine { samples } => (samples, 6, 0),
            Tier::Final { samples, salt, .. } => (samples, 8, salt),
            Tier::Fast => unreachable!(),
        };
        if let Some(q) = quality {
            samples = samples.min(q.max_samples);
            bounces = q.max_bounces;
            r.caustics_enabled = q.caustics;
            r.set_precomputed_caustics(q.caustics);
            r.set_adaptive_sampling_with_floor(
                crate::environment::AdaptiveSampling {
                    enabled: q.noise_threshold > 0.,
                    min_samples: q.min_samples.min(samples),
                    relative_error: q.noise_threshold,
                    absolute_error: 0.001,
                },
                true,
            );
        } else {
            r.caustics_enabled = true;
            r.set_precomputed_caustics(false);
            r.set_adaptive_sampling(adaptive);
        }
        r.set_time_seed(salt);
        // Stills/deliveries stop at the error target or the sample ceiling.
        // The saved time limit remains a preview hint, never an export deadline.
        timings.prepare_ms = frame_start.elapsed().as_secs_f64() * 1000.;
        #[cfg(all(feature = "metalfx", target_os = "macos"))]
        let mut metalfx = r.live && r.metalfx_enabled && !live_guided;
        #[cfg(not(all(feature = "metalfx", target_os = "macos")))]
        let metalfx = false;
        use crate::status::{RefineProgress, RefineStage};
        let refining = matches!(tier, Tier::QualityRefine { .. });
        let cancelled = std::cell::Cell::new(false);
        let completed = std::cell::Cell::new(0);
        let progress = |done, target| {
            completed.set(done);
            let keep = source.refine_progress(&request, RefineProgress {
                samples_done: done, samples_target: target, stage: RefineStage::Sampling,
            });
            if !keep { cancelled.set(true); }
            keep
        };
        if refining { r.trace_with_progress(samples, bounces, reset, 64, &progress); }
        else if r.live && metalfx { r.queue_live_trace(bounces,6)?; }
        else { r.trace(samples, bounces, reset, if r.live { 6 } else { 64 }); }
        if cancelled.get() { return Err("Studio refinement superseded".into()); }
        if refining && !source.refine_progress(&request, RefineProgress {
            samples_done: completed.get(), samples_target: samples, stage: RefineStage::Denoising,
        }) { return Err("Studio refinement superseded".into()); }
        r.set_output_mask(None, false);
        let passes = if r.live || denoise == Denoise::Guided {
            4
        } else {
            0
        };
        #[cfg(feature = "oidn")]
        if denoise == Denoise::Oidn || matches!(tier, Tier::QualityRefine { .. }) {
            let half_guides = quality.is_some_and(|q| q.half_resolution_guides)
                || matches!(tier, Tier::QualityRefine { .. });
            prepare_oidn(r, width, height, half_guides, &mut timings)?;
        } else if !metalfx {
            r.prepare_frame(passes, None)?;
        }
        #[cfg(not(feature = "oidn"))]
        if !metalfx {
            r.prepare_frame(passes, None)?;
        }
        let mut layers = Vec::new();
        for (index, (layer, mask, include_void)) in outputs.iter().enumerate() {
            #[cfg(all(feature = "metalfx", target_os = "macos"))]
            if metalfx {
                r.set_output_mask(Some(*mask), *include_void);
                match r.finish_metalfx_texture(request.width, request.height, index > 0, flip_y) {
                    Ok((source, _)) => {
                        let texture = r.device().create_texture(&wgpu::TextureDescriptor {
                            label: Some("Studio native MetalFX layer"),
                            size: source.size(),
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: source.format(),
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        });
                        let mut encoder = r.device().create_command_encoder(&Default::default());
                        encoder.copy_texture_to_texture(
                            source.as_image_copy(),
                            texture.as_image_copy(),
                            source.size(),
                        );
                        r.queue().submit([encoder.finish()]);
                        layers.push(LayerTexture {
                            layer: layer.clone(),
                            texture,
                        });
                        continue;
                    }
                    Err(error) => {
                        eprintln!(
                            "Studio MetalFX unavailable; using portable temporal denoising: {error}"
                        );
                        metalfx = false;
                        r.set_output_mask(None, false);
                        r.prepare_frame(4, None)?;
                    }
                }
            }
            let texture = r.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("studio compositor layer"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let target = PresentTarget {
                view: &view,
                format: wgpu::TextureFormat::Rgba8Unorm,
                layer: Some(*mask),
                include_void: *include_void,
                flip_y,
            };
            r.present_frame(target);
            layers.push(LayerTexture {
                layer: layer.clone(),
                texture,
            });
        }
        if refining && !source.refine_progress(&request, RefineProgress {
            samples_done: completed.get(), samples_target: samples, stage: RefineStage::Complete,
        }) { return Err("Studio refinement superseded".into()); }
        timings.pipeline = r.timings;
        timings.total_ms = frame_start.elapsed().as_secs_f64() * 1000.;
        self.resource_start = resource_start;
        self.previous = Some((request.clone(), key));
        self.previous_tier = Some(tier);
        Ok(RenderedScene {
            request,
            layers,
            samples: r.samples,
            adapter: r.adapter.clone(),
            hardware_rt: r.uses_hardware_rt(),
            timings,
            failures,
        })
    }

    pub fn geometry_uploads(&self) -> u64 {
        self.geometry_uploads
    }
    pub fn scene_updates(&self) -> u64 {
        self.scene_updates
    }
    pub fn texture_uploads(&self) -> u64 {
        self.texture_uploads
    }
    pub fn blas_builds(&self) -> u64 {
        self.extractor.builds
    }
    pub fn device(&self) -> Option<&wgpu::Device> {
        self.renderer.as_ref().map(Renderer::device)
    }
    pub fn queue(&self) -> Option<&wgpu::Queue> {
        self.renderer.as_ref().map(Renderer::queue)
    }
}

fn texture_key(maps: Option<&crate::extract::TextureMaps>) -> Vec<u8> {
    let Some(maps) = maps else { return vec![] };
    let mut key = Vec::new();
    key.extend_from_slice(&maps.width.to_le_bytes());
    key.extend_from_slice(&maps.height.to_le_bytes());
    key.extend_from_slice(&(maps.layers.len() as u64).to_le_bytes());
    for layer in &maps.layers { key.extend_from_slice(layer); }
    key
}

fn content(input: &SceneInput, e: &Extracted) -> (Vec<u8>, usize) {
    let p = &e.prepared;
    let mut key = format!(
        "{:?}{:?}{:?}{:?}{:?}{}{}:{}",
        e.layers,
        input.scene.camera,
        input.scene.backdrop,
        input.scene.environment,
        e.bounds,
        input.scene.exposure,
        input.scene.shadow_catcher,
        input.scene.meters_per_unit
    )
    .into_bytes();
    key.extend_from_slice(format!("{:?}", input.view).as_bytes());
    let resource_start = key.len();
    for data in [
        bytemuck::cast_slice(&p.positions),
        bytemuck::cast_slice(&p.tris),
        bytemuck::cast_slice(&p.nodes),
        bytemuck::cast_slice(&p.materials),
        bytemuck::cast_slice(&p.sdata),
    ] {
        key.extend_from_slice(&(data.len() as u64).to_le_bytes());
        key.extend_from_slice(data);
    }
    if let Some(maps) = &input.maps {
        key.extend_from_slice(&maps.width.to_le_bytes());
        key.extend_from_slice(&maps.height.to_le_bytes());
        for map in &maps.layers {
            key.extend_from_slice(map);
        }
    }
    (key, resource_start)
}

fn validate(input: &SceneInput, e: &Extracted) -> Result<(), String> {
    let view = input.view;
    if view.iris.iter().any(|v| !v.is_finite())
        || !(0. ..=16.).contains(&view.iris[0])
        || !(0. ..=1.).contains(&view.iris[2])
        || !(0.1..=10.).contains(&view.iris[3])
        || view
            .orthographic_span
            .is_some_and(|v| !v.is_finite() || v <= 0.)
        || view.clip_planes.is_some_and(|[near, far]| {
            !near.is_finite() || !far.is_finite() || near < 0. || far <= near
        })
        || !input.scene.meters_per_unit.is_finite()
        || input.scene.meters_per_unit <= 0.
    {
        return Err("Invalid Studio projection, iris, clip planes or scene scale".into());
    }
    let c = input.scene.camera;
    if ![
        c.position.x,
        c.position.y,
        c.position.z,
        c.target.x,
        c.target.y,
        c.target.z,
        c.fov,
        c.f_stop,
        c.focal_mm,
        input.scene.exposure,
    ]
    .iter()
    .all(|v| v.is_finite())
        || c.fov <= 0.
        || c.fov >= 180.
        || c.position == c.target
        || c.f_stop < 0.
        || c.focal_mm <= 0.
        || input.scene.exposure < 0.
    {
        return Err("invalid Studio camera/exposure".into());
    }
    for v in [
        input.scene.backdrop,
        input.scene.environment.0,
        input.scene.environment.1,
        input.view.ambient,
    ]
    .into_iter()
    .chain(c.focus)
    {
        if ![v.x, v.y, v.z].iter().all(|x| x.is_finite()) {
            return Err("non-finite Studio environment or focus".into());
        }
    }
    if input.view.basis.is_none()
        && (c.target - c.position).cross(crate::math::Vec3::Y).length() == 0.
    {
        return Err("vertical Studio cameras require their authored basis".into());
    }
    if let Some(maps) = &input.maps {
        let bytes = u64::from(maps.width)
            .checked_mul(u64::from(maps.height))
            .and_then(|n| n.checked_mul(4))
            .ok_or("Studio map size overflow")?;
        if maps.width == 0
            || maps.height == 0
            || maps.layers.is_empty()
            || maps.layers.len() > 256
            || maps.layers.iter().any(|m| m.len() as u64 != bytes)
        {
            return Err("invalid Studio texture array".into());
        }
    }
    input.scene.world.validate()?;
    for m in &e.prepared.materials {
        m.validate()?;
        if bytemuck::cast_slice::<_, f32>(std::slice::from_ref(m))
            .iter()
            .any(|f| !f.is_finite())
        {
            return Err("non-finite Studio material".into());
        }
        for index in m.maps.iter().chain(m.maps2[..3].iter()).copied().chain(
            m.physical_maps
                .iter()
                .filter(|_| m.imported[5][3] > 0.5)
                .map(|map| map.info[0]),
        ) {
            if index != -1.
                && (index < 0.
                    || index.fract() != 0.
                    || input
                        .maps
                        .as_ref()
                        .is_none_or(|m| index as usize >= m.layers.len()))
            {
                return Err("unresolved Studio material map (including live screens)".into());
            }
        }
    }
    if e.prepared
        .lights
        .iter()
        .flat_map(|l| bytemuck::cast_slice::<_, f32>(std::slice::from_ref(l)))
        .any(|v| !v.is_finite())
    {
        return Err("non-finite Studio light".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        extract::View,
        math::{Vec3, v3},
        scene::Material,
    };
    #[test]
    fn invalid_camera_focus_and_environment_are_explicit_failures() {
        let mut input = SceneInput {
            scene: crate::swatch::scene(Material::dielectric(Vec3::splat(0.5), 0.5)),
            instances: vec![],
            materials: None,
            maps: None,
            view: View::default(),
        };
        let extracted = Extractor::default().extract(&input).unwrap();
        assert!(validate(&input, &extracted).is_ok());
        input.scene.camera.fov = 0.;
        assert!(validate(&input, &extracted).is_err());
        input.scene.camera.fov = 28.;
        input.scene.camera.focus = Some(v3(f32::NAN, 0., 0.));
        assert!(validate(&input, &extracted).is_err());
        input.scene.camera.focus = None;
        input.scene.environment.0.x = f32::NAN;
        assert!(validate(&input, &extracted).is_err());
        input.scene.environment.0.x = 0.;
        input.scene.camera.position = v3(0., 5., 0.);
        assert!(validate(&input, &extracted).is_err());
        input.view.basis = Some([v3(0., -1., 0.), v3(1., 0., 0.), v3(0., 0., 1.)]);
        assert!(validate(&input, &extracted).is_ok());
    }
    #[test]
    fn secondary_and_physical_maps_cannot_reference_missing_images() {
        let input = SceneInput {
            scene: crate::swatch::scene(Material::dielectric(Vec3::splat(0.5), 0.5)),
            instances: vec![],
            materials: None,
            maps: None,
            view: View::default(),
        };
        let mut extracted = Extractor::default().extract(&input).unwrap();
        extracted.prepared.materials[0].maps2[2] = 0.;
        assert!(validate(&input, &extracted).is_err());
        extracted.prepared.materials[0].maps2[2] = -1.;
        let m = &mut extracted.prepared.materials[0];
        m.imported[5][3] = 1.;
        m.imported[3][2] = 1.38;
        for map in &mut m.physical_maps {
            map.info[0] = -1.;
        }
        assert!(validate(&input, &extracted).is_ok());
        extracted.prepared.materials[0].physical_maps[17].info[0] = 0.;
        assert!(validate(&input, &extracted).is_err());
    }
}

#[cfg(feature = "oidn")]
fn prepare_oidn(
    renderer: &mut Renderer,
    width: u32,
    height: u32,
    half_guides: bool,
    timings: &mut FrameTimings,
) -> Result<(), String> {
    // Handles and RefCells stay on their worker. One cache follows that
    // thread's lifetime, including immutable document/source replacement.
    thread_local! { static DENOISER: std::cell::RefCell<Option<crate::oidn::Denoiser>> = const { std::cell::RefCell::new(None) }; }
    DENOISER.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| "Reentrant Studio denoise")?;
        timings.oidn_reused = slot.is_some();
        if slot.is_none() {
            let start = std::time::Instant::now();
            *slot = Some(crate::oidn::Denoiser::new()?);
            timings.oidn_init_ms = start.elapsed().as_secs_f64() * 1000.;
        }
        let denoiser = slot.as_ref().unwrap();
        denoiser.set_half_resolution_guides(half_guides);
        denoiser.set_quality(if half_guides {
            crate::oidn::Quality::Balanced
        } else {
            crate::oidn::Quality::High
        });
        let catcher=renderer.has_shadow_catcher();
        renderer.prepare_frame(
            0,
            Some(&|color, shade, albedo, normal| {
                let mut layers=vec![color];if catcher{layers.push(shade);}
                denoiser.run_layers_interruptible(
                    &mut layers,
                    albedo,
                    normal,
                    4,
                    width as usize,
                    height as usize,
                    None,
                )
            }),
        )?;
        timings.oidn = denoiser.timings();
        Ok(())
    })
}
