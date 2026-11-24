use super::{
    mesh::{Alpha, Material, Mesh, Part, Sampler, Scene},
    texture,
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
}

impl Raster {
    unsafe fn read(self) -> Option<(RgbaImage, Sampler)> {
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
    usize,
    *const NativePart,
    usize,
) -> bool;
type EmitNode = unsafe extern "C" fn(*mut c_void, usize, *const c_char, *const [f32; 16]) -> bool;

unsafe extern "C" {
    fn pm_scene3d_modelio_supports(ext: *const c_char) -> bool;
    fn pm_scene3d_modelio_read_scene(
        path: *const c_char,
        context: *mut c_void,
        material: EmitMaterial,
        mesh: EmitMesh,
        node: EmitNode,
    ) -> bool;
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
    let mut output = Output::default();
    // The bridge invokes callbacks synchronously and owns the arrays for the
    // duration of each call.
    let ok = unsafe {
        pm_scene3d_modelio_read_scene(
            path.as_ptr(),
            (&mut output as *mut Output).cast(),
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
struct Output {
    scene: Scene,
    roots: Vec<usize>,
    meshes: HashMap<usize, usize>,
    materials: HashMap<usize, usize>,
    error: Option<Error>,
}

impl Output {
    fn record(&mut self, result: Result<()>) -> bool {
        if let Err(error) = result {
            self.error = Some(error);
            return false;
        }
        true
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
        if let Some((opacity, sampler)) = unsafe { data.opacity.read() } {
            let (color, sampler) =
                base.map_or((None, sampler), |(image, sampler)| (Some(image), sampler));
            base = Some((texture::with_opacity(color, opacity), sampler));
        }
        let mut has_alpha = material.base_color[3] < 1.;
        if let Some((base, sampler)) = base {
            has_alpha |= base.pixels().any(|pixel| pixel[3] < 255);
            material.base_map = Some(output.scene.texture(base, true, sampler)?);
        }
        material.alpha = if data.alpha_cutoff > 0. {
            Alpha::Mask(factor(data.alpha_cutoff))
        } else if has_alpha {
            Alpha::Blend
        } else {
            Alpha::Opaque
        };
        let (rough, rough_sampler) = unsafe { data.rough.read() }.unzip();
        let (metal, metal_sampler) = unsafe { data.metal.read() }.unzip();
        if let Some(image) = texture::metallic_roughness(rough, metal) {
            material.metallic_roughness_map = Some(output.scene.texture(
                image,
                false,
                rough_sampler.or(metal_sampler).unwrap_or_default(),
            )?);
        }
        for (target, raster, color) in [
            (&mut material.normal_map, data.normal, false),
            (&mut material.occlusion_map, data.occlusion, false),
            (&mut material.emissive_map, data.emission, true),
        ] {
            if let Some((image, sampler)) = unsafe { raster.read() } {
                *target = Some(output.scene.texture(image, color, sampler)?);
            }
        }
        let index = output.scene.pbr_material(&label, material);
        output.materials.insert(id, index);
        Ok(())
    })();
    output.record(result)
}

unsafe extern "C" fn emit_mesh(
    context: *mut c_void,
    id: usize,
    label: *const c_char,
    positions: *const [f32; 3],
    normals: *const [f32; 3],
    uv: *const [f32; 2],
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
        ..Mesh::default()
    };
    let parts = unsafe { std::slice::from_raw_parts(parts, num_parts) }
        .iter()
        .map(|part| Part {
            indices: unsafe { std::slice::from_raw_parts(part.indices, part.count) }.to_vec(),
            material: output.materials.get(&part.material_id).copied(),
        })
        .collect();
    let result = output
        .scene
        .partitioned_mesh(&label, vertices, parts)
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
