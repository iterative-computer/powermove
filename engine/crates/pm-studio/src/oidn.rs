//! Intel Open Image Denoise for Final frames: a minimal binding to the C API
//! (only what the RT filter needs).

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};

type Device = *mut c_void;
type Filter = *mut c_void;
const DEVICE_TYPE_CPU: i32 = 1;
mod metal;
const FORMAT_FLOAT3: i32 = 3;
const QUALITY_HIGH: i32 = 6;

unsafe extern "C" {
    fn oidnNewDevice(kind: i32) -> Device;
    fn oidnCommitDevice(device: Device);
    fn oidnReleaseDevice(device: Device);
    fn oidnGetDeviceError(device: Device, message: *mut *const c_char) -> i32;
    #[cfg(test)]
    fn oidnGetDeviceInt(device: Device, name: *const c_char) -> i32;
    fn oidnNewFilter(device: Device, kind: *const c_char) -> Filter;
    fn oidnSetSharedFilterImage(
        filter: Filter,
        name: *const c_char,
        ptr: *mut c_void,
        format: i32,
        width: usize,
        height: usize,
        byte_offset: usize,
        pixel_stride: usize,
        row_stride: usize,
    );
    fn oidnSetFilterBool(filter: Filter, name: *const c_char, value: bool);
    fn oidnSetFilterInt(filter: Filter, name: *const c_char, value: i32);
    fn oidnSetFilterFloat(filter: Filter, name: *const c_char, value: f32);
    fn oidnCommitFilter(filter: Filter);
    fn oidnExecuteFilter(filter: Filter);
    fn oidnReleaseFilter(filter: Filter);
    fn oidnSetFilterProgressMonitorFunction(
        filter: Filter,
        callback: Option<unsafe extern "C" fn(*mut c_void, f64) -> bool>,
        user: *mut c_void,
    );
}

#[derive(Clone, Copy)]
pub enum Quality {
    Fast,
    Balanced,
    High,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Cpu,
    Metal,
}
#[derive(Clone, Copy, Debug)]
pub enum DevicePreference {
    Auto,
    Cpu,
    Metal,
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct FilterTimings {
    pub backend: Backend,
    pub cpu_fallback: bool,
    pub guide_scale: u32,
    pub upload_ms: f64,
    pub download_ms: f64,
    pub albedo_ms: f64,
    pub normal_ms: f64,
    pub beauty_ms: f64,
    pub total_ms: f64,
    pub beauty_pixels: u64,
}

pub struct Denoiser {
    device: Cell<Device>,
    metal: RefCell<Option<metal::Metal>>,
    backend: Cell<Backend>,
    cpu_fallback: Cell<bool>,
    timings: Cell<FilterTimings>,
    half_guides: Cell<bool>,
    crop: Cell<bool>,
    quality: Cell<i32>,
    filter: RefCell<Option<Filters>>,
}

struct Filters {
    beauty: Filter,
    albedo: Filter,
    normal: Filter,
    clean_albedo: Vec<f32>,
    clean_normal: Vec<f32>,
    output: Vec<f32>,
}
impl Drop for Filters {
    fn drop(&mut self) {
        unsafe {
            for f in [self.beauty, self.albedo, self.normal] {
                if !f.is_null() {
                    oidnReleaseFilter(f);
                }
            }
        }
    }
}
unsafe extern "C" fn progress(user: *mut c_void, _: f64) -> bool {
    let (revision, expected) = unsafe { &*(user as *const (&AtomicU64, u64)) };
    revision.load(Ordering::Relaxed) == *expected
}

fn name(s: &str) -> CString {
    CString::new(s).expect("filter parameter name")
}

impl Denoiser {
    pub fn new() -> Result<Denoiser, String> {
        let preference = match std::env::var("PM_STUDIO_OIDN_DEVICE").as_deref() {
            Ok("cpu") => DevicePreference::Cpu,
            Ok("metal") => DevicePreference::Metal,
            _ => DevicePreference::Auto,
        };
        Self::new_with_device(preference)
    }
    pub fn new_with_device(preference: DevicePreference) -> Result<Self, String> {
        let metal = if matches!(preference, DevicePreference::Cpu) {
            None
        } else {
            match metal::Metal::new() {
                Ok(m) => Some(m),
                Err(error) => {
                    if matches!(preference, DevicePreference::Metal) {
                        return Err(error);
                    }
                    None
                }
            }
        };
        let available = metal.is_some();
        let d = Self {
            device: Cell::new(std::ptr::null_mut()),
            metal: RefCell::new(metal),
            backend: Cell::new(if available {
                Backend::Metal
            } else {
                Backend::Cpu
            }),
            cpu_fallback: Cell::new(!available && !matches!(preference, DevicePreference::Cpu)),
            timings: Cell::new(FilterTimings::default()),
            half_guides: Cell::new(false),
            crop: Cell::new(true),
            quality: Cell::new(QUALITY_HIGH),
            filter: RefCell::new(None),
        };
        if !available {
            d.ensure_cpu()?;
        }
        Ok(d)
    }
    pub fn backend(&self) -> Backend {
        self.backend.get()
    }
    fn ensure_cpu(&self) -> Result<(), String> {
        if !self.device.get().is_null() {
            return Ok(());
        }
        let device = unsafe { oidnNewDevice(DEVICE_TYPE_CPU) };
        if device.is_null() {
            return Err("Open Image Denoise CPU fallback is unavailable".into());
        }
        unsafe { oidnCommitDevice(device) };
        self.device.set(device);
        if let Err(error) = self.check() {
            self.device.set(std::ptr::null_mut());
            unsafe { oidnReleaseDevice(device) };
            return Err(error);
        }
        Ok(())
    }

