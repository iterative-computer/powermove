//! three.js r180 program assembly (`WebGLProgram.js`): the prefix of
//! `#define`s, precision statements, built-in uniforms and attributes, the
//! GLSL3 conversion header, `#include` resolution against the vendored
//! chunks, light and clipping-plane counts, and loop unrolling. The output
//! is byte for byte the source WebGL receives (checked against programs
//! recorded from three.js by `tests/three.rs`).

use std::sync::OnceLock;

use regex::{Captures, Regex};

use super::chunks::CHUNKS;

/// A chunk by name.
pub fn chunk(name: &str) -> Option<&'static str> {
    CHUNKS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// three.js tone mapping constants used by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneMapping {
    None,
    AcesFilmic,
}

/// The `WebGLPrograms.getParameters` result fields the program prefix reads.
/// Every flag defaults to off; map UV channels to `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameters {
    pub shader_type: String,
    pub shader_name: String,
    /// `material.defines`, in insertion order (`name`, `value`).
    pub defines: Vec<(String, String)>,
    pub vertex_shader: String,
    pub fragment_shader: String,
    pub precision: &'static str,

    pub extension_clip_cull_distance: bool,
    pub extension_multi_draw: bool,
    pub batching: bool,
    pub batching_color: bool,
    pub instancing: bool,
    pub instancing_color: bool,
    pub instancing_morph: bool,
    pub use_fog: bool,
    pub fog: bool,
    pub fog_exp2: bool,
    pub alpha_to_coverage: bool,
    pub map: bool,
    pub matcap: bool,
    pub env_map: bool,
    pub light_map: bool,
    pub ao_map: bool,
    pub bump_map: bool,
    pub normal_map: bool,
    pub normal_map_object_space: bool,
    pub normal_map_tangent_space: bool,
    pub displacement_map: bool,
    pub emissive_map: bool,
    pub anisotropy: bool,
    pub anisotropy_map: bool,
    pub clearcoat: bool,
    pub clearcoat_map: bool,
    pub clearcoat_roughness_map: bool,
    pub clearcoat_normal_map: bool,
    pub dispersion: bool,
    pub iridescence: bool,
    pub iridescence_map: bool,
    pub iridescence_thickness_map: bool,
    pub specular_map: bool,
    pub specular_color_map: bool,
    pub specular_intensity_map: bool,
    pub roughness_map: bool,
    pub metalness_map: bool,
    pub alpha_map: bool,
    pub alpha_test: bool,
    pub alpha_hash: bool,
    pub sheen: bool,
    pub sheen_color_map: bool,
    pub sheen_roughness_map: bool,
    pub transmission: bool,
    pub transmission_map: bool,
    pub thickness_map: bool,
    pub gradient_map: bool,
    pub map_uv: Option<String>,
    pub alpha_map_uv: Option<String>,
    pub light_map_uv: Option<String>,
    pub ao_map_uv: Option<String>,
    pub emissive_map_uv: Option<String>,
    pub bump_map_uv: Option<String>,
    pub normal_map_uv: Option<String>,
    pub displacement_map_uv: Option<String>,
    pub metalness_map_uv: Option<String>,
    pub roughness_map_uv: Option<String>,
    pub anisotropy_map_uv: Option<String>,
    pub clearcoat_map_uv: Option<String>,
    pub clearcoat_normal_map_uv: Option<String>,
    pub clearcoat_roughness_map_uv: Option<String>,
    pub iridescence_map_uv: Option<String>,
    pub iridescence_thickness_map_uv: Option<String>,
    pub sheen_color_map_uv: Option<String>,
    pub sheen_roughness_map_uv: Option<String>,
    pub specular_map_uv: Option<String>,
    pub specular_color_map_uv: Option<String>,
    pub specular_intensity_map_uv: Option<String>,
    pub transmission_map_uv: Option<String>,
    pub thickness_map_uv: Option<String>,
    pub vertex_tangents: bool,
    pub vertex_colors: bool,
    pub vertex_alphas: bool,
    pub vertex_uv1s: bool,
    pub vertex_uv2s: bool,
    pub vertex_uv3s: bool,
    pub points_uvs: bool,
    pub flat_shading: bool,
    pub skinning: bool,
    pub morph_targets: bool,
    pub morph_normals: bool,
    pub morph_colors: bool,
    pub morph_targets_count: u32,
    pub morph_texture_stride: u32,
    pub double_sided: bool,
    pub flip_sided: bool,
    pub shadow_map_enabled: bool,
    /// `SHADOWMAP_TYPE_PCF_SOFT` and the like.
    pub shadow_map_type: &'static str,
    pub size_attenuation: bool,
    pub num_light_probes: u32,
    pub logarithmic_depth_buffer: bool,
    pub reversed_depth_buffer: bool,
    pub premultiplied_alpha: bool,
    pub decode_video_texture: bool,
    pub decode_video_texture_emissive: bool,
    pub tone_mapping: ToneMapping,
    pub dithering: bool,
    pub opaque: bool,
    /// The output color space's transfer: sRGB (`sRGBTransferOETF`) when
    /// true, linear otherwise. Rec.709 primaries both ways (identity matrix).
    pub output_srgb: bool,
    pub use_depth_packing: bool,
    pub depth_packing: u32,

    pub num_dir_lights: u32,
    pub num_point_lights: u32,
    pub num_spot_lights: u32,
    pub num_spot_light_maps: u32,
    pub num_rect_area_lights: u32,
    pub num_hemi_lights: u32,
    pub num_dir_light_shadows: u32,
    pub num_point_light_shadows: u32,
    pub num_spot_light_shadows: u32,
    pub num_spot_light_shadows_with_maps: u32,
    pub num_clipping_planes: u32,
    pub num_clip_intersection: u32,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            shader_type: String::new(),
            shader_name: String::new(),
            defines: Vec::new(),
            vertex_shader: String::new(),
            fragment_shader: String::new(),
            precision: "highp",
            extension_clip_cull_distance: false,
            extension_multi_draw: false,
            batching: false,
            batching_color: false,
            instancing: false,
            instancing_color: false,
            instancing_morph: false,
            use_fog: false,
            fog: false,
            fog_exp2: false,
            alpha_to_coverage: false,
            map: false,
            matcap: false,
            env_map: false,
            light_map: false,
            ao_map: false,
            bump_map: false,
            normal_map: false,
            normal_map_object_space: false,
            normal_map_tangent_space: false,
            displacement_map: false,
            emissive_map: false,
            anisotropy: false,
            anisotropy_map: false,
            clearcoat: false,
            clearcoat_map: false,
            clearcoat_roughness_map: false,
            clearcoat_normal_map: false,
            dispersion: false,
            iridescence: false,
            iridescence_map: false,
            iridescence_thickness_map: false,
            specular_map: false,
            specular_color_map: false,
            specular_intensity_map: false,
            roughness_map: false,
            metalness_map: false,
            alpha_map: false,
            alpha_test: false,
            alpha_hash: false,
            sheen: false,
            sheen_color_map: false,
            sheen_roughness_map: false,
            transmission: false,
            transmission_map: false,
            thickness_map: false,
            gradient_map: false,
            map_uv: None,
            alpha_map_uv: None,
            light_map_uv: None,
            ao_map_uv: None,
            emissive_map_uv: None,
            bump_map_uv: None,
            normal_map_uv: None,
            displacement_map_uv: None,
            metalness_map_uv: None,
            roughness_map_uv: None,
            anisotropy_map_uv: None,
            clearcoat_map_uv: None,
            clearcoat_normal_map_uv: None,
            clearcoat_roughness_map_uv: None,
            iridescence_map_uv: None,
            iridescence_thickness_map_uv: None,
            sheen_color_map_uv: None,
            sheen_roughness_map_uv: None,
            specular_map_uv: None,
            specular_color_map_uv: None,
            specular_intensity_map_uv: None,
            transmission_map_uv: None,
            thickness_map_uv: None,
            vertex_tangents: false,
            vertex_colors: false,
            vertex_alphas: false,
            vertex_uv1s: false,
            vertex_uv2s: false,
            vertex_uv3s: false,
            points_uvs: false,
            flat_shading: false,
            skinning: false,
            morph_targets: false,
            morph_normals: false,
            morph_colors: false,
            morph_targets_count: 0,
            morph_texture_stride: 0,
            double_sided: false,
            flip_sided: false,
            shadow_map_enabled: false,
            shadow_map_type: "SHADOWMAP_TYPE_PCF_SOFT",
            size_attenuation: false,
            num_light_probes: 0,
            logarithmic_depth_buffer: false,
            reversed_depth_buffer: false,
            premultiplied_alpha: false,
            decode_video_texture: false,
            decode_video_texture_emissive: false,
            tone_mapping: ToneMapping::None,
            dithering: false,
            opaque: false,
            output_srgb: true,
            use_depth_packing: false,
            depth_packing: 0,
            num_dir_lights: 0,
            num_point_lights: 0,
            num_spot_lights: 0,
            num_spot_light_maps: 0,
            num_rect_area_lights: 0,
            num_hemi_lights: 0,
            num_dir_light_shadows: 0,
            num_point_light_shadows: 0,
            num_spot_light_shadows: 0,
            num_spot_light_shadows_with_maps: 0,
            num_clipping_planes: 0,
            num_clip_intersection: 0,
        }
    }
}

