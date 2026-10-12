//! Hand-written WGSL for three.js r180's `MeshStandardMaterial` (the S7 comparison path).
//!
//! A port of the program three.js generates for the standard material, written directly
//! in WGSL: directional lights with PCF-soft shadows (RGBA-packed depth, nine-tap
//! bilinear-weighted compare), hemisphere and ambient light, the physical BRDF (GGX with
//! Smith-correlated visibility, Lambert, the multiscattering DFG approximation), ACES
//! Filmic with r180's `exposure / 0.6` pre-scale, and the piecewise sRGB output.
//!
//! The variant is read from the GLSL three.js generated (its feature `#define`s and the
//! light array sizes it substituted). Variants outside this subset (maps, point, spot,
//! and area lights, skinning, morphs, instancing, fog, flat shading, clipping, alpha test,
//! other tone mappings) return `None`, so the replay keeps the compiled GLSL for them.

use anyhow::Result;

/// Feature defines this port implements. Everything else three.js can define for the
/// standard material makes the override decline.
const SUPPORTED_DEFINES: &[&str] = &["STANDARD", "DOUBLE_SIDED", "USE_SHADOWMAP", "SHADOWMAP_TYPE_PCF_SOFT", "TONE_MAPPING", "OPAQUE"];

/// Defines present in every three.js r180 program (precision, helpers, constants, GLSL
/// 3 aliases, and the physical lighting model's macro names).
const INFRASTRUCTURE_DEFINES: &[&str] = &[
    "varying", "attribute", "gl_FragColor", "gl_FragDepthEXT", "texture2D", "textureCube", "texture2DProj", "texture2DLodEXT", "texture2DProjLodEXT", "textureCubeLodEXT",
    "texture2DGradEXT", "texture2DProjGradEXT", "textureCubeGradEXT", "HIGH_PRECISION", "SHADER_TYPE", "SHADER_NAME", "saturate", "PI", "PI2", "PI_HALF", "RECIPROCAL_PI",
    "RECIPROCAL_PI2", "EPSILON", "whiteComplement", "RE_Direct", "RE_Direct_RectArea", "RE_IndirectDiffuse", "RE_IndirectSpecular", "Material_LightProbeLOD",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub dir_lights: usize,
    pub dir_shadows: usize,
    pub hemi_lights: usize,
    pub double_sided: bool,
    pub opaque: bool,
}

/// Feature defines: three.js puts every program parameter in the prefix, before the
/// first uniform declaration. Later `#define`s live inside shader chunks (often in
/// inactive `#ifdef` branches) and say nothing about the variant.
fn defines(source: &str) -> Vec<String> {
    source
        .lines()
        .take_while(|l| !l.trim_start().starts_with("uniform "))
        .filter_map(|l| l.trim().strip_prefix("#define "))
        .filter_map(|rest| rest.split(|c: char| c.is_whitespace() || c == '(').next())
        .map(str::to_string)
        .collect()
}

/// `uniform T name[ N ];` → N (0 when absent).
fn array_len(source: &str, name: &str) -> usize {
    for line in source.lines() {
        let t = line.trim();
        if !t.starts_with("uniform ") {
            continue;
        }
        let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(pos) = compact.find(&format!("{name}[")) {
            let rest = &compact[pos + name.len() + 1..];
            if let Some(end) = rest.find(']') {
                if let Ok(n) = rest[..end].parse() {
                    return n;
                }
            }
        }
    }
    0
}

/// Why [`variant`] declines a program (empty when it is covered).
pub fn explain(vs: &str, fs: &str) -> Vec<String> {
    let mut reasons = Vec::new();
    if !fs.contains("#define SHADER_TYPE MeshStandardMaterial") {
        reasons.push("not MeshStandardMaterial".into());
    }
    for d in defines(vs).iter().chain(defines(fs).iter()) {
        if !SUPPORTED_DEFINES.contains(&d.as_str()) && !INFRASTRUCTURE_DEFINES.contains(&d.as_str()) {
            reasons.push(format!("define {d}"));
        }
    }
    for name in ["pointLights", "spotLights", "rectAreaLights", "pointLightShadows", "spotLightShadows"] {
        if array_len(fs, name) > 0 {
            reasons.push(format!("{name}[{}]", array_len(fs, name)));
        }
    }
    let compact: String = fs.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.contains("vec3toneMapping(vec3color){returnACESFilmicToneMapping(color);}") {
        reasons.push("tone mapping is not ACES Filmic".into());
    }
    if !compact.contains("returnsRGBTransferOETF(") {
        reasons.push("output transfer is not sRGB".into());
    }
    reasons
}

