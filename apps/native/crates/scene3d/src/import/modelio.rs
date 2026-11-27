use super::{
    mesh::{Alpha, Material, Mesh, Part, Sampler, Scene},
    texture,
    usd_mtlx::{MtlxMaterial, UsdMtlxIndex},
};
use crate::{Error, Result, fail};
use image::RgbaImage;
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct Raster {
    pixels: *const u8,
    width: u32,
    height: u32,
    wrap_s: u32,
    wrap_t: u32,
    mag_filter: u32,
    min_filter: u32,
    tex_coord: u32,
}

impl Raster {
    unsafe fn read(self) -> Option<(RgbaImage, Sampler, u32)> {
        if self.pixels.is_null() {
            return None;
        }
        let len = self.width as usize * self.height as usize * 4;
        let bytes = unsafe { std::slice::from_raw_parts(self.pixels, len) };
        let image = RgbaImage::from_raw(self.width, self.height, bytes.to_vec())?;
        Some((
            image,
            Sampler {
                wrap_s: self.wrap_s,
                wrap_t: self.wrap_t,
                mag_filter: self.mag_filter,
                min_filter: self.min_filter,
            },
            self.tex_coord,
        ))
    }
}

#[repr(C)]
struct NativeMaterial {
    rgba: [f32; 4],
    emissive: [f32; 3],
    metallic: f32,
    roughness: f32,
    occlusion_strength: f32,
    alpha_cutoff: f32,
    double_sided: bool,
    base: Raster,
    opacity: Raster,
    metal: Raster,
    rough: Raster,
    normal: Raster,
    occlusion: Raster,
    emission: Raster,
}

#[repr(C)]
struct NativePart {
    indices: *const u32,
    count: usize,
    material_id: usize,
}

type EmitMaterial =
    unsafe extern "C" fn(*mut c_void, usize, *const c_char, *const NativeMaterial) -> bool;
type EmitMesh = unsafe extern "C" fn(
    *mut c_void,
    usize,
    *const c_char,
    *const [f32; 3],
    *const [f32; 3],
    *const [f32; 2],
    *const [f32; 2],
    usize,
    *const NativePart,
    usize,
) -> bool;
type EmitNode = unsafe extern "C" fn(*mut c_void, usize, *const c_char, *const [f32; 16]) -> bool;

type EmitImage = unsafe extern "C" fn(*mut c_void, *const u8, u32, u32) -> bool;

unsafe extern "C" {
    fn pm_scene3d_modelio_supports(ext: *const c_char) -> bool;
    fn pm_scene3d_modelio_decode_image(
        data: *const u8,
        len: usize,
        color: bool,
        context: *mut c_void,
        emit: EmitImage,
    ) -> bool;
    fn pm_scene3d_modelio_read_scene(
        path: *const c_char,
        context: *mut c_void,
        material: EmitMaterial,
        mesh: EmitMesh,
        node: EmitNode,
    ) -> bool;
}

fn decode_native_image(bytes: &[u8], color: bool) -> Option<RgbaImage> {
    let mut out: Option<RgbaImage> = None;
    unsafe extern "C" fn capture(
        context: *mut c_void,
        pixels: *const u8,
        width: u32,
        height: u32,
    ) -> bool {
        if pixels.is_null() || width == 0 || height == 0 {
            return false;
        }
        let len = width as usize * height as usize * 4;
        let slice = unsafe { std::slice::from_raw_parts(pixels, len) };
        if let Some(image) = RgbaImage::from_raw(width, height, slice.to_vec()) {
            unsafe { *context.cast::<Option<RgbaImage>>() = Some(image) };
            true
        } else {
            false
        }
    }
    let ok = unsafe {
        pm_scene3d_modelio_decode_image(
            bytes.as_ptr(),
            bytes.len(),
            color,
            (&mut out as *mut Option<RgbaImage>).cast(),
            capture,
        )
    };
    ok.then_some(out).flatten()
}

pub fn supports(ext: &str) -> bool {
    !ext.is_empty()
        && ext.len() <= 16
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        && CString::new(ext).is_ok_and(|s| unsafe { pm_scene3d_modelio_supports(s.as_ptr()) })
}