fn line(on: bool, text: &str) -> String {
    if on { text.to_owned() } else { String::new() }
}

fn uv_define(name: &str, value: &Option<String>) -> String {
    match value {
        Some(v) => format!("#define {name} {v}"),
        None => String::new(),
    }
}

/// `generatePrecision`.
fn precision(p: &str) -> String {
    let mut s = format!(
        "precision {p} float;\n\tprecision {p} int;\n\tprecision {p} sampler2D;\n\tprecision {p} samplerCube;\n\tprecision {p} sampler3D;\n\tprecision {p} sampler2DArray;\n\tprecision {p} sampler2DShadow;\n\tprecision {p} samplerCubeShadow;\n\tprecision {p} sampler2DArrayShadow;\n\tprecision {p} isampler2D;\n\tprecision {p} isampler3D;\n\tprecision {p} isamplerCube;\n\tprecision {p} isampler2DArray;\n\tprecision {p} usampler2D;\n\tprecision {p} usampler3D;\n\tprecision {p} usamplerCube;\n\tprecision {p} usampler2DArray;\n\t"
    );
    match p {
        "highp" => s.push_str("\n#define HIGH_PRECISION"),
        "mediump" => s.push_str("\n#define MEDIUM_PRECISION"),
        "lowp" => s.push_str("\n#define LOW_PRECISION"),
        _ => {}
    }
    s
}

