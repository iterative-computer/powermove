//! GLSL ES 3.00 to GLSL 4.50 source normalization (plan section 11, step 2).
//!
//! Every edit keeps the line count unchanged so glslang's line numbers are the
//! program's line numbers; `errors::map` turns those into the author's lines.
//!
//! Rules:
//! * `#version 300 es` becomes `#version 450`. Anything else is not a WebGL2
//!   program the compositor can link (its vertex shader is 300 es), so it is
//!   rejected as WebGL does.
//! * `precision` statements are blanked and `lowp`/`mediump`/`highp` removed.
//! * Identifiers that are keywords or built-in functions in GLSL 4.50 but plain
//!   identifiers in GLSL ES 3.00 are renamed with a `pm_` prefix. In a valid ES
//!   program every such token is user-defined, so the rename cannot change meaning.
//! * `gl_FragCoord` is left alone: the engine runs legacy GLSL in GL row order
//!   (see S3.md, "Origin"), where Metal's `[[position]]` equals GL's
//!   `gl_FragCoord` and `dFdy` keeps WebGL's sign.

/// GLSL 4.50 keywords that are legal identifiers in GLSL ES 3.00.
const KEYWORDS_450_ONLY: &[&str] = &[
    "buffer",
    "shared",
    "precise",
    "dmat2",
    "dmat3",
    "dmat4",
    "dmat2x2",
    "dmat2x3",
    "dmat2x4",
    "dmat3x2",
    "dmat3x3",
    "dmat3x4",
    "dmat4x2",
    "dmat4x3",
    "dmat4x4",
    "samplerCubeArray",
    "samplerCubeArrayShadow",
    "isamplerCubeArray",
    "usamplerCubeArray",
    "imageCubeArray",
    "iimageCubeArray",
    "uimageCubeArray",
    "iimage2DRect",
    "uimage2DRect",
    "image2DRect",
    "image2DMS",
    "iimage2DMS",
    "uimage2DMS",
    "image2DMSArray",
    "iimage2DMSArray",
    "uimage2DMSArray",
    // `dvec2`, `dvec3`, `dvec4` are not here: GLSL ES 3.00 reserves them, so
    // WebGL refuses them as identifiers, and so must the engine (coverage
    // sweep, `identifiers/local-dvec2`).
    // Types of the Vulkan GLSL dialect glslang parses under relaxed Vulkan
    // rules (separate textures and samplers, subpass inputs). Plain
    // identifiers in GLSL ES 3.00: `texture2D` is a common user function or
    // macro name in shaders ported from WebGL 1.
    "sampler",
    "samplerShadow",
    "texture1D",
    "texture2D",
    "texture3D",
    "textureCube",
    "texture2DRect",
    "texture1DArray",
    "texture2DArray",
    "textureBuffer",
    "texture2DMS",
    "texture2DMSArray",
    "textureCubeArray",
    "itexture1D",
    "itexture2D",
    "itexture3D",
    "itextureCube",
    "itexture2DRect",
    "itexture1DArray",
    "itexture2DArray",
    "itextureBuffer",
    "itexture2DMS",
    "itexture2DMSArray",
    "itextureCubeArray",
    "utexture1D",
    "utexture2D",
    "utexture3D",
    "utextureCube",
    "utexture2DRect",
    "utexture1DArray",
    "utexture2DArray",
    "utextureBuffer",
    "utexture2DMS",
    "utexture2DMSArray",
    "utextureCubeArray",
    "subpassInput",
    "isubpassInput",
    "usubpassInput",
    "subpassInputMS",
    "isubpassInputMS",
    "usubpassInputMS",
];