    pub fn timings(&self) -> FilterTimings {
        self.timings.get()
    }
    /// Restrict zero-surrounded beauty work with a 64-pixel context halo.
    /// This changes no image resolution or sample/noise target.
    pub fn set_crop(&self, enabled: bool) {
        self.crop.set(enabled);
    }
    /// CPU fallback for bounded preview/production work. Beauty stays at full
    /// resolution; only stochastic albedo/normal prefiltering runs at half size.
    /// Final defaults to full-resolution guides unless the caller opts in.
    pub fn set_half_resolution_guides(&self, enabled: bool) {
        self.half_guides.set(enabled);
    }

    /// Balanced OIDN is available for bounded movie presets. Final retains
    /// the high-quality network; both still prefilter stochastic auxiliaries.
    pub fn set_quality(&self, quality: Quality) {
        self.quality.set(match quality {
            Quality::Fast => 4,
            Quality::Balanced => 5,
            Quality::High => QUALITY_HIGH,
        });
    }

    fn check(&self) -> Result<(), String> {
        let mut message: *const c_char = std::ptr::null();
        let code = unsafe { oidnGetDeviceError(self.device.get(), &mut message) };
        if code == 0 {
            return Ok(());
        }
        let text = if message.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        };
        Err(format!("Open Image Denoise error {code}: {text}"))
    }

    /// Denoise `color` in place. Images are RGBA f32 (`stride` floats per
    /// pixel, only RGB read); `albedo` and `normal` guide edges and texture.
    pub fn run(
        &self,
        color: &mut [f32],
        albedo: &[f32],
        normal: &[f32],
        stride: usize,
        width: usize,
        height: usize,
    ) -> Result<(), String> {
        self.run_interruptible(color, albedo, normal, stride, width, height, None)
    }
    pub fn run_interruptible(
        &self,
        color: &mut [f32],
        albedo: &[f32],
        normal: &[f32],
        stride: usize,
        width: usize,
        height: usize,
        cancel: Option<(&AtomicU64, u64)>,
    ) -> Result<(), String> {
        self.run_layers_interruptible(&mut [color], albedo, normal, stride, width, height, cancel)
    }