pub fn convert(ext: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let scratch = tempfile::Builder::new()
        .prefix("powermove-modelio-")
        .tempdir()
        .map_err(|e| Error(format!("Cannot create Model I/O scratch space: {e}")))?;
    let path = scratch.path().join(format!("source.{ext}"));
    std::fs::write(&path, bytes)
        .map_err(|e| Error(format!("Cannot stage Model I/O source: {e}")))?;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| Error(e.to_string()))?;
    let mut output = Output {
        source_ext: ext,
        source_bytes: bytes,
        ..Output::default()
    };
    // The bridge invokes callbacks synchronously and owns the arrays for the
    // duration of each call.
    let ok = unsafe {
        pm_scene3d_modelio_read_scene(
            path.as_ptr(),
            (&mut output as *mut Output<'_>).cast(),
            emit_material,
            emit_mesh,
            emit_node,
        )
    };
    if let Some(error) = output.error {
        return Err(error);
    }
    if !ok {
        return fail(format!(
            "Model I/O could not read this {} mesh",
            ext.to_uppercase()
        ));
    }
    output.scene.finish(output.roots)
}

#[derive(Default)]
struct Output<'a> {
    scene: Scene,
    roots: Vec<usize>,
    meshes: HashMap<usize, usize>,
    materials: HashMap<usize, usize>,
    mtlx_materials: HashMap<String, usize>,
    mtlx_index: Option<UsdMtlxIndex>,
    source_ext: &'a str,
    source_bytes: &'a [u8],
    error: Option<Error>,
}

impl Output<'_> {
    fn record(&mut self, result: Result<()>) -> bool {
        if let Err(error) = result {
            self.error = Some(error);
            return false;
        }
        true
    }

    fn resolve_mtlx_material(&mut self, material_name: &str) -> Result<Option<usize>> {
        if !matches!(
            self.source_ext.to_ascii_lowercase().as_str(),
            "usdz" | "usdc" | "usda" | "usd"
        ) {
            return Ok(None);
        }
        let key = format!("mat:{material_name}");
        if let Some(&idx) = self.mtlx_materials.get(&key) {
            return Ok(Some(idx));
        }
        let ext = self.source_ext;
        let bytes = self.source_bytes;
        let index = self
            .mtlx_index
            .get_or_insert_with(|| UsdMtlxIndex::parse(ext, bytes));
        let Some(mtlx) = index.get_material(material_name).cloned() else {
            return Ok(None);
        };
        let mat_index = self.install_mtlx_material(&mtlx)?;
        self.mtlx_materials.insert(key, mat_index);
        Ok(Some(mat_index))
    }

    fn resolve_mtlx_fallback(&mut self, mesh_name: &str) -> Result<Option<usize>> {
        if !matches!(
            self.source_ext.to_ascii_lowercase().as_str(),
            "usdz" | "usdc" | "usda" | "usd"
        ) {
            return Ok(None);
        }
        if let Some(&idx) = self.mtlx_materials.get(mesh_name) {
            return Ok(Some(idx));
        }
        let ext = self.source_ext;
        let bytes = self.source_bytes;
        let index = self
            .mtlx_index
            .get_or_insert_with(|| UsdMtlxIndex::parse(ext, bytes));
        let Some(mtlx) = index.get(mesh_name).cloned() else {
            return Ok(None);
        };
        let mat_index = self.install_mtlx_material(&mtlx)?;
        self.mtlx_materials.insert(mesh_name.to_string(), mat_index);
        Ok(Some(mat_index))
    }

    fn install_mtlx_material(&mut self, mtlx: &MtlxMaterial) -> Result<usize> {
        let mut material = Material {
            base_color: [
                factor(mtlx.base_color[0]),
                factor(mtlx.base_color[1]),
                factor(mtlx.base_color[2]),
                factor(mtlx.opacity),
            ],
            roughness: factor(mtlx.roughness),
            metallic: factor(mtlx.metallic),
            double_sided: mtlx.double_sided,
            ..Material::default()
        };
        let mut has_alpha = material.base_color[3] < 1.;
        if let Some(tex_bytes) = &mtlx.texture_bytes
            && let Some(mut img) = decode_native_image(tex_bytes, true)
        {
            if !mtlx.no_flip_v {
                image::imageops::flip_vertical_in_place(&mut img);
            }
            let scale = if mtlx.color_scale.is_finite() && mtlx.color_scale > 0. {
                mtlx.color_scale
            } else {
                1.
            };
            // Apply linear color scaling (e.g. RealityKit unlit `multiply` node)
            // and clear RGB on fully transparent pixels to prevent mip halos.
            for px in img.pixels_mut() {
                if px[3] == 0 {
                    px[0] = 0;
                    px[1] = 0;
                    px[2] = 0;
                    continue;
                }
                if (scale - 1.).abs() > 1e-4 {
                    for c in 0..3 {
                        let srgb = f32::from(px[c]) / 255.;
                        let linear = srgb.powf(2.2) * scale;
                        let out = linear.clamp(0., 1.).powf(1. / 2.2);
                        px[c] = (out * 255.).round().clamp(0., 255.) as u8;
                    }
                }
            }
            has_alpha |= img.pixels().any(|p| p[3] < 255);
            let sampler = Sampler {
                wrap_s: 33071,
                wrap_t: 33071,
                ..Sampler::default()
            };
            let tex_id = self.scene.texture(img, true, sampler)?;
            material.base_map = Some(tex_id);
            if mtlx.unlit {
                material.emissive_map = Some(tex_id);
                material.emissive = [0.6, 0.6, 0.6];
            }
        }
        material.alpha = if has_alpha {
            Alpha::Blend
        } else {
            Alpha::Opaque
        };
        Ok(self.scene.pbr_material(&mtlx.name, material))
    }
}

