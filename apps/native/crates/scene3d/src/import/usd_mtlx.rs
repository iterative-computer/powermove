//! Pure-Rust USD (`.usdz` / `.usdc` / `.usda`) fallback reader for RealityKit
//! MaterialX materials (`outputs:mtlx:surface`) that Apple Model I/O skips when
//! a material lacks `outputs:surface` (`UsdPreviewSurface`).
//!
//! Only invoked when Model I/O leaves a submesh with no material (`material_id == 0`).
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
};

/// Resolved MaterialX surface fallback for a mesh prim.
#[derive(Clone, Debug)]
pub struct MtlxMaterial {
    pub name: String,
    pub texture_bytes: Option<Vec<u8>>,
    pub base_color: [f32; 3],
    pub color_scale: f32,
    pub opacity: f32,
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: bool,
    pub double_sided: bool,
    pub no_flip_v: bool,
}

impl Default for MtlxMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            texture_bytes: None,
            base_color: [1., 1., 1.],
            color_scale: 1.,
            opacity: 1.,
            roughness: 0.5,
            metallic: 0.,
            unlit: false,
            double_sided: false,
            no_flip_v: true,
        }
    }
}

/// Mesh-name and material-name -> resolved MaterialX material lookup extracted
/// from USDZ/USDC/USDA.
#[derive(Default)]
pub struct UsdMtlxIndex {
    by_mesh_name: HashMap<String, MtlxMaterial>,
    by_material_name: HashMap<String, MtlxMaterial>,
}

impl UsdMtlxIndex {
    pub fn parse(ext: &str, bytes: &[u8]) -> Self {
        let mut index = Self::default();
        if ext.eq_ignore_ascii_case("usdz") || bytes.starts_with(b"PK\x03\x04") {
            index.parse_usdz(bytes);
        } else if bytes.starts_with(b"PXR-USDC") {
            let mut layer = LayerData::default();
            if parse_usdc(bytes, &mut layer).is_ok() {
                index.resolve_layer(&layer, "", &|_| None);
            }
        } else if let Ok(text) = std::str::from_utf8(bytes)
            && text.trim_start().starts_with("#usda")
        {
            let mut layer = LayerData::default();
            parse_usda(text, &mut layer);
            index.resolve_layer(&layer, "", &|_| None);
        }
        index
    }

    pub fn get(&self, mesh_name: &str) -> Option<&MtlxMaterial> {
        self.by_mesh_name.get(mesh_name)
    }

    pub fn get_material(&self, material_name: &str) -> Option<&MtlxMaterial> {
        self.by_material_name.get(material_name)
    }

    fn parse_usdz(&mut self, bytes: &[u8]) {
        let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) else {
            return;
        };
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        let mut usd_entries: Vec<String> = Vec::new();
        for i in 0..archive.len() {
            let Ok(mut file) = archive.by_index(i) else {
                continue;
            };
            if file.is_dir() || file.size() > 128 * 1024 * 1024 {
                continue;
            }
            let name = file.name().trim_start_matches("./").to_string();
            let lower = name.to_ascii_lowercase();
            let is_usd = lower.ends_with(".usdc")
                || lower.ends_with(".usda")
                || lower.ends_with(".usd");
            let mut buf = Vec::with_capacity(file.size().min(16 * 1024 * 1024) as usize);
            if file.read_to_end(&mut buf).is_ok() {
                if is_usd {
                    usd_entries.push(name.clone());
                }
                files.insert(name, buf);
            }
        }
        for entry in usd_entries {
            let Some(data) = files.get(&entry) else {
                continue;
            };
            let base_dir = entry
                .rsplit_once('/')
                .map_or("", |(dir, _)| dir);
            let mut layer = LayerData::default();
            if data.starts_with(b"PXR-USDC") {
                if parse_usdc(data, &mut layer).is_err() {
                    continue;
                }
            } else if let Ok(text) = std::str::from_utf8(data)
                && text.trim_start().starts_with("#usda")
            {
                parse_usda(text, &mut layer);
            } else {
                continue;
            }
            self.resolve_layer(&layer, base_dir, &|asset_path| {
                resolve_archive_asset(&files, base_dir, asset_path)
            });
        }
    }

    fn resolve_layer(
        &mut self,
        layer: &LayerData,
        _base_dir: &str,
        load_asset: &dyn Fn(&str) -> Option<Vec<u8>>,
    ) {
        // Resolve every Material that authors `outputs:mtlx:surface.connect`
        // (and lacks a UsdPreviewSurface `outputs:surface.connect`).
        let mut materials: HashMap<String, MtlxMaterial> = HashMap::new();
        for (mat_path, mtlx_target) in &layer.mtlx_surfaces {
            if layer.usd_surfaces.contains(mat_path) {
                continue;
            }
            if let Some(mat) = eval_mtlx_material(layer, mat_path, mtlx_target, load_asset) {
                materials.insert(mat_path.clone(), mat);
            }
        }
        if materials.is_empty() {
            return;
        }
        let mut double_sided_mats: HashSet<String> = HashSet::new();
        for (mesh_path, bound_material) in &layer.material_bindings {
            let Some(mut mat) = materials.get(bound_material).cloned() else {
                continue;
            };
            if layer.double_sided_meshes.contains(mesh_path) {
                mat.double_sided = true;
                double_sided_mats.insert(bound_material.clone());
            }
            let mesh_name = mesh_path.rsplit('/').next().unwrap_or(mesh_path);
            if !mesh_name.is_empty() {
                self.by_mesh_name.insert(mesh_name.to_string(), mat);
            }
        }
        for (mat_path, mut mat) in materials {
            if double_sided_mats.contains(&mat_path) {
                mat.double_sided = true;
            }
            if !mat.name.is_empty() {
                self.by_material_name.insert(mat.name.clone(), mat);
            }
        }
    }
}

