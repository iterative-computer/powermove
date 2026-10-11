//! Metal math precision through wgpu: naga-generated MSL (wgpu compiles with default
//! MTLCompileOptions, i.e. fast math), MSL passthrough (default, `precise::`, pragma),
//! and offline metallibs built with and without fast math.

use std::borrow::Cow;
use std::process::Command;

use serde::Serialize;

use crate::cf::Result;
use crate::color;
use crate::gpu::Gpu;

/// Functions evaluated per input, in output order.
pub const FUNCS: [&str; 6] = ["pow(x,1/2.4)", "pow(x,2.4)", "exp2(x)", "log2(x)", "E(x)", "E⁻¹(x)"];

const WGSL: &str = r#"
@group(0) @binding(0) var<storage, read> xs: array<f32>;
@group(0) @binding(1) var<storage, read_write> ys: array<f32>;

fn e_enc(x: f32) -> f32 {
    let a = abs(x);
    var v: f32;
    if (a <= 0.0031308) { v = 12.92 * a; } else { v = 1.055 * pow(a, 1.0 / 2.4) - 0.055; }
    return select(v, -v, x < 0.0);
}
fn e_dec(x: f32) -> f32 {
    let a = abs(x);
    var v: f32;
    if (a <= 0.04045) { v = a / 12.92; } else { v = pow((a + 0.055) / 1.055, 2.4); }
    return select(v, -v, x < 0.0);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= arrayLength(&xs)) { return; }
    let x = xs[i];
    ys[i * 6u + 0u] = pow(x, 1.0 / 2.4);
    ys[i * 6u + 1u] = pow(x, 2.4);
    ys[i * 6u + 2u] = exp2(x);
    ys[i * 6u + 3u] = log2(x);
    ys[i * 6u + 4u] = e_enc(x);
    ys[i * 6u + 5u] = e_dec(x);
}
"#;

fn msl(prefix: &str, ns: &str) -> String {
    format!(
        r#"{prefix}
#include <metal_stdlib>
using namespace metal;

static float e_enc(float x) {{
    float a = fabs(x);
    float v = a <= 0.0031308f ? 12.92f * a : 1.055f * {ns}pow(a, 1.0f / 2.4f) - 0.055f;
    return x < 0.0f ? -v : v;
}}
static float e_dec(float x) {{
    float a = fabs(x);
    float v = a <= 0.04045f ? a / 12.92f : {ns}pow((a + 0.055f) / 1.055f, 2.4f);
    return x < 0.0f ? -v : v;
}}

kernel void main0(device const float* xs [[buffer(0)]],
                  device float* ys [[buffer(1)]],
                  constant uint& n [[buffer(2)]],
                  uint i [[thread_position_in_grid]]) {{
    if (i >= n) return;
    float x = xs[i];
    ys[i * 6 + 0] = {ns}pow(x, 1.0f / 2.4f);
    ys[i * 6 + 1] = {ns}pow(x, 2.4f);
    ys[i * 6 + 2] = {ns}exp2(x);
    ys[i * 6 + 3] = {ns}log2(x);
    ys[i * 6 + 4] = e_enc(x);
    ys[i * 6 + 5] = e_dec(x);
}}
"#
    )
}

#[derive(Serialize, Clone)]
pub struct FuncError {
    pub func: String,
    pub max_ulp: f64,
    pub max_abs: f64,
    /// Over inputs in (0, 1] only (encoded and linear values of SDR content).
    pub max_ulp_unit: f64,
    pub max_abs_unit: f64,
}

#[derive(Serialize)]
pub struct Variant {
    pub name: String,
    pub compiled: bool,
    pub error: Option<String>,
    pub funcs: Vec<FuncError>,
    /// E(E⁻¹(v / 65535)) rounded back to 16-bit equals v, over all 65536 codes.
    pub e_roundtrip16_failures: Option<u64>,
    /// The same at 12-bit codes (4096 codes, v / 4095).
    pub e_roundtrip12_failures: Option<u64>,
}

fn ref_f64(k: usize, x: f64) -> f64 {
    match k {
        0 => x.powf(1.0 / 2.4),
        1 => x.powf(2.4),
        2 => x.exp2(),
        3 => x.log2(),
        4 => color::e_encode(x),
        _ => color::e_decode(x),
    }
}

fn ulp(v: f32) -> f64 {
    let a = v.abs();
    if a == 0.0 || !a.is_finite() {
        return f32::MIN_POSITIVE as f64;
    }
    let next = f32::from_bits(a.to_bits() + 1);
    (next - a) as f64
}

