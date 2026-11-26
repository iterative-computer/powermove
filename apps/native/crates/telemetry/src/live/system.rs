//! The Mac's facts for the super properties and the performance summary,
//! read on the telemetry thread.

fn sysctl_string(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut len = 0usize;
    // SAFETY: a size query, then a read into a buffer of that size.
    unsafe {
        if libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buf.truncate(len);
        let s = String::from_utf8_lossy(&buf)
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        (!s.is_empty()).then_some(s)
    }
}

fn sysctl_u64(name: &str) -> Option<u64> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value = 0u64;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: reads one integer into `value`.
    let ok = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) == 0
    };
    ok.then_some(value)
}

pub fn chip() -> String {
    sysctl_string("machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into())
}

pub fn ram_bytes() -> u64 {
    sysctl_u64("hw.memsize").unwrap_or(0)
}

/// e.g. "26.1".
pub fn macos_version() -> String {
    #[cfg(target_os = "macos")]
    {
        let v = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
        format!("{}.{}", v.majorVersion, v.minorVersion)
    }
    #[cfg(not(target_os = "macos"))]
    "unknown".into()
}

/// The language and region codes only (e.g. "en", "US").
pub fn locale() -> (String, String) {
    #[cfg(target_os = "macos")]
    {
        let locale = objc2_foundation::NSLocale::currentLocale();
        let language = locale.languageCode().to_string();
        let region = locale
            .regionCode()
            .map(|r| r.to_string())
            .unwrap_or_default();
        (language, region)
    }
    #[cfg(not(target_os = "macos"))]
    (String::new(), String::new())
}

/// This process's physical footprint (what Activity Monitor shows), bytes.
pub fn footprint() -> u64 {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: fills `info` for this process.
    let ok = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            (&mut info as *mut libc::rusage_info_v4).cast(),
        ) == 0
    };
    if ok { info.ri_phys_footprint } else { 0 }
}

/// 0 nominal … 3 critical.
pub fn thermal_state() -> u8 {
    #[cfg(target_os = "macos")]
    return objc2_foundation::NSProcessInfo::processInfo()
        .thermalState()
        .0
        .clamp(0, 3) as u8;
    #[cfg(not(target_os = "macos"))]
    0
}

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> *const std::ffi::c_void;
    fn IOPSGetProvidingPowerSourceType(
        snapshot: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
}
#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFStringGetCString(
        s: *const std::ffi::c_void,
        buf: *mut i8,
        len: isize,
        encoding: u32,
    ) -> u8;
}

/// Running on battery power now.
pub fn on_battery() -> bool {
    #[cfg(target_os = "macos")]
    // SAFETY: the snapshot is released after its (non-owned) type string is
    // copied out.
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return false;
        }
        let kind = IOPSGetProvidingPowerSourceType(snapshot);
        let mut buf = [0i8; 64];
        let ok =
            !kind.is_null() && CFStringGetCString(kind, buf.as_mut_ptr(), 64, 0x0800_0100) != 0;
        CFRelease(snapshot);
        ok && std::ffi::CStr::from_ptr(buf.as_ptr()).to_bytes() == b"Battery Power"
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// When this process started (seconds since 1970).
pub fn process_start() -> Option<f64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: fills `info` for this process.
    let n = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (n == size).then(|| info.pbi_start_tvsec as f64 + info.pbi_start_tvusec as f64 / 1e6)
}

/// When the Mac last started (seconds since 1970).
pub fn boot_time() -> Option<f64> {
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::timeval>();
    let name = c"kern.boottime";
    // SAFETY: reads one timeval.
    let ok = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut tv as *mut libc::timeval).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) == 0
    };
    ok.then(|| tv.tv_sec as f64 + tv.tv_usec as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_mac_answers() {
        assert!(super::ram_bytes() > 1 << 30);
        assert_ne!(super::chip(), "unknown");
        assert!(super::macos_version().contains('.'));
        assert!(super::footprint() > 0);
        let start = super::process_start().unwrap();
        assert!(start > super::boot_time().unwrap());
        let _ = (super::on_battery(), super::thermal_state(), super::locale());
    }
}
