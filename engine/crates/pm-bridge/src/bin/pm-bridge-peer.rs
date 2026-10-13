//! Plays the engine in pm-bridge's cross-process rendezvous tests.
//!
//! Usage: `pm-bridge-peer <mode>`; the service name comes from
//! `POWERMOVE_FRAME_BRIDGE`. Prints one result line on stdout.

#[cfg(target_os = "macos")]
fn main() {
    use pm_bridge::rendezvous::{Client, SendRight};
    use pm_bridge::wire::{PixelFormat, SurfaceInfo};
    use std::time::Duration;

    let mode = std::env::args().nth(1).unwrap_or_default();
    let name = std::env::var("POWERMOVE_FRAME_BRIDGE").expect("POWERMOVE_FRAME_BRIDGE");
    let timeout = Duration::from_secs(3);
    let wait_for_stdin_close = || {
        let mut sink = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut sink);
    };
    match mode.as_str() {
        "session" => {
            let client = Client::check_in(&name, timeout).expect("check-in");
            // Two real IOSurfaces in generation 1 of client port 7, then a retire.
            let surfaces: Vec<_> = (0..2)
                .map(|_| iosurface(64, 32, PixelFormat::Rgba16F))
                .collect();
            for (index, surface) in surfaces.iter().enumerate() {
                let info = SurfaceInfo {
                    client_port: 7,
                    generation: 1,
                    pool_index: index as u32,
                    pool_size: 4,
                    width: 64,
                    height: 32,
                    format: PixelFormat::Rgba16F,
                    bytes_per_row: surface.bytes_per_row() as u32,
                };
                let port = SendRight::from_raw(surface.create_mach_port());
                client.send_surface(port, &info).expect("send surface");
            }
            client.retire(7, 1).expect("retire");
            client.goodbye().expect("goodbye");
            println!("ok bridge-pid={}", client.bridge_peer().pid());
            // Our surfaces stay alive until the test has looked them up.
            wait_for_stdin_close();
        }
        "lying-surface" => {
            let client = Client::check_in(&name, timeout).expect("check-in");
            let surface = iosurface(64, 32, PixelFormat::Bgra8);
            let info = SurfaceInfo {
                client_port: 7,
                generation: 1,
                pool_index: 0,
                pool_size: 4,
                width: 128,
                height: 32,
                format: PixelFormat::Bgra8,
                bytes_per_row: 1024,
            };
            let port = SendRight::from_raw(surface.create_mach_port());
            client.send_surface(port, &info).expect("send surface");
            client.goodbye().expect("goodbye");
            println!("ok");
            wait_for_stdin_close();
        }
        "twice" => {
            let _client = Client::check_in(&name, timeout).expect("first check-in");
            match Client::check_in(&name, Duration::from_millis(500)) {
                Ok(_) => println!("second check-in ACCEPTED"),
                Err(e) => println!("second check-in refused: {e}"),
            }
        }
        "checkin-expect-refusal" => match Client::check_in(&name, Duration::from_millis(800)) {
            Ok(_) => println!("UNEXPECTED accepted"),
            Err(e) => println!("refused: {e}"),
        },
        "die-after-checkin" => {
            let _client = Client::check_in(&name, timeout).expect("check-in");
            std::process::exit(0);
        }
        other => panic!("unknown mode {other}"),
    }
}

#[cfg(target_os = "macos")]
fn iosurface(
    width: usize,
    height: usize,
    format: pm_bridge::wire::PixelFormat,
) -> objc2_core_foundation::CFRetained<objc2_io_surface::IOSurfaceRef> {
    use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CFType};
    use objc2_io_surface::{
        IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfaceHeight,
        kIOSurfacePixelFormat, kIOSurfaceWidth,
    };
    let bpe = format.bytes_per_pixel() as usize;
    let fourcc = match format {
        pm_bridge::wire::PixelFormat::Bgra8 => i32::from_be_bytes(*b"BGRA"),
        pm_bridge::wire::PixelFormat::Rgba16F => i32::from_be_bytes(*b"RGhA"),
    };
    // SAFETY: the property key statics are exported by IOSurface.framework.
    let keys: [&CFString; 5] = unsafe {
        [
            kIOSurfaceWidth,
            kIOSurfaceHeight,
            kIOSurfaceBytesPerElement,
            kIOSurfaceBytesPerRow,
            kIOSurfacePixelFormat,
        ]
    };
    // SAFETY: as above.
    let bpr = IOSurfaceRef::align_property(unsafe { kIOSurfaceBytesPerRow }, width * bpe);
    let values = [
        CFNumber::new_i64(width as i64),
        CFNumber::new_i64(height as i64),
        CFNumber::new_i64(bpe as i64),
        CFNumber::new_i64(bpr as i64),
        CFNumber::new_i32(fourcc),
    ];
    let key_refs: Vec<&CFType> = keys.iter().map(|k| k.as_ref()).collect();
    let value_refs: Vec<&CFType> = values.iter().map(|v| v.as_ref()).collect();
    let props = CFDictionary::<CFType, CFType>::from_slices(&key_refs, &value_refs);
    // SAFETY: documented keys with CFNumber values.
    unsafe { IOSurfaceRef::new(props.as_opaque()) }.expect("IOSurfaceCreate")
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("pm-bridge-peer runs on macOS only");
    std::process::exit(2);
}