/// Inputs: log-spaced positives over [1e-4, 16] and all 16-bit code values.
fn inputs() -> Vec<f32> {
    let mut v = Vec::new();
    let n = 1 << 20;
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        v.push((1e-4f64.ln() + t * (16f64.ln() - 1e-4f64.ln())).exp() as f32);
    }
    for c in 0..65536u32 {
        v.push(c as f32 / 65535.0);
    }
    v
}

enum Source {
    Wgsl,
    /// MSL source and its compute entry point name.
    Msl(String, String),
    Metallib(Vec<u8>),
}

/// The WGSL kernel without `arrayLength` (no naga buffer-sizes buffer), for
/// translation to MSL outside wgpu: bindings 0, 1, 2 map to buffers 0, 1, 2.
const WGSL_N: &str = r#"
@group(0) @binding(0) var<storage, read> xs: array<f32>;
@group(0) @binding(1) var<storage, read_write> ys: array<f32>;
@group(0) @binding(2) var<uniform> n: vec4<u32>;

fn e_enc(x: f32) -> f32 {
    let a = abs(x);
    var v: f32;
    if (a <= 0.0031308) { v = 12.92 * a; } else { v = 1.055 * pow(a, 1.0 / 2.4) - 0.055; }
    return select(v, -v, x < 0.0);
}
fn e_dec(x: f32) -> f32 {
    let a = abs(x);
    var v: f32;
    if (a <= 0.04045) { v = a / 12.92; } else { v = pow((a + 0.055) / 1.055, 2.4); }
    return select(v, -v, x < 0.0);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= n.x) { return; }
    let x = xs[i];
    ys[i * 6u + 0u] = pow(x, 1.0 / 2.4);
    ys[i * 6u + 1u] = pow(x, 2.4);
    ys[i * 6u + 2u] = exp2(x);
    ys[i * 6u + 3u] = log2(x);
    ys[i * 6u + 4u] = e_enc(x);
    ys[i * 6u + 5u] = e_dec(x);
}
"#;

/// Translate WGSL to MSL with naga (the translator wgpu uses), optionally rewriting
/// transcendental calls into `metal::precise::`. Returns the MSL and entry point name.
pub fn naga_msl(wgsl: &str, precise: bool) -> std::result::Result<(String, String), String> {
    use wgpu::naga;
    use naga::back::msl;
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| e.emit_to_string(wgsl))?;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| format!("{e:?}"))?;
    let mut resources = msl::BindingMap::default();
    for (binding, slot, mutable) in [(0u32, 0u8, false), (1, 1, true), (2, 2, false)] {
        resources.insert(
            naga::ResourceBinding { group: 0, binding },
            msl::BindTarget { buffer: Some(slot), mutable, ..Default::default() },
        );
    }
    let mut per_entry_point_map = msl::EntryPointResourceMap::default();
    per_entry_point_map.insert("main".into(), msl::EntryPointResources { resources, immediates_buffer: None, sizes_buffer: None });
    let options = msl::Options { lang_version: (3, 0), per_entry_point_map, ..Default::default() };
    let (mut src, tinfo) = msl::write_string(&module, &info, &options, &msl::PipelineOptions::default()).map_err(|e| format!("{e:?}"))?;
    let entry = tinfo.entry_point_names.first().cloned().ok_or("no entry point")?.map_err(|e| format!("{e:?}"))?;
    if precise {
        for f in ["pow", "exp2", "log2", "exp", "log", "sqrt", "sin", "cos", "tan", "powr"] {
            src = src.replace(&format!("metal::{f}("), &format!("metal::precise::{f}("));
        }
    }
    Ok((src, entry))
}

