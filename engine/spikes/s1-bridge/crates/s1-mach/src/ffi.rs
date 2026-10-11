//! Minimal hand-written bindings for the Mach, bootstrap, libbsm, libproc, and
//! Security calls the bridge needs. Kept in one file so the surface that runs in
//! Electron main is easy to audit.
#![allow(non_camel_case_types, non_upper_case_globals, dead_code)]

use std::ffi::{c_char, c_int, c_void};

pub type kern_return_t = c_int;
pub type mach_port_t = u32;
pub type mach_port_name_t = u32;
pub type mach_msg_return_t = kern_return_t;
pub type mach_msg_option_t = c_int;
pub type mach_msg_size_t = u32;
pub type mach_msg_timeout_t = u32;
pub type mach_port_right_t = u32;
pub type mach_port_delta_t = i32;
pub type natural_t = u32;
pub type task_flavor_t = natural_t;
pub type mach_msg_type_number_t = natural_t;

pub const KERN_SUCCESS: kern_return_t = 0;
pub const MACH_PORT_NULL: mach_port_t = 0;
pub const MACH_PORT_DEAD: mach_port_t = !0;

pub const MACH_PORT_RIGHT_SEND: mach_port_right_t = 0;
pub const MACH_PORT_RIGHT_RECEIVE: mach_port_right_t = 1;

pub const MACH_MSG_TYPE_MOVE_RECEIVE: u32 = 16;
pub const MACH_MSG_TYPE_MOVE_SEND: u32 = 17;
pub const MACH_MSG_TYPE_MOVE_SEND_ONCE: u32 = 18;
pub const MACH_MSG_TYPE_COPY_SEND: u32 = 19;
pub const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
pub const MACH_MSG_TYPE_MAKE_SEND_ONCE: u32 = 21;
/// Disposition of a send right as it appears in a received descriptor.
pub const MACH_MSG_TYPE_PORT_SEND: u32 = MACH_MSG_TYPE_MOVE_SEND;

pub const MACH_MSGH_BITS_COMPLEX: u32 = 0x8000_0000;
pub const MACH_MSG_PORT_DESCRIPTOR: u8 = 0;

pub const MACH_SEND_MSG: mach_msg_option_t = 0x0000_0001;
pub const MACH_RCV_MSG: mach_msg_option_t = 0x0000_0002;
pub const MACH_RCV_LARGE: mach_msg_option_t = 0x0000_0004;
pub const MACH_SEND_TIMEOUT: mach_msg_option_t = 0x0000_0010;
pub const MACH_RCV_TIMEOUT: mach_msg_option_t = 0x0000_0100;
pub const MACH_RCV_TRAILER_AUDIT: u32 = 3;
pub const MACH_MSG_TRAILER_FORMAT_0: u32 = 0;

pub const MACH_RCV_TIMED_OUT: mach_msg_return_t = 0x1000_4003;
pub const MACH_RCV_TOO_LARGE: mach_msg_return_t = 0x1000_4004;
pub const MACH_RCV_INTERRUPTED: mach_msg_return_t = 0x1000_4005;
pub const MACH_SEND_TIMED_OUT: mach_msg_return_t = 0x1000_0004;
pub const MACH_SEND_INVALID_DEST: mach_msg_return_t = 0x1000_0003;

pub const MACH_PORT_LIMITS_INFO: c_int = 1;
pub const MACH_PORT_LIMITS_INFO_COUNT: mach_msg_type_number_t = 1;

pub const MACH_NOTIFY_NO_SENDERS: i32 = 0o106;

pub const TASK_AUDIT_TOKEN: task_flavor_t = 15;
pub const TASK_AUDIT_TOKEN_COUNT: mach_msg_type_number_t = 8;

