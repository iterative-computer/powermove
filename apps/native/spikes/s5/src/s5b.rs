//! S5b: immutable engine-side surfaces. The control channel carries only IDs and
//! recorder commands. Encoded image resources are an explicit pixel-demand path,
//! not an IOSurface/zero-copy claim. Everything here is synthetic, not pm-render.
use serde_json::json;
use std::{
    collections::HashMap,
    ffi::{c_char, c_void},
    sync::{Arc, Mutex},
    time::Instant,
};
type Ref = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(p: Ref);
    fn CFDataCreateMutable(a: Ref, capacity: isize) -> Ref;
    fn CFDataGetLength(p: Ref) -> isize;
    fn CFDataGetBytePtr(p: Ref) -> *const u8;
    fn CFStringCreateWithCString(a: Ref, s: *const c_char, encoding: u32) -> Ref;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateWithName(name: Ref) -> Ref;
    static kCGColorSpaceSRGB: Ref;
    fn CGDataProviderCreateWithData(
        info: *mut c_void,
        data: Ref,
        size: usize,
        release: Option<unsafe extern "C" fn(*mut c_void, Ref, usize)>,
    ) -> Ref;
    fn CGImageCreate(
        w: usize,
        h: usize,
        bpc: usize,
        bpp: usize,
        row: usize,
        space: Ref,
        info: u32,
        provider: Ref,
        decode: *const f64,
        interpolate: bool,
        intent: u32,
    ) -> Ref;
}
#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithData(data: Ref, ty: Ref, count: usize, options: Ref) -> Ref;
    fn CGImageDestinationAddImage(dest: Ref, img: Ref, properties: Ref);
    fn CGImageDestinationFinalize(dest: Ref) -> bool;
}
fn encode(rgba: &[u8], ty: &std::ffi::CStr) -> Vec<u8> {
    // Borrowed provider remains valid until image/destination have been released.
    unsafe {
        let nil = std::ptr::null();
        let data = CFDataCreateMutable(nil, 0);
        let ty = CFStringCreateWithCString(nil, ty.as_ptr(), 0x08000100);
        let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        let provider = CGDataProviderCreateWithData(
            std::ptr::null_mut(),
            rgba.as_ptr().cast(),
            rgba.len(),
            None,
        );
        let image = CGImageCreate(
            1920,
            1080,
            8,
            32,
            1920 * 4,
            space,
            3,
            provider,
            nil.cast(),
            false,
            0,
        ); // unpremultiplied RGBA
        assert!(!image.is_null());
        let dest = CGImageDestinationCreateWithData(data, ty, 1, nil);
        assert!(!dest.is_null());
        CGImageDestinationAddImage(dest, image, nil);
        assert!(CGImageDestinationFinalize(dest));
        let out =
            std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize)
                .to_vec();
        for p in [dest, image, provider, space, ty, data] {
            CFRelease(p)
        }
        out
    }
}
fn bmp(rgba: &[u8]) -> Vec<u8> {
    // Conventional bottom-up BGRA32 BI_RGB BMP. Benchmark pixels are opaque.
    let mut out = vec![0u8; 54 + rgba.len()];
    out[..2].copy_from_slice(b"BM");
    let len = out.len() as u32;
    out[2..6].copy_from_slice(&len.to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&1920i32.to_le_bytes());
    out[22..26].copy_from_slice(&1080i32.to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&32u16.to_le_bytes());
    for y in 0..1080 {
        for x in 0..1920 {
            let src = (y * 1920 + x) * 4;
            let dst = 54 + ((1079 - y) * 1920 + x) * 4;
            out[dst..dst + 4].copy_from_slice(&[rgba[src + 2], rgba[src + 1], rgba[src], 255]);
        }
    }
    out
}
#[derive(Default)]
struct Jobs {
    next: u64,
    pending: HashMap<u64, u64>,
}
pub struct Surfaces {
    images: HashMap<String, Arc<Vec<u8>>>,
    jobs: Mutex<Jobs>,
}
impl Surfaces {
    pub fn new() -> Self {
        let mut images = HashMap::new();
        for kind in ["pattern", "noise", "outer"] {
            let mut rgba = crate::pixels::frame();
            if kind == "noise" {
                let mut n = 0x13245678u32;
                for p in rgba.chunks_exact_mut(4) {
                    n ^= n << 13;
                    n ^= n >> 17;
                    n ^= n << 5;
                    p[..3].copy_from_slice(&n.to_le_bytes()[..3]);
                }
            }
            if kind == "outer" {
                for y in 0..16 {
                    for x in 0..16 {
                        rgba[(y * 1920 + x) * 4..(y * 1920 + x) * 4 + 4]
                            .copy_from_slice(&[35, 180, 90, 255]);
                    }
                }
            }
            for format in ["bmp", "png", "tiff"] {
                let start = Instant::now();
                let bytes = match format {
                    "bmp" => bmp(&rgba),
                    "png" => encode(&rgba, c"public.png"),
                    _ => encode(&rgba, c"public.tiff"),
                };
                println!(
                    "{}",
                    json!({"test":"s5b_encode","surface":kind,"format":format,"bytes":bytes.len(),"ms":start.elapsed().as_secs_f64()*1000.})
                );
                images.insert(format!("{kind}.{format}"), Arc::new(bytes));
            }
        }
        Self {
            images,
            jobs: Mutex::new(Jobs::default()),
        }
    }
    pub fn request(&self, path: &str) -> (&'static str, Arc<Vec<u8>>) {
        let key = path.strip_prefix("/s5b/").unwrap_or("");
        let key = key.strip_prefix("stream/").unwrap_or(key);
        if let Some(bytes) = self.images.get(key) {
            let mime = if key.ends_with("bmp") {
                "image/bmp"
            } else if key.ends_with("png") {
                "image/png"
            } else {
                "image/tiff"
            };
            return (mime, bytes.clone());
        }
        let fields: Vec<_> = key.split('/').collect();
        let reply = match fields.as_slice() {
            ["id"] => "surface:pattern:7".into(),
            ["begin"] => {
                let mut jobs = self.jobs.lock().unwrap();
                jobs.next += 1;
                let id = jobs.next;
                jobs.pending.insert(id, 7);
                format!("need:{id}:7:pattern:4")
            }
            ["nested", "7"] => "surface:pattern:7".into(),
            // Four recorded fillRect commands are batched. No canvas readback or pixel upload.
            ["resume", id, rev, "fill-35-180-90-255x4"] => {
                let mut jobs = self.jobs.lock().unwrap();
                let id = id.parse::<u64>().unwrap_or(0);
                let rev = rev.parse::<u64>().unwrap_or(0);
                if jobs.pending.remove(&id) == Some(rev) && rev == 7 {
                    "surface:outer:7".into()
                } else {
                    "error:stale".into()
                }
            }
            _ => "error:unknown".into(),
        };
        ("text/plain", Arc::new(reply.into_bytes()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_is_pinned_single_use_and_unknown_ids_fail() {
        let s = Surfaces {
            images: HashMap::new(),
            jobs: Mutex::new(Jobs::default()),
        };
        let (_, b) = s.request("/s5b/begin");
        assert_eq!(&**b, b"need:1:7:pattern:4");
        for (path, want) in [
            ("/s5b/resume/1/8/fill-35-180-90-255x4", "error:stale"),
            ("/s5b/resume/1/7/fill-35-180-90-255x4", "error:stale"),
            ("/s5b/pixels", "error:unknown"),
        ] {
            assert_eq!(s.request(path).1.as_slice(), want.as_bytes());
        }
        s.request("/s5b/begin");
        assert_eq!(
            s.request("/s5b/resume/2/7/fill-35-180-90-255x4")
                .1
                .as_slice(),
            b"surface:outer:7"
        );
        assert_eq!(
            s.request("/s5b/resume/2/7/fill-35-180-90-255x4")
                .1
                .as_slice(),
            b"error:stale"
        );
    }
}
