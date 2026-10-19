//! Legacy GLSL programs (effects, transitions, shader and extension
//! fragment layers) as Metal compiles them in WebGL today (plan section 11;
//! amendments 9 and 22: level 1 content draws as today).
//!
//! Today's compositor runs on ANGLE's Metal back end: ANGLE translates the
//! GLSL to its own MSL and Metal compiles it with fast math. The engine
//! translates the same GLSL with glslang and naga (`pm_glsl`) and, before
//! this module, let wgpu write naga's MSL. The two MSL texts call different
//! Metal built-ins for some GLSL built-ins, and under fast math those differ
//! in the last bits; at a threshold or a steep curve the last bit is a code
//! value or a whole posterize level. So the engine writes the MSL itself
//! (naga's writer with wgpu-hal's options and resource slots, compiled as a
//! passthrough module with Metal's default fast math, as wgpu compiles naga
//! modules) and makes the calls ANGLE makes ([`angle_calls`]).
//!
//! The mapping comes from ANGLE's own output: `WEBGL_debug_shaders`'
//! `getTranslatedShaderSource` in Chromium with `--use-angle=metal` and
//! `--enable-privileged-webgl-extensions` (the dump script is described in
//! `docs/engine/shaders.md`, "ANGLE's MSL"). Each entry is proved by the
//! shader corpus against WebGL (`tests/effects_webgl.rs`, bit-exact counts
//! with `PM_SHADER_STATS`).
//!
//! A device without passthrough MSL (not the Metal back end) keeps wgpu's
//! own translation ([`stage_module`] returns the naga module).

use std::borrow::Cow;

use pm_gpu::glsl::naga;

/// GLSL built-ins whose naga MSL call differs from ANGLE's, as
/// (naga's call, ANGLE's call) prefixes. naga writes every built-in with
/// the `metal::` namespace, which no user identifier can carry, so a
/// prefix match is exact.
///
/// - `pow`: ANGLE emits `metal::powr` (GLSL leaves `pow(x, y)` undefined for
///   `x < 0`, Metal's `powr` likewise); naga emits `metal::pow`. Under fast
///   math they round differently: built-in Levels at gamma 1 differed in
///   about 10% of pixels by one binary16 step (`effects_webgl`,
///   `builtin/effect/levels` defaults, viewport, hdr) and is bit-exact with
///   `powr`.
/// - `normalize`: ANGLE emits `metal::fast::normalize` (vectors; the scalar
///   overload is `sign`, `pm_glsl`'s polyfill).
/// - `tanh`: ANGLE emits `metal::precise::tanh`.
pub const ANGLE_CALLS: &[(&str, &str)] = &[
    ("metal::pow(", "metal::powr("),
    ("metal::normalize(", "metal::fast::normalize("),
    ("metal::tanh(", "metal::precise::tanh("),
];

/// Calls replaced only inside the named polyfills `pm_glsl` injects, as
/// (function name prefix, naga's call, ANGLE's call): GLSL's `round` reaches
/// naga as `roundEven` inside `pm_round` (glslang and naga give both the same
/// IR), and ANGLE writes `round` as `metal::round` (ties away from zero).
pub const ANGLE_POLYFILL_CALLS: &[(&str, &str, &str)] =
    &[("pm_round_u0028", "metal::rint(", "metal::round(")];

/// naga's MSL with ANGLE's calls ([`ANGLE_CALLS`], [`ANGLE_POLYFILL_CALLS`]).
pub fn angle_calls(msl: &str) -> String {
    let mut out = msl.to_owned();
    for (naga_call, angle_call) in ANGLE_CALLS {
        out = out.replace(naga_call, angle_call);
    }
    for (function, naga_call, angle_call) in ANGLE_POLYFILL_CALLS {
        out = in_functions(&out, function, |body| body.replace(naga_call, angle_call));
    }
    out
}

/// `msl` with `edit` applied to the body of every function whose name
/// starts with `prefix`. naga writes a function as its signature from
/// column 0 (`<type> <name>(`), its parameters, then the body up to a `}`
/// alone on a line.
fn in_functions(msl: &str, prefix: &str, edit: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(msl.len());
    let mut lines = msl.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let starts = !line.starts_with(char::is_whitespace)
            && line
                .split_whitespace()
                .nth(1)
                .is_some_and(|name| name.starts_with(prefix));
        if !starts {
            out.push_str(line);
            continue;
        }
        let mut body = String::from(line);
        for line in lines.by_ref() {
            body.push_str(line);
            if line.trim_end() == "}" {
                break;
            }
        }
        out.push_str(&edit(&body));
    }
    out
}

/// A stage of a legacy program for a pipeline: the module and its entry
/// point name.
pub struct Stage {
    pub module: wgpu::ShaderModule,
    pub entry_point: String,
    /// The MSL the module was compiled from (`None`: wgpu's own
    /// translation of the naga module).
    pub msl: Option<String>,
}

/// One bind group layout entry as wgpu-hal sees it: group, binding, and
/// kind. The legacy layouts (`effects::program`) are a fragment group
/// (the default block, then each sampler's texture and sampler) and a
/// vertex group (the vertex stage's default block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Buffer { group: u32, binding: u32 },
    Texture { group: u32, binding: u32 },
    Sampler { group: u32, binding: u32 },
}