pub const fn mach_rcv_trailer_type(x: u32) -> mach_msg_option_t {
    ((x & 0xf) << 28) as mach_msg_option_t
}
pub const fn mach_rcv_trailer_elements(x: u32) -> mach_msg_option_t {
    ((x & 0xf) << 24) as mach_msg_option_t
}
pub const fn mach_msgh_bits(remote: u32, local: u32) -> u32 {
    remote | (local << 8)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct mach_msg_header_t {
    pub msgh_bits: u32,
    pub msgh_size: mach_msg_size_t,
    pub msgh_remote_port: mach_port_t,
    pub msgh_local_port: mach_port_t,
    pub msgh_voucher_port: mach_port_name_t,
    pub msgh_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct mach_msg_body_t {
    pub msgh_descriptor_count: u32,
}

/// `mach_msg_port_descriptor_t` on LP64: name, pad1, pad2:16, disposition:8, type:8.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct mach_msg_port_descriptor_t {
    pub name: mach_port_t,
    pub pad1: u32,
    pub pad2: u16,
    pub disposition: u8,
    pub type_: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct audit_token_t {
    pub val: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct mach_port_limits_t {
    pub mpl_qlimit: u32,
}

/// `proc_bsdinfo` from <sys/proc_info.h>; only the leading fields are read.
#[repr(C)]
pub struct proc_bsdinfo {
    pub pbi_flags: u32,
    pub pbi_status: u32,
    pub pbi_xstatus: u32,
    pub pbi_pid: u32,
    pub pbi_ppid: u32,
    pub pbi_uid: u32,
    pub pbi_gid: u32,
    pub pbi_ruid: u32,
    pub pbi_rgid: u32,
    pub pbi_svuid: u32,
    pub pbi_svgid: u32,
    pub rfu_1: u32,
    pub pbi_comm: [c_char; 16],
    pub pbi_name: [c_char; 32],
    pub pbi_nfiles: u32,
    pub pbi_pgid: u32,
    pub pbi_pjobc: u32,
    pub e_tdev: u32,
    pub e_tpgid: u32,
    pub pbi_nice: i32,
    pub pbi_start_tvsec: u64,
    pub pbi_start_tvusec: u64,
}
pub const PROC_PIDTBSDINFO: c_int = 3;

unsafe extern "C" {
    pub static mach_task_self_: mach_port_t;
    pub static bootstrap_port: mach_port_t;

    pub fn mach_msg(
        msg: *mut mach_msg_header_t,
        option: mach_msg_option_t,
        send_size: mach_msg_size_t,
        rcv_size: mach_msg_size_t,
        rcv_name: mach_port_name_t,
        timeout: mach_msg_timeout_t,
        notify: mach_port_name_t,
    ) -> mach_msg_return_t;
    pub fn mach_msg_destroy(msg: *mut mach_msg_header_t);
    pub fn mach_port_allocate(
        task: mach_port_t,
        right: mach_port_right_t,
        name: *mut mach_port_name_t,
    ) -> kern_return_t;
    pub fn mach_port_insert_right(
        task: mach_port_t,
        name: mach_port_name_t,
        poly: mach_port_t,
        poly_poly: u32,
    ) -> kern_return_t;
    pub fn mach_port_mod_refs(
        task: mach_port_t,
        name: mach_port_name_t,
        right: mach_port_right_t,
        delta: mach_port_delta_t,
    ) -> kern_return_t;
    pub fn mach_port_deallocate(task: mach_port_t, name: mach_port_name_t) -> kern_return_t;
    pub fn mach_port_set_attributes(
        task: mach_port_t,
        name: mach_port_name_t,
        flavor: c_int,
        info: *const c_int,
        count: mach_msg_type_number_t,
    ) -> kern_return_t;
    pub fn mach_port_request_notification(
        task: mach_port_t,
        name: mach_port_name_t,
        msgid: i32,
        sync: u32,
        notify: mach_port_t,
        notify_poly: u32,
        previous: *mut mach_port_t,
    ) -> kern_return_t;
    pub fn task_name_for_pid(target: mach_port_t, pid: c_int, tn: *mut mach_port_t) -> kern_return_t;
    pub fn task_info(
        target: mach_port_t,
        flavor: task_flavor_t,
        info: *mut c_int,
        count: *mut mach_msg_type_number_t,
    ) -> kern_return_t;
    pub fn mach_error_string(kr: kern_return_t) -> *const c_char;

    pub fn bootstrap_check_in(bp: mach_port_t, name: *const c_char, sp: *mut mach_port_t) -> kern_return_t;
    pub fn bootstrap_look_up(bp: mach_port_t, name: *const c_char, sp: *mut mach_port_t) -> kern_return_t;
    pub fn bootstrap_strerror(r: kern_return_t) -> *const c_char;

    pub fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int) -> c_int;
}

#[link(name = "bsm")]
unsafe extern "C" {
    pub fn audit_token_to_pid(token: audit_token_t) -> libc::pid_t;
    pub fn audit_token_to_pidversion(token: audit_token_t) -> c_int;
    pub fn audit_token_to_euid(token: audit_token_t) -> libc::uid_t;
}

// --- CoreFoundation and Security -------------------------------------------------

pub type CFTypeRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFDataRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFIndex = isize;
pub type OSStatus = i32;
pub type SecCodeRef = *const c_void;
pub type SecRequirementRef = *const c_void;
pub type SecCSFlags = u32;

pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;
pub const kSecCSDefaultFlags: SecCSFlags = 0;
pub const kSecCSSigningInformation: SecCSFlags = 1 << 1;
pub const errSecSuccess: OSStatus = 0;

#[repr(C)]
pub struct CFDictionaryKeyCallBacks {
    _private: [u8; 0],
}
#[repr(C)]
pub struct CFDictionaryValueCallBacks {
    _private: [u8; 0],
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    pub static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFDataCreate(alloc: CFAllocatorRef, bytes: *const u8, len: CFIndex) -> CFDataRef;
    pub fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        len: CFIndex,
        encoding: u32,
        external: u8,
    ) -> CFStringRef;
    pub fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    pub fn CFGetTypeID(cf: CFTypeRef) -> usize;
    pub fn CFStringGetTypeID() -> usize;
    pub fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: CFIndex,
        key_cb: *const CFDictionaryKeyCallBacks,
        value_cb: *const CFDictionaryValueCallBacks,
    ) -> CFDictionaryRef;
    pub fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    pub static kSecGuestAttributeAudit: CFStringRef;
    pub static kSecCodeInfoTeamIdentifier: CFStringRef;
    pub fn SecCodeCopySelf(flags: SecCSFlags, code: *mut SecCodeRef) -> OSStatus;
    pub fn SecCodeCopyGuestWithAttributes(
        host: SecCodeRef,
        attributes: CFDictionaryRef,
        flags: SecCSFlags,
        guest: *mut SecCodeRef,
    ) -> OSStatus;
    pub fn SecCodeCheckValidity(code: SecCodeRef, flags: SecCSFlags, req: SecRequirementRef) -> OSStatus;
    pub fn SecCodeCopySigningInformation(code: SecCodeRef, flags: SecCSFlags, info: *mut CFDictionaryRef) -> OSStatus;
    pub fn SecRequirementCreateWithString(
        text: CFStringRef,
        flags: SecCSFlags,
        req: *mut SecRequirementRef,
    ) -> OSStatus;
}

/// The calling task's own port.
pub fn task_self() -> mach_port_t {
    // SAFETY: libSystem initialises this before any user code runs.
    unsafe { mach_task_self_ }
}

pub fn kr_string(kr: kern_return_t) -> String {
    // SAFETY: mach_error_string returns a static C string for any value.
    unsafe {
        let p = mach_error_string(kr);
        if p.is_null() {
            return format!("kern_return {kr:#x}");
        }
        format!("{} ({kr:#x})", std::ffi::CStr::from_ptr(p).to_string_lossy())
    }
}