/// Reads the variant three.js compiled, or `None` when the port does not cover it.
pub fn variant(vs: &str, fs: &str) -> Option<Variant> {
    if !fs.contains("#define SHADER_TYPE MeshStandardMaterial") {
        return None;
    }
    for d in defines(vs).iter().chain(defines(fs).iter()) {
        if !SUPPORTED_DEFINES.contains(&d.as_str()) && !INFRASTRUCTURE_DEFINES.contains(&d.as_str()) {
            return None;
        }
    }
    // Light kinds the port does not implement.
    for name in ["pointLights", "spotLights", "rectAreaLights", "pointLightShadows", "spotLightShadows"] {
        if array_len(fs, name) > 0 {
            return None;
        }
    }
    // Tone mapping and output transfer must be the ones the port implements.
    let compact: String = fs.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.contains("vec3toneMapping(vec3color){returnACESFilmicToneMapping(color);}") || !compact.contains("returnsRGBTransferOETF(") {
        return None;
    }
    let fs_defines = defines(fs);
    let shadows = fs_defines.iter().any(|d| d == "USE_SHADOWMAP");
    if shadows && !fs_defines.iter().any(|d| d == "SHADOWMAP_TYPE_PCF_SOFT") {
        return None;
    }
    Some(Variant {
        dir_lights: array_len(fs, "directionalLights"),
        dir_shadows: if shadows { array_len(fs, "directionalLightShadows") } else { 0 },
        hemi_lights: array_len(fs, "hemisphereLights"),
        double_sided: fs_defines.iter().any(|d| d == "DOUBLE_SIDED"),
        opaque: fs_defines.iter().any(|d| d == "OPAQUE"),
    })
}

/// The uniform block shared by both stages; member names are three.js's uniform names.
fn uniforms(v: &Variant) -> String {
    let mut s = String::from(
        "struct DirectionalLight { direction: vec3f, color: vec3f }\n\
         struct HemisphereLight { direction: vec3f, skyColor: vec3f, groundColor: vec3f }\n\
         struct DirectionalLightShadow { shadowIntensity: f32, shadowBias: f32, shadowNormalBias: f32, shadowRadius: f32, @size(16) shadowMapSize: vec2f }\n\
         struct Uniforms {\n  modelMatrix: mat4x4f,\n  modelViewMatrix: mat4x4f,\n  projectionMatrix: mat4x4f,\n  viewMatrix: mat4x4f,\n  normalMatrix: mat3x3f,\n  cameraPosition: vec3f,\n  isOrthographic: u32,\n  toneMappingExposure: f32,\n  diffuse: vec3f,\n  emissive: vec3f,\n  roughness: f32,\n  metalness: f32,\n  opacity: f32,\n  ambientLightColor: vec3f,\n  receiveShadow: u32,\n",
    );
    if v.dir_lights > 0 {
        s += &format!("  directionalLights: array<DirectionalLight, {}>,\n", v.dir_lights);
    }
    if v.hemi_lights > 0 {
        s += &format!("  hemisphereLights: array<HemisphereLight, {}>,\n", v.hemi_lights);
    }
    if v.dir_shadows > 0 {
        s += &format!("  directionalLightShadows: array<DirectionalLightShadow, {}>,\n", v.dir_shadows);
        s += &format!("  directionalShadowMatrix: array<mat4x4f, {}>,\n", v.dir_shadows);
    }
    s += "}\n@group(0) @binding(0) var<uniform> u: Uniforms;\n";
    s
}

fn varyings(v: &Variant) -> String {
    let mut s = String::from("  @location(0) vViewPosition: vec3f,\n  @location(1) vNormal: vec3f,\n");
    for i in 0..v.dir_shadows {
        s += &format!("  @location({}) vDirectionalShadowCoord_pm{i}: vec4f,\n", 2 + i);
    }
    s
}