unsafe fn name<'a>(value: *const c_char, fallback: &'static str) -> std::borrow::Cow<'a, str> {
    if value.is_null() {
        fallback.into()
    } else {
        unsafe { CStr::from_ptr(value) }.to_string_lossy()
    }
}

fn factor(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0., 1.)
    } else {
        1.
    }
}

unsafe extern "C" fn emit_material(
    context: *mut c_void,
    id: usize,
    label: *const c_char,
    data: *const NativeMaterial,
) -> bool {
    let output = unsafe { &mut *context.cast::<Output>() };
    let label = unsafe { name(label, "Model I/O material") };
    let data = unsafe { &*data };
    let result = (|| {
        // Model I/O emits an empty placeholder `MDLMaterial` (no maps) for USD
        // materials that only author RealityKit MaterialX `outputs:mtlx:surface`.
        if data.base.pixels.is_null()
            && data.opacity.pixels.is_null()
            && data.emission.pixels.is_null()
            && let Some(index) = output.resolve_mtlx_material(&label)?
        {
            output.materials.insert(id, index);
            return Ok(());
        }
        let mut material = Material {
            base_color: data.rgba.map(factor),
            roughness: factor(data.roughness),
            metallic: factor(data.metallic),
            emissive: data.emissive.map(factor),
            occlusion_strength: factor(data.occlusion_strength),
            double_sided: data.double_sided,
            ..Material::default()
        };
        let mut base = unsafe { data.base.read() };
        if let Some((opacity, sampler, tex_coord)) = unsafe { data.opacity.read() } {
            let (color, sampler, tex_coord) = base.map_or(
                (None, sampler, tex_coord),
                |(image, sampler, base_coord)| {
                    (Some(image), sampler, packed_coord(Some(base_coord), Some(tex_coord)))
                },
            );
            base = Some((texture::with_opacity(color, opacity), sampler, tex_coord));
        }
        let mut has_alpha = material.base_color[3] < 1.;
        if let Some((base, sampler, tex_coord)) = base {
            has_alpha |= base.pixels().any(|pixel| pixel[3] < 255);
            material.base_map = Some(output.scene.texture(base, true, sampler)?);
            material.base_tex_coord = tex_coord;
        }
        material.alpha = if data.alpha_cutoff > 0. {
            Alpha::Mask(factor(data.alpha_cutoff))
        } else if has_alpha {
            Alpha::Blend
        } else {
            Alpha::Opaque
        };
        let (rough, rough_sampler, rough_coord) = match unsafe { data.rough.read() } {
            Some((image, sampler, coord)) => (Some(image), Some(sampler), Some(coord)),
            None => (None, None, None),
        };
        let (metal, metal_sampler, metal_coord) = match unsafe { data.metal.read() } {
            Some((image, sampler, coord)) => (Some(image), Some(sampler), Some(coord)),
            None => (None, None, None),
        };
        if let Some(image) = texture::metallic_roughness(rough, metal) {
            material.metallic_roughness_map = Some(output.scene.texture(
                image,
                false,
                rough_sampler.or(metal_sampler).unwrap_or_default(),
            )?);
            material.metallic_roughness_tex_coord = packed_coord(rough_coord, metal_coord);
        }
        for (target, target_coord, raster, color) in [
            (
                &mut material.normal_map,
                &mut material.normal_tex_coord,
                data.normal,
                false,
            ),
            (
                &mut material.occlusion_map,
                &mut material.occlusion_tex_coord,
                data.occlusion,
                false,
            ),
            (
                &mut material.emissive_map,
                &mut material.emissive_tex_coord,
                data.emission,
                true,
            ),
        ] {
            if let Some((image, sampler, tex_coord)) = unsafe { raster.read() } {
                *target = Some(output.scene.texture(image, color, sampler)?);
                *target_coord = tex_coord;
            }
        }
        let index = output.scene.pbr_material(&label, material);
        output.materials.insert(id, index);
        Ok(())
    })();
    output.record(result)
}

