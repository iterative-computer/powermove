//! Shader modules compiled with an explicit Metal math mode (plan section 8,
//! "Device": Metal math mode for color-critical shaders; amendment 11: the
//! 3D programs compile with safe math).
//!
//! wgpu 30 compiles every naga module with Metal's default, fast math, and
//! has no API for `MTLCompileOptions.mathMode`. Instead of patching
//! wgpu-hal, this module writes the MSL itself with naga's MSL back end and
//! puts MSL 3.2's `#pragma METAL fp math_mode(...)` at the top of the source,
//! then hands it to wgpu as a passthrough module (`Features::PASSTHROUGH_SHADERS`).
//! The pragma governs every function in the translation unit; Metal rejects
//! an unknown mode, and its AIR output loses the `fast` flags in safe mode
//! (checked when this module was written: `fdiv fast` becomes `fdiv`).
//!
//! The MSL follows wgpu-hal's conventions so wgpu's pipeline layouts and
//! vertex descriptors drive it unchanged: resources go to the Metal slots
//! the caller names (wgpu-hal numbers buffers, textures, and samplers per
//! stage in bind group order); vertex inputs are `[[stage_in]]` attributes at
//! their shader locations, which wgpu-hal's vertex descriptor fills.

use crate::glsl::naga;
use naga::back::msl;

/// Revision of the translation this module and `crate::glsl` produce. Disk
/// caches of translated programs include it in their keys; bump it whenever
/// a rewrite, option, or naga upgrade changes the MSL.
pub const TRANSLATION_REVISION: u32 = 1;

/// Metal's floating-point math mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MathMode {
    /// IEEE 754 semantics (no reassociation, NaN and infinity kept).
    Safe,
    /// Fast math without the NaN and infinity assumptions.
    Relaxed,
    /// Metal's default (what wgpu's own compile uses).
    Fast,
}

impl MathMode {
    fn pragma(self) -> &'static str {
        match self {
            MathMode::Safe => "#pragma METAL fp math_mode(safe)\n",
            MathMode::Relaxed => "#pragma METAL fp math_mode(relaxed)\n",
            MathMode::Fast => "#pragma METAL fp math_mode(fast)\n",
        }
    }
}

/// Whether `device` accepts passthrough MSL (the Metal backend with
/// `Features::PASSTHROUGH_SHADERS`).
pub fn supported(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::PASSTHROUGH_SHADERS)
}

/// A compiled stage: its module and the entry point name to use in the
/// pipeline (naga renames entry points that collide with MSL keywords, such
/// as `main`).
pub struct MslStage {
    pub module: wgpu::ShaderModule,
    pub entry_point: String,
    /// The MSL source (diagnostics).
    pub source: String,
}

/// One resource's Metal slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub group: u32,
    pub binding: u32,
    pub buffer: Option<u8>,
    pub texture: Option<u8>,
    pub sampler: Option<u8>,
}

/// The MSL of the single `stage` entry point of `module` with `mode`.
pub fn write(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: naga::ShaderStage,
    slots: &[Slot],
    mode: MathMode,
) -> Result<(String, String), String> {
    let ep = module
        .entry_points
        .iter()
        .find(|e| e.stage == stage)
        .ok_or_else(|| format!("no {stage:?} entry point"))?;
    let mut resources = msl::BindingMap::default();
    for s in slots {
        resources.insert(
            naga::ResourceBinding {
                group: s.group,
                binding: s.binding,
            },
            msl::BindTarget {
                buffer: s.buffer,
                texture: s.texture,
                sampler: s.sampler.map(msl::BindSamplerTarget::Resource),
                external_texture: None,
                mutable: false,
            },
        );
    }
    let mut per_entry_point_map = msl::EntryPointResourceMap::default();
    per_entry_point_map.insert(
        ep.name.clone(),
        msl::EntryPointResources {
            resources,
            immediates_buffer: None,
            sizes_buffer: None,
        },
    );
    let options = msl::Options {
        // The pragma needs MSL 3.2; wgpu compiles passthrough MSL with the
        // newest language version the OS has.
        lang_version: (3, 2),
        per_entry_point_map,
        inline_samplers: Vec::new(),
        spirv_cross_compatibility: false,
        fake_missing_bindings: false,
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            index: naga::proc::BoundsCheckPolicy::Restrict,
            buffer: naga::proc::BoundsCheckPolicy::Restrict,
            image_load: naga::proc::BoundsCheckPolicy::Restrict,
            binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
        },
        zero_initialize_workgroup_memory: true,
        force_loop_bounding: true,
        task_dispatch_limits: None,
        mesh_shader_primitive_indices_clamp: true,
        emit_int_div_checks: true,
        ray_query_initialization_tracking: true,
    };
    let pipeline = msl::PipelineOptions {
        entry_point: Some((stage, ep.name.clone())),
        allow_and_force_point_size: false,
        vertex_pulling_transform: false,
        vertex_buffer_mappings: Vec::new(),
        binding_array_length_map: Default::default(),
    };
    let (source, translation) =
        msl::write_string(module, info, &options, &pipeline).map_err(|e| format!("MSL: {e:?}"))?;
    let name = translation
        .entry_point_names
        .into_iter()
        .next()
        .ok_or("MSL: no entry point")?
        .map_err(|e| format!("MSL entry point: {e}"))?;
    Ok((format!("{}{source}", mode.pragma()), name))
}

/// A wgpu module from MSL [`write`] produced earlier (a disk cache of
/// translated programs). Needs [`supported`].
pub fn from_source(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    entry_point: &str,
) -> Result<MslStage, String> {
    if !supported(device) {
        return Err("the device does not accept passthrough MSL".to_owned());
    }
    let entry = [wgpu::PassthroughShaderEntryPoint {
        name: entry_point.to_owned().into(),
        workgroup_size: (0, 0, 0),
    }];
    // SAFETY: as in `module`: the source is this module's own translation.
    let module = unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some(label),
            entry_points: std::borrow::Cow::Borrowed(&entry),
            msl: Some(std::borrow::Cow::Borrowed(source)),
            ..Default::default()
        })
    };
    Ok(MslStage {
        module,
        entry_point: entry_point.to_owned(),
        source: source.to_owned(),
    })
}

/// A wgpu module for the single `stage` entry point of `module`, compiled
/// by Metal with `mode`. Needs [`supported`].
pub fn module(
    device: &wgpu::Device,
    label: &str,
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: naga::ShaderStage,
    slots: &[Slot],
    mode: MathMode,
) -> Result<MslStage, String> {
    if !supported(device) {
        return Err("the device does not accept passthrough MSL".to_owned());
    }
    let (source, entry_point) = write(module, info, stage, slots, mode)?;
    from_source(device, label, &source, &entry_point)
}