    /// Prefilter stochastic lens/specular guides once, then reuse them for all
    /// noisy layers. OIDN's cleanAux contract requires actual prefiltering.
    pub fn run_layers_interruptible(
        &self,
        colors: &mut [&mut [f32]],
        albedo: &[f32],
        normal: &[f32],
        stride: usize,
        width: usize,
        height: usize,
        cancel: Option<(&AtomicU64, u64)>,
    ) -> Result<(), String> {
        let full_start = std::time::Instant::now();
        let mut timings = FilterTimings::default();
        let pixels = width
            .checked_mul(height)
            .ok_or("invalid denoising dimensions")?;
        let needed = pixels
            .checked_mul(stride)
            .ok_or("invalid denoising dimensions")?;
        if stride < 3
            || pixels == 0
            || colors.iter().any(|c| c.len() < needed)
            || albedo.len() < needed
            || normal.len() < needed
        {
            return Err("invalid denoising image buffers".into());
        }
        if cancel.is_some_and(|(revision, expected)| revision.load(Ordering::Relaxed) != expected) {
            return Err("Open Image Denoise error 6: cancelled".into());
        }
        let result = {
            let mut backend = self.metal.borrow_mut();
            backend.as_mut().map(|m| {
                m.run(
                    colors,
                    albedo,
                    normal,
                    stride,
                    width,
                    height,
                    self.half_guides.get(),
                    self.crop.get(),
                    self.quality.get(),
                    cancel,
                )
            })
        };
        if let Some(result) = result {
            match result {
                Ok(timings) => {
                    self.timings.set(timings);
                    return Ok(());
                }
                Err(error) => {
                    if error.contains("error 6:")
                        || cancel.is_some_and(|(r, e)| r.load(Ordering::Relaxed) != e)
                    {
                        return Err(error);
                    }
                    // GPU output publication is transactional; original layers
                    // remain intact if any allocation/filter/readback fails.
                    self.metal.borrow_mut().take();
                    self.backend.set(Backend::Cpu);
                    self.cpu_fallback.set(true);
                    self.ensure_cpu()?;
                }
            }
        }
        timings.backend = Backend::Cpu;
        timings.cpu_fallback = self.cpu_fallback.get();
        self.ensure_cpu()?;
        let mut cached = self.filter.borrow_mut();
        if cached.is_none() {
            let mut filters = Filters {
                beauty: std::ptr::null_mut(),
                albedo: std::ptr::null_mut(),
                normal: std::ptr::null_mut(),
                clean_albedo: vec![],
                clean_normal: vec![],
                output: vec![],
            };
            for slot in [
                &mut filters.beauty,
                &mut filters.albedo,
                &mut filters.normal,
            ] {
                *slot = unsafe { oidnNewFilter(self.device.get(), name("RT").as_ptr()) };
                if slot.is_null() {
                    self.check()?;
                    return Err("Open Image Denoise could not create a filter".into());
                }
            }
            *cached = Some(filters);
        }
        let f = cached.as_mut().unwrap();
        f.clean_albedo.resize(pixels * 3, 0.);
        f.clean_normal.resize(pixels * 3, 0.);
        f.output.resize(pixels * 3, 0.);
        let execute = |filter: Filter| -> Result<(), String> {
            let mut monitor = cancel;
            unsafe {
                oidnCommitFilter(filter);
                if let Some(state) = monitor.as_mut() {
                    oidnSetFilterProgressMonitorFunction(
                        filter,
                        Some(progress),
                        (state as *mut (&AtomicU64, u64)).cast(),
                    );
                }
                oidnExecuteFilter(filter);
                oidnSetFilterProgressMonitorFunction(filter, None, std::ptr::null_mut());
            }
            self.check()
        };
        let bind = |filter: Filter,
                    parameter: &str,
                    ptr: *mut f32,
                    pixel_stride: usize,
                    dims: (usize, usize)| unsafe {
            oidnSetSharedFilterImage(
                filter,
                name(parameter).as_ptr(),
                ptr.cast(),
                FORMAT_FLOAT3,
                dims.0,
                dims.1,
                0,
                pixel_stride * 4,
                pixel_stride * 4 * dims.0,
            );
        };
        let half = self.half_guides.get() && width > 1 && height > 1;
        timings.guide_scale = if half { 2 } else { 1 };
        let guide_size = if half {
            (width.div_ceil(2), height.div_ceil(2))
        } else {
            (width, height)
        };
        let reduced = half.then(|| {
            (
                reduce_guide(albedo, stride, width, height, false),
                reduce_guide(normal, stride, width, height, true),
            )
        });
        let (guide_albedo, guide_normal) = reduced
            .as_ref()
            .map_or((albedo, normal), |(a, n)| (a.as_slice(), n.as_slice()));
        let guide_stride = if half { 3 } else { stride };
        for (filter, parameter, input, output) in [
            (f.albedo, "albedo", guide_albedo, &mut f.clean_albedo),
            (f.normal, "normal", guide_normal, &mut f.clean_normal),
        ] {
            bind(
                filter,
                parameter,
                input.as_ptr() as *mut f32,
                guide_stride,
                guide_size,
            );
            bind(filter, "output", output.as_mut_ptr(), 3, guide_size);
            unsafe {
                oidnSetFilterInt(filter, name("quality").as_ptr(), self.quality.get());
            }
            let start = std::time::Instant::now();
            execute(filter)?;
            let ms = start.elapsed().as_secs_f64() * 1000.;
            if parameter == "albedo" {
                timings.albedo_ms = ms;
            } else {
                timings.normal_ms = ms;
            }
        }
        if half {
            let albedo = expand_guide(
                &f.clean_albedo[..guide_size.0 * guide_size.1 * 3],
                guide_size,
                width,
                height,
                false,
            );
            let normal = expand_guide(
                &f.clean_normal[..guide_size.0 * guide_size.1 * 3],
                guide_size,
                width,
                height,
                true,
            );
            f.clean_albedo.copy_from_slice(&albedo);
            f.clean_normal.copy_from_slice(&normal);
        }

        unsafe {
            oidnSetFilterBool(f.beauty, name("hdr").as_ptr(), true);
            oidnSetFilterBool(f.beauty, name("cleanAux").as_ptr(), true);
            oidnSetFilterInt(f.beauty, name("quality").as_ptr(), self.quality.get());
            // NaN explicitly requests OIDN's HDR autoexposure. Color is linear
            // scene radiance, before the renderer's display exposure/tone map.
            oidnSetFilterFloat(f.beauty, name("inputScale").as_ptr(), f32::NAN);
        }
        let beauty_start = std::time::Instant::now();
        for color in colors {
            // Closed rooms and visible skies often have exactly unit catcher
            // shading. Preserve a constant RGB layer instead of filtering it.
            let constant = [color[0], color[1], color[2]];
            if constant.iter().all(|v| v.is_finite())
                && color
                    .chunks_exact(stride)
                    .take(pixels)
                    .all(|p| p[..3] == constant)
            {
                continue;
            }
            let region = if self.crop.get() && self.quality.get() != QUALITY_HIGH {
                beauty_region(color, stride, width, height)
            } else {
                (0, 0, width, height)
            };
            let (x0, y0, rw, rh) = region;
            let offset = y0 * width + x0;
            let bind_region =
                |parameter: &str, ptr: *mut f32, pixel_stride: usize, row_width: usize| unsafe {
                    oidnSetSharedFilterImage(
                        f.beauty,
                        name(parameter).as_ptr(),
                        ptr.cast(),
                        FORMAT_FLOAT3,
                        rw,
                        rh,
                        0,
                        pixel_stride * 4,
                        row_width * pixel_stride * 4,
                    );
                };
            // Guides keep their full-image row stride; output is tightly packed.
            bind_region(
                "albedo",
                unsafe { f.clean_albedo.as_mut_ptr().add(offset * 3) },
                3,
                width,
            );
            bind_region(
                "normal",
                unsafe { f.clean_normal.as_mut_ptr().add(offset * 3) },
                3,
                width,
            );
            bind_region(
                "color",
                unsafe { color.as_mut_ptr().add(offset * stride) },
                stride,
                width,
            );
            bind_region("output", f.output.as_mut_ptr(), 3, rw);
            execute(f.beauty)?;
            timings.beauty_pixels += (rw * rh) as u64;
            let output = &f.output;
            crate::parallel::rows(
                &mut color[y0 * width * stride..(y0 + rh) * width * stride],
                width * stride,
                |y, row| {
                    for (x, pixel) in row[x0 * stride..(x0 + rw) * stride]
                        .chunks_exact_mut(stride)
                        .enumerate()
                    {
                        pixel[..3].copy_from_slice(&output[(y * rw + x) * 3..][..3]);
                    }
                },
            );
        }
        timings.beauty_ms = beauty_start.elapsed().as_secs_f64() * 1000.;
        timings.total_ms = full_start.elapsed().as_secs_f64() * 1000.;
        self.timings.set(timings);
        Ok(())
    }
}

