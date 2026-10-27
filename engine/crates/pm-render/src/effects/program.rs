//! A translated GLSL program as wgpu pipelines, and the compositor's uniform
//! binding rules (`legacy/gl/compositor.ts` `setU`, `setI`, `setParam`).
//!
//! GL semantics reproduced, as spike S3 measured them against WebGL:
//!
//! - `glUniform*` on a uniform of another type is `INVALID_OPERATION` and
//!   changes nothing: a single float never sets a `vec2`, `uniform1f` never
//!   sets an `int` (`u_pass` declared `float` stays 0), `bool` takes 1f or 1i.
//! - Uniforms are program-wide: a name both stages declare (`u_res`) is set
//!   in both default blocks.
//! - A sampler uniform samples unit 0 unless `setI` moved it; an empty unit
//!   samples (0, 0, 0, 1); element 0 of a sampler array follows the array's
//!   name.
//! - Named parameters (`u_<key>`) bind when the linked program has them
//!   active; otherwise the positional alias (`u_p<i>`, `u_c<i>`).
//!
//! Legacy GLSL draws in GL row order (see `pm_glsl`'s `adjust_coordinate_space`
//! and S3.md "Origin"), like every pm-render target.

use std::collections::HashMap;

use pm_glsl::reflect::{Block, Scalar, UniformType};
use pm_glsl::{FRAGMENT_SET, Failure, FailureStage, Program, SAMPLER_BINDING_OFFSET, VERTEX_SET};

/// A `glUniform*` call as the compositor issues it.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// `uniform1f`..`uniform4f` (1 to 4 values).
    F(Vec<f32>),
    /// `uniform1i`.
    I(i32),
    /// `uniformMatrix3fv`, column-major.
    M3([f32; 9]),
}

