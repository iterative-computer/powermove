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
    "buffer", "shared", "precise", "dmat2", "dmat3", "dmat4", "dmat2x2", "dmat2x3", "dmat2x4", "dmat3x2", "dmat3x3", "dmat3x4", "dmat4x2",
    "dmat4x3", "dmat4x4", "samplerCubeArray", "samplerCubeArrayShadow", "isamplerCubeArray", "usamplerCubeArray", "imageCubeArray",
    "iimageCubeArray", "uimageCubeArray", "iimage2DRect", "uimage2DRect", "image2DRect", "image2DMS", "iimage2DMS", "uimage2DMS",
    "image2DMSArray", "iimage2DMSArray", "uimage2DMSArray", "dvec2", "dvec3", "dvec4",
];

/// GLSL 4.50 built-in functions that GLSL ES 3.00 does not have. A 4.50 shader
/// may not redefine a built-in, so user functions with these names are renamed.
const BUILTINS_450_ONLY: &[&str] = &[
    "fma", "frexp", "ldexp", "bitCount", "findLSB", "findMSB", "bitfieldExtract", "bitfieldInsert", "bitfieldReverse", "uaddCarry",
    "usubBorrow", "umulExtended", "imulExtended", "textureGather", "textureGatherOffset", "textureGatherOffsets", "textureQueryLod",
    "textureQueryLevels", "textureSamples", "interpolateAtCentroid", "interpolateAtSample", "interpolateAtOffset", "packUnorm4x8",
    "packSnorm4x8", "unpackUnorm4x8", "unpackSnorm4x8", "packDouble2x32", "unpackDouble2x32", "noise1", "noise2", "noise3", "noise4",
    "dFdxFine", "dFdyFine", "dFdxCoarse", "dFdyCoarse", "fwidthFine", "fwidthCoarse", "memoryBarrier", "memoryBarrierAtomicCounter",
    "memoryBarrierBuffer", "memoryBarrierShared", "memoryBarrierImage", "groupMemoryBarrier", "barrier", "atomicAdd", "atomicMin",
    "atomicMax", "atomicAnd", "atomicOr", "atomicXor", "atomicExchange", "atomicCompSwap", "atomicCounter", "atomicCounterIncrement",
    "atomicCounterDecrement", "imageLoad", "imageStore", "imageSize", "imageSamples", "imageAtomicAdd", "imageAtomicMin",
    "imageAtomicMax", "imageAtomicAnd", "imageAtomicOr", "imageAtomicXor", "imageAtomicExchange", "imageAtomicCompSwap", "EmitVertex",
    "EndPrimitive", "EmitStreamVertex", "EndStreamPrimitive", "texture1D", "texture1DProj", "texture1DLod", "texture1DProjLod",
    "texture3DLod", "texture3DProjLod", "shadow1D", "shadow2D", "shadow1DProj", "shadow2DProj", "shadow1DLod", "shadow2DLod",
    "shadow1DProjLod", "shadow2DProjLod", "textureCubeLod", "anyInvocation", "allInvocations", "allInvocationsEqual", "subpassLoad",
];

#[derive(Debug, Clone)]
pub struct NormalizeError(pub String);

#[derive(Debug, Clone)]
pub struct Normalized {
    pub source: String,
    /// Identifiers renamed to avoid GLSL 4.50 keywords or built-ins.
    pub renamed: Vec<String>,
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
            let end = source[i + 2..].find("*/").map_or(bytes.len(), |e| i + 2 + e + 2);
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
        loop {
            let Some(start) = find_token(&line, "precision") else { break };
            let Some(end) = line[start..].find(';') else { break };
            let blank: String = line[start..start + end + 1].chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect();
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
        let comment = haystack[..at].rfind("//").is_some_and(|c| !haystack[c..at].contains('\n'));
        if before && after && !comment {
            return Some(at);
        }
        from = at + token.len();
    }
    None
}

pub fn normalize(source: &str) -> Result<Normalized, NormalizeError> {
    // The version directive must be the first non-whitespace token (GLSL ES 3.00 §3.3).
    let trimmed = source.trim_start();
    let leading = &source[..source.len() - trimmed.len()];
    let first_line_end = trimmed.find('\n').unwrap_or(trimmed.len());
    let first_line = trimmed[..first_line_end].trim();
    let mut parts = first_line.split_whitespace();
    if parts.next() != Some("#version") || parts.next() != Some("300") || parts.next() != Some("es") {
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
    Ok(Normalized { source: body, renamed })
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
    fn rejects_programs_without_version_300_es() {
        assert!(normalize("void main(){ gl_FragColor = vec4(1.); }").is_err());
        assert!(normalize("#version 100\nvoid main(){}").is_err());
    }
}