// OIDN need not evaluate a black beauty field far outside the product. Retain
// a wide context halo and full-size stochastic guides; nonzero surroundings,
// small images or a large product keep the original full-frame path.
fn beauty_region(color: &[f32], stride: usize, w: usize, h: usize) -> (usize, usize, usize, usize) {
    if w * h < 300_000 || color[..3] != [0.; 3] {
        return (0, 0, w, h);
    }
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let rows = h.div_ceil(workers);
    let bounds = std::thread::scope(|scope| {
        let jobs: Vec<_> = color[..w * h * stride]
            .chunks(rows * w * stride)
            .enumerate()
            .map(|(group, chunk)| {
                scope.spawn(move || {
                    let mut lo = (w, h);
                    let mut hi = (0, 0);
                    for (row_index, row) in chunk.chunks_exact(w * stride).enumerate() {
                        let y = group * rows + row_index;
                        let nonzero = |p: &[f32]| p[0] != 0. || p[1] != 0. || p[2] != 0.;
                        if let Some(left) = row.chunks_exact(stride).position(nonzero) {
                            let right = row.chunks_exact(stride).rposition(nonzero).unwrap();
                            lo = (lo.0.min(left), lo.1.min(y));
                            hi = (hi.0.max(right + 1), hi.1.max(y + 1));
                        }
                    }
                    (lo, hi)
                })
            })
            .collect();
        jobs.into_iter()
            .map(|j| j.join().expect("beauty bounds worker"))
            .collect::<Vec<_>>()
    });
    let mut lo = (w, h);
    let mut hi = (0, 0);
    for (a, b) in bounds {
        lo = (lo.0.min(a.0), lo.1.min(a.1));
        hi = (hi.0.max(b.0), hi.1.max(b.1));
    }
    if hi == (0, 0) {
        return (0, 0, w, h);
    }
    let x0 = lo.0.saturating_sub(64);
    let y0 = lo.1.saturating_sub(64);
    let x1 = (hi.0 + 64).min(w);
    let y1 = (hi.1 + 64).min(h);
    if (x1 - x0) * (y1 - y0) > w * h * 4 / 5 {
        (0, 0, w, h)
    } else {
        (x0, y0, x1 - x0, y1 - y0)
    }
}