pub fn vertex_wgsl(v: &Variant) -> String {
    let mut s = uniforms(v);
    s += &format!("struct VertexOut {{\n  @builtin(position) position: vec4f,\n{}}}\n", varyings(v));
    s += r#"
fn inverseTransformDirection(dir: vec3f, m: mat4x4f) -> vec3f {
  return normalize((vec4f(dir, 0.0) * m).xyz);
}

@vertex
fn main(@location(0) position: vec3f, @location(1) normal: vec3f) -> VertexOut {
  var out: VertexOut;
  let transformedNormal = u.normalMatrix * normal;
  out.vNormal = normalize(transformedNormal);
  let mvPosition = u.modelViewMatrix * vec4f(position, 1.0);
  var clip = u.projectionMatrix * mvPosition;
  out.vViewPosition = -mvPosition.xyz;
  let worldPosition = u.modelMatrix * vec4f(position, 1.0);
"#;
    if v.dir_shadows > 0 {
        s += "  let shadowWorldNormal = inverseTransformDirection(transformedNormal, u.viewMatrix);\n";
        for i in 0..v.dir_shadows {
            s += &format!("  out.vDirectionalShadowCoord_pm{i} = u.directionalShadowMatrix[{i}] * (worldPosition + vec4f(shadowWorldNormal * u.directionalLightShadows[{i}].shadowNormalBias, 0.0));\n");
        }
    }
    s += r#"  // Replay conventions (s7_glsl::VERTEX_EPILOGUE): GL rows, [0, w] depth.
  clip.y = -clip.y;
  clip.z = (clip.z + clip.w) * 0.5;
  out.position = clip;
  return out;
}
"#;
    s
}

