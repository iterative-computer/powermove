//! Intel Open Image Denoise for Final frames: a minimal binding to the C API
//! (only what the RT filter needs).

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};

type Device = *mut c_void;
type Filter = *mut c_void;
const DEVICE_TYPE_DEFAULT: i32 = 0;
const FORMAT_FLOAT3: i32 = 3;
const QUALITY_HIGH: i32 = 6;

unsafe extern "C" {
    fn oidnNewDevice(kind: i32) -> Device;
    fn oidnCommitDevice(device: Device);
    fn oidnReleaseDevice(device: Device);
    fn oidnGetDeviceError(device: Device, message: *mut *const c_char) -> i32;
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

pub struct Denoiser {
    device: Device,
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
        let device = unsafe { oidnNewDevice(DEVICE_TYPE_DEFAULT) };
        if device.is_null() {
            return Err("Open Image Denoise has no device".into());
        }
        unsafe { oidnCommitDevice(device) };
        let d = Denoiser {
            device,
            filter: RefCell::new(None),
        };
        d.check()?;
        Ok(d)
    }

    fn check(&self) -> Result<(), String> {
        let mut message: *const c_char = std::ptr::null();
        let code = unsafe { oidnGetDeviceError(self.device, &mut message) };
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
                *slot = unsafe { oidnNewFilter(self.device, name("RT").as_ptr()) };
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
        let bind = |filter: Filter, parameter: &str, ptr: *mut f32, pixel_stride: usize| unsafe {
            oidnSetSharedFilterImage(
                filter,
                name(parameter).as_ptr(),
                ptr.cast(),
                FORMAT_FLOAT3,
                width,
                height,
                0,
                pixel_stride * 4,
                pixel_stride * 4 * width,
            );
        };
        for (filter, parameter, input, output) in [
            (f.albedo, "albedo", albedo, &mut f.clean_albedo),
            (f.normal, "normal", normal, &mut f.clean_normal),
        ] {
            bind(filter, parameter, input.as_ptr() as *mut f32, stride);
            bind(filter, "output", output.as_mut_ptr(), 3);
            unsafe {
                oidnSetFilterInt(filter, name("quality").as_ptr(), QUALITY_HIGH);
            }
            execute(filter)?;
        }
        bind(f.beauty, "albedo", f.clean_albedo.as_mut_ptr(), 3);
        bind(f.beauty, "normal", f.clean_normal.as_mut_ptr(), 3);
        bind(f.beauty, "output", f.output.as_mut_ptr(), 3);
        unsafe {
            oidnSetFilterBool(f.beauty, name("hdr").as_ptr(), true);
            oidnSetFilterBool(f.beauty, name("cleanAux").as_ptr(), true);
            oidnSetFilterInt(f.beauty, name("quality").as_ptr(), QUALITY_HIGH);
            // NaN explicitly requests OIDN's HDR autoexposure. Color is linear
            // scene radiance, before the renderer's display exposure/tone map.
            oidnSetFilterFloat(f.beauty, name("inputScale").as_ptr(), f32::NAN);
        }
        for color in colors {
            bind(f.beauty, "color", color.as_mut_ptr(), stride);
            execute(f.beauty)?;
            for i in 0..pixels {
                color[i * stride..i * stride + 3].copy_from_slice(&f.output[i * 3..i * 3 + 3]);
            }
        }
        Ok(())
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        drop(self.filter.get_mut().take());
        unsafe { oidnReleaseDevice(self.device) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