/// Writes one call into a default block with GL's type rules. Returns
/// whether anything was written.
pub fn apply(block: &Block, bytes: &mut [u8], name: &str, call: &Call) -> bool {
    let Some(member) = block.members.iter().find(|m| m.name == name) else {
        return false;
    };
    let at = member.offset as usize;
    let mut put = |index: usize, word: [u8; 4]| {
        if let Some(slot) = bytes.get_mut(at + index * 4..at + index * 4 + 4) {
            slot.copy_from_slice(&word);
        }
    };
    let b = |v: f32| u32::from(v != 0.0).to_le_bytes();
    match (call, &member.ty) {
        (Call::F(v), UniformType::Scalar(Scalar::Float)) if v.len() == 1 => {
            put(0, v[0].to_le_bytes())
        }
        (Call::F(v), UniformType::Scalar(Scalar::Bool)) if v.len() == 1 => put(0, b(v[0])),
        (Call::I(i), UniformType::Scalar(Scalar::Bool)) => put(0, u32::from(*i != 0).to_le_bytes()),
        (Call::I(i), UniformType::Scalar(Scalar::Int)) => put(0, i.to_le_bytes()),
        (Call::F(v), UniformType::Vector(scalar, n)) if v.len() == *n as usize && v.len() > 1 => {
            match scalar {
                Scalar::Float => (0..v.len()).for_each(|i| put(i, v[i].to_le_bytes())),
                Scalar::Bool => (0..v.len()).for_each(|i| put(i, b(v[i]))),
                _ => return false,
            }
        }
        (
            Call::M3(m),
            UniformType::Matrix {
                columns: 3,
                rows: 3,
                stride,
            },
        ) => {
            for column in 0..3 {
                for row in 0..3 {
                    let offset = at + column * *stride as usize + row * 4;
                    if let Some(slot) = bytes.get_mut(offset..offset + 4) {
                        slot.copy_from_slice(&m[column * 3 + row].to_le_bytes());
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

/// Whether `name` is an active member of `block` (`getUniformLocation`
/// returns a location).
pub fn active(block: Option<&Block>, name: &str) -> bool {
    block.is_some_and(|b| b.members.iter().any(|m| m.name == name && m.active))
}

/// One compiled program, with pipelines per target format.
pub struct GpuProgram {
    vertex: crate::glsl::Stage,
    fragment: crate::glsl::Stage,
    /// The fragment stage for RGBA16F targets ([`crate::glsl::half_store`]),
    /// built on first use.
    fragment_half: Option<crate::glsl::Stage>,
    layout: wgpu::PipelineLayout,
    pub fragment_layout: wgpu::BindGroupLayout,
    pub vertex_layout: wgpu::BindGroupLayout,
    pub fragment_block: Option<Block>,
    pub vertex_block: Option<Block>,
    /// Sampler name and image binding, in binding order.
    pub samplers: Vec<(String, u32)>,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

/// Validation, internal (translation and Metal), and out-of-memory scopes,
/// so one bad program fails its definition instead of the frame.
struct Scopes([wgpu::ErrorScopeGuard; 3]);

fn push_scopes(device: &wgpu::Device) -> Scopes {
    Scopes([
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory),
        device.push_error_scope(wgpu::ErrorFilter::Internal),
        device.push_error_scope(wgpu::ErrorFilter::Validation),
    ])
}

fn pop_error(scopes: Scopes) -> Option<String> {
    let [oom, internal, validation] = scopes.0;
    let mut errors = Vec::new();
    for guard in [validation, internal, oom] {
        if let Some(error) = pollster::block_on(guard.pop()) {
            errors.push(error.to_string());
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

fn layout_entries(
    block: Option<&Block>,
    samplers: &[(String, u32)],
    visibility: wgpu::ShaderStages,
    filterable: bool,
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = Vec::new();
    if let Some(block) = block {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: block.binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
    }
    for (_, binding) in samplers {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: *binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: binding + SAMPLER_BINDING_OFFSET,
            visibility,
            ty: wgpu::BindingType::Sampler(if filterable {
                wgpu::SamplerBindingType::Filtering
            } else {
                wgpu::SamplerBindingType::NonFiltering
            }),
            count: None,
        });
    }
    entries.sort_by_key(|e| e.binding);
    entries
}

impl GpuProgram {
    /// Creates the shader modules and layouts. Errors are engine-only
    /// failures (the program passed WebGL's rules): `FailureStage::Metal`
    /// is not a pipeline stage of `pm_glsl`, so they are reported as
    /// `NagaValidate` with wgpu's message.
    pub fn new(
        device: &wgpu::Device,
        program: Program,
        label: &str,
    ) -> Result<GpuProgram, Failure> {
        let fragment_block = program.fragment.reflection.block.clone();
        let vertex_block = program.vertex.reflection.block.clone();
        let mut samplers: Vec<(String, u32)> = program
            .fragment
            .reflection
            .samplers
            .iter()
            .map(|s| (s.name.clone(), s.binding))
            .collect();
        samplers.sort_by_key(|s| s.1);
        for split in &program.fragment.splits {
            let known = samplers
                .iter()
                .any(|(name, binding)| *name == split.name && *binding == split.image_binding);
            if !known
                || split.set != FRAGMENT_SET
                || split.sampler_binding != split.image_binding + SAMPLER_BINDING_OFFSET
            {
                return Err(Failure::new(
                    FailureStage::Rewrite,
                    format!("sampler split {split:?} does not match the bind group layout"),
                ));
            }
        }
        if program.fragment.splits.len() != samplers.len() {
            return Err(Failure::new(
                FailureStage::Rewrite,
                format!(
                    "{} sampler2D uniforms but {} splits",
                    samplers.len(),
                    program.fragment.splits.len()
                ),
            ));
        }
        // Each stage sees its own group: the fragment's block and samplers
        // (set 0), the vertex stage's block (set 1). Metal compiles the MSL
        // with the built-in calls ANGLE makes (`crate::glsl`).
        let mut fragment_entries = Vec::new();
        if let Some(b) = &fragment_block {
            fragment_entries.push(crate::glsl::Entry::Buffer {
                group: FRAGMENT_SET,
                binding: b.binding,
            });
        }
        for (_, binding) in &samplers {
            fragment_entries.push(crate::glsl::Entry::Texture {
                group: FRAGMENT_SET,
                binding: *binding,
            });
            fragment_entries.push(crate::glsl::Entry::Sampler {
                group: FRAGMENT_SET,
                binding: binding + SAMPLER_BINDING_OFFSET,
            });
        }
        let vertex_entries: Vec<_> = vertex_block
            .iter()
            .map(|b| crate::glsl::Entry::Buffer {
                group: VERTEX_SET,
                binding: b.binding,
            })
            .collect();
        let scopes = push_scopes(device);
        let stages = crate::glsl::stage_module(
            device,
            "pm legacy vertex",
            program.vertex.module,
            pm_gpu::glsl::naga::ShaderStage::Vertex,
            &vertex_entries,
        )
        .and_then(|vertex| {
            crate::glsl::stage_module(
                device,
                label,
                program.fragment.module,
                pm_gpu::glsl::naga::ShaderStage::Fragment,
                &fragment_entries,
            )
            .map(|fragment| (vertex, fragment))
        });
        let (vertex, fragment) = match stages {
            Ok(s) => s,
            Err(e) => {
                let scoped = pop_error(scopes);
                return Err(Failure::new(
                    FailureStage::NagaValidate,
                    scoped.map_or(e.clone(), |s| format!("{e}; {s}")),
                ));
            }
        };
        // Every legacy texture is filterable (`rgba16float`, or
        // `rgba32float` on devices with `FLOAT32_FILTERABLE`; pm-render
        // refuses 32-bit precision elsewhere).
        let fragment_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm legacy fragment"),
            entries: &layout_entries(
                fragment_block.as_ref(),
                &samplers,
                wgpu::ShaderStages::FRAGMENT,
                true,
            ),
        });
        let vertex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm legacy vertex"),
            entries: &layout_entries(vertex_block.as_ref(), &[], wgpu::ShaderStages::VERTEX, true),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&fragment_layout), Some(&vertex_layout)],
            immediate_size: 0,
        });
        if let Some(error) = pop_error(scopes) {
            return Err(Failure::new(FailureStage::NagaValidate, error));
        }
        Ok(GpuProgram {
            vertex,
            fragment,
            fragment_half: None,
            layout,
            fragment_layout,
            vertex_layout,
            fragment_block,
            vertex_block,
            samplers,
            pipelines: HashMap::new(),
        })
    }

    /// The pipeline for a target format, created on first use (this is
    /// where the Metal compiler runs).
    pub fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Result<&wgpu::RenderPipeline, Failure> {
        pm_perf::cache(
            pm_perf::Counter::PipelineCacheHits,
            pm_perf::Counter::PipelineCacheMisses,
            self.pipelines.contains_key(&format),
        );
        if !self.pipelines.contains_key(&format) {
            // RGBA16F stores the nearest half of the program's output.
            if format == wgpu::TextureFormat::Rgba16Float && self.fragment_half.is_none() {
                let half = self.fragment.msl.as_deref().and_then(|msl| {
                    crate::glsl::half_store(msl, &self.fragment.entry_point)
                });
                if let Some(source) = half {
                    let stage = crate::glsl::msl_stage(
                        device,
                        "pm legacy fragment (nearest half)",
                        source,
                        &self.fragment.entry_point,
                    )
                    .map_err(|e| Failure::new(FailureStage::NagaValidate, e))?;
                    self.fragment_half = Some(stage);
                }
            }
            let scopes = push_scopes(device);
            let fragment = match &self.fragment_half {
                Some(half) if format == wgpu::TextureFormat::Rgba16Float => half,
                _ => &self.fragment,
            };
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pm legacy glsl"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.vertex.module,
                    entry_point: Some(&self.vertex.entry_point),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                // The vertex output is flipped to keep GL row order, which
                // reverses the winding: clockwise is GL's counter-clockwise,
                // so `gl_FrontFacing` matches WebGL (false for the quad).
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment.module,
                    entry_point: Some(&fragment.entry_point),
                    compilation_options: Default::default(),
                    // Blending off (`gl.disable(gl.BLEND)` around every
                    // effect and transition draw).
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            if let Some(error) = pop_error(scopes) {
                return Err(Failure::new(FailureStage::NagaValidate, error));
            }
            self.pipelines.insert(format, pipeline);
        }
        Ok(&self.pipelines[&format])
    }

    /// The uniform block bytes of one draw, from the compositor's calls in
    /// issue order (a later call overwrites an earlier one, as in GL).
    pub fn block_bytes(block: &Block, calls: &[(String, Call)]) -> Vec<u8> {
        let mut bytes = vec![0u8; block.size.max(16) as usize];
        for (name, call) in calls {
            apply(block, &mut bytes, name, call);
        }
        bytes
    }

    /// Which unit each sampler reads: unit 0 unless a `uniform1i` call
    /// names it (or, for element 0 of an array, the array).
    pub fn sampler_units(&self, calls: &[(String, Call)]) -> Vec<u32> {
        self.samplers
            .iter()
            .map(|(name, _)| {
                let base = name.strip_suffix("[0]");
                calls
                    .iter()
                    .rev()
                    .find_map(|(n, c)| match c {
                        Call::I(unit) if n == name || Some(n.as_str()) == base => {
                            Some((*unit).max(0) as u32)
                        }
                        _ => None,
                    })
                    .unwrap_or(0)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_glsl::reflect::Member;

    fn block() -> Block {
        let m = |name: &str, offset, ty| Member {
            name: name.into(),
            offset,
            ty,
            active: true,
        };
        Block {
            set: 0,
            binding: 0,
            size: 96,
            members: vec![
                m("f", 0, UniformType::Scalar(Scalar::Float)),
                m("i", 4, UniformType::Scalar(Scalar::Int)),
                m("b", 8, UniformType::Scalar(Scalar::Bool)),
                m("v2", 16, UniformType::Vector(Scalar::Float, 2)),
                m("v3", 32, UniformType::Vector(Scalar::Float, 3)),
                m(
                    "m",
                    48,
                    UniformType::Matrix {
                        columns: 3,
                        rows: 3,
                        stride: 16,
                    },
                ),
            ],
        }
    }

    fn word(bytes: &[u8], at: usize) -> [u8; 4] {
        bytes[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn follows_gl_uniform_type_rules() {
        let block = block();
        let mut bytes = vec![0u8; 96];
        assert!(apply(&block, &mut bytes, "f", &Call::F(vec![2.5])));
        assert!(
            !apply(&block, &mut bytes, "i", &Call::F(vec![3.0])),
            "uniform1f on int"
        );
        assert!(
            apply(&block, &mut bytes, "b", &Call::I(1)),
            "bool takes uniform1i"
        );
        assert!(
            apply(&block, &mut bytes, "b", &Call::F(vec![1.0])),
            "bool takes uniform1f"
        );
        assert!(
            !apply(&block, &mut bytes, "v2", &Call::F(vec![1.0])),
            "one float leaves a vec2 at 0"
        );
        assert!(!apply(
            &block,
            &mut bytes,
            "v2",
            &Call::F(vec![1.0, 2.0, 3.0])
        ));
        assert!(apply(
            &block,
            &mut bytes,
            "v3",
            &Call::F(vec![1.0, 2.0, 3.0])
        ));
        assert!(apply(
            &block,
            &mut bytes,
            "m",
            &Call::M3([1., 2., 3., 4., 5., 6., 7., 8., 9.])
        ));
        assert!(!apply(&block, &mut bytes, "nope", &Call::F(vec![1.0])));
        assert_eq!(f32::from_le_bytes(word(&bytes, 0)), 2.5);
        assert_eq!(word(&bytes, 4), [0; 4]);
        assert_eq!(u32::from_le_bytes(word(&bytes, 8)), 1);
        assert_eq!(&bytes[16..24], &[0; 8]);
        assert_eq!(f32::from_le_bytes(word(&bytes, 40)), 3.0);
        // Column 1, row 2 of the matrix.
        assert_eq!(f32::from_le_bytes(word(&bytes, 48 + 16 + 8)), 6.0);
        assert_eq!(word(&bytes, 48 + 12), [0; 4], "padding stays");
    }
}