pub fn fragment_wgsl(v: &Variant) -> String {
    let mut s = uniforms(v);
    s += &format!("struct FragmentIn {{\n  @builtin(front_facing) frontFacing: bool,\n{}}}\n", varyings(v));
    s += "struct FragmentOut {\n  @location(0) color: vec4f,\n  @builtin(sample_mask) mask: u32,\n}\n";
    for i in 0..v.dir_shadows {
        s += &format!("@group(0) @binding({}) var directionalShadowMap_pm{i}: texture_2d<f32>;\n@group(0) @binding({}) var directionalShadowMap_pm{i}_sampler: sampler;\n", 1 + i, 1001 + i);
    }
    s += r#"
const RECIPROCAL_PI: f32 = 0.3183098861837907;

struct PhysicalMaterial { diffuseColor: vec3f, roughness: f32, specularColor: vec3f, specularF90: f32 }
struct IncidentLight { color: vec3f, direction: vec3f, visible: bool }
struct ReflectedLight { directDiffuse: vec3f, directSpecular: vec3f, indirectDiffuse: vec3f, indirectSpecular: vec3f }

fn pow2(x: f32) -> f32 { return x * x; }

fn BRDF_Lambert(diffuseColor: vec3f) -> vec3f { return RECIPROCAL_PI * diffuseColor; }

fn F_Schlick(f0: vec3f, f90: f32, dotVH: f32) -> vec3f {
  let fresnel = exp2((-5.55473 * dotVH - 6.98316) * dotVH);
  return f0 * (1.0 - fresnel) + (f90 * fresnel);
}

fn V_GGX_SmithCorrelated(alpha: f32, dotNL: f32, dotNV: f32) -> f32 {
  let a2 = pow2(alpha);
  let gv = dotNL * sqrt(a2 + (1.0 - a2) * pow2(dotNV));
  let gl = dotNV * sqrt(a2 + (1.0 - a2) * pow2(dotNL));
  return 0.5 / max(gv + gl, 1e-6);
}

fn D_GGX(alpha: f32, dotNH: f32) -> f32 {
  let a2 = pow2(alpha);
  let denom = pow2(dotNH) * (a2 - 1.0) + 1.0;
  return RECIPROCAL_PI * a2 / pow2(denom);
}

fn BRDF_GGX(lightDir: vec3f, viewDir: vec3f, normal: vec3f, material: PhysicalMaterial) -> vec3f {
  let alpha = pow2(material.roughness);
  let halfDir = normalize(lightDir + viewDir);
  let dotNL = clamp(dot(normal, lightDir), 0.0, 1.0);
  let dotNV = clamp(dot(normal, viewDir), 0.0, 1.0);
  let dotNH = clamp(dot(normal, halfDir), 0.0, 1.0);
  let dotVH = clamp(dot(viewDir, halfDir), 0.0, 1.0);
  let F = F_Schlick(material.specularColor, material.specularF90, dotVH);
  let V = V_GGX_SmithCorrelated(alpha, dotNL, dotNV);
  let D = D_GGX(alpha, dotNH);
  return F * (V * D);
}

fn DFGApprox(normal: vec3f, viewDir: vec3f, roughness: f32) -> vec2f {
  let dotNV = clamp(dot(normal, viewDir), 0.0, 1.0);
  let c0 = vec4f(-1.0, -0.0275, -0.572, 0.022);
  let c1 = vec4f(1.0, 0.0425, 1.04, -0.04);
  let r = roughness * c0 + c1;
  let a004 = min(r.x * r.x, exp2(-9.28 * dotNV)) * r.x + r.y;
  return vec2f(-1.04, 1.04) * a004 + r.zw;
}

fn RE_Direct_Physical(directLight: IncidentLight, geometryNormal: vec3f, geometryViewDir: vec3f, material: PhysicalMaterial, reflected: ptr<function, ReflectedLight>) {
  let dotNL = clamp(dot(geometryNormal, directLight.direction), 0.0, 1.0);
  let irradiance = dotNL * directLight.color;
  (*reflected).directSpecular += irradiance * BRDF_GGX(directLight.direction, geometryViewDir, geometryNormal, material);
  (*reflected).directDiffuse += irradiance * BRDF_Lambert(material.diffuseColor);
}

fn RE_IndirectSpecular_Physical(radiance: vec3f, irradiance: vec3f, geometryNormal: vec3f, geometryViewDir: vec3f, material: PhysicalMaterial, reflected: ptr<function, ReflectedLight>) {
  // computeMultiscattering
  let fab = DFGApprox(geometryNormal, geometryViewDir, material.roughness);
  let Fr = material.specularColor;
  let FssEss = Fr * fab.x + material.specularF90 * fab.y;
  let Ess = fab.x + fab.y;
  let Ems = 1.0 - Ess;
  let Favg = Fr + (1.0 - Fr) * 0.047619;
  let Fms = FssEss * Favg / (1.0 - Ems * Favg);
  let singleScattering = FssEss;
  let multiScattering = Fms * Ems;
  let cosineWeightedIrradiance = irradiance * RECIPROCAL_PI;
  let totalScattering = singleScattering + multiScattering;
  let diffuse = material.diffuseColor * (1.0 - max(max(totalScattering.r, totalScattering.g), totalScattering.b));
  (*reflected).indirectSpecular += radiance * singleScattering;
  (*reflected).indirectSpecular += multiScattering * cosineWeightedIrradiance;
  (*reflected).indirectDiffuse += diffuse * cosineWeightedIrradiance;
}

fn RRTAndODTFit(v: vec3f) -> vec3f {
  let a = v * (v + 0.0245786) - 0.000090537;
  let b = v * (0.983729 * v + 0.4329510) + 0.238081;
  return a / b;
}

fn ACESFilmicToneMapping(c: vec3f) -> vec3f {
  let ACESInputMat = mat3x3f(vec3f(0.59719, 0.07600, 0.02840), vec3f(0.35458, 0.90834, 0.13383), vec3f(0.04823, 0.01566, 0.83777));
  let ACESOutputMat = mat3x3f(vec3f(1.60475, -0.10208, -0.00327), vec3f(-0.53108, 1.10813, -0.07276), vec3f(-0.07367, -0.00605, 1.07602));
  var color = c * (u.toneMappingExposure / 0.6);
  color = ACESInputMat * color;
  color = RRTAndODTFit(color);
  color = ACESOutputMat * color;
  return clamp(color, vec3f(0.0), vec3f(1.0));
}

fn sRGBTransferOETF(value: vec4f) -> vec4f {
  let curve = pow(value.rgb, vec3f(0.41666)) * 1.055 - vec3f(0.055);
  return vec4f(select(curve, value.rgb * 12.92, value.rgb <= vec3f(0.0031308)), value.a);
}

fn unpackRGBAToDepth(v: vec4f) -> f32 {
  let UnpackDownscale = 255.0 / 256.0;
  return dot(v, vec4f(UnpackDownscale / 1.0, UnpackDownscale / 256.0, UnpackDownscale / 65536.0, 1.0 / 16777216.0));
}
"#;
    // One PCF-soft shadow function per shadow map (WGSL has no texture parameters by value).
    for i in 0..v.dir_shadows {
        s += &format!(
            r#"
fn compare_pm{i}(uv: vec2f, compare: f32) -> f32 {{
  let depth = unpackRGBAToDepth(textureSampleLevel(directionalShadowMap_pm{i}, directionalShadowMap_pm{i}_sampler, uv, 0.0));
  return step(compare, depth);
}}

fn getShadow_pm{i}(shadowMapSize: vec2f, shadowIntensity: f32, shadowBias: f32, coord: vec4f) -> f32 {{
  var shadow = 1.0;
  var shadowCoord = vec4f(coord.xyz / coord.w, coord.w);
  shadowCoord.z += shadowBias;
  let inFrustum = shadowCoord.x >= 0.0 && shadowCoord.x <= 1.0 && shadowCoord.y >= 0.0 && shadowCoord.y <= 1.0;
  if (inFrustum && shadowCoord.z <= 1.0) {{
    let texelSize = vec2f(1.0) / shadowMapSize;
    let dx = texelSize.x;
    let dy = texelSize.y;
    var uv = shadowCoord.xy;
    let f = fract(uv * shadowMapSize + 0.5);
    uv -= f * texelSize;
    let z = shadowCoord.z;
    shadow = (
      compare_pm{i}(uv, z) +
      compare_pm{i}(uv + vec2f(dx, 0.0), z) +
      compare_pm{i}(uv + vec2f(0.0, dy), z) +
      compare_pm{i}(uv + texelSize, z) +
      mix(compare_pm{i}(uv + vec2f(-dx, 0.0), z), compare_pm{i}(uv + vec2f(2.0 * dx, 0.0), z), f.x) +
      mix(compare_pm{i}(uv + vec2f(-dx, dy), z), compare_pm{i}(uv + vec2f(2.0 * dx, dy), z), f.x) +
      mix(compare_pm{i}(uv + vec2f(0.0, -dy), z), compare_pm{i}(uv + vec2f(0.0, 2.0 * dy), z), f.y) +
      mix(compare_pm{i}(uv + vec2f(dx, -dy), z), compare_pm{i}(uv + vec2f(dx, 2.0 * dy), z), f.y) +
      mix(mix(compare_pm{i}(uv + vec2f(-dx, -dy), z), compare_pm{i}(uv + vec2f(2.0 * dx, -dy), z), f.x),
          mix(compare_pm{i}(uv + vec2f(-dx, 2.0 * dy), z), compare_pm{i}(uv + vec2f(2.0 * dx, 2.0 * dy), z), f.x), f.y)
    ) * (1.0 / 9.0);
  }}
  return mix(1.0, shadow, shadowIntensity);
}}
"#
        );
    }
    s += r#"
@fragment
fn main(fin: FragmentIn) -> FragmentOut {
  var diffuseColor = vec4f(u.diffuse, u.opacity);
  var reflected = ReflectedLight(vec3f(0.0), vec3f(0.0), vec3f(0.0), vec3f(0.0));
  let totalEmissiveRadiance = u.emissive;
  let faceDirection = select(-1.0, 1.0, fin.frontFacing);
  var normal = normalize(fin.vNormal);
"#;
    if v.double_sided {
        s += "  normal *= faceDirection;\n";
    }
    s += r#"  let nonPerturbedNormal = normal;
  var material: PhysicalMaterial;
  material.diffuseColor = diffuseColor.rgb * (1.0 - u.metalness);
  let dxy = max(abs(dpdx(nonPerturbedNormal)), abs(dpdy(nonPerturbedNormal)));
  let geometryRoughness = max(max(dxy.x, dxy.y), dxy.z);
  material.roughness = max(u.roughness, 0.0525);
  material.roughness += geometryRoughness;
  material.roughness = min(material.roughness, 1.0);
  material.specularColor = mix(vec3f(0.04), diffuseColor.rgb, u.metalness);
  material.specularF90 = 1.0;
  let geometryNormal = normal;
  let geometryViewDir = select(normalize(fin.vViewPosition), vec3f(0.0, 0.0, 1.0), u.isOrthographic != 0u);
  var directLight: IncidentLight;
"#;
    for i in 0..v.dir_lights {
        s += &format!("  directLight = IncidentLight(u.directionalLights[{i}].color, u.directionalLights[{i}].direction, true);\n");
        if i < v.dir_shadows {
            s += &format!(
                "  if (directLight.visible && u.receiveShadow != 0u) {{ directLight.color *= getShadow_pm{i}(u.directionalLightShadows[{i}].shadowMapSize, u.directionalLightShadows[{i}].shadowIntensity, u.directionalLightShadows[{i}].shadowBias, fin.vDirectionalShadowCoord_pm{i}); }}\n"
            );
        }
        s += "  RE_Direct_Physical(directLight, geometryNormal, geometryViewDir, material, &reflected);\n";
    }
    s += "  let iblIrradiance = vec3f(0.0);\n  var irradiance = u.ambientLightColor;\n";
    for i in 0..v.hemi_lights {
        s += &format!(
            "  {{ let dotNL = dot(geometryNormal, u.hemisphereLights[{i}].direction); let w = 0.5 * dotNL + 0.5; irradiance += mix(u.hemisphereLights[{i}].groundColor, u.hemisphereLights[{i}].skyColor, w); }}\n"
        );
    }
    s += "  let radiance = vec3f(0.0);\n  reflected.indirectDiffuse += irradiance * BRDF_Lambert(material.diffuseColor);\n  RE_IndirectSpecular_Physical(radiance, iblIrradiance, geometryNormal, geometryViewDir, material, &reflected);\n";
    s += r#"  let totalDiffuse = reflected.directDiffuse + reflected.indirectDiffuse;
  let totalSpecular = reflected.directSpecular + reflected.indirectSpecular;
  let outgoingLight = totalDiffuse + totalSpecular + totalEmissiveRadiance;
"#;
    if v.opaque {
        s += "  diffuseColor.a = 1.0;\n";
    }
    s += r#"  var color = vec4f(outgoingLight, diffuseColor.a);
  color = vec4f(ACESFilmicToneMapping(color.rgb), color.a);
  color = sRGBTransferOETF(color);
  // ANGLE writes an all-ones sample mask from every fragment shader (see s7_glsl).
  return FragmentOut(color, 0xffffffffu);
}
"#;
    s
}