fn run_variant(gpu: &Gpu, name: &str, src: Source, xs: &[f32]) -> Variant {
    let ys = match dispatch(gpu, name, src, xs) {
        Ok(ys) => ys,
        Err(e) => {
            return Variant {
                name: name.into(),
                compiled: false,
                error: Some(e),
                funcs: vec![],
                e_roundtrip16_failures: None,
                e_roundtrip12_failures: None,
            };
        }
    };
    let mut funcs: Vec<FuncError> =
        FUNCS.iter().map(|f| FuncError { func: f.to_string(), max_ulp: 0.0, max_abs: 0.0, max_ulp_unit: 0.0, max_abs_unit: 0.0 }).collect();
    for (i, &x) in xs.iter().enumerate() {
        for (k, fe) in funcs.iter_mut().enumerate() {
            let got = ys[i * 6 + k] as f64;
            let want = ref_f64(k, x as f64);
            if !want.is_finite() {
                continue;
            }
            let abs = (got - want).abs();
            let u = abs / ulp(want as f32);
            fe.max_ulp = fe.max_ulp.max(u);
            fe.max_abs = fe.max_abs.max(abs);
            if x > 0.0 && x <= 1.0 {
                fe.max_ulp_unit = fe.max_ulp_unit.max(u);
                fe.max_abs_unit = fe.max_abs_unit.max(abs);
            }
        }
    }
    Variant {
        name: name.into(),
        compiled: true,
        error: None,
        funcs,
        e_roundtrip16_failures: None,
        e_roundtrip12_failures: None,
    }
}

/// Second pass: E(E⁻¹(code)) round trips, computed by feeding E⁻¹ outputs back in.
fn roundtrip(gpu: &Gpu, name: &str, make: &dyn Fn() -> Source, levels: u32) -> Option<u64> {
    let codes: Vec<f32> = (0..levels).map(|c| c as f32 / (levels - 1) as f32).collect();
    let first = dispatch(gpu, name, make(), &codes).ok()?;
    let lin: Vec<f32> = (0..codes.len()).map(|i| first[i * 6 + 5]).collect();
    let second = dispatch(gpu, name, make(), &lin).ok()?;
    let mut fails = 0;
    for c in 0..levels as usize {
        let back = (second[c * 6 + 4] as f64 * (levels - 1) as f64).round() as i64;
        if back != c as i64 {
            fails += 1;
        }
    }
    Some(fails)
}

/// Compile and dispatch one variant over `xs`; returns six outputs per input.
fn dispatch(gpu: &Gpu, name: &str, src: Source, xs: &[f32]) -> std::result::Result<Vec<f32>, String> {
    let dev = &gpu.device;
    let n = xs.len();
    let mk = |size: usize, usage| dev.create_buffer(&wgpu::BufferDescriptor { label: None, size: size as u64, usage, mapped_at_creation: false });
    let xbuf = mk(n * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
    gpu.queue.write_buffer(&xbuf, 0, &xs.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>());
    let ybuf = mk(n * 24, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
    let nbuf = mk(16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
    gpu.queue.write_buffer(&nbuf, 0, &[(n as u32).to_le_bytes(), [0; 4], [0; 4], [0; 4]].concat());
    let rb = mk(n * 24, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
    let is_wgsl = matches!(src, Source::Wgsl);
    let entry_name = match &src {
        Source::Msl(_, e) => e.clone(),
        _ => "main0".to_string(),
    };
    let scope = dev.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = match src {
        Source::Wgsl => dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(name), source: wgpu::ShaderSource::Wgsl(WGSL.into()) }),
        Source::Msl(code, entry) => unsafe {
            dev.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some(name),
                entry_points: Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint { name: entry.clone().into(), workgroup_size: (64, 1, 1) }]),
                msl: Some(Cow::Owned(code)),
                ..Default::default()
            })
        },
        Source::Metallib(bytes) => unsafe {
            dev.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some(name),
                entry_points: Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint { name: "main0".into(), workgroup_size: (64, 1, 1) }]),
                metallib: Some(Cow::Owned(bytes)),
                ..Default::default()
            })
        },
    };
    let storage = |binding, ro| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: ro }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    let mut entries = vec![storage(0, true), storage(1, false)];
    if !is_wgsl {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        });
    }
    let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
    let layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
    let pipeline = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: Some(&layout),
        module: &module,
        entry_point: Some(if is_wgsl { "main" } else { entry_name.as_str() }),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut bg_entries = vec![
        wgpu::BindGroupEntry { binding: 0, resource: xbuf.as_entire_binding() },
        wgpu::BindGroupEntry { binding: 1, resource: ybuf.as_entire_binding() },
    ];
    if !is_wgsl {
        bg_entries.push(wgpu::BindGroupEntry { binding: 2, resource: nbuf.as_entire_binding() });
    }
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &bg_entries });
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(e.to_string().lines().take(6).collect::<Vec<_>>().join(" "));
    }
    let mut enc = dev.create_command_encoder(&Default::default());
    {
        let mut cp = enc.begin_compute_pass(&Default::default());
        cp.set_pipeline(&pipeline);
        cp.set_bind_group(0, &bg, &[]);
        cp.dispatch_workgroups((n as u32).div_ceil(64), 1, 1);
    }
    enc.copy_buffer_to_buffer(&ybuf, 0, &rb, 0, (n * 24) as u64);
    gpu.queue.submit([enc.finish()]);
    rb.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.wait();
    let data = rb.slice(..).get_mapped_range().map_err(|e| format!("map: {e:?}"))?;
    let ys: Vec<f32> = data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    drop(data);
    rb.unmap();
    Ok(ys)
}