/// The UV set of two maps packed into one texture (opacity into base colour,
/// metallic with roughness). Packed, they share one set: when theirs differ,
/// both sample UV0 as they did before second UV sets were read, so neither
/// map moves onto a set it was not authored for (w83 review P0-3).
fn packed_coord(a: Option<u32>, b: Option<u32>) -> u32 {
    match (a, b) {
        (Some(a), Some(b)) if a != b => 0,
        (a, b) => a.or(b).unwrap_or(0),
    }
}

#[cfg(test)]
mod packed_coord_tests {
    use super::packed_coord;
    #[test]
    fn packed_maps_keep_a_set_both_were_authored_for() {
        assert_eq!(packed_coord(Some(1), Some(1)), 1);
        assert_eq!(packed_coord(Some(1), None), 1);
        assert_eq!(packed_coord(None, Some(1)), 1);
        assert_eq!(packed_coord(Some(1), Some(0)), 0);
        assert_eq!(packed_coord(Some(0), Some(1)), 0);
        assert_eq!(packed_coord(None, None), 0);
    }
}

unsafe extern "C" fn emit_mesh(
    context: *mut c_void,
    id: usize,
    label: *const c_char,
    positions: *const [f32; 3],
    normals: *const [f32; 3],
    uv: *const [f32; 2],
    uv1: *const [f32; 2],
    count: usize,
    parts: *const NativePart,
    num_parts: usize,
) -> bool {
    let output = unsafe { &mut *context.cast::<Output>() };
    let label = unsafe { name(label, "Model I/O mesh") };
    let vertices = Mesh {
        positions: unsafe { std::slice::from_raw_parts(positions, count) }.to_vec(),
        normals: unsafe { copy_attribute(normals, count) },
        uv: unsafe { copy_attribute(uv, count) },
        uv1: unsafe { copy_attribute(uv1, count) },
        ..Mesh::default()
    };
    let parts = unsafe { std::slice::from_raw_parts(parts, num_parts) };
    let mut built_parts = Vec::with_capacity(parts.len());
    for part in parts {
        let material = if let Some(&idx) = output.materials.get(&part.material_id) {
            Some(idx)
        } else if part.material_id == 0 {
            match output.resolve_mtlx_fallback(&label) {
                Ok(found) => found,
                Err(error) => {
                    output.error = Some(error);
                    return false;
                }
            }
        } else {
            None
        };
        built_parts.push(Part {
            indices: unsafe { std::slice::from_raw_parts(part.indices, part.count) }.to_vec(),
            material,
        });
    }
    let result = output
        .scene
        .partitioned_mesh(&label, vertices, built_parts)
        .map(|index| {
            output.meshes.insert(id, index);
        });
    output.record(result)
}

unsafe fn copy_attribute<T: Copy>(values: *const T, count: usize) -> Vec<T> {
    if values.is_null() {
        vec![]
    } else {
        unsafe { std::slice::from_raw_parts(values, count) }.to_vec()
    }
}

unsafe extern "C" fn emit_node(
    context: *mut c_void,
    mesh: usize,
    label: *const c_char,
    matrix: *const [f32; 16],
) -> bool {
    let output = unsafe { &mut *context.cast::<Output>() };
    let label = unsafe { name(label, "Model I/O mesh") };
    let Some(&mesh) = output.meshes.get(&mesh) else {
        return false;
    };
    let result = output
        .scene
        .node(
            &label,
            Some(mesh),
            Some(unsafe { (*matrix).map(f64::from) }),
            vec![],
        )
        .map(|node| output.roots.push(node));
    output.record(result)
}