/// GLSL 4.50 built-in functions that GLSL ES 3.00 does not have. A 4.50 shader
/// may not redefine a built-in, so user functions with these names are renamed.
const BUILTINS_450_ONLY: &[&str] = &[
    "fma",
    "frexp",
    "ldexp",
    "bitCount",
    "findLSB",
    "findMSB",
    "bitfieldExtract",
    "bitfieldInsert",
    "bitfieldReverse",
    "uaddCarry",
    "usubBorrow",
    "umulExtended",
    "imulExtended",
    "textureGather",
    "textureGatherOffset",
    "textureGatherOffsets",
    "textureQueryLod",
    "textureQueryLevels",
    "textureSamples",
    "interpolateAtCentroid",
    "interpolateAtSample",
    "interpolateAtOffset",
    "packUnorm4x8",
    "packSnorm4x8",
    "unpackUnorm4x8",
    "unpackSnorm4x8",
    "packDouble2x32",
    "unpackDouble2x32",
    "noise1",
    "noise2",
    "noise3",
    "noise4",
    "dFdxFine",
    "dFdyFine",
    "dFdxCoarse",
    "dFdyCoarse",
    "fwidthFine",
    "fwidthCoarse",
    "memoryBarrier",
    "memoryBarrierAtomicCounter",
    "memoryBarrierBuffer",
    "memoryBarrierShared",
    "memoryBarrierImage",
    "groupMemoryBarrier",
    "barrier",
    "atomicAdd",
    "atomicMin",
    "atomicMax",
    "atomicAnd",
    "atomicOr",
    "atomicXor",
    "atomicExchange",
    "atomicCompSwap",
    "atomicCounter",
    "atomicCounterIncrement",
    "atomicCounterDecrement",
    "imageLoad",
    "imageStore",
    "imageSize",
    "imageSamples",
    "imageAtomicAdd",
    "imageAtomicMin",
    "imageAtomicMax",
    "imageAtomicAnd",
    "imageAtomicOr",
    "imageAtomicXor",
    "imageAtomicExchange",
    "imageAtomicCompSwap",
    "EmitVertex",
    "EndPrimitive",
    "EmitStreamVertex",
    "EndStreamPrimitive",
    "texture1D",
    "texture1DProj",
    "texture1DLod",
    "texture1DProjLod",
    "texture3DLod",
    "texture3DProjLod",
    "shadow1D",
    "shadow2D",
    "shadow1DProj",
    "shadow2DProj",
    "shadow1DLod",
    "shadow2DLod",
    "shadow1DProjLod",
    "shadow2DProjLod",
    "textureCubeLod",
    "anyInvocation",
    "allInvocations",
    "allInvocationsEqual",
    "subpassLoad",
];