fn resolve_archive_asset(
    files: &HashMap<String, Vec<u8>>,
    base_dir: &str,
    raw: &str,
) -> Option<Vec<u8>> {
    let cleaned = raw.trim().trim_matches('@').trim_start_matches("./");
    if cleaned.is_empty() {
        return None;
    }
    if !base_dir.is_empty() {
        let joined = normalize_rel_path(&format!("{base_dir}/{cleaned}"));
        if let Some(bytes) = files.get(&joined) {
            return Some(bytes.clone());
        }
    }
    let direct = normalize_rel_path(cleaned);
    files.get(&direct).cloned()
}

fn normalize_rel_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[derive(Clone, Debug)]
enum AttrVal {
    Bool(bool),
    Float(f32),
    Vec3([f32; 3]),
    Token(String),
    Asset(String),
}

#[derive(Default)]
struct LayerData {
    /// Material prim path -> connected shader output path for `outputs:mtlx:surface`
    mtlx_surfaces: HashMap<String, String>,
    /// Material prim paths that already author `outputs:surface.connect`
    usd_surfaces: HashSet<String>,
    /// Mesh prim path -> bound material prim path (`rel material:binding`)
    material_bindings: HashMap<String, String>,
    /// Mesh prim paths with `doubleSided = 1`
    double_sided_meshes: HashSet<String>,
    /// Shader prim path -> `info:id`
    shader_ids: HashMap<String, String>,
    /// Property path (`/a/b/shader.inputs:foo`) -> scalar/token/asset default
    defaults: HashMap<String, AttrVal>,
    /// Property path (`/a/b/shader.inputs:foo`) -> connected source property path
    connections: HashMap<String, String>,
}

fn prim_of(prop_path: &str) -> (&str, &str) {
    prop_path.split_once('.').unwrap_or((prop_path, ""))
}