fn normalize_guide(p: &mut [f32]) {
    let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    if length > 1e-8 {
        for v in p {
            *v /= length;
        }
    }
}
fn reduce_guide(input: &[f32], stride: usize, w: usize, h: usize, normal: bool) -> Vec<f32> {
    let (rw, rh) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0.; rw * rh * 3];
    crate::parallel::rows(&mut out, rw * 3, |y, row| {
        for x in 0..rw {
            let pixel = &mut row[x * 3..][..3];
            let mut count = 0.;
            for sy in y * 2..(y * 2 + 2).min(h) {
                for sx in x * 2..(x * 2 + 2).min(w) {
                    for c in 0..3 {
                        pixel[c] += input[(sy * w + sx) * stride + c];
                    }
                    count += 1.;
                }
            }
            for v in pixel.iter_mut() {
                *v /= count;
            }
            if normal {
                normalize_guide(pixel);
            }
        }
    });
    out
}
fn expand_guide(input: &[f32], size: (usize, usize), w: usize, h: usize, normal: bool) -> Vec<f32> {
    let (rw, rh) = size;
    let mut out = vec![0.; w * h * 3];
    crate::parallel::rows(&mut out, w * 3, |y, row| {
        let sy = ((y as f32 + 0.5) * rh as f32 / h as f32 - 0.5).clamp(0., (rh - 1) as f32);
        let y0 = sy as usize;
        let y1 = (y0 + 1).min(rh - 1);
        let fy = sy - y0 as f32;
        for x in 0..w {
            let sx = ((x as f32 + 0.5) * rw as f32 / w as f32 - 0.5).clamp(0., (rw - 1) as f32);
            let x0 = sx as usize;
            let x1 = (x0 + 1).min(rw - 1);
            let fx = sx - x0 as f32;
            let pixel = &mut row[x * 3..][..3];
            for c in 0..3 {
                let top =
                    input[(y0 * rw + x0) * 3 + c] * (1. - fx) + input[(y0 * rw + x1) * 3 + c] * fx;
                let bottom =
                    input[(y1 * rw + x0) * 3 + c] * (1. - fx) + input[(y1 * rw + x1) * 3 + c] * fx;
                pixel[c] = top * (1. - fy) + bottom * fy;
            }
            if normal {
                normalize_guide(pixel);
            }
        }
    });
    out
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        drop(self.metal.get_mut().take());
        drop(self.filter.get_mut().take());
        if !self.device.get().is_null() {
            unsafe { oidnReleaseDevice(self.device.get()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_pointer_binding_explicitly_uses_a_cpu_device() {
        let denoiser=Denoiser::new_with_device(DevicePreference::Cpu).unwrap();
        assert_eq!(unsafe{oidnGetDeviceInt(denoiser.device.get(),name("type").as_ptr())},1);
    }

    #[test]
    fn half_resolution_guides_keep_full_size_beauty_and_survive_rebinding() {
        let reused = Denoiser::new().unwrap();
        for (w, h, half) in [
            (33, 25, true),
            (20, 16, false),
            (33, 25, true),
            (513, 257, true),
        ] {
            let input: Vec<f32> = (0..w * h)
                .flat_map(|i| [0.5 + if i % 2 == 0 { 0.1 } else { -0.1 }, 0.4, 0.3, 0.25])
                .collect();
            let albedo = vec![0.5; w * h * 4];
            let normal: Vec<f32> = (0..w * h).flat_map(|_| [0., 0., 1., 0.]).collect();
            reused.set_half_resolution_guides(half);
            let mut actual = input.clone();
            reused.run(&mut actual, &albedo, &normal, 4, w, h).unwrap();
            let fresh = Denoiser::new().unwrap();
            fresh.set_half_resolution_guides(half);
            let mut expected = input.clone();
            fresh.run(&mut expected, &albedo, &normal, 4, w, h).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(actual.len(), w * h * 4);
            assert!(actual.chunks_exact(4).all(|p| p[3] == 0.25));
            let error = |p: &[f32]| p.chunks_exact(4).map(|p| (p[0] - 0.5).powi(2)).sum::<f32>();
            assert!(error(&actual) < error(&input) * 0.3);
        }
    }

    #[test]
    fn a_constant_layer_is_exact_while_its_noisy_companion_is_filtered() {
        let (w, h) = (32, 24);
        let expected = 0.5;
        let mut color: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = if i % 2 == 0 { 0.3 } else { 0.7 };
                [v, v, v, 0.25]
            })
            .collect();
        let mut shade = vec![1.; w * h * 4];
        let original_shade = shade.clone();
        let albedo = vec![0.5; w * h * 4];
        let normal: Vec<f32> = (0..w * h).flat_map(|_| [0., 0., 1., 0.]).collect();
        let error = |pixels: &[f32]| {
            pixels
                .chunks_exact(4)
                .map(|p| (p[0] - expected).powi(2))
                .sum::<f32>()
        };
        let before = error(&color);
        Denoiser::new()
            .unwrap()
            .run_layers_interruptible(
                &mut [&mut color, &mut shade],
                &albedo,
                &normal,
                4,
                w,
                h,
                None,
            )
            .unwrap();
        assert_eq!(shade, original_shade);
        assert!(error(&color) < before * 0.3);
        assert!(color.chunks_exact(4).all(|p| p[3] == 0.25));
    }

    #[test]
    fn changing_quality_on_cached_filters_matches_a_fresh_filter() {
        let (w, h) = (32, 24);
        let input: Vec<f32> = (0..w * h)
            .flat_map(|i| [0.2 + (i % 11) as f32 * 0.01, 0.4, 0.6, 1.])
            .collect();
        let albedo = vec![0.6; w * h * 4];
        let normal: Vec<f32> = (0..w * h).flat_map(|_| [0., 0., 1., 0.]).collect();
        let reused = Denoiser::new().unwrap();
        for quality in [Quality::Fast, Quality::Balanced, Quality::High] {
            reused.set_quality(quality);
            let mut actual = input.clone();
            reused.run(&mut actual, &albedo, &normal, 4, w, h).unwrap();
            let fresh = Denoiser::new().unwrap();
            fresh.set_quality(quality);
            let mut expected = input.clone();
            fresh.run(&mut expected, &albedo, &normal, 4, w, h).unwrap();
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-6)
            );
        }
    }

    #[test]
    fn it_removes_noise_from_a_flat_image() {
        let (w, h) = (64usize, 64usize);
        let mut seed = 1u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        // A flat 0.5 grey under heavy noise, flat albedo and normals.
        let mut color: Vec<f32> = (0..w * h)
            .flat_map(|_| {
                let v = 0.5 + (rnd() - 0.5) * 0.6;
                [v, v, v, 1.0]
            })
            .collect();
        let albedo = vec![0.8f32; w * h * 4];
        let normal: Vec<f32> = (0..w * h).flat_map(|_| [0.0, 0.0, 1.0, 0.0]).collect();
        let spread = |c: &[f32]| {
            let m = c.chunks(4).map(|p| p[0]).sum::<f32>() / (w * h) as f32;
            c.chunks(4).map(|p| (p[0] - m).powi(2)).sum::<f32>() / (w * h) as f32
        };
        let before = spread(&color);
        Denoiser::new()
            .unwrap()
            .run(&mut color, &albedo, &normal, 4, w, h)
            .unwrap();
        assert!(
            spread(&color) < before / 20.0,
            "{} -> {}",
            before,
            spread(&color)
        );
    }
    #[test]
    fn reused_filter_matches_a_fresh_filter_after_resizing_and_rebinding() {
        let reused = Denoiser::new().unwrap();
        for (width, height) in [(32, 24), (64, 48), (32, 24)] {
            let input: Vec<f32> = (0..width * height)
                .flat_map(|i| [0.2 + (i % 13) as f32 * 0.01, 0.4, 0.6, 1.])
                .collect();
            let albedo = vec![0.6; width * height * 4];
            let normal: Vec<f32> = (0..width * height).flat_map(|_| [0., 0., 1., 0.]).collect();
            let mut a = input.clone();
            let mut b = input;
            reused
                .run(&mut a, &albedo, &normal, 4, width, height)
                .unwrap();
            Denoiser::new()
                .unwrap()
                .run(&mut b, &albedo, &normal, 4, width, height)
                .unwrap();
            assert!(a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-6));
        }
    }
    #[test]
    fn interrupted_filter_can_render_the_next_image() {
        let denoiser = Denoiser::new().unwrap();
        let width = 128;
        let height = 128;
        let input = vec![0.5; width * height * 4];
        let albedo = input.clone();
        let normal: Vec<f32> = (0..width * height).flat_map(|_| [0., 0., 1., 0.]).collect();
        let revision = AtomicU64::new(1);
        let mut interrupted = input.clone();
        let _ = denoiser.run_interruptible(
            &mut interrupted,
            &albedo,
            &normal,
            4,
            width,
            height,
            Some((&revision, 0)),
        );
        let mut actual = input.clone();
        let mut expected = input;
        denoiser
            .run(&mut actual, &albedo, &normal, 4, width, height)
            .unwrap();
        Denoiser::new()
            .unwrap()
            .run(&mut expected, &albedo, &normal, 4, width, height)
            .unwrap();
        assert!(
            actual
                .iter()
                .zip(expected.iter())
                .all(|(x, y)| (x - y).abs() < 1e-6)
        );
    }
}