/// GLSL built-ins naga cannot take as glslang emits them. They are renamed to
/// `pm_` overloads injected after the version directive.
///
/// * `inverse`, `outerProduct`: naga's MSL back end has neither
///   (`UnsupportedCall("Inverse")`, `UnsupportedCall("Outer")`).
/// * `modf`: naga's SPIR-V front end builds the `__modf_result` struct without
///   registering naga's special type, so validation fails with
///   `MissingSpecialType` (coverage sweep, `builtin-common/modf*`).
/// * `normalize`, `faceforward`, `reflect`, `refract` on `float`: GLSL's
///   genType overloads include scalars, naga's built-ins take only vectors
///   (`BuiltinArgumentsInvalid("Normalize")`). The vector overloads call the
///   built-ins unchanged; the scalar ones use the specification's formulas.
/// * `round`, `radians`, `degrees`: kept apart from the built-ins naga folds
///   them into, so the MSL can make ANGLE's calls (`pm_render::glsl`).
///   glslang and naga give `round` and `roundEven` the same IR (`metal::rint`,
///   ties to even), while ANGLE writes `round` as `metal::round` (ties away
///   from zero: `round(2.5)` is 3 in WebGL); `pm_round` wraps `roundEven`
///   in a function the MSL writer recognizes by name and gives
///   `metal::round`. naga writes `radians(x)` as `(x) * c`; ANGLE's
///   `ANGLE_radians` is `c * x` (the constant first), and likewise `degrees`.
const POLYFILLS: &[(&str, &str)] = &[
    (
        // Same expressions and evaluation order as ANGLE's Metal translator
        // (`ANGLE_inverse`, the reference renderer), including the reciprocal
        // of the determinant, so rounding follows WebGL.
        "inverse",
        "mat2 pm_inverse(mat2 m){ mat2 adj = mat2(m[1][1], -m[0][1], -m[1][0], m[0][0]); \
float det = (adj[0][0] * m[0][0]) + (adj[0][1] * m[1][0]); return adj * (1.0 / det); } \
mat3 pm_inverse(mat3 m){ \
float a = m[1][1] * m[2][2] - m[2][1] * m[1][2]; float b = m[1][0] * m[2][2]; float c = m[1][2] * m[2][0]; float d = m[1][0] * m[2][1]; \
float det = m[0][0] * (a) - m[0][1] * (b - c) + m[0][2] * (d - m[1][1] * m[2][0]); det = 1.0 / det; mat3 minv; \
minv[0][0] = (a) * det; minv[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * det; minv[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * det; \
minv[1][0] = (c - b) * det; minv[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * det; minv[1][2] = (m[1][0] * m[0][2] - m[0][0] * m[1][2]) * det; \
minv[2][0] = (d - m[2][0] * m[1][1]) * det; minv[2][1] = (m[2][0] * m[0][1] - m[0][0] * m[2][1]) * det; minv[2][2] = (m[0][0] * m[1][1] - m[1][0] * m[0][1]) * det; \
return minv; } \
mat4 pm_inverse(mat4 m){ \
float A2323 = m[2][2] * m[3][3] - m[2][3] * m[3][2]; float A1323 = m[2][1] * m[3][3] - m[2][3] * m[3][1]; float A1223 = m[2][1] * m[3][2] - m[2][2] * m[3][1]; \
float A0323 = m[2][0] * m[3][3] - m[2][3] * m[3][0]; float A0223 = m[2][0] * m[3][2] - m[2][2] * m[3][0]; float A0123 = m[2][0] * m[3][1] - m[2][1] * m[3][0]; \
float A2313 = m[1][2] * m[3][3] - m[1][3] * m[3][2]; float A1313 = m[1][1] * m[3][3] - m[1][3] * m[3][1]; float A1213 = m[1][1] * m[3][2] - m[1][2] * m[3][1]; \
float A2312 = m[1][2] * m[2][3] - m[1][3] * m[2][2]; float A1312 = m[1][1] * m[2][3] - m[1][3] * m[2][1]; float A1212 = m[1][1] * m[2][2] - m[1][2] * m[2][1]; \
float A0313 = m[1][0] * m[3][3] - m[1][3] * m[3][0]; float A0213 = m[1][0] * m[3][2] - m[1][2] * m[3][0]; float A0312 = m[1][0] * m[2][3] - m[1][3] * m[2][0]; \
float A0212 = m[1][0] * m[2][2] - m[1][2] * m[2][0]; float A0113 = m[1][0] * m[3][1] - m[1][1] * m[3][0]; float A0112 = m[1][0] * m[2][1] - m[1][1] * m[2][0]; \
float a = m[1][1] * A2323 - m[1][2] * A1323 + m[1][3] * A1223; float b = m[1][0] * A2323 - m[1][2] * A0323 + m[1][3] * A0223; \
float c = m[1][0] * A1323 - m[1][1] * A0323 + m[1][3] * A0123; float d = m[1][0] * A1223 - m[1][1] * A0223 + m[1][2] * A0123; \
float det = m[0][0] * (a) - m[0][1] * (b) + m[0][2] * (c) - m[0][3] * (d); det = 1.0 / det; mat4 im; \
im[0][0] = det * (a); im[0][1] = det * -(m[0][1] * A2323 - m[0][2] * A1323 + m[0][3] * A1223); \
im[0][2] = det * (m[0][1] * A2313 - m[0][2] * A1313 + m[0][3] * A1213); im[0][3] = det * -(m[0][1] * A2312 - m[0][2] * A1312 + m[0][3] * A1212); \
im[1][0] = det * -(b); im[1][1] = det * (m[0][0] * A2323 - m[0][2] * A0323 + m[0][3] * A0223); \
im[1][2] = det * -(m[0][0] * A2313 - m[0][2] * A0313 + m[0][3] * A0213); im[1][3] = det * (m[0][0] * A2312 - m[0][2] * A0312 + m[0][3] * A0212); \
im[2][0] = det * (c); im[2][1] = det * -(m[0][0] * A1323 - m[0][1] * A0323 + m[0][3] * A0123); \
im[2][2] = det * (m[0][0] * A1313 - m[0][1] * A0313 + m[0][3] * A0113); im[2][3] = det * -(m[0][0] * A1312 - m[0][1] * A0312 + m[0][3] * A0112); \
im[3][0] = det * -(d); im[3][1] = det * (m[0][0] * A1223 - m[0][1] * A0223 + m[0][2] * A0123); \
im[3][2] = det * -(m[0][0] * A1213 - m[0][1] * A0213 + m[0][2] * A0113); im[3][3] = det * (m[0][0] * A1212 - m[0][1] * A0212 + m[0][2] * A0112); \
return im; }",
    ),
    (
        "outerProduct",
        "mat2 pm_outerProduct(vec2 c, vec2 r){ return mat2(c * r.x, c * r.y); } \
mat3 pm_outerProduct(vec3 c, vec3 r){ return mat3(c * r.x, c * r.y, c * r.z); } \
mat4 pm_outerProduct(vec4 c, vec4 r){ return mat4(c * r.x, c * r.y, c * r.z, c * r.w); } \
mat2x3 pm_outerProduct(vec3 c, vec2 r){ return mat2x3(c * r.x, c * r.y); } \
mat3x2 pm_outerProduct(vec2 c, vec3 r){ return mat3x2(c * r.x, c * r.y, c * r.z); } \
mat2x4 pm_outerProduct(vec4 c, vec2 r){ return mat2x4(c * r.x, c * r.y); } \
mat4x2 pm_outerProduct(vec2 c, vec4 r){ return mat4x2(c * r.x, c * r.y, c * r.z, c * r.w); } \
mat3x4 pm_outerProduct(vec4 c, vec3 r){ return mat3x4(c * r.x, c * r.y, c * r.z); } \
mat4x3 pm_outerProduct(vec3 c, vec4 r){ return mat4x3(c * r.x, c * r.y, c * r.z, c * r.w); }",
    ),
    (
        // GLSL: the whole part (`trunc`) goes to `i`, the fraction with the
        // sign of `x` is returned; an infinite `x` has fraction 0, as
        // `metal::modf` (ANGLE's translation) and C's `modf` return.
        "modf",
        "float pm_modf(float x, out float i){ i = trunc(x); return isinf(x) ? 0.0 : x - i; } \
vec2 pm_modf(vec2 x, out vec2 i){ i = trunc(x); return mix(x - i, vec2(0.0), isinf(x)); } \
vec3 pm_modf(vec3 x, out vec3 i){ i = trunc(x); return mix(x - i, vec3(0.0), isinf(x)); } \
vec4 pm_modf(vec4 x, out vec4 i){ i = trunc(x); return mix(x - i, vec4(0.0), isinf(x)); }",
    ),
    (
        "round",
        "float pm_round(float x){ return roundEven(x); } vec2 pm_round(vec2 x){ return roundEven(x); } \
vec3 pm_round(vec3 x){ return roundEven(x); } vec4 pm_round(vec4 x){ return roundEven(x); }",
    ),
    (
        "radians",
        "float pm_radians(float x){ return 1.7453292519943295e-2 * x; } vec2 pm_radians(vec2 x){ return vec2(1.7453292519943295e-2) * x; } \
vec3 pm_radians(vec3 x){ return vec3(1.7453292519943295e-2) * x; } vec4 pm_radians(vec4 x){ return vec4(1.7453292519943295e-2) * x; }",
    ),
    (
        "degrees",
        "float pm_degrees(float x){ return 57.29577951308232 * x; } vec2 pm_degrees(vec2 x){ return vec2(57.29577951308232) * x; } \
vec3 pm_degrees(vec3 x){ return vec3(57.29577951308232) * x; } vec4 pm_degrees(vec4 x){ return vec4(57.29577951308232) * x; }",
    ),
    (
        "normalize",
        "float pm_normalize(float x){ return sign(x); } \
vec2 pm_normalize(vec2 x){ return normalize(x); } vec3 pm_normalize(vec3 x){ return normalize(x); } vec4 pm_normalize(vec4 x){ return normalize(x); }",
    ),
    (
        "faceforward",
        "float pm_faceforward(float n, float i, float r){ return r * i < 0.0 ? n : -n; } \
vec2 pm_faceforward(vec2 n, vec2 i, vec2 r){ return faceforward(n, i, r); } vec3 pm_faceforward(vec3 n, vec3 i, vec3 r){ return faceforward(n, i, r); } \
vec4 pm_faceforward(vec4 n, vec4 i, vec4 r){ return faceforward(n, i, r); }",
    ),
    (
        "reflect",
        "float pm_reflect(float i, float n){ return i - 2.0 * (n * i) * n; } \
vec2 pm_reflect(vec2 i, vec2 n){ return reflect(i, n); } vec3 pm_reflect(vec3 i, vec3 n){ return reflect(i, n); } vec4 pm_reflect(vec4 i, vec4 n){ return reflect(i, n); }",
    ),
    (
        "refract",
        "float pm_refract(float i, float n, float eta){ float d = n * i; float k = 1.0 - eta * eta * (1.0 - d * d); return k < 0.0 ? 0.0 : eta * i - (eta * d + sqrt(k)) * n; } \
vec2 pm_refract(vec2 i, vec2 n, float eta){ return refract(i, n, eta); } vec3 pm_refract(vec3 i, vec3 n, float eta){ return refract(i, n, eta); } \
vec4 pm_refract(vec4 i, vec4 n, float eta){ return refract(i, n, eta); }",
    ),
];

