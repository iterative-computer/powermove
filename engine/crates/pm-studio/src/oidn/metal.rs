//! OIDN-owned Metal allocations. Ordinary Rust Vec pointers are never bound.
use super::*;
type Buffer = *mut c_void;
unsafe extern "C" {
    fn oidnNewBufferWithStorage(device: Device, bytes: usize, storage: i32) -> Buffer;
    fn oidnReleaseBuffer(buffer: Buffer);
    fn oidnWriteBuffer(buffer: Buffer, offset: usize, bytes: usize, source: *const c_void);
    fn oidnReadBuffer(buffer: Buffer, offset: usize, bytes: usize, target: *mut c_void);
    fn oidnSetFilterImage(
        filter: Filter,
        name: *const c_char,
        buffer: Buffer,
        format: i32,
        width: usize,
        height: usize,
        offset: usize,
        pixel_stride: usize,
        row_stride: usize,
    );
    fn oidnGetDeviceInt(device: Device, name: *const c_char) -> i32;
}
struct Owned {
    raw: Buffer,
    bytes: usize,
}
impl Default for Owned {
    fn default() -> Self {
        Self {
            raw: std::ptr::null_mut(),
            bytes: 0,
        }
    }
}
impl Owned {
    fn release(&mut self) {
        if !self.raw.is_null() {
            unsafe { oidnReleaseBuffer(self.raw) };
            self.raw = std::ptr::null_mut();
            self.bytes = 0;
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.release();
    }
}
pub(super) struct Metal {
    device: Device,
    filters: [Filter; 3],
    buffers: [Owned; 6],
}
impl Drop for Metal {
    fn drop(&mut self) {
        for f in self.filters {
            if !f.is_null() {
                unsafe { oidnReleaseFilter(f) };
            }
        }
        for b in &mut self.buffers {
            b.release();
        }
        unsafe { oidnReleaseDevice(self.device) };
    }
}
impl Metal {
    pub(super) fn new() -> Result<Self, String> {
        let device = unsafe { oidnNewDevice(5) };
        if device.is_null() {
            let mut message = std::ptr::null();
            unsafe { oidnGetDeviceError(device, &mut message) };
            return Err("Open Image Denoise Metal device unavailable".into());
        }
        let mut this = Self {
            device,
            filters: [std::ptr::null_mut(); 3],
            buffers: Default::default(),
        };
        unsafe { oidnCommitDevice(device) };
        this.check()?;
        if unsafe { oidnGetDeviceInt(device, name("type").as_ptr()) } != 5 {
            return Err("Open Image Denoise did not select Metal".into());
        }
        for i in 0..3 {
            this.filters[i] = unsafe { oidnNewFilter(device, name("RT").as_ptr()) };
            this.check()?;
            if this.filters[i].is_null() {
                return Err("Open Image Denoise Metal filter unavailable".into());
            }
        }
        Ok(this)
    }
    fn check(&self) -> Result<(), String> {
        let mut message = std::ptr::null();
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
    fn ensure(&mut self, slot: usize, floats: usize) -> Result<(), String> {
        let bytes = floats.checked_mul(4).ok_or("OIDN Metal buffer overflow")?;
        if self.buffers[slot].bytes < bytes {
            let raw = unsafe { oidnNewBufferWithStorage(self.device, bytes, 2) };
            let owned = Owned { raw, bytes };
            self.check()?;
            if owned.raw.is_null() {
                return Err("OIDN Metal allocation failed".into());
            }
            self.buffers[slot].release();
            self.buffers[slot] = owned;
        }
        Ok(())
    }
    fn upload(&mut self, slot: usize, values: &[f32]) -> Result<(), String> {
        self.ensure(slot, values.len())?;
        unsafe {
            oidnWriteBuffer(
                self.buffers[slot].raw,
                0,
                values.len() * 4,
                values.as_ptr().cast(),
            )
        };
        self.check()
    }
    fn download(&self, slot: usize, floats: usize) -> Result<Vec<f32>, String> {
        let mut out = vec![0f32; floats];
        unsafe {
            oidnReadBuffer(
                self.buffers[slot].raw,
                0,
                floats * 4,
                out.as_mut_ptr().cast(),
            )
        };
        self.check()?;
        if out.iter().any(|v| !v.is_finite()) {
            return Err("Open Image Denoise Metal produced non-finite output".into());
        }
        Ok(out)
    }
    fn bind(
        &self,
        filter: usize,
        param: &str,
        slot: usize,
        dims: (usize, usize),
        offset: usize,
        stride: usize,
        row: usize,
    ) {
        unsafe {
            oidnSetFilterImage(
                self.filters[filter],
                name(param).as_ptr(),
                self.buffers[slot].raw,
                FORMAT_FLOAT3,
                dims.0,
                dims.1,
                offset * 4,
                stride * 4,
                row * stride * 4,
            )
        };
    }
    fn execute(&self, index: usize, cancel: Option<(&AtomicU64, u64)>) -> Result<(), String> {
        let filter = self.filters[index];
        let mut monitor = cancel;
        unsafe { oidnCommitFilter(filter) };
        self.check()?;
        unsafe {
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
    }
    pub(super) fn run(
        &mut self,
        colors: &mut [&mut [f32]],
        albedo: &[f32],
        normal: &[f32],
        stride: usize,
        w: usize,
        h: usize,
        half: bool,
        crop: bool,
        quality: i32,
        cancel: Option<(&AtomicU64, u64)>,
    ) -> Result<FilterTimings, String> {
        let start = std::time::Instant::now();
        let pixels = w * h;
        let mut times = FilterTimings {
            backend: Backend::Metal,
            guide_scale: 1,
            ..Default::default()
        };
        let constant = |color: &[f32]| {
            let rgb = &color[..3];
            rgb.iter().all(|v| v.is_finite())
                && color
                    .chunks_exact(stride)
                    .take(pixels)
                    .all(|p| &p[..3] == rgb)
        };
        if colors.iter().all(|c| constant(c)) {
            return Ok(times);
        }
        let half = half && w > 1 && h > 1;
        let dims = if half {
            (w.div_ceil(2), h.div_ceil(2))
        } else {
            (w, h)
        };
        times.guide_scale = if half { 2 } else { 1 };
        let reduced = half.then(|| {
            (
                reduce_guide(albedo, stride, w, h, false),
                reduce_guide(normal, stride, w, h, true),
            )
        });
        let (a, n) = reduced.as_ref().map_or(
            (&albedo[..pixels * stride], &normal[..pixels * stride]),
            |(a, n)| (a.as_slice(), n.as_slice()),
        );
        let guide_stride = if half { 3 } else { stride };
        for (filter, param, raw, clean, input, is_normal) in
            [(1, "albedo", 1, 3, a, false), (2, "normal", 2, 4, n, true)]
        {
            let t = std::time::Instant::now();
            self.upload(raw, input)?;
            times.upload_ms += t.elapsed().as_secs_f64() * 1000.;
            self.ensure(clean, pixels * 3)?;
            self.bind(filter, param, raw, dims, 0, guide_stride, dims.0);
            self.bind(filter, "output", clean, dims, 0, 3, dims.0);
            unsafe { oidnSetFilterInt(self.filters[filter], name("quality").as_ptr(), quality) };
            let t = std::time::Instant::now();
            self.execute(filter, cancel)?;
            let elapsed = t.elapsed().as_secs_f64() * 1000.;
            if is_normal {
                times.normal_ms = elapsed;
            } else {
                times.albedo_ms = elapsed;
            }
            if half {
                let t = std::time::Instant::now();
                let small = self.download(clean, dims.0 * dims.1 * 3)?;
                times.download_ms += t.elapsed().as_secs_f64() * 1000.;
                let full = expand_guide(&small, dims, w, h, is_normal);
                let t = std::time::Instant::now();
                self.upload(clean, &full)?;
                times.upload_ms += t.elapsed().as_secs_f64() * 1000.;
            }
        }
        unsafe {
            oidnSetFilterBool(self.filters[0], name("hdr").as_ptr(), true);
            oidnSetFilterBool(self.filters[0], name("cleanAux").as_ptr(), true);
            oidnSetFilterInt(self.filters[0], name("quality").as_ptr(), quality);
            oidnSetFilterFloat(self.filters[0], name("inputScale").as_ptr(), f32::NAN);
        }
        // Defer publication until every layer succeeds: CPU fallback sees the
        // original inputs, never a partially or twice-denoised color layer.
        let mut pending = Vec::new();
        for (index, color) in colors.iter().enumerate() {
            if constant(color) {
                continue;
            }
            let (x, y, rw, rh) = if crop && quality != QUALITY_HIGH {
                beauty_region(color, stride, w, h)
            } else {
                (0, 0, w, h)
            };
            let offset = y * w + x;
            let t = std::time::Instant::now();
            self.upload(0, &color[..pixels * stride])?;
            times.upload_ms += t.elapsed().as_secs_f64() * 1000.;
            self.ensure(5, rw * rh * 3)?;
            self.bind(0, "color", 0, (rw, rh), offset * stride, stride, w);
            self.bind(0, "albedo", 3, (rw, rh), offset * 3, 3, w);
            self.bind(0, "normal", 4, (rw, rh), offset * 3, 3, w);
            self.bind(0, "output", 5, (rw, rh), 0, 3, rw);
            let t = std::time::Instant::now();
            self.execute(0, cancel)?;
            times.beauty_ms += t.elapsed().as_secs_f64() * 1000.;
            times.beauty_pixels += (rw * rh) as u64;
            let t = std::time::Instant::now();
            let output = self.download(5, rw * rh * 3)?;
            times.download_ms += t.elapsed().as_secs_f64() * 1000.;
            pending.push((index, (x, y, rw, rh), output));
        }
        for (index, (x, y, rw, rh), output) in pending {
            crate::parallel::rows(
                &mut colors[index][y * w * stride..(y + rh) * w * stride],
                w * stride,
                |row, pixels| {
                    for (col, p) in pixels[x * stride..(x + rw) * stride]
                        .chunks_exact_mut(stride)
                        .enumerate()
                    {
                        p[..3].copy_from_slice(&output[(row * rw + col) * 3..][..3]);
                    }
                },
            );
        }
        times.total_ms = start.elapsed().as_secs_f64() * 1000.;
        Ok(times)
    }
}