/// The Metal slots wgpu-hal gives `entries` (each group's entries in
/// binding order, only those visible to the stage): every visible buffer,
/// texture, and sampler takes the next slot of its kind
/// (`wgpu-hal/src/metal/device.rs`, `create_pipeline_layout`).
pub fn slots(entries: &[Entry]) -> Vec<pm_gpu::msl::Slot> {
    let mut sorted = entries.to_vec();
    let key = |e: &Entry| match *e {
        Entry::Buffer { group, binding }
        | Entry::Texture { group, binding }
        | Entry::Sampler { group, binding } => (group, binding),
    };
    sorted.sort_by_key(key);
    let (mut buffers, mut textures, mut samplers) = (0u8, 0u8, 0u8);
    sorted
        .iter()
        .map(|e| {
            let (group, binding) = key(e);
            let mut slot = pm_gpu::msl::Slot {
                group,
                binding,
                buffer: None,
                texture: None,
                sampler: None,
            };
            match e {
                Entry::Buffer { .. } => {
                    slot.buffer = Some(buffers);
                    buffers += 1;
                }
                Entry::Texture { .. } => {
                    slot.texture = Some(textures);
                    textures += 1;
                }
                Entry::Sampler { .. } => {
                    slot.sampler = Some(samplers);
                    samplers += 1;
                }
            }
            slot
        })
        .collect()
}

/// naga's MSL for the single `stage` entry point of `module` with ANGLE's
/// calls, and the entry point's MSL name.
pub fn write(
    module: &naga::Module,
    stage: naga::ShaderStage,
    entries: &[Entry],
) -> Result<(String, String), String> {
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(|e| format!("naga validation: {:?}", e.into_inner()))?;
    let (source, entry) = pm_gpu::msl::write(
        module,
        &info,
        stage,
        &slots(entries),
        pm_gpu::msl::MathMode::Fast,
    )?;
    Ok((angle_calls(&source), entry))
}

/// The module for one stage of a legacy program: ANGLE's calls through
/// passthrough MSL where the device takes it, else wgpu's translation.
pub fn stage_module(
    device: &wgpu::Device,
    label: &str,
    module: naga::Module,
    stage: naga::ShaderStage,
    entries: &[Entry],
) -> Result<Stage, String> {
    if !pm_gpu::msl::supported(device) {
        return Ok(Stage {
            module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
            }),
            entry_point: "main".to_owned(),
            msl: None,
        });
    }
    let (source, entry_point) = write(&module, stage, entries)?;
    let compiled = pm_gpu::msl::from_source(device, label, &source, &entry_point)?;
    Ok(Stage {
        module: compiled.module,
        entry_point: compiled.entry_point,
        msl: Some(source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_calls_replace_only_namespaced_builtins() {
        let msl = "float pow_(float x){ return x; }\nfloat a = metal::pow(x, y) + metal::powr(x, y) + pow_(1.0);\nmetal::float3 b = metal::pow(v, metal::float3(2.0));";
        let out = angle_calls(msl);
        assert_eq!(
            out,
            "float pow_(float x){ return x; }\nfloat a = metal::powr(x, y) + metal::powr(x, y) + pow_(1.0);\nmetal::float3 b = metal::powr(v, metal::float3(2.0));"
        );
    }

    #[test]
    fn round_is_angles_only_inside_the_round_polyfill() {
        let msl = "float pm_round_u0028_f1_u003b(\n    thread float& x\n) {\n    float _e1 = x;\n    return metal::rint(_e1);\n}\n\nfragment main_(){\n    float r = metal::rint(y);\n}\n";
        let out = angle_calls(msl);
        assert!(out.contains("return metal::round(_e1);"), "{out}");
        assert!(
            out.contains("float r = metal::rint(y);"),
            "roundEven stays: {out}"
        );
    }

    #[test]
    fn round_polyfill_reaches_naga_as_a_named_function() {
        // The prefix `ANGLE_POLYFILL_CALLS` matches what naga writes for
        // `pm_glsl`'s polyfill.
        let src = "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform float u_x;\nout vec4 o;\nvoid main(){ o = vec4(round(u_x), roundEven(u_x), round(v_uv)); }\n";
        let p = pm_glsl::translate(src, 0).expect("translates");
        let msl = pm_glsl::pipeline::msl(&p.fragment.module).expect("msl");
        let out = angle_calls(&msl);
        assert_eq!(out.matches("metal::round(").count(), 2, "{out}");
        assert!(out.contains("metal::rint("), "roundEven: {out}");
    }

    #[test]
    fn slots_follow_wgpu_hal_per_kind_in_binding_order() {
        // The fragment group of a program with two samplers (images at 1
        // and 2, samplers at 33 and 34) and its default block at 0.
        let s = slots(&[
            Entry::Sampler {
                group: 0,
                binding: 34,
            },
            Entry::Texture {
                group: 0,
                binding: 2,
            },
            Entry::Buffer {
                group: 0,
                binding: 0,
            },
            Entry::Sampler {
                group: 0,
                binding: 33,
            },
            Entry::Texture {
                group: 0,
                binding: 1,
            },
        ]);
        type Row = (u32, Option<u8>, Option<u8>, Option<u8>);
        let got: Vec<Row> = s
            .iter()
            .map(|s| (s.binding, s.buffer, s.texture, s.sampler))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, Some(0), None, None),
                (1, None, Some(0), None),
                (2, None, Some(1), None),
                (33, None, None, Some(0)),
                (34, None, None, Some(1)),
            ]
        );
    }
}