#[derive(Debug, Clone)]
pub struct NormalizeError(pub String);

#[derive(Debug, Clone)]
pub struct Normalized {
    pub source: String,
    /// Identifiers renamed to avoid GLSL 4.50 keywords or built-ins.
    pub renamed: Vec<String>,
    /// Built-ins replaced with injected `pm_` polyfills.
    pub polyfilled: Vec<String>,
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Replace every whole-token occurrence of `from` with `to` outside comments.
fn rename_token(source: &str, from: &str, to: &str) -> (String, bool) {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        // Skip comments verbatim.
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let end = source[i..].find('\n').map_or(bytes.len(), |e| i + e);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let end = source[i + 2..]
                .find("*/")
                .map_or(bytes.len(), |e| i + 2 + e + 2);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        if is_ident_char(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let token = &source[start..i];
            if token == from {
                out.push_str(to);
                changed = true;
            } else {
                out.push_str(token);
            }
            continue;
        }
        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, changed)
}

/// Blank `precision <q> <type>;` statements (keeping newlines) and drop
/// precision qualifiers elsewhere.
fn strip_precision(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let mut line = line.to_string();
        while let Some(start) = find_token(&line, "precision") {
            let Some(end) = line[start..].find(';') else {
                break;
            };
            let blank: String = line[start..start + end + 1]
                .chars()
                .map(|c| if c == '\n' { '\n' } else { ' ' })
                .collect();
            line.replace_range(start..start + end + 1, &blank);
        }
        for q in ["highp", "mediump", "lowp"] {
            while let Some(at) = find_token(&line, q) {
                line.replace_range(at..at + q.len(), &" ".repeat(q.len()));
            }
        }
        out.push_str(&line);
    }
    out
}