/// Compile MSL offline with `xcrun metal` (the Metal toolchain) into a metallib.
fn offline_metallib(src: &str, flags: &[&str], tag: &str) -> std::result::Result<Vec<u8>, String> {
    let dir = std::env::temp_dir().join(format!("s2-metal-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let msl = dir.join("k.metal");
    let air = dir.join("k.air");
    let lib = dir.join("k.metallib");
    std::fs::write(&msl, src).map_err(|e| e.to_string())?;
    let out = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "-c"])
        .args(flags)
        .arg(&msl)
        .arg("-o")
        .arg(&air)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let out = Command::new("xcrun").args(["-sdk", "macosx", "metallib"]).arg(&air).arg("-o").arg(&lib).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let bytes = std::fs::read(&lib).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(bytes)
}

#[derive(Serialize)]
pub struct MathReport {
    pub wgpu_compile_options_note: String,
    pub variants: Vec<Variant>,
}

pub fn run(gpu: &Gpu) -> Result<MathReport> {
    let xs = inputs();
    let mut variants = vec![];
    type Maker = Box<dyn Fn() -> std::result::Result<Source, String>>;
    let makers: Vec<(&str, Maker)> = vec![
        ("WGSL via naga (wgpu default MTLCompileOptions)", Box::new(|| Ok(Source::Wgsl))),
        ("MSL passthrough, default options, metal::", Box::new(|| Ok(Source::Msl(msl("", ""), "main0".into())))),
        ("MSL passthrough, metal::precise::", Box::new(|| Ok(Source::Msl(msl("", "precise::"), "main0".into())))),
        ("MSL passthrough, metal::fast::", Box::new(|| Ok(Source::Msl(msl("", "fast::"), "main0".into())))),
        (
            "MSL passthrough, #pragma METAL fp math_mode(safe)",
            Box::new(|| Ok(Source::Msl(msl("#pragma METAL fp math_mode(safe)", ""), "main0".into()))),
        ),
        (
            "naga MSL, unmodified, via passthrough",
            Box::new(|| naga_msl(WGSL_N, false).map(|(s, e)| Source::Msl(s, e))),
        ),
        (
            "naga MSL rewritten to metal::precise::, via passthrough",
            Box::new(|| naga_msl(WGSL_N, true).map(|(s, e)| Source::Msl(s, e))),
        ),
        (
            "metallib offline, -fno-fast-math (xcrun metal)",
            Box::new(|| offline_metallib(&msl("", ""), &["-fno-fast-math"], "nofast").map(Source::Metallib)),
        ),
        (
            "metallib offline, -fmetal-math-mode=safe (xcrun metal)",
            Box::new(|| offline_metallib(&msl("", ""), &["-fmetal-math-mode=safe"], "safe").map(Source::Metallib)),
        ),
        (
            "metallib offline, default (fast math)",
            Box::new(|| offline_metallib(&msl("", ""), &[], "fast").map(Source::Metallib)),
        ),
    ];
    for (name, make) in makers {
        match make() {
            Err(e) => variants.push(Variant {
                name: name.into(),
                compiled: false,
                error: Some(e.lines().take(4).collect::<Vec<_>>().join(" ")),
                funcs: vec![],
                e_roundtrip16_failures: None,
                e_roundtrip12_failures: None,
            }),
            Ok(src) => {
                let mut v = run_variant(gpu, name, src, &xs);
                if v.compiled {
                    let mk = || make().unwrap();
                    v.e_roundtrip16_failures = roundtrip(gpu, name, &mk, 65536);
                    v.e_roundtrip12_failures = roundtrip(gpu, name, &mk, 4096);
                }
                variants.push(v);
            }
        }
    }
    Ok(MathReport {
        wgpu_compile_options_note: "wgpu-hal 30.0.1 metal/device.rs compiles naga output with MTLCompileOptions::new() \
            plus languageVersion and preserveInvariance; MSL passthrough uses MTLCompileOptions::new() unchanged. \
            Neither sets fastMathEnabled or mathMode, so both use Metal's default (fast math)."
            .into(),
        variants,
    })
}
