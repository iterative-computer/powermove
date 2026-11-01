//! Bounded CoreGraphics/CoreText stand-in. NOT pm-vector analytic/float paint.
//! Owns native bytes. Only the V8 sandbox allocator creates JS-visible buffers.
use serde_json::Value;
use std::{
    collections::HashMap,
    ffi::{c_char, c_void},
    sync::Arc,
};
type R = *const c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    w: f64,
    h: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    p: Point,
    s: Size,
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(p: R);
    fn CFStringCreateWithCString(a: R, s: *const c_char, encoding: u32) -> R;
    fn CFDataCreateMutable(a: R, capacity: isize) -> R;
    fn CFDataGetLength(d: R) -> isize;
    fn CFDataGetBytePtr(d: R) -> *const u8;
    fn CFDictionaryCreate(a: R, keys: *const R, values: *const R, n: isize, k: R, v: R) -> R;
    fn CFAttributedStringCreate(a: R, s: R, attributes: R) -> R;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> R;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        w: usize,
        h: usize,
        bpc: usize,
        row: usize,
        space: R,
        info: u32,
    ) -> R;
    fn CGBitmapContextCreateImage(c: R) -> R;
    fn CGContextRelease(c: R);
    fn CGContextSetRGBFillColor(c: R, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(c: R, r: Rect);
    fn CGContextBeginPath(c: R);
    fn CGContextMoveToPoint(c: R, x: f64, y: f64);
    fn CGContextAddLineToPoint(c: R, x: f64, y: f64);
    fn CGContextAddCurveToPoint(c: R, x: f64, y: f64, a: f64, b: f64, u: f64, v: f64);
    fn CGContextClosePath(c: R);
    fn CGContextFillPath(c: R);
    fn CGContextSetTextPosition(c: R, x: f64, y: f64);
    fn CGContextDrawImage(c: R, r: Rect, img: R);
    fn CGContextFlush(c: R);
    fn CGContextTranslateCTM(c: R, x: f64, y: f64);
    fn CGContextScaleCTM(c: R, x: f64, y: f64);
    fn CGContextSaveGState(c: R);
    fn CGContextRestoreGState(c: R);
}
#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    static kCTFontAttributeName: R;
    fn CTFontCreateWithName(name: R, size: f64, matrix: R) -> R;
    fn CTLineCreateWithAttributedString(s: R) -> R;
    fn CTLineDraw(line: R, c: R);
}
#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithData(data: R, ty: R, count: usize, opts: R) -> R;
    fn CGImageDestinationAddImage(dest: R, img: R, opts: R);
    fn CGImageDestinationFinalize(dest: R) -> bool;
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        p: Point { x, y },
        s: Size { w, h },
    }
}
fn number(v: &Value, i: usize) -> f64 {
    v[i].as_f64().unwrap()
}
unsafe fn string(s: &str) -> R {
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            std::ffi::CString::new(s).unwrap().as_ptr(),
            0x08000100,
        )
    }
}
pub struct Surface {
    pub w: usize,
    pub h: usize,
    pub bytes: Vec<u8>,
}
#[derive(Default)]
pub struct Raster {
    pub surfaces: HashMap<u64, Arc<Surface>>,
    next: u64,
    next_job: u64,
    pub pending: HashMap<u64, u64>,
}
impl Raster {
    pub fn begin(&mut self) -> u64 {
        self.next_job += 1;
        self.pending.insert(self.next_job, 7);
        self.next_job
    }
    pub fn resume(&mut self, id: u64, revision: u64, surface: u64) -> u64 {
        if self.pending.remove(&id) == Some(revision) && self.surfaces.contains_key(&surface) {
            surface
        } else {
            0
        }
    }
    pub fn retain(&mut self, s: Surface) -> u64 {
        assert!(self.surfaces.len() < 128, "bounded spike surfaces");
        self.next += 1;
        self.surfaces.insert(self.next, Arc::new(s));
        self.next
    }
    pub fn render(&mut self, v: &Value) -> u64 {
        let w = v["w"].as_u64().unwrap() as usize;
        let h = v["h"].as_u64().unwrap() as usize;
        assert!(
            w > 0 && h > 0 && w <= 1920 && h <= 1080 && self.surfaces.len() < 128,
            "bounded spike surfaces"
        );
        let mut bytes = vec![0u8; w * h * 4];
        if let Some(kind) = v["pattern"].as_str() {
            let mut n = 0x13245678u32;
            for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
                if kind == "noise" {
                    n ^= n << 13;
                    n ^= n >> 17;
                    n ^= n << 5;
                    p[..3].copy_from_slice(&n.to_le_bytes()[..3]);
                } else {
                    for k in 0..3 {
                        p[k] = ((i * 4 + k) * 17 + 23) as u8;
                    }
                }
                p[3] = 255;
            }
        }
        unsafe {
            let space = CGColorSpaceCreateDeviceRGB();
            let c = CGBitmapContextCreate(bytes.as_mut_ptr().cast(), w, h, 8, w * 4, space, 0x4001); // RGBA premultiplied-last, big endian
            assert!(!c.is_null());
            CGContextTranslateCTM(c, 0., h as f64);
            CGContextScaleCTM(c, 1., -1.);
            for cmd in v["cmds"].as_array().unwrap() {
                let a = &cmd[1];
                match cmd[0].as_str().unwrap() {
                    "color" => CGContextSetRGBFillColor(
                        c,
                        number(a, 0) / 255.,
                        number(a, 1) / 255.,
                        number(a, 2) / 255.,
                        number(a, 3) / 255.,
                    ),
                    "rect" => CGContextFillRect(
                        c,
                        rect(number(a, 0), number(a, 1), number(a, 2), number(a, 3)),
                    ),
                    "begin" => CGContextBeginPath(c),
                    "move" => CGContextMoveToPoint(c, number(a, 0), number(a, 1)),
                    "line" => CGContextAddLineToPoint(c, number(a, 0), number(a, 1)),
                    "curve" => CGContextAddCurveToPoint(
                        c,
                        number(a, 0),
                        number(a, 1),
                        number(a, 2),
                        number(a, 3),
                        number(a, 4),
                        number(a, 5),
                    ),
                    "close" => CGContextClosePath(c),
                    "fill" => CGContextFillPath(c),
                    "image" => {
                        let s = &self.surfaces[&a[0].as_u64().unwrap()];
                        // Immutable source version; source remains borrowed through CG image draw.
                        let sc = CGBitmapContextCreate(
                            s.bytes.as_ptr() as *mut _,
                            s.w,
                            s.h,
                            8,
                            s.w * 4,
                            space,
                            0x4001,
                        );
                        assert!(!sc.is_null());
                        let img = CGBitmapContextCreateImage(sc);
                        CGContextSaveGState(c);
                        CGContextTranslateCTM(c, number(a, 1), number(a, 2) + s.h as f64);
                        CGContextScaleCTM(c, 1., -1.);
                        CGContextDrawImage(c, rect(0., 0., s.w as f64, s.h as f64), img);
                        CGContextRestoreGState(c);
                        CFRelease(img);
                        CGContextRelease(sc);
                    }
                    "text" => {
                        let name = string("Helvetica");
                        let font = CTFontCreateWithName(name, 16., std::ptr::null());
                        let attrs = CFDictionaryCreate(
                            std::ptr::null(),
                            &kCTFontAttributeName,
                            &font,
                            1,
                            std::ptr::null(),
                            std::ptr::null(),
                        );
                        let text = string(a[0].as_str().unwrap());
                        let attr = CFAttributedStringCreate(std::ptr::null(), text, attrs);
                        let line = CTLineCreateWithAttributedString(attr);
                        CGContextSaveGState(c);
                        CGContextTranslateCTM(c, number(a, 1), number(a, 2));
                        CGContextScaleCTM(c, 1., -1.);
                        CGContextSetTextPosition(c, 0., 0.);
                        CTLineDraw(line, c);
                        CGContextRestoreGState(c);
                        for p in [line, attr, text, attrs, font, name] {
                            CFRelease(p)
                        }
                    }
                    _ => panic!("unsupported recorder command"),
                }
            }
            CGContextFlush(c);
            CGContextRelease(c);
            CFRelease(space);
        }
        self.retain(Surface { w, h, bytes })
    }
    pub fn jpeg(&self, id: u64) -> Vec<u8> {
        let s = &self.surfaces[&id];
        unsafe {
            let nil = std::ptr::null();
            let space = CGColorSpaceCreateDeviceRGB();
            let c = CGBitmapContextCreate(
                s.bytes.as_ptr() as *mut _,
                s.w,
                s.h,
                8,
                s.w * 4,
                space,
                0x4001,
            );
            let image = CGBitmapContextCreateImage(c);
            let data = CFDataCreateMutable(nil, 0);
            let ty = string("public.jpeg");
            let dest = CGImageDestinationCreateWithData(data, ty, 1, nil);
            assert!(!dest.is_null());
            CGImageDestinationAddImage(dest, image, nil);
            assert!(CGImageDestinationFinalize(dest));
            let out =
                std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize)
                    .to_vec();
            for p in [dest, ty, data, image, space] {
                CFRelease(p)
            }
            CGContextRelease(c);
            out
        }
    }
}
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        out.push(T[(n >> 18) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