fn find_token(haystack: &str, token: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(token) {
        let at = from + pos;
        let before = at == 0 || !is_ident_char(bytes[at - 1]);
        let after = at + token.len() >= bytes.len() || !is_ident_char(bytes[at + token.len()]);
        // Ignore matches inside a line comment.
        let comment = haystack[..at]
            .rfind("//")
            .is_some_and(|c| !haystack[c..at].contains('\n'));
        if before && after && !comment {
            return Some(at);
        }
        from = at + token.len();
    }
    None
}

/// Insert `text` (one physical line) after the
/// version directive and any leading `#extension` lines, followed by a `#line`
/// directive so every later line keeps its original number.
fn inject(body: &str, text: &str) -> String {
    let mut out = String::with_capacity(body.len() + text.len() + 16);
    let mut line_number = 0u32;
    let mut injected = false;
    for line in body.split_inclusive('\n') {
        line_number += 1;
        let trimmed = line.trim();
        let skip = line_number == 1
            || trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("#extension");
        if !injected && !skip {
            out.push_str(text);
            out.push('\n');
            // GLSL 3.30+ and ES 3.00: `#line N` numbers the following line N.
            out.push_str(&format!("#line {line_number}\n"));
            injected = true;
        }
        out.push_str(line);
    }
    if !injected {
        out.push('\n');
        out.push_str(text);
        out.push('\n');
    }
    out
}

/// Rename identifiers that are legal in WebGL's GLSL ES 3.00 but keywords or
/// built-in functions to glslang (`shared`, `buffer`, `precise`, and
/// `textureGather`, which glslang's ES 3.00 front end declares although it is
/// GLSL ES 3.10), leaving the version directive alone. Used before the ES
/// validation step so glslang does not reject what ANGLE accepts. A call to
/// one of these names without a user definition is still refused (as an
/// undeclared `pm_` function).
pub fn rename_reserved(source: &str) -> String {
    let mut body = source.to_string();
    for name in KEYWORDS_450_ONLY.iter().chain(BUILTINS_450_ONLY) {
        if find_token(&body, name).is_some() {
            body = rename_token(&body, name, &format!("pm_{name}")).0;
        }
    }
    body
}

