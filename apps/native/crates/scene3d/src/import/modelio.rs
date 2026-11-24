use super::mesh::{Mesh, Scene};
use crate::{Error, Result, fail};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
};

type Emit = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const [f32; 3],
    *const [f32; 3],
    *const [f32; 2],
    usize,
    *const u32,
    usize,
    *const [f32; 16],
    *const c_char,
    *const [f32; 4],
) -> bool;
unsafe extern "C" {
    fn pm_scene3d_modelio_supports(ext: *const c_char) -> bool;
    fn pm_scene3d_modelio_read(path: *const c_char, context: *mut c_void, emit: Emit) -> bool;
}

pub fn supports(ext: &str) -> bool {
    // Model I/O selects readers using the extension. Limit the extension
    // before it can become part of a scratch filename.
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
    // Model I/O calls emit synchronously. The callback copies each borrowed
    // array before returning to the bridge, where the arrays are owned.
    let ok = unsafe {
        pm_scene3d_modelio_read(path.as_ptr(), (&mut output as *mut Output).cast(), emit)
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
    error: Option<Error>,
}

unsafe extern "C" fn emit(
    context: *mut c_void,
    name: *const c_char,
    positions: *const [f32; 3],
    normals: *const [f32; 3],
    uv: *const [f32; 2],
    count: usize,
    indices: *const u32,
    index_count: usize,
    matrix: *const [f32; 16],
    material: *const c_char,
    color: *const [f32; 4],
) -> bool {
    unsafe {
        let output = &mut *context.cast::<Output>();
        let name = if name.is_null() {
            "Model I/O mesh".into()
        } else {
            CStr::from_ptr(name).to_string_lossy()
        };
        let material = if material.is_null() {
            None
        } else {
            let material = CStr::from_ptr(material).to_string_lossy();
            let color = (*color).map(|v| if v.is_finite() { v.clamp(0., 1.) } else { 1. });
            Some(output.scene.material(&material, color))
        };
        let mesh = Mesh {
            positions: std::slice::from_raw_parts(positions, count).to_vec(),
            normals: if normals.is_null() {
                vec![]
            } else {
                std::slice::from_raw_parts(normals, count).to_vec()
            },
            uv: if uv.is_null() {
                vec![]
            } else {
                std::slice::from_raw_parts(uv, count).to_vec()
            },
            indices: std::slice::from_raw_parts(indices, index_count).to_vec(),
            material,
            ..Mesh::default()
        };
        let result = output.scene.mesh(&name, [mesh]).and_then(|index| {
            output
                .scene
                .node(&name, Some(index), Some((*matrix).map(f64::from)), vec![])
        });
        match result {
            Ok(node) => {
                output.roots.push(node);
                true
            }
            Err(error) => {
                output.error = Some(error);
                false
            }
        }
    }
}