/// Replay override: the hand-written program for supported standard-material variants.
pub fn override_standard(vs: &str, fs: &str) -> Option<Result<s7_glsl::Program>> {
    let v = variant(vs, fs)?;
    Some(s7_glsl::from_wgsl(&vertex_wgsl(&v), &fragment_wgsl(&v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_compiles_and_reflects_three_names() {
        for dir_lights in 0..3 {
            for dir_shadows in 0..=dir_lights {
                for hemi_lights in 0..2 {
                    for (double_sided, opaque) in [(false, false), (true, true)] {
                        let v = Variant { dir_lights, dir_shadows, hemi_lights, double_sided, opaque };
                        let p = s7_glsl::from_wgsl(&vertex_wgsl(&v), &fragment_wgsl(&v)).unwrap_or_else(|e| panic!("{v:?}: {e:#}"));
                        assert!(p.uniforms.contains_key("toneMappingExposure"));
                        assert_eq!(p.attributes["position"].location, 0);
                        if dir_shadows > 0 {
                            assert!(p.textures.contains_key("directionalShadowMap[0]"));
                            assert!(p.uniforms.contains_key("directionalLightShadows[0].shadowMapSize"));
                            assert_eq!(p.uniforms["directionalLightShadows[0].shadowMapSize"].offset % 16, 0);
                        }
                        if dir_lights > 1 {
                            assert_eq!(p.uniforms["directionalLights[1].color"].offset - p.uniforms["directionalLights[0].color"].offset, 32);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn declines_unsupported_variants() {
        let fs = "#define SHADER_TYPE MeshStandardMaterial\n#define USE_MAP\nvec3 toneMapping( vec3 color ) { return ACESFilmicToneMapping( color ); }\nreturn sRGBTransferOETF( x );\n";
        assert!(variant("", fs).is_none());
        let fs = "#define SHADER_TYPE MeshStandardMaterial\nuniform PointLight pointLights[ 1 ];\nvec3 toneMapping( vec3 color ) { return ACESFilmicToneMapping( color ); }\nreturn sRGBTransferOETF( x );\n";
        assert!(variant("", fs).is_none());
        let fs = "#define SHADER_TYPE MeshStandardMaterial\n#define OPAQUE\nuniform DirectionalLight directionalLights[ 2 ];\nuniform HemisphereLight hemisphereLights[ 1 ];\nvec3 toneMapping( vec3 color ) { return ACESFilmicToneMapping( color ); }\nreturn sRGBTransferOETF( x );\n";
        assert_eq!(variant("", fs), Some(Variant { dir_lights: 2, dir_shadows: 0, hemi_lights: 1, double_sided: false, opaque: true }));
    }
}