#[cfg(test)]
mod parallel_buffer_tests {
    #[test]
    fn large_padded_color_keeps_tail_and_non_rgb_channels() {
        let (w, h, stride) = (513, 257, 5);
        let needed = w * h * stride;
        let mut color: Vec<_> = (0..w * h)
            .flat_map(|i| {
                [
                    0.5 + if i % 2 == 0 { 0.1 } else { -0.1 },
                    0.4,
                    0.3,
                    0.25,
                    0.875,
                ]
            })
            .collect();
        color.extend([0.123; 17]);
        let albedo = vec![0.5; needed];
        let normal: Vec<_> = (0..w * h).flat_map(|_| [0., 0., 1., 0., 0.]).collect();
        let denoiser = super::Denoiser::new().unwrap();
        denoiser.set_half_resolution_guides(true);
        denoiser
            .run(&mut color, &albedo, &normal, stride, w, h)
            .unwrap();
        assert!(
            color[..needed]
                .chunks_exact(stride)
                .all(|p| p[..3].iter().all(|v| v.is_finite()) && p[3] == 0.25 && p[4] == 0.875)
        );
        assert_eq!(&color[needed..], &[0.123; 17]);
    }
}

#[cfg(test)]
mod crop_tests {
    use super::*;
    #[test]
    fn occupied_beauty_crop_matches_full_frame_and_preserves_padded_channels() {
        let (w, h, stride) = (1024, 512, 5);
        let mut input = vec![0.; w * h * stride];
        let mut albedo = input.clone();
        let mut normal = input.clone();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * stride;
                input[i + 3] = 0.25;
                input[i + 4] = 0.875;
                if (400..624).contains(&x) && (200..312).contains(&y) {
                    input[i..i + 3].fill(0.5 + if (x + y) % 2 == 0 { 0.08 } else { -0.08 });
                    albedo[i..i + 3].fill(0.5);
                    normal[i + 2] = 1.;
                }
            }
        }
        let run = |crop| {
            let d = Denoiser::new().unwrap();
            d.set_quality(Quality::Balanced);
            d.set_half_resolution_guides(true);
            d.set_crop(crop);
            let mut out = input.clone();
            d.run(&mut out, &albedo, &normal, stride, w, h).unwrap();
            (out, d.timings())
        };
        let (expected, full) = run(false);
        let (actual, cropped) = run(true);
        assert!(
            cropped.beauty_pixels < full.beauty_pixels / 2,
            "crop was not exercised"
        );
        let mut error = 0.;
        let mut n = 0;
        for y in 200..312 {
            for x in 400..624 {
                let i = (y * w + x) * stride;
                for c in 0..3 {
                    error += (actual[i + c] - expected[i + c]).powi(2);
                    n += 1;
                }
            }
        }
        let rms = (error / n as f32).sqrt();
        assert!(rms < 0.005, "crop/full object RMS {rms}");
        assert!(
            actual
                .chunks_exact(stride)
                .all(|p| p[3] == 0.25 && p[4] == 0.875)
        );
        assert_eq!(&actual[..3], &[0.; 3]);
        let final_denoiser = Denoiser::new().unwrap();
        let mut final_image = input.clone();
        final_denoiser
            .run(&mut final_image, &albedo, &normal, stride, w, h)
            .unwrap();
        assert_eq!(
            final_denoiser.timings().beauty_pixels,
            (w * h) as u64,
            "Final retains full-frame context"
        );
    }
}