/// `generateDefines`: `#define NAME value` for every define (a `false`
/// value is not representable here and never used by the engine).
fn defines(d: &[(String, String)]) -> String {
    d.iter()
        .map(|(k, v)| format!("#define {k} {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `ColorManagement._getMatrix(m, LinearSRGBColorSpace, target)` for a
/// target with Rec.709 primaries: `LINEAR_REC709_TO_XYZ × XYZ_TO_LINEAR_REC709`
/// in f64, in `Matrix3.multiplyMatrices`'s order. The product is the
/// identity only up to rounding; three.js prints it with `toFixed(4)`, so its
/// tiny negative entries appear as `-0.0000` in the program text.
fn rec709_round_trip() -> [f64; 9] {
    // Row-major as `Matrix3.set` takes them.
    let a = [
        0.4123908, 0.3575843, 0.1804808, 0.2126390, 0.7151687, 0.0721923, 0.0193308, 0.1191948,
        0.9505322,
    ];
    let b = [
        3.2409699, -1.5373832, -0.4986108, -0.9692436, 1.8759675, 0.0415551, 0.0556301,
        -0.2039770, 1.0569715,
    ];
    let (a11, a12, a13, a21, a22, a23, a31, a32, a33) =
        (a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8]);
    let (b11, b12, b13, b21, b22, b23, b31, b32, b33) =
        (b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8]);
    // Column-major elements (`te`).
    [
        a11 * b11 + a12 * b21 + a13 * b31,
        a21 * b11 + a22 * b21 + a23 * b31,
        a31 * b11 + a32 * b21 + a33 * b31,
        a11 * b12 + a12 * b22 + a13 * b32,
        a21 * b12 + a22 * b22 + a23 * b32,
        a31 * b12 + a32 * b22 + a33 * b32,
        a11 * b13 + a12 * b23 + a13 * b33,
        a21 * b13 + a22 * b23 + a23 * b33,
        a31 * b13 + a32 * b23 + a33 * b33,
    ]
}

/// `Number.prototype.toFixed(4)` (a negative value that rounds to zero keeps
/// its sign; negative zero does not).
fn to_fixed4(v: f64) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    format!("{v:.4}")
}

/// `getTexelEncodingFunction('linearToOutputTexel', outputColorSpace)` for
/// sRGB or linear-sRGB output.
fn texel_encoding(srgb: bool) -> String {
    let m = format!(
        "mat3( {} )",
        rec709_round_trip()
            .iter()
            .map(|v| to_fixed4(*v))
            .collect::<Vec<_>>()
            .join(",")
    );
    let oetf = if srgb {
        "sRGBTransferOETF"
    } else {
        "LinearTransferOETF"
    };
    format!("vec4 linearToOutputTexel( vec4 value ) {{\n\treturn {oetf}( vec4( value.rgb * {m}, value.a ) );\n}}")
}

