//! Worker-owned rendering facade. The core lane supplies evaluated scenes by
//! stable ID; a compositor consumes per-layer textures from the shared trace.
use crate::extract::{Extracted, Extractor, SceneInput, SceneRequest, SceneSource};
use crate::gpu::{PresentTarget, Renderer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denoise {
    None,
    Guided,
    Oidn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Real-time rasterization is supplied by the compositor, not emulated by
    /// reducing path-tracer samples.
    Fast,
    QualityLive,
    QualityRefine {
        samples: u32,
    },
    Final {
        samples: u32,
        salt: u32,
        denoise: Denoise,
    },
}

pub struct LayerTexture {
    pub layer: String,
    /// Premultiplied display sRGB, in top-down row order. This is the source
    /// Studio SDR contract, before the compositor's working-space conversion.
    pub texture: wgpu::Texture,
}

pub struct RenderedScene {
    pub request: SceneRequest,
    pub layers: Vec<LayerTexture>,
    pub samples: u32,
    pub adapter: String,
    pub hardware_rt: bool,
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
}

impl StudioService {
    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        Ok(Self {
            renderer: Some(Renderer::with_device(
                device,
                queue,
                "shared engine device".into(),
                false,
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
        if request.width == 0 || request.height == 0 {
            return Err("Studio output is empty".into());
        }
        // Refuse unsupported optional requests before extraction or GPU work.
        #[cfg(not(feature = "oidn"))]
        if matches!(
            tier,
            Tier::Final {
                denoise: Denoise::Oidn,
                ..
            }
        ) {
            return Err("Studio built without optional OIDN support".into());
        }
        if matches!(
            tier,
            Tier::Final { samples: 0, .. } | Tier::QualityRefine { samples: 0 }
        ) {
            return Err("Studio sample count must be positive".into());
        }
        let input = source.scene(&request)?;
        if tier == Tier::Fast {
            return self
                .fast
                .as_mut()
                .ok_or("Fast compositor adapter is not attached")?
                .render(&input, &request);
        }
        let extracted = self.extractor.extract(&input)?;
        validate(&input, &extracted)?;
        // Compare exact bytes, including resources, instead of trusting a revision
        // or a lossy/partial scene key. No stale frame survives a material edit.
        let key = content(&input, &extracted);
        let reset = self
            .previous
            .as_ref()
            .is_none_or(|(r, k)| r != &request || *k != key)
            || !matches!(self.previous_tier, Some(Tier::QualityRefine { .. }))
            || !matches!(tier, Tier::QualityRefine { .. });
        let (width, height) = match tier {
            Tier::QualityLive => (request.width.div_ceil(2), request.height.div_ceil(2)),
            _ => (request.width, request.height),
        };
        if self
            .renderer
            .as_ref()
            .is_none_or(|r| r.size() != (width, height))
        {
            if let Some(old) = self.renderer.take() {
                self.renderer = Some(Renderer::with_device(
                    old.device().clone(),
                    old.queue().clone(),
                    old.adapter.clone(),
                    false,
                    width,
                    height,
                )?);
            } else {
                self.renderer = Some(Renderer::new(width, height)?);
            }
        }
        let r = self.renderer.as_mut().unwrap();
        r.check_prepared(&extracted.prepared)?;
        if reset {
            r.set_extracted(&input.scene, &extracted);
            if let Some(maps) = &input.maps {
                r.set_maps(maps)?;
            }
            // No maps must retire a previous live-screen array too.
            else {
                r.set_screen(&[0; 4], 1, 1);
            }
        }
        r.set_view(input.view)?;
        r.live = tier == Tier::QualityLive;
        let (samples, bounces, salt) = match tier {
            Tier::QualityLive => (1, 2, 0),
            Tier::QualityRefine { samples } => (samples, 6, 0),
            Tier::Final { samples, salt, .. } => (samples, 8, salt),
            Tier::Fast => unreachable!(),
        };
        // Final always starts afresh, including repeated export/capture requests.
        r.trace_seeded(
            samples,
            bounces,
            reset || matches!(tier, Tier::Final { .. }),
            if r.live { 6 } else { 64 },
            salt,
        );
        let mut layers = Vec::new();
        for (index, layer) in extracted.layers.iter().enumerate() {
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
                layer: Some(index as u32),
                include_void: false,
                flip_y: false,
            };
            match tier {
                Tier::Final {
                    denoise: Denoise::Guided,
                    ..
                } => {
                    r.present_guided(target);
                }
                #[cfg(feature = "oidn")]
                Tier::Final {
                    denoise: Denoise::Oidn,
                    ..
                } => {
                    let denoiser = crate::oidn::Denoiser::new()?;
                    r.try_present_with(
                        &|c, a, n| denoiser.run(c, a, n, 4, width as usize, height as usize),
                        target,
                    )?;
                }
                _ => {
                    r.present_to(if r.live { 4 } else { 0 }, target);
                }
            }
            layers.push(LayerTexture {
                layer: layer.clone(),
                texture,
            });
        }
        self.previous = Some((request.clone(), key));
        self.previous_tier = Some(tier);
        Ok(RenderedScene {
            request,
            layers,
            samples: r.samples,
            adapter: r.adapter.clone(),
            hardware_rt: r.uses_hardware_rt(),
        })
    }

    pub fn geometry_uploads(&self) -> u64 {
        self.renderer.as_ref().map_or(0, |r| r.geometry_uploads)
    }
    pub fn scene_updates(&self) -> u64 {
        self.renderer.as_ref().map_or(0, |r| r.scene_updates)
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

fn content(input: &SceneInput, e: &Extracted) -> Vec<u8> {
    let p = &e.prepared;
    let mut key = format!(
        "{:?}{:?}{:?}{:?}{:?}{}{}",
        e.layers,
        input.scene.camera,
        input.scene.backdrop,
        input.scene.environment,
        e.bounds,
        input.scene.exposure,
        input.scene.shadow_catcher
    )
    .into_bytes();
    key.extend_from_slice(format!("{:?}", input.view).as_bytes());
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
    key
}

fn validate(input: &SceneInput, e: &Extracted) -> Result<(), String> {
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
    for m in &e.prepared.materials {
        if bytemuck::cast_slice::<_, f32>(std::slice::from_ref(m))
            .iter()
            .any(|f| !f.is_finite())
        {
            return Err("non-finite Studio material".into());
        }
        for &index in &m.maps {
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
}
