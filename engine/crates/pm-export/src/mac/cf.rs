//! Toll-free bridging helpers and OSStatus checks.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use crate::Error;

/// Maps an OSStatus to an encode error with context.
pub fn check(status: i32, what: &str) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Encode(format!(
            "{what} failed: OSStatus {status}{}",
            fourcc_hint(status)
        )))
    }
}

fn fourcc_hint(v: i32) -> String {
    let b = v.to_be_bytes();
    if b.iter().all(|c| c.is_ascii_graphic()) {
        format!(" ('{}')", String::from_utf8_lossy(&b))
    } else {
        String::new()
    }
}

pub fn fourcc_str(v: u32) -> String {
    String::from_utf8_lossy(&v.to_be_bytes()).into_owned()
}

/// Toll-free bridge a CFString to NSString.
pub fn ns(s: &CFString) -> &NSString {
    unsafe { &*(s as *const CFString as *const NSString) }
}

/// Toll-free bridge an NSDictionary to CFDictionary.
pub fn cf_dict(d: &NSDictionary<NSString, AnyObject>) -> &CFDictionary {
    unsafe { &*(d as *const NSDictionary<NSString, AnyObject> as *const CFDictionary) }
}

/// A retained CF object as an Objective-C object.
pub fn any_cf(t: &CFType) -> Retained<AnyObject> {
    unsafe { Retained::retain(t as *const CFType as *mut AnyObject).expect("non-null CF object") }
}

pub fn any_str(s: &CFString) -> Retained<AnyObject> {
    any_cf(s.as_ref())
}

pub fn any_num_f64(v: f64) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(NSNumber::new_f64(v))).into()
}

pub fn any_num_i64(v: i64) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(NSNumber::new_i64(v))).into()
}

pub fn any_bool(v: bool) -> Retained<AnyObject> {
    let b: &CFBoolean = unsafe {
        if v {
            objc2_core_foundation::kCFBooleanTrue
        } else {
            objc2_core_foundation::kCFBooleanFalse
        }
    }
    .expect("CFBoolean constants");
    any_cf(b.as_ref())
}

/// A dictionary builder over NSString keys and arbitrary object values.
#[derive(Default)]
pub struct Dict {
    keys: Vec<Retained<NSString>>,
    vals: Vec<Retained<AnyObject>>,
}

impl Dict {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn obj(mut self, key: &NSString, val: &AnyObject) -> Self {
        self.keys.push(NSString::from_str(&key.to_string()));
        self.vals.push(unsafe {
            Retained::retain(val as *const AnyObject as *mut AnyObject).expect("non-null value")
        });
        self
    }

    pub fn cfkey(self, key: &CFString, val: &AnyObject) -> Self {
        self.obj(ns(key), val)
    }

    pub fn u32(self, key: &NSString, v: u32) -> Self {
        let n = NSNumber::new_u32(v);
        self.obj(key, &n)
    }

    pub fn i64(self, key: &NSString, v: i64) -> Self {
        let n = NSNumber::new_i64(v);
        self.obj(key, &n)
    }

    pub fn f64(self, key: &NSString, v: f64) -> Self {
        let n = NSNumber::new_f64(v);
        self.obj(key, &n)
    }

    pub fn bool(self, key: &NSString, v: bool) -> Self {
        let n = NSNumber::new_bool(v);
        self.obj(key, &n)
    }

    pub fn build(self) -> Retained<NSDictionary<NSString, AnyObject>> {
        let k: Vec<&NSString> = self.keys.iter().map(|k| &**k).collect();
        NSDictionary::from_retained_objects(&k, &self.vals)
    }
}

/// Describe an object with `-description`.
pub fn describe(obj: &AnyObject) -> String {
    let d: Retained<NSString> = unsafe { objc2::msg_send![obj, description] };
    d.to_string()
}

/// Runs `f`, turning an Objective-C exception into an error (AVFoundation raises
/// NSInvalidArgumentException and friends on misuse; uncaught, they abort the process).
pub fn objc_try<R>(what: &str, f: impl FnOnce() -> R) -> Result<R, Error> {
    objc2::exception::catch(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        let detail = e
            .map(|e| describe(&e))
            .unwrap_or_else(|| "unknown exception".into());
        Error::Mux(format!("{what}: {detail}"))
    })
}