/// `getLuminanceFunction` for the linear-sRGB working space.
fn luminance() -> &'static str {
    "float luminance( const in vec3 rgb ) {\n\tconst vec3 weights = vec3( 0.2126, 0.7152, 0.0722 );\n\treturn dot( weights, rgb );\n}"
}

fn join(lines: Vec<String>) -> String {
    lines
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn include_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^[ \t]*#include +<([\w\d./]+)>").unwrap())
}

/// `resolveIncludes`.
pub fn resolve_includes(source: &str) -> Result<String, String> {
    let mut error = None;
    let out = include_pattern().replace_all(source, |c: &Captures<'_>| {
        let name = &c[1];
        match chunk(name) {
            Some(s) => match resolve_includes(s) {
                Ok(r) => r,
                Err(e) => {
                    error.get_or_insert(e);
                    String::new()
                }
            },
            None => {
                error.get_or_insert(format!("Can not resolve #include <{name}>"));
                String::new()
            }
        }
    });
    match error {
        Some(e) => Err(e),
        None => Ok(out.into_owned()),
    }
}

/// `replaceLightNums`. Each `.replace(/NAME/g, n)` runs in this order, so
/// `NUM_SPOT_LIGHT_COORDS` is replaced before `NUM_SPOT_LIGHTS` could
/// match its prefix, exactly as the JavaScript does.
fn replace_light_nums(s: &str, p: &Parameters) -> String {
    let coords = p.num_spot_light_shadows + p.num_spot_light_maps - p.num_spot_light_shadows_with_maps;
    let pairs: [(&str, u32); 11] = [
        ("NUM_DIR_LIGHTS", p.num_dir_lights),
        ("NUM_SPOT_LIGHTS", p.num_spot_lights),
        ("NUM_SPOT_LIGHT_MAPS", p.num_spot_light_maps),
        ("NUM_SPOT_LIGHT_COORDS", coords),
        ("NUM_RECT_AREA_LIGHTS", p.num_rect_area_lights),
        ("NUM_POINT_LIGHTS", p.num_point_lights),
        ("NUM_HEMI_LIGHTS", p.num_hemi_lights),
        ("NUM_DIR_LIGHT_SHADOWS", p.num_dir_light_shadows),
        ("NUM_SPOT_LIGHT_SHADOWS_WITH_MAPS", p.num_spot_light_shadows_with_maps),
        ("NUM_SPOT_LIGHT_SHADOWS", p.num_spot_light_shadows),
        ("NUM_POINT_LIGHT_SHADOWS", p.num_point_light_shadows),
    ];
    let mut out = s.to_owned();
    for (name, n) in pairs {
        out = out.replace(name, &n.to_string());
    }
    out
}

/// `replaceClippingPlaneNums`.
fn replace_clipping_plane_nums(s: &str, p: &Parameters) -> String {
    s.replace("NUM_CLIPPING_PLANES", &p.num_clipping_planes.to_string())
        .replace(
            "UNION_CLIPPING_PLANES",
            &(p.num_clipping_planes as i64 - p.num_clip_intersection as i64).to_string(),
        )
}

fn unroll_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"#pragma unroll_loop_start\s+for\s*\(\s*int\s+i\s*=\s*(\d+)\s*;\s*i\s*<\s*(\d+)\s*;\s*i\s*\+\+\s*\)\s*\{([\s\S]+?)\}\s+#pragma unroll_loop_end",
        )
        .unwrap()
    })
}

fn index_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\[\s*i\s*\]").unwrap())
}

/// `unrollLoops`.
fn unroll_loops(s: &str) -> String {
    unroll_pattern()
        .replace_all(s, |c: &Captures<'_>| {
            let start: i64 = c[1].parse().unwrap_or(0);
            let end: i64 = c[2].parse().unwrap_or(0);
            let snippet = &c[3];
            let mut out = String::new();
            for i in start..end {
                let replaced = index_pattern().replace_all(snippet, format!("[ {i} ]").as_str());
                out.push_str(&replaced.replace("UNROLLED_LOOP_INDEX", &i.to_string()));
            }
            out
        })
        .into_owned()
}