fn eval_mtlx_material(
    layer: &LayerData,
    mat_path: &str,
    surface_output: &str,
    load_asset: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Option<MtlxMaterial> {
    let (shader_path, _) = prim_of(surface_output);
    let shader_id = layer.shader_ids.get(shader_path)?.as_str();
    let mut mat = MtlxMaterial {
        name: mat_path.rsplit('/').next().unwrap_or("MaterialX").to_string(),
        ..MtlxMaterial::default()
    };

    let (color_prop, opacity_prop) = if shader_id.contains("unlit") {
        mat.unlit = true;
        mat.roughness = 0.25;
        mat.metallic = 0.0;
        ("inputs:color", "inputs:opacity")
    } else if shader_id.contains("pbr") || shader_id.contains("standard_surface") || shader_id.contains("UsdPreviewSurface") {
        if let Some(AttrVal::Float(r)) = layer.defaults.get(&format!("{shader_path}.inputs:roughness")) {
            mat.roughness = r.clamp(0., 1.);
        }
        if let Some(AttrVal::Float(m)) = layer.defaults.get(&format!("{shader_path}.inputs:metallic")) {
            mat.metallic = m.clamp(0., 1.);
        }
        let color_key = if layer.connections.contains_key(&format!("{shader_path}.inputs:baseColor"))
            || layer.defaults.contains_key(&format!("{shader_path}.inputs:baseColor"))
        {
            "inputs:baseColor"
        } else if layer.connections.contains_key(&format!("{shader_path}.inputs:diffuseColor"))
            || layer.defaults.contains_key(&format!("{shader_path}.inputs:diffuseColor"))
        {
            "inputs:diffuseColor"
        } else {
            "inputs:base_color"
        };
        (color_key, "inputs:opacity")
    } else {
        ("inputs:color", "inputs:opacity")
    };

    let mut visited = HashSet::new();
    let color_trace = trace_input(layer, &format!("{shader_path}.{color_prop}"), &mut visited);
    if let Some(tex) = color_trace.texture {
        mat.texture_bytes = load_asset(&tex.file);
        mat.no_flip_v = tex.no_flip_v;
    }
    if let Some(c) = color_trace.color {
        mat.base_color = c;
    }
    mat.color_scale = color_trace.scale;

    visited.clear();
    let opacity_trace = trace_input(layer, &format!("{shader_path}.{opacity_prop}"), &mut visited);
    if mat.texture_bytes.is_none()
        && let Some(tex) = opacity_trace.texture
    {
        mat.texture_bytes = load_asset(&tex.file);
        mat.no_flip_v = tex.no_flip_v;
    }
    mat.opacity = opacity_trace.scale.clamp(0., 1.);

    // Require at least a decoded texture or explicit color so we don't invent blank materials.
    if mat.texture_bytes.is_none() && color_trace.color.is_none() {
        return None;
    }
    Some(mat)
}

#[derive(Clone, Debug)]
struct TextureRef {
    file: String,
    no_flip_v: bool,
}

#[derive(Default, Clone, Debug)]
struct TraceResult {
    texture: Option<TextureRef>,
    color: Option<[f32; 3]>,
    scale: f32,
}

fn trace_input(layer: &LayerData, prop_path: &str, visited: &mut HashSet<String>) -> TraceResult {
    if visited.len() > 64 || !visited.insert(prop_path.to_string()) {
        return TraceResult {
            scale: 1.,
            ..Default::default()
        };
    }
    if let Some(src) = layer.connections.get(prop_path) {
        let (src_shader, _) = prim_of(src);
        let id = layer
            .shader_ids
            .get(src_shader)
            .map(String::as_str)
            .unwrap_or("");
        if id.contains("Texture2D") || id.contains("ND_image") || id == "UsdUVTexture" {
            let file = match layer.defaults.get(&format!("{src_shader}.inputs:file")) {
                Some(AttrVal::Asset(s) | AttrVal::Token(s)) => Some(s.clone()),
                _ => None,
            };
            let no_flip_v = match layer.defaults.get(&format!("{src_shader}.inputs:no_flip_v")) {
                Some(AttrVal::Bool(b)) => *b,
                _ => true,
            };
            return TraceResult {
                texture: file.map(|file| TextureRef { file, no_flip_v }),
                color: None,
                scale: 1.,
            };
        }
        if id.contains("multiply") {
            let mut a = trace_input(layer, &format!("{src_shader}.inputs:in1"), visited);
            let b = trace_input(layer, &format!("{src_shader}.inputs:in2"), visited);
            let scale = a.scale * b.scale;
            if a.texture.is_none() {
                a.texture = b.texture;
            }
            if a.color.is_none() {
                a.color = b.color;
            }
            a.scale = scale;
            return a;
        }
        if id.contains("combine") {
            for name in ["inputs:in1", "inputs:in2", "inputs:in3", "inputs:in"] {
                let r = trace_input(layer, &format!("{src_shader}.{name}"), visited);
                if r.texture.is_some() || r.color.is_some() {
                    return r;
                }
            }
        }
        if id.contains("separate")
            || id.contains("extract")
            || id.contains("convert")
            || id.contains("swizzle")
            || id.contains("srgb")
        {
            return trace_input(layer, &format!("{src_shader}.inputs:in"), visited);
        }
        // Generic single-input passthrough
        for name in ["inputs:in", "inputs:in1", "inputs:fg", "inputs:bg"] {
            let p = format!("{src_shader}.{name}");
            if layer.connections.contains_key(&p) || layer.defaults.contains_key(&p) {
                return trace_input(layer, &p, visited);
            }
        }
        return TraceResult {
            scale: 1.,
            ..Default::default()
        };
    }

    match layer.defaults.get(prop_path) {
        Some(AttrVal::Float(v)) => TraceResult {
            texture: None,
            color: None,
            scale: *v,
        },
        Some(AttrVal::Vec3(c)) => TraceResult {
            texture: None,
            color: Some(*c),
            scale: 1.,
        },
        _ => TraceResult {
            scale: 1.,
            ..Default::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Minimal OpenUSD Crate (PXR-USDC v0.4 .. v0.10) reader
// ---------------------------------------------------------------------------

fn parse_usdc(data: &[u8], out: &mut LayerData) -> Result<(), ()> {
    if data.len() < 88 || &data[0..8] != b"PXR-USDC" {
        return Err(());
    }
    let toc_offset = read_u64(data, 16)? as usize;
    let n_sections = read_u64(data, toc_offset)? as usize;
    if n_sections > 64 {
        return Err(());
    }
    let mut sections: HashMap<String, (usize, usize)> = HashMap::new();
    let mut pos = toc_offset + 8;
    for _ in 0..n_sections {
        if pos + 32 > data.len() {
            return Err(());
        }
        let name_bytes = &data[pos..pos + 16];
        let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let name = std::str::from_utf8(&name_bytes[..end]).map_err(|_| ())?;
        let start = read_u64(data, pos + 16)? as usize;
        let size = read_u64(data, pos + 24)? as usize;
        if start.checked_add(size).is_none_or(|e| e > data.len()) {
            return Err(());
        }
        sections.insert(name.to_string(), (start, size));
        pos += 32;
    }

    // 1. TOKENS
    let &(tok_start, _) = sections.get("TOKENS").ok_or(())?;
    let n_tokens = read_u64(data, tok_start)? as usize;
    let uncomp_len = read_u64(data, tok_start + 8)? as usize;
    let comp_len = read_u64(data, tok_start + 16)? as usize;
    if n_tokens > 2_000_000 || uncomp_len > 64 * 1024 * 1024 {
        return Err(());
    }
    let tok_raw = usd_lz4_decompress(
        data.get(tok_start + 24..tok_start + 24 + comp_len).ok_or(())?,
        uncomp_len,
    )?;
    let tokens: Vec<String> = tok_raw
        .split(|&b| b == 0)
        .take(n_tokens)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();

    // 2. FIELDS
    let &(f_start, _) = sections.get("FIELDS").ok_or(())?;
    let n_fields = read_u64(data, f_start)? as usize;
    if n_fields > 4_000_000 {
        return Err(());
    }
    let (field_toks, p2) = read_compressed_u32(data, f_start + 8, n_fields)?;
    let reps_comp_len = read_u64(data, p2)? as usize;
    let reps_raw = usd_lz4_decompress(
        data.get(p2 + 8..p2 + 8 + reps_comp_len).ok_or(())?,
        n_fields.checked_mul(8).ok_or(())?,
    )?;
    if reps_raw.len() < n_fields * 8 {
        return Err(());
    }
    let reps: Vec<u64> = (0..n_fields)
        .map(|i| u64::from_le_bytes(reps_raw[i * 8..i * 8 + 8].try_into().unwrap()))
        .collect();

    // 3. FIELDSETS
    let &(fs_start, _) = sections.get("FIELDSETS").ok_or(())?;
    let n_fs = read_u64(data, fs_start)? as usize;
    if n_fs > 8_000_000 {
        return Err(());
    }
    let (fs_indices, _) = read_compressed_u32(data, fs_start + 8, n_fs)?;

    // 4. PATHS
    let &(p_start, _) = sections.get("PATHS").ok_or(())?;
    let n_paths = read_u64(data, p_start)? as usize;
    let n_enc = read_u64(data, p_start + 8)? as usize;
    if n_paths > 4_000_000 || n_enc > 4_000_000 {
        return Err(());
    }
    let (p_indices, pos) = read_compressed_u32(data, p_start + 16, n_enc)?;
    let (e_tokens, pos) = read_compressed_u32(data, pos, n_enc)?;
    let (jumps, _) = read_compressed_u32(data, pos, n_enc)?;
    let mut paths = vec![String::new(); n_paths];
    build_paths(0, "", &p_indices, &e_tokens, &jumps, &tokens, &mut paths, 0);

    // 5. SPECS
    let &(sp_start, _) = sections.get("SPECS").ok_or(())?;
    let n_specs = read_u64(data, sp_start)? as usize;
    if n_specs > 4_000_000 {
        return Err(());
    }
    let (sp_paths, pos) = read_compressed_u32(data, sp_start + 8, n_specs)?;
    let (sp_fsets, _) = read_compressed_u32(data, pos, n_specs)?;

    for i in 0..n_specs {
        let Some(path) = paths.get(sp_paths[i] as usize) else {
            continue;
        };
        let mut k = sp_fsets[i] as usize;
        while let Some(&fi) = fs_indices.get(k) {
            if fi == u32::MAX {
                break;
            }
            k += 1;
            let fi = fi as usize;
            let Some(&tok_idx) = field_toks.get(fi) else {
                continue;
            };
            let Some(fname) = tokens.get(tok_idx as usize) else {
                continue;
            };
            let Some(&rep) = reps.get(fi) else {
                continue;
            };
            let tid = ((rep >> 48) & 0xFF) as u8;
            let is_array = ((rep >> 63) & 1) != 0;
            let inlined = ((rep >> 62) & 1) != 0;
            let payload = rep & ((1u64 << 48) - 1);

            match fname.as_str() {
                "targetPaths" if path.ends_with(".material:binding") && tid == 34 && !is_array => {
                    if let Some(target) = read_first_path_list_op(data, payload as usize, &paths) {
                        let (mesh_prim, _) = prim_of(path);
                        out.material_bindings.insert(mesh_prim.to_string(), target);
                    }
                }
                "connectionPaths" if tid == 34 && !is_array => {
                    if let Some(target) = read_first_path_list_op(data, payload as usize, &paths) {
                        if let Some(mat_prim) = path.strip_suffix(".outputs:mtlx:surface") {
                            out.mtlx_surfaces.insert(mat_prim.to_string(), target);
                        } else if let Some(mat_prim) = path.strip_suffix(".outputs:surface") {
                            out.usd_surfaces.insert(mat_prim.to_string());
                        } else {
                            out.connections.insert(path.clone(), target);
                        }
                    }
                }
                "default" if !is_array => {
                    if let Some(val) = decode_scalar_value(data, tid, inlined, payload, &tokens) {
                        if let Some(shader_prim) = path.strip_suffix(".info:id") {
                            if let AttrVal::Token(id) = val {
                                out.shader_ids.insert(shader_prim.to_string(), id);
                            }
                        } else if let Some(mesh_prim) = path.strip_suffix(".doubleSided") {
                            if matches!(val, AttrVal::Bool(true)) {
                                out.double_sided_meshes.insert(mesh_prim.to_string());
                            }
                        } else {
                            out.defaults.insert(path.clone(), val);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn decode_scalar_value(
    data: &[u8],
    tid: u8,
    inlined: bool,
    payload: u64,
    tokens: &[String],
) -> Option<AttrVal> {
    match tid {
        1 => Some(AttrVal::Bool(payload != 0)), // Bool
        7 => {
            // Float
            let bits = if inlined {
                payload as u32
            } else {
                u32::from_le_bytes(data.get(payload as usize..payload as usize + 4)?.try_into().ok()?)
            };
            Some(AttrVal::Float(f32::from_bits(bits)))
        }
        8 => {
            // Double
            let v = if inlined {
                f32::from_bits(payload as u32) as f64
            } else {
                f64::from_le_bytes(data.get(payload as usize..payload as usize + 8)?.try_into().ok()?)
            };
            Some(AttrVal::Float(v as f32))
        }
        11 => {
            // Token
            let idx = if inlined {
                payload as usize
            } else {
                u32::from_le_bytes(data.get(payload as usize..payload as usize + 4)?.try_into().ok()?) as usize
            };
            Some(AttrVal::Token(tokens.get(idx)?.clone()))
        }
        12 => {
            // AssetPath
            let idx = if inlined {
                payload as usize
            } else {
                u32::from_le_bytes(data.get(payload as usize..payload as usize + 4)?.try_into().ok()?) as usize
            };
            Some(AttrVal::Asset(tokens.get(idx)?.clone()))
        }
        24 => {
            // Vec3f
            if inlined {
                let bytes = payload.to_le_bytes();
                Some(AttrVal::Vec3([
                    bytes[0] as i8 as f32,
                    bytes[1] as i8 as f32,
                    bytes[2] as i8 as f32,
                ]))
            } else {
                let off = payload as usize;
                let slice = data.get(off..off + 12)?;
                Some(AttrVal::Vec3([
                    f32::from_le_bytes(slice[0..4].try_into().ok()?),
                    f32::from_le_bytes(slice[4..8].try_into().ok()?),
                    f32::from_le_bytes(slice[8..12].try_into().ok()?),
                ]))
            }
        }
        _ => None,
    }
}

fn read_first_path_list_op(data: &[u8], offset: usize, paths: &[String]) -> Option<String> {
    let header = *data.get(offset)?;
    let mut pos = offset + 1;
    // Bits 1..=6 indicate presence of Explicit, Added, Prepended, Appended, Deleted, Ordered lists.
    for bit in 1..=6u8 {
        if (header & (1 << bit)) != 0 {
            let count = read_u64(data, pos).ok()? as usize;
            pos += 8;
            if count > 0 && bit != 5 {
                let idx = u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
                return paths.get(idx).cloned();
            }
            pos = pos.checked_add(count.checked_mul(4)?)?;
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn build_paths(
    start: usize,
    parent: &str,
    p_indices: &[u32],
    e_tokens: &[u32],
    jumps: &[u32],
    tokens: &[String],
    out: &mut [String],
    depth: usize,
) {
    if depth > 128 {
        return;
    }
    let mut idx = start;
    while idx < p_indices.len() {
        let pi = p_indices[idx] as usize;
        let raw_tok = e_tokens[idx] as i32;
        let is_prop = raw_tok < 0;
        let tok_idx = raw_tok.unsigned_abs() as usize;
        let tok = tokens.get(tok_idx).map(String::as_str).unwrap_or("");
        let cur = if parent.is_empty() {
            if tok.is_empty() {
                "/".to_string()
            } else {
                format!("/{tok}")
            }
        } else {
            let base = if parent == "/" { "" } else { parent };
            if is_prop {
                format!("{base}.{tok}")
            } else {
                format!("{base}/{tok}")
            }
        };
        if let Some(slot) = out.get_mut(pi) {
            *slot = cur.clone();
        }
        let j = jumps[idx] as i32;
        let has_child = j == -1 || j > 0;
        if has_child && idx + 1 < p_indices.len() {
            build_paths(idx + 1, &cur, p_indices, e_tokens, jumps, tokens, out, depth + 1);
        }
        if j > 0 {
            idx = idx.saturating_add(j as usize);
        } else if j == 0 {
            idx += 1;
        } else {
            break;
        }
    }
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ()> {
    let slice = data.get(offset..offset + 8).ok_or(())?;
    Ok(u64::from_le_bytes(slice.try_into().map_err(|_| ())?))
}

fn read_compressed_u32(data: &[u8], pos: usize, count: usize) -> Result<(Vec<u32>, usize), ()> {
    if count == 0 {
        return Ok((Vec::new(), pos));
    }
    let comp_len = read_u64(data, pos)? as usize;
    let max_uncomp = 4 + (count * 2).div_ceil(8) + count * 4;
    let raw = usd_lz4_decompress(data.get(pos + 8..pos + 8 + comp_len).ok_or(())?, max_uncomp)?;
    let decoded = decode_delta_u32(&raw, count)?;
    Ok((decoded, pos + 8 + comp_len))
}

fn decode_delta_u32(data: &[u8], count: usize) -> Result<Vec<u32>, ()> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let common = i32::from_le_bytes(data.get(0..4).ok_or(())?.try_into().map_err(|_| ())?);
    let n_code_bytes = (count * 2).div_ceil(8);
    let codes = data.get(4..4 + n_code_bytes).ok_or(())?;
    let mut vpos = 4 + n_code_bytes;
    let mut out = Vec::with_capacity(count);
    let mut prev: u32 = 0;
    for idx in 0..count {
        let code = (codes[idx >> 2] >> ((idx & 3) * 2)) & 3;
        let delta: i32 = match code {
            0 => common,
            1 => {
                let b = *data.get(vpos).ok_or(())? as i8;
                vpos += 1;
                i32::from(b)
            }
            2 => {
                let v = i16::from_le_bytes(
                    data.get(vpos..vpos + 2).ok_or(())?.try_into().map_err(|_| ())?,
                );
                vpos += 2;
                i32::from(v)
            }
            _ => {
                let v = i32::from_le_bytes(
                    data.get(vpos..vpos + 4).ok_or(())?.try_into().map_err(|_| ())?,
                );
                vpos += 4;
                v
            }
        };
        prev = prev.wrapping_add(delta as u32);
        out.push(prev);
    }
    Ok(out)
}

fn usd_lz4_decompress(src: &[u8], uncompressed_size: usize) -> Result<Vec<u8>, ()> {
    if src.is_empty() {
        return Err(());
    }
    let n_chunks = src[0];
    if n_chunks == 0 {
        return lz4_decompress_block(&src[1..], uncompressed_size);
    }
    let mut out = Vec::with_capacity(uncompressed_size.min(16 * 1024 * 1024));
    let mut pos = 1usize;
    for _ in 0..n_chunks {
        let csize = i32::from_le_bytes(
            src.get(pos..pos + 4).ok_or(())?.try_into().map_err(|_| ())?,
        );
        if csize < 0 {
            return Err(());
        }
        pos += 4;
        let csize = csize as usize;
        let rem = uncompressed_size.saturating_sub(out.len());
        let chunk = lz4_decompress_block(src.get(pos..pos + csize).ok_or(())?, rem)?;
        out.extend_from_slice(&chunk);
        pos += csize;
    }
    Ok(out)
}

fn lz4_decompress_block(src: &[u8], max_out: usize) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(max_out.min(16 * 1024 * 1024));
    let mut i = 0usize;
    while i < src.len() && out.len() < max_out {
        let token = src[i];
        i += 1;
        let mut lit_len = (token >> 4) as usize;
        if lit_len == 15 {
            loop {
                let b = *src.get(i).ok_or(())? as usize;
                i += 1;
                lit_len = lit_len.checked_add(b).ok_or(())?;
                if b != 255 {
                    break;
                }
            }
        }
        let lit = src.get(i..i + lit_len).ok_or(())?;
        out.extend_from_slice(lit);
        i += lit_len;
        if i >= src.len() || out.len() >= max_out {
            break;
        }
        let lo = *src.get(i).ok_or(())? as usize;
        let hi = *src.get(i + 1).ok_or(())? as usize;
        i += 2;
        let offset = lo | (hi << 8);
        if offset == 0 || offset > out.len() {
            return Err(());
        }
        let mut match_len = ((token & 0x0F) as usize) + 4;
        if (token & 0x0F) == 15 {
            loop {
                let b = *src.get(i).ok_or(())? as usize;
                i += 1;
                match_len = match_len.checked_add(b).ok_or(())?;
                if b != 255 {
                    break;
                }
            }
        }
        let start = out.len() - offset;
        for k in 0..match_len {
            let b = out[start + k];
            out.push(b);
            if out.len() >= max_out {
                break;
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Minimal USDA text fallback parser (for text-authored .usda/.usdz assets)
// ---------------------------------------------------------------------------

fn parse_usda(text: &str, out: &mut LayerData) {
    let mut stack: Vec<String> = Vec::new();
    let mut pending_prim: Option<String> = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if (line.starts_with("def ") || line.starts_with("over "))
            && let Some(q1) = line.find('"')
            && let Some(q2) = line[q1 + 1..].find('"')
        {
            pending_prim = Some(line[q1 + 1..q1 + 1 + q2].to_string());
        }
        if line.contains('{') {
            if let Some(name) = pending_prim.take() {
                let parent = stack.last().map(String::as_str).unwrap_or("");
                let full = if parent.is_empty() {
                    format!("/{name}")
                } else {
                    format!("{parent}/{name}")
                };
                stack.push(full);
            } else if let Some(last) = stack.last().cloned() {
                stack.push(last);
            }
        }
        if let Some(cur) = stack.last()
            && let Some((lhs, rhs)) = line.split_once('=')
        {
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            let prop = lhs.split_whitespace().last().unwrap_or("");
            if prop == "material:binding"
                && let Some(target) = extract_angle_path(rhs)
            {
                out.material_bindings.insert(cur.clone(), target);
            } else if prop == "outputs:mtlx:surface.connect"
                && let Some(target) = extract_angle_path(rhs)
            {
                out.mtlx_surfaces.insert(cur.clone(), target);
            } else if prop == "outputs:surface.connect" {
                out.usd_surfaces.insert(cur.clone());
            } else if prop == "doubleSided" && (rhs == "1" || rhs == "true") {
                out.double_sided_meshes.insert(cur.clone());
            } else if prop == "info:id"
                && let Some(val) = extract_quoted(rhs)
            {
                out.shader_ids.insert(cur.clone(), val);
            } else if let Some(attr) = prop.strip_suffix(".connect")
                && let Some(target) = extract_angle_path(rhs)
            {
                out.connections.insert(format!("{cur}.{attr}"), target);
            } else if prop.starts_with("inputs:") {
                let full = format!("{cur}.{prop}");
                if let Some(asset) = extract_at_path(rhs) {
                    out.defaults.insert(full, AttrVal::Asset(asset));
                } else if rhs == "1" || rhs == "true" {
                    out.defaults.insert(full, AttrVal::Bool(true));
                } else if rhs == "0" || rhs == "false" {
                    out.defaults.insert(full, AttrVal::Bool(false));
                } else if let Ok(v) = rhs.parse::<f32>() {
                    out.defaults.insert(full, AttrVal::Float(v));
                } else if let Some(v) = extract_vec3(rhs) {
                    out.defaults.insert(full, AttrVal::Vec3(v));
                }
            }
        }
        if line.contains('}') {
            stack.pop();
        }
    }
}

fn extract_angle_path(s: &str) -> Option<String> {
    let a = s.find('<')?;
    let b = s[a + 1..].find('>')?;
    Some(s[a + 1..a + 1 + b].to_string())
}

fn extract_at_path(s: &str) -> Option<String> {
    let a = s.find('@')?;
    let b = s[a + 1..].find('@')?;
    Some(s[a + 1..a + 1 + b].to_string())
}

fn extract_quoted(s: &str) -> Option<String> {
    let a = s.find('"')?;
    let b = s[a + 1..].find('"')?;
    Some(s[a + 1..a + 1 + b].to_string())
}

fn extract_vec3(s: &str) -> Option<[f32; 3]> {
    let inner = s.strip_prefix('(')?.split(')').next()?;
    let mut parts = inner.split(',').map(|p| p.trim().parse::<f32>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usda_realitykit_unlit_materialx() {
        let usda = r#"#usda 1.0
def Xform "root" {
    def Scope "mats" {
        def Material "logo_mat" {
            token outputs:mtlx:surface.connect = </root/mats/logo_mat/unlit.outputs:out>
            def Shader "tex" {
                uniform token info:id = "ND_RealityKitTexture2D_color4"
                asset inputs:file = @textures/logo.png@
                bool inputs:no_flip_v = 1
            }
            def Shader "sep" {
                uniform token info:id = "ND_separate4_color4"
                color4f inputs:in.connect = </root/mats/logo_mat/tex.outputs:out>
            }
            def Shader "comb" {
                uniform token info:id = "ND_combine3_color3"
                float inputs:in1.connect = </root/mats/logo_mat/sep.outputs:outr>
            }
            def Shader "mul_color" {
                uniform token info:id = "ND_multiply_color3FA"
                color3f inputs:in1.connect = </root/mats/logo_mat/comb.outputs:out>
                float inputs:in2 = 2
            }
            def Shader "mul_op" {
                uniform token info:id = "ND_multiply_float"
                float inputs:in1.connect = </root/mats/logo_mat/sep.outputs:outa>
                float inputs:in2 = 0.75
            }
            def Shader "unlit" {
                uniform token info:id = "ND_realitykit_unlit_surfaceshader"
                color3f inputs:color.connect = </root/mats/logo_mat/mul_color.outputs:out>
                float inputs:opacity.connect = </root/mats/logo_mat/mul_op.outputs:out>
            }
        }
    }
    def Mesh "mesh_logo" {
        uniform bool doubleSided = 1
        rel material:binding = </root/mats/logo_mat>
    }
}
"#;
        let mut layer = LayerData::default();
        parse_usda(usda, &mut layer);
        let mut idx = UsdMtlxIndex::default();
        idx.resolve_layer(&layer, "", &|p| {
            (p == "textures/logo.png").then(|| vec![1, 2, 3, 4])
        });
        let mat = idx.get("mesh_logo").expect("logo material resolved");
        assert!(mat.unlit);
        assert!(mat.double_sided);
        assert!(mat.no_flip_v);
        assert!((mat.color_scale - 2.0).abs() < 1e-5);
        assert!((mat.opacity - 0.75).abs() < 1e-5);
        assert_eq!(mat.texture_bytes.as_deref(), Some(&[1, 2, 3, 4][..]));
    }
}
