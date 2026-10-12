//! A minimal C ABI over [`Scene`] so V8 (Node or Chromium) can drive the wasm build
//! without wasm-bindgen: linear-memory buffers in, numbers out. Single-threaded, as the
//! editor's evaluation worker is.
//!
//! In wasm the host supplies a millisecond clock as the import `env.s7_now`.

use crate::model::{LoadOptions, load};
use crate::scene::{Camera, Layer, Prepared, Scene};
use glam::DMat4;
use std::cell::RefCell;
use std::sync::Arc;

#[derive(Default)]
struct State {
    models: Vec<Arc<Prepared>>,
    scene: Option<Scene>,
    error: String,
    hit: [f64; 8],
    load_ms: f64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn s7_now() -> f64;
}

/// Milliseconds from an arbitrary origin.
pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe { s7_now() }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
}

fn identity_camera() -> Camera {
    Camera { projection: DMat4::IDENTITY, world: DMat4::IDENTITY, orthographic: false, near: 0.1, far: 100.0 }
}

unsafe fn mat(ptr: *const f64) -> DMat4 {
    let s = unsafe { std::slice::from_raw_parts(ptr, 16) };
    DMat4::from_cols_slice(s)
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from [`s7_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s7_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) });
}

/// Loads a GLB; returns the model index or −1 (see [`s7_error_ptr`]).
///
/// # Safety
/// `ptr` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s7_model_load(ptr: *const u8, len: usize, decode_images: u32) -> i32 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    STATE.with_borrow_mut(|s| {
        let t0 = now_ms();
        match load(bytes, LoadOptions { decode_images: decode_images != 0 }) {
            Ok(model) => {
                s.models.push(Arc::new(Prepared::new(model)));
                s.load_ms = now_ms() - t0;
                s.models.len() as i32 - 1
            }
            Err(e) => {
                s.error = e.0;
                -1
            }
        }
    })
}

/// Milliseconds the last `s7_model_load` took (decode + BVH build).
#[unsafe(no_mangle)]
pub extern "C" fn s7_model_load_ms() -> f64 {
    STATE.with_borrow(|s| s.load_ms)
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_error_ptr() -> *const u8 {
    STATE.with_borrow(|s| s.error.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_error_len() -> usize {
    STATE.with_borrow(|s| s.error.len())
}

/// Resets the scene (layers removed, models kept).
#[unsafe(no_mangle)]
pub extern "C" fn s7_scene_reset() {
    STATE.with_borrow_mut(|s| {
        let mut scene = Scene::new(identity_camera());
        scene.clock = now_ms;
        s.scene = Some(scene);
    })
}

/// Adds a layer showing `model`; returns its index.
#[unsafe(no_mangle)]
pub extern "C" fn s7_layer_add(model: u32) -> i32 {
    STATE.with_borrow_mut(|s| {
        let Some(prepared) = s.models.get(model as usize).cloned() else { return -1 };
        let scene = s.scene.get_or_insert_with(|| {
            let mut sc = Scene::new(identity_camera());
            sc.clock = now_ms;
            sc
        });
        scene.layers.push(Layer::new("", prepared, DMat4::IDENTITY));
        scene.layers.len() as i32 - 1
    })
}

/// # Safety
/// `m` points to 16 f64 (column-major, three.js `elements` order).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s7_layer_transform(layer: u32, m: *const f64) {
    let m = unsafe { mat(m) };
    STATE.with_borrow_mut(|s| {
        if let Some(l) = s.scene.as_mut().and_then(|sc| sc.layers.get_mut(layer as usize)) {
            l.gltf_to_world = m;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_layer_time(layer: u32, time: f64) {
    STATE.with_borrow_mut(|s| {
        if let Some(l) = s.scene.as_mut().and_then(|sc| sc.layers.get_mut(layer as usize)) {
            l.set_time(time);
        }
    })
}

/// 1: lazy skinned picks (default); 0: re-skin and refit everything on a time change.
#[unsafe(no_mangle)]
pub extern "C" fn s7_layer_lazy(layer: u32, on: u32) {
    STATE.with_borrow_mut(|s| {
        if let Some(l) = s.scene.as_mut().and_then(|sc| sc.layers.get_mut(layer as usize)) {
            l.lazy = on != 0;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_layer_update_ms(layer: u32) -> f64 {
    STATE.with_borrow(|s| s.scene.as_ref().and_then(|sc| sc.layers.get(layer as usize)).map(|l| l.last_update_ms).unwrap_or(-1.0))
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_layer_vertices(layer: u32) -> u32 {
    STATE.with_borrow(|s| s.scene.as_ref().and_then(|sc| sc.layers.get(layer as usize)).map(|l| l.vertex_count() as u32).unwrap_or(0))
}

/// # Safety
/// `projection` and `world` point to 16 f64 each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s7_camera(projection: *const f64, world: *const f64, orthographic: u32, near: f64, far: f64) {
    let camera = Camera { projection: unsafe { mat(projection) }, world: unsafe { mat(world) }, orthographic: orthographic != 0, near, far };
    STATE.with_borrow_mut(|s| {
        if let Some(sc) = s.scene.as_mut() {
            sc.camera = camera;
        }
    })
}

/// Picks at NDC (x, y) over every layer; returns the layer index or −1. Hit details are
/// at [`s7_hit_ptr`]: distance, node, kind (0 triangles, 1 points, 2+ lines), index,
/// instance (−1 none), point x, y, z.
#[unsafe(no_mangle)]
pub extern "C" fn s7_pick(x: f64, y: f64) -> i32 {
    STATE.with_borrow_mut(|s| {
        let Some(sc) = s.scene.as_mut() else { return -1 };
        match sc.pick(x, y, None) {
            Some(h) => {
                s.hit = [
                    h.distance,
                    h.node as f64,
                    match h.kind {
                        crate::model::Topology::Triangles => 0.0,
                        crate::model::Topology::Points => 1.0,
                        _ => 2.0,
                    },
                    h.index as f64,
                    h.instance.map(|i| i as f64).unwrap_or(-1.0),
                    h.point.x,
                    h.point.y,
                    h.point.z,
                ];
                h.layer as i32
            }
            None => -1,
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn s7_hit_ptr() -> *const f64 {
    STATE.with_borrow(|s| s.hit.as_ptr())
}