pub fn normalize(source: &str) -> Result<Normalized, NormalizeError> {
    // The version directive must be the first non-whitespace token (GLSL ES 3.00 §3.3).
    let trimmed = source.trim_start();
    let leading = &source[..source.len() - trimmed.len()];
    let first_line_end = trimmed.find('\n').unwrap_or(trimmed.len());
    let first_line = trimmed[..first_line_end].trim();
    let mut parts = first_line.split_whitespace();
    if parts.next() != Some("#version") || parts.next() != Some("300") || parts.next() != Some("es")
    {
        return Err(NormalizeError(format!(
            "expected `#version 300 es` as the first line, found `{}`",
            first_line.chars().take(60).collect::<String>()
        )));
    }
    let mut body = format!("{leading}#version 450{}", &trimmed[first_line_end..]);
    body = strip_precision(&body);
    let mut renamed = Vec::new();
    for name in KEYWORDS_450_ONLY.iter().chain(BUILTINS_450_ONLY) {
        if find_token(&body, name).is_some() {
            let (next, changed) = rename_token(&body, name, &format!("pm_{name}"));
            if changed {
                body = next;
                renamed.push((*name).to_string());
            }
        }
    }
    let mut polyfilled = Vec::new();
    let mut text = String::new();
    for (name, code) in POLYFILLS {
        if find_token(&body, name).is_some() {
            let (next, changed) = rename_token(&body, name, &format!("pm_{name}"));
            if changed {
                body = next;
                polyfilled.push((*name).to_string());
                text.push_str(code);
                text.push(' ');
            }
        }
    }
    if !text.is_empty() {
        body = inject(&body, text.trim_end());
    }
    Ok(Normalized {
        source: body,
        renamed,
        polyfilled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_version_and_precision_keeping_lines() {
        let src = "#version 300 es\nprecision highp float;\nuniform mediump float x; // highp note\nvoid main(){}\n";
        let out = normalize(src).unwrap();
        assert!(out.source.starts_with("#version 450\n"));
        assert_eq!(out.source.lines().count(), src.lines().count());
        assert!(!out.source.contains("precision highp"));
        assert!(!out.source.contains("mediump"));
        assert!(out.source.contains("// highp note"));
    }

    #[test]
    fn renames_450_only_identifiers_outside_comments() {
        let src = "#version 300 es\nfloat precise(float shared){ return shared; } // precise\nfloat noise1(float v){ return precise(v); }\n";
        let out = normalize(src).unwrap();
        assert!(out.source.contains("float pm_precise(float pm_shared)"));
        assert!(out.source.contains("// precise"));
        assert!(out.source.contains("pm_noise1"));
        assert_eq!(out.renamed.len(), 3);
    }

    #[test]
    fn polyfills_inverse_and_outer_product_for_naga_keeping_line_numbers() {
        let src = "#version 300 es\nprecision highp float;\nuniform mat3 m;\nout vec4 o;\nvoid main(){\n  mat3 i = inverse(m);\n  o = vec4(i[0] + outerProduct(vec3(1.0), vec2(2.0))[1], 1.0) + vec4(undefinedName);\n}\n";
        let naga = normalize(src).unwrap();
        assert_eq!(
            naga.polyfilled,
            vec!["inverse".to_string(), "outerProduct".to_string()]
        );
        assert!(naga.source.contains("pm_inverse(m)"));
        assert!(naga.source.starts_with("#version 450\n"));
        // glslang reports the error on the author's line 7 despite the injection.
        let error = crate::compile::compile(&naga.source, crate::compile::Stage::Fragment)
            .err()
            .expect("undefined name");
        assert!(error.log.contains("0:7:"), "{}", error.log);
    }

    #[test]
    fn polyfill_goes_after_extension_directives() {
        let src = "#version 300 es\n#extension GL_OES_standard_derivatives : enable\nuniform mat2 m;\nout vec4 o;\nvoid main(){ o = vec4(inverse(m)[0], 0.0, 1.0); }\n";
        let out = normalize(src).unwrap();
        let lines: Vec<&str> = out.source.lines().collect();
        assert!(lines[1].starts_with("#extension"));
        assert!(lines[2].starts_with("mat2 pm_inverse"));
        assert_eq!(lines[3], "#line 3");
    }

    #[test]
    fn rejects_programs_without_version_300_es() {
        assert!(normalize("void main(){ gl_FragColor = vec4(1.); }").is_err());
        assert!(normalize("#version 100\nvoid main(){}").is_err());
    }
}
