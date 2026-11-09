//! Intel Open Image Denoise for Final frames: a minimal binding to the C API
//! (only what the RT filter needs).

use std::ffi::{CStr, CString, c_char, c_void};

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
    fn oidnSetSharedFilterImage(filter: Filter, name: *const c_char, ptr: *mut c_void, format: i32, width: usize, height: usize, byte_offset: usize, pixel_stride: usize, row_stride: usize);
    fn oidnSetFilterBool(filter: Filter, name: *const c_char, value: bool);
    fn oidnSetFilterInt(filter: Filter, name: *const c_char, value: i32);
    fn oidnCommitFilter(filter: Filter);
    fn oidnExecuteFilter(filter: Filter);
    fn oidnReleaseFilter(filter: Filter);
}

pub struct Denoiser {
    device: Device,
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
        let d = Denoiser { device };
        d.check()?;
        Ok(d)
    }

    fn check(&self) -> Result<(), String> {
        let mut message: *const c_char = std::ptr::null();
        let code = unsafe { oidnGetDeviceError(self.device, &mut message) };
        if code == 0 {
            return Ok(());
        }
        let text = if message.is_null() { String::new() } else { unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned() };
        Err(format!("Open Image Denoise error {code}: {text}"))
    }

    /// Denoise `color` in place. Images are RGBA f32 (`stride` floats per
    /// pixel, only RGB read); `albedo` and `normal` guide edges and texture.
    pub fn run(&self, color: &mut [f32], albedo: &[f32], normal: &[f32], stride: usize, width: usize, height: usize) -> Result<(), String> {
        let px = stride * 4;
        let mut out = vec![0f32; width * height * 3];
        unsafe {
            let f = oidnNewFilter(self.device, name("RT").as_ptr());
            oidnSetSharedFilterImage(f, name("color").as_ptr(), color.as_mut_ptr().cast(), FORMAT_FLOAT3, width, height, 0, px, px * width);
            oidnSetSharedFilterImage(f, name("albedo").as_ptr(), albedo.as_ptr() as *mut c_void, FORMAT_FLOAT3, width, height, 0, px, px * width);
            oidnSetSharedFilterImage(f, name("normal").as_ptr(), normal.as_ptr() as *mut c_void, FORMAT_FLOAT3, width, height, 0, px, px * width);
            oidnSetSharedFilterImage(f, name("output").as_ptr(), out.as_mut_ptr().cast(), FORMAT_FLOAT3, width, height, 0, 12, 12 * width);
            oidnSetFilterBool(f, name("hdr").as_ptr(), true);
            oidnSetFilterInt(f, name("quality").as_ptr(), QUALITY_HIGH);
            oidnCommitFilter(f);
            oidnExecuteFilter(f);
            oidnReleaseFilter(f);
        }
        self.check()?;
        for i in 0..width * height {
            color[i * stride..i * stride + 3].copy_from_slice(&out[i * 3..i * 3 + 3]);
        }
        Ok(())
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
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
        let mut color: Vec<f32> = (0..w * h).flat_map(|_| { let v = 0.5 + (rnd() - 0.5) * 0.6; [v, v, v, 1.0] }).collect();
        let albedo = vec![0.8f32; w * h * 4];
        let normal: Vec<f32> = (0..w * h).flat_map(|_| [0.0, 0.0, 1.0, 0.0]).collect();
        let spread = |c: &[f32]| { let m = c.chunks(4).map(|p| p[0]).sum::<f32>() / (w * h) as f32; c.chunks(4).map(|p| (p[0] - m).powi(2)).sum::<f32>() / (w * h) as f32 };
        let before = spread(&color);
        Denoiser::new().unwrap().run(&mut color, &albedo, &normal, 4, w, h).unwrap();
        assert!(spread(&color) < before / 20.0, "{} -> {}", before, spread(&color));
    }
}