/// The vertex and fragment sources WebGL receives for `p`.
pub fn sources(p: &Parameters) -> Result<(String, String), String> {
    let shadow_type = p.shadow_map_type;
    let prefix_vertex = join(vec![
        precision(p.precision),
        format!("#define SHADER_TYPE {}", p.shader_type),
        format!("#define SHADER_NAME {}", p.shader_name),
        defines(&p.defines),
        line(p.extension_clip_cull_distance, "#define USE_CLIP_DISTANCE"),
        line(p.batching, "#define USE_BATCHING"),
        line(p.batching_color, "#define USE_BATCHING_COLOR"),
        line(p.instancing, "#define USE_INSTANCING"),
        line(p.instancing_color, "#define USE_INSTANCING_COLOR"),
        line(p.instancing_morph, "#define USE_INSTANCING_MORPH"),
        line(p.use_fog && p.fog, "#define USE_FOG"),
        line(p.use_fog && p.fog_exp2, "#define FOG_EXP2"),
        line(p.map, "#define USE_MAP"),
        line(p.env_map, "#define USE_ENVMAP"),
        line(p.env_map, "#define ENVMAP_MODE_REFLECTION"),
        line(p.light_map, "#define USE_LIGHTMAP"),
        line(p.ao_map, "#define USE_AOMAP"),
        line(p.bump_map, "#define USE_BUMPMAP"),
        line(p.normal_map, "#define USE_NORMALMAP"),
        line(p.normal_map_object_space, "#define USE_NORMALMAP_OBJECTSPACE"),
        line(p.normal_map_tangent_space, "#define USE_NORMALMAP_TANGENTSPACE"),
        line(p.displacement_map, "#define USE_DISPLACEMENTMAP"),
        line(p.emissive_map, "#define USE_EMISSIVEMAP"),
        line(p.anisotropy, "#define USE_ANISOTROPY"),
        line(p.anisotropy_map, "#define USE_ANISOTROPYMAP"),
        line(p.clearcoat_map, "#define USE_CLEARCOATMAP"),
        line(p.clearcoat_roughness_map, "#define USE_CLEARCOAT_ROUGHNESSMAP"),
        line(p.clearcoat_normal_map, "#define USE_CLEARCOAT_NORMALMAP"),
        line(p.iridescence_map, "#define USE_IRIDESCENCEMAP"),
        line(p.iridescence_thickness_map, "#define USE_IRIDESCENCE_THICKNESSMAP"),
        line(p.specular_map, "#define USE_SPECULARMAP"),
        line(p.specular_color_map, "#define USE_SPECULAR_COLORMAP"),
        line(p.specular_intensity_map, "#define USE_SPECULAR_INTENSITYMAP"),
        line(p.roughness_map, "#define USE_ROUGHNESSMAP"),
        line(p.metalness_map, "#define USE_METALNESSMAP"),
        line(p.alpha_map, "#define USE_ALPHAMAP"),
        line(p.alpha_hash, "#define USE_ALPHAHASH"),
        line(p.transmission, "#define USE_TRANSMISSION"),
        line(p.transmission_map, "#define USE_TRANSMISSIONMAP"),
        line(p.thickness_map, "#define USE_THICKNESSMAP"),
        line(p.sheen_color_map, "#define USE_SHEEN_COLORMAP"),
        line(p.sheen_roughness_map, "#define USE_SHEEN_ROUGHNESSMAP"),
        uv_define("MAP_UV", &p.map_uv),
        uv_define("ALPHAMAP_UV", &p.alpha_map_uv),
        uv_define("LIGHTMAP_UV", &p.light_map_uv),
        uv_define("AOMAP_UV", &p.ao_map_uv),
        uv_define("EMISSIVEMAP_UV", &p.emissive_map_uv),
        uv_define("BUMPMAP_UV", &p.bump_map_uv),
        uv_define("NORMALMAP_UV", &p.normal_map_uv),
        uv_define("DISPLACEMENTMAP_UV", &p.displacement_map_uv),
        uv_define("METALNESSMAP_UV", &p.metalness_map_uv),
        uv_define("ROUGHNESSMAP_UV", &p.roughness_map_uv),
        uv_define("ANISOTROPYMAP_UV", &p.anisotropy_map_uv),
        uv_define("CLEARCOATMAP_UV", &p.clearcoat_map_uv),
        uv_define("CLEARCOAT_NORMALMAP_UV", &p.clearcoat_normal_map_uv),
        uv_define("CLEARCOAT_ROUGHNESSMAP_UV", &p.clearcoat_roughness_map_uv),
        uv_define("IRIDESCENCEMAP_UV", &p.iridescence_map_uv),
        uv_define("IRIDESCENCE_THICKNESSMAP_UV", &p.iridescence_thickness_map_uv),
        uv_define("SHEEN_COLORMAP_UV", &p.sheen_color_map_uv),
        uv_define("SHEEN_ROUGHNESSMAP_UV", &p.sheen_roughness_map_uv),
        uv_define("SPECULARMAP_UV", &p.specular_map_uv),
        uv_define("SPECULAR_COLORMAP_UV", &p.specular_color_map_uv),
        uv_define("SPECULAR_INTENSITYMAP_UV", &p.specular_intensity_map_uv),
        uv_define("TRANSMISSIONMAP_UV", &p.transmission_map_uv),
        uv_define("THICKNESSMAP_UV", &p.thickness_map_uv),
        line(p.vertex_tangents && !p.flat_shading, "#define USE_TANGENT"),
        line(p.vertex_colors, "#define USE_COLOR"),
        line(p.vertex_alphas, "#define USE_COLOR_ALPHA"),
        line(p.vertex_uv1s, "#define USE_UV1"),
        line(p.vertex_uv2s, "#define USE_UV2"),
        line(p.vertex_uv3s, "#define USE_UV3"),
        line(p.points_uvs, "#define USE_POINTS_UV"),
        line(p.flat_shading, "#define FLAT_SHADED"),
        line(p.skinning, "#define USE_SKINNING"),
        line(p.morph_targets, "#define USE_MORPHTARGETS"),
        line(p.morph_normals && !p.flat_shading, "#define USE_MORPHNORMALS"),
        line(p.morph_colors, "#define USE_MORPHCOLORS"),
        if p.morph_targets_count > 0 {
            format!("#define MORPHTARGETS_TEXTURE_STRIDE {}", p.morph_texture_stride)
        } else {
            String::new()
        },
        if p.morph_targets_count > 0 {
            format!("#define MORPHTARGETS_COUNT {}", p.morph_targets_count)
        } else {
            String::new()
        },
        line(p.double_sided, "#define DOUBLE_SIDED"),
        line(p.flip_sided, "#define FLIP_SIDED"),
        line(p.shadow_map_enabled, "#define USE_SHADOWMAP"),
        line(p.shadow_map_enabled, &format!("#define {shadow_type}")),
        line(p.size_attenuation, "#define USE_SIZEATTENUATION"),
        line(p.num_light_probes > 0, "#define USE_LIGHT_PROBES"),
        line(p.logarithmic_depth_buffer, "#define USE_LOGARITHMIC_DEPTH_BUFFER"),
        line(p.reversed_depth_buffer, "#define USE_REVERSED_DEPTH_BUFFER"),
        "uniform mat4 modelMatrix;".into(),
        "uniform mat4 modelViewMatrix;".into(),
        "uniform mat4 projectionMatrix;".into(),
        "uniform mat4 viewMatrix;".into(),
        "uniform mat3 normalMatrix;".into(),
        "uniform vec3 cameraPosition;".into(),
        "uniform bool isOrthographic;".into(),
        "#ifdef USE_INSTANCING".into(),
        "\tattribute mat4 instanceMatrix;".into(),
        "#endif".into(),
        "#ifdef USE_INSTANCING_COLOR".into(),
        "\tattribute vec3 instanceColor;".into(),
        "#endif".into(),
        "#ifdef USE_INSTANCING_MORPH".into(),
        "\tuniform sampler2D morphTexture;".into(),
        "#endif".into(),
        "attribute vec3 position;".into(),
        "attribute vec3 normal;".into(),
        "attribute vec2 uv;".into(),
        "#ifdef USE_UV1".into(),
        "\tattribute vec2 uv1;".into(),
        "#endif".into(),
        "#ifdef USE_UV2".into(),
        "\tattribute vec2 uv2;".into(),
        "#endif".into(),
        "#ifdef USE_UV3".into(),
        "\tattribute vec2 uv3;".into(),
        "#endif".into(),
        "#ifdef USE_TANGENT".into(),
        "\tattribute vec4 tangent;".into(),
        "#endif".into(),
        "#if defined( USE_COLOR_ALPHA )".into(),
        "\tattribute vec4 color;".into(),
        "#elif defined( USE_COLOR )".into(),
        "\tattribute vec3 color;".into(),
        "#endif".into(),
        "#ifdef USE_SKINNING".into(),
        "\tattribute vec4 skinIndex;".into(),
        "\tattribute vec4 skinWeight;".into(),
        "#endif".into(),
        "\n".into(),
    ]);
    let tone = p.tone_mapping != ToneMapping::None;
    let tonemapping_pars = chunk("tonemapping_pars_fragment").ok_or("tonemapping_pars_fragment")?;
    let colorspace_pars = chunk("colorspace_pars_fragment").ok_or("colorspace_pars_fragment")?;
    let prefix_fragment = join(vec![
        precision(p.precision),
        format!("#define SHADER_TYPE {}", p.shader_type),
        format!("#define SHADER_NAME {}", p.shader_name),
        defines(&p.defines),
        line(p.use_fog && p.fog, "#define USE_FOG"),
        line(p.use_fog && p.fog_exp2, "#define FOG_EXP2"),
        line(p.alpha_to_coverage, "#define ALPHA_TO_COVERAGE"),
        line(p.map, "#define USE_MAP"),
        line(p.matcap, "#define USE_MATCAP"),
        line(p.env_map, "#define USE_ENVMAP"),
        line(p.env_map, "#define ENVMAP_TYPE_CUBE"),
        line(p.env_map, "#define ENVMAP_MODE_REFLECTION"),
        line(p.env_map, "#define ENVMAP_BLENDING_NONE"),
        line(p.light_map, "#define USE_LIGHTMAP"),
        line(p.ao_map, "#define USE_AOMAP"),
        line(p.bump_map, "#define USE_BUMPMAP"),
        line(p.normal_map, "#define USE_NORMALMAP"),
        line(p.normal_map_object_space, "#define USE_NORMALMAP_OBJECTSPACE"),
        line(p.normal_map_tangent_space, "#define USE_NORMALMAP_TANGENTSPACE"),
        line(p.emissive_map, "#define USE_EMISSIVEMAP"),
        line(p.anisotropy, "#define USE_ANISOTROPY"),
        line(p.anisotropy_map, "#define USE_ANISOTROPYMAP"),
        line(p.clearcoat, "#define USE_CLEARCOAT"),
        line(p.clearcoat_map, "#define USE_CLEARCOATMAP"),
        line(p.clearcoat_roughness_map, "#define USE_CLEARCOAT_ROUGHNESSMAP"),
        line(p.clearcoat_normal_map, "#define USE_CLEARCOAT_NORMALMAP"),
        line(p.dispersion, "#define USE_DISPERSION"),
        line(p.iridescence, "#define USE_IRIDESCENCE"),
        line(p.iridescence_map, "#define USE_IRIDESCENCEMAP"),
        line(p.iridescence_thickness_map, "#define USE_IRIDESCENCE_THICKNESSMAP"),
        line(p.specular_map, "#define USE_SPECULARMAP"),
        line(p.specular_color_map, "#define USE_SPECULAR_COLORMAP"),
        line(p.specular_intensity_map, "#define USE_SPECULAR_INTENSITYMAP"),
        line(p.roughness_map, "#define USE_ROUGHNESSMAP"),
        line(p.metalness_map, "#define USE_METALNESSMAP"),
        line(p.alpha_map, "#define USE_ALPHAMAP"),
        line(p.alpha_test, "#define USE_ALPHATEST"),
        line(p.alpha_hash, "#define USE_ALPHAHASH"),
        line(p.sheen, "#define USE_SHEEN"),
        line(p.sheen_color_map, "#define USE_SHEEN_COLORMAP"),
        line(p.sheen_roughness_map, "#define USE_SHEEN_ROUGHNESSMAP"),
        line(p.transmission, "#define USE_TRANSMISSION"),
        line(p.transmission_map, "#define USE_TRANSMISSIONMAP"),
        line(p.thickness_map, "#define USE_THICKNESSMAP"),
        line(p.vertex_tangents && !p.flat_shading, "#define USE_TANGENT"),
        line(p.vertex_colors || p.instancing_color || p.batching_color, "#define USE_COLOR"),
        line(p.vertex_alphas, "#define USE_COLOR_ALPHA"),
        line(p.vertex_uv1s, "#define USE_UV1"),
        line(p.vertex_uv2s, "#define USE_UV2"),
        line(p.vertex_uv3s, "#define USE_UV3"),
        line(p.points_uvs, "#define USE_POINTS_UV"),
        line(p.gradient_map, "#define USE_GRADIENTMAP"),
        line(p.flat_shading, "#define FLAT_SHADED"),
        line(p.double_sided, "#define DOUBLE_SIDED"),
        line(p.flip_sided, "#define FLIP_SIDED"),
        line(p.shadow_map_enabled, "#define USE_SHADOWMAP"),
        line(p.shadow_map_enabled, &format!("#define {shadow_type}")),
        line(p.premultiplied_alpha, "#define PREMULTIPLIED_ALPHA"),
        line(p.num_light_probes > 0, "#define USE_LIGHT_PROBES"),
        line(p.decode_video_texture, "#define DECODE_VIDEO_TEXTURE"),
        line(p.decode_video_texture_emissive, "#define DECODE_VIDEO_TEXTURE_EMISSIVE"),
        line(p.logarithmic_depth_buffer, "#define USE_LOGARITHMIC_DEPTH_BUFFER"),
        line(p.reversed_depth_buffer, "#define USE_REVERSED_DEPTH_BUFFER"),
        "uniform mat4 viewMatrix;".into(),
        "uniform vec3 cameraPosition;".into(),
        "uniform bool isOrthographic;".into(),
        line(tone, "#define TONE_MAPPING"),
        line(tone, tonemapping_pars),
        if tone {
            "vec3 toneMapping( vec3 color ) { return ACESFilmicToneMapping( color ); }".into()
        } else {
            String::new()
        },
        line(p.dithering, "#define DITHERING"),
        line(p.opaque, "#define OPAQUE"),
        colorspace_pars.to_owned(),
        texel_encoding(p.output_srgb),
        luminance().to_owned(),
        if p.use_depth_packing {
            format!("#define DEPTH_PACKING {}", p.depth_packing)
        } else {
            String::new()
        },
        "\n".into(),
    ]);
    let mut vertex = resolve_includes(&p.vertex_shader)?;
    vertex = replace_light_nums(&vertex, p);
    vertex = replace_clipping_plane_nums(&vertex, p);
    let mut fragment = resolve_includes(&p.fragment_shader)?;
    fragment = replace_light_nums(&fragment, p);
    fragment = replace_clipping_plane_nums(&fragment, p);
    vertex = unroll_loops(&vertex);
    fragment = unroll_loops(&fragment);
    // GLSL 3.0 conversion for built-in materials.
    let vertex_extensions = join(vec![
        line(p.extension_clip_cull_distance, "#extension GL_ANGLE_clip_cull_distance : require"),
        line(p.extension_multi_draw, "#extension GL_ANGLE_multi_draw : require"),
    ]);
    let prefix_vertex = [
        vertex_extensions.as_str(),
        "#define attribute in",
        "#define varying out",
        "#define texture2D texture",
    ]
    .join("\n")
        + "\n"
        + prefix_vertex.as_str();
    let prefix_fragment = [
        "#define varying in",
        "layout(location = 0) out highp vec4 pc_fragColor;",
        "#define gl_FragColor pc_fragColor",
        "#define gl_FragDepthEXT gl_FragDepth",
        "#define texture2D texture",
        "#define textureCube texture",
        "#define texture2DProj textureProj",
        "#define texture2DLodEXT textureLod",
        "#define texture2DProjLodEXT textureProjLod",
        "#define textureCubeLodEXT textureLod",
        "#define texture2DGradEXT textureGrad",
        "#define texture2DProjGradEXT textureProjGrad",
        "#define textureCubeGradEXT textureGrad",
    ]
    .join("\n")
        + "\n"
        + prefix_fragment.as_str();
    let version = "#version 300 es\n";
    Ok((
        format!("{version}{prefix_vertex}{vertex}"),
        format!("{version}{prefix_fragment}{fragment}"),
    ))
}

