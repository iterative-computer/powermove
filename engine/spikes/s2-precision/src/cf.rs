//! Small helpers for building toll-free bridged dictionaries and reading CF values.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

/// Error type for the spike: a message with context.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// Checks an OSStatus-like code.
pub fn check(status: i32, what: &str) -> Result<()> {
    if status == 0 { Ok(()) } else { err(format!("{what} failed: OSStatus {status} ({})", fourcc_or_num(status))) }
}

fn fourcc_or_num(v: i32) -> String {
    let b = v.to_be_bytes();
    if b.iter().all(|c| c.is_ascii_graphic()) { format!("'{}'", String::from_utf8_lossy(&b)) } else { v.to_string() }
}

/// Toll-free bridge a CFString key to NSString.
pub fn ns(s: &CFString) -> &NSString {
    unsafe { &*(s as *const CFString as *const NSString) }
}

/// Toll-free bridge an NSDictionary to CFDictionary.
pub fn cf_dict(d: &NSDictionary<NSString, AnyObject>) -> &CFDictionary {
    unsafe { &*(d as *const NSDictionary<NSString, AnyObject> as *const CFDictionary) }
}

/// Toll-free bridge a CF object to AnyObject.
pub fn cf_any(t: &CFType) -> &AnyObject {
    unsafe { &*(t as *const CFType as *const AnyObject) }
}

pub fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

pub fn fourcc_str(v: u32) -> String {
    String::from_utf8_lossy(&v.to_be_bytes()).into_owned()
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
        self.vals.push(unsafe { Retained::retain(val as *const AnyObject as *mut AnyObject).unwrap() });
        self
    }

    pub fn cfkey(self, key: &CFString, val: &AnyObject) -> Self {
        self.obj(ns(key), val)
    }

    pub fn i64(self, key: &NSString, v: i64) -> Self {
        let n = NSNumber::new_i64(v);
        self.obj(key, &n)
    }

    pub fn u32(self, key: &NSString, v: u32) -> Self {
        let n = NSNumber::new_u32(v);
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

    pub fn str(self, key: &NSString, v: &NSString) -> Self {
        self.obj(key, v)
    }

    pub fn dict(self, key: &NSString, d: &NSDictionary<NSString, AnyObject>) -> Self {
        self.obj(key, d)
    }

    pub fn build(self) -> Retained<NSDictionary<NSString, AnyObject>> {
        let k: Vec<&NSString> = self.keys.iter().map(|k| &**k).collect();
        NSDictionary::from_retained_objects(&k, &self.vals)
    }
}

pub fn ns_array_u32(v: &[u32]) -> Retained<NSArray<NSNumber>> {
    let nums: Vec<Retained<NSNumber>> = v.iter().map(|x| NSNumber::new_u32(*x)).collect();
    NSArray::from_retained_slice(&nums)
}

/// Describe an arbitrary CF/NS object with `-description` for reports.
pub fn describe(obj: &AnyObject) -> String {
    let d: Retained<NSString> = unsafe { objc2::msg_send![obj, description] };
    d.to_string()
}

pub fn describe_cf(t: &CFType) -> String {
    describe(cf_any(t))
}

/// Read a string-ish value from a CF dictionary given a CFString key.
pub fn dict_get_string(d: &NSDictionary<NSString, AnyObject>, key: &CFString) -> Option<String> {
    let v = d.objectForKey(ns(key))?;
    Some(describe(&v))
}

pub fn retained_cf<T: objc2_core_foundation::Type>(p: *mut T) -> Option<CFRetained<T>> {
    std::ptr::NonNull::new(p).map(|p| unsafe { CFRetained::from_raw(p) })
}