/// The parameters of today's `MeshStandardMaterial` (the 3D runtime's
/// material for primitives) with hemisphere and directional lights, ACES
/// Filmic, sRGB output, no shadows, no maps.
pub fn standard_material(
    name: &str,
    double_sided: bool,
    hemi_lights: u32,
    dir_lights: u32,
) -> Result<Parameters, String> {
    Ok(Parameters {
        shader_type: "MeshStandardMaterial".into(),
        shader_name: name.into(),
        defines: vec![("STANDARD".into(), String::new())],
        vertex_shader: chunk("meshphysical_vert").ok_or("meshphysical_vert")?.into(),
        fragment_shader: chunk("meshphysical_frag").ok_or("meshphysical_frag")?.into(),
        tone_mapping: ToneMapping::AcesFilmic,
        opaque: true,
        output_srgb: true,
        double_sided,
        num_hemi_lights: hemi_lights,
        num_dir_lights: dir_lights,
        ..Parameters::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_program_resolves_and_unrolls() {
        let p = standard_material("Surface", true, 1, 1).unwrap();
        let (v, f) = sources(&p).unwrap();
        assert!(v.starts_with("#version 300 es\n"));
        assert!(v.contains("\n#define attribute in\n"));
        assert!(f.contains("#define SHADER_NAME Surface"));
        assert!(f.contains("#define STANDARD \n") || f.contains("#define STANDARD "));
        assert!(!v.contains("#include"));
        assert!(!f.contains("#include"));
        assert!(!f.contains("#pragma unroll_loop_start"));
        assert!(f.contains("directionalLights[ 0 ]"));
        assert!(f.contains("ACESFilmicToneMapping( color )"));
        assert!(f.contains("#define OPAQUE"));
        assert!(f.contains("#define DOUBLE_SIDED"));
    }

    #[test]
    fn missing_includes_are_errors() {
        assert!(resolve_includes("#include <nope>").is_err());
    }
}
