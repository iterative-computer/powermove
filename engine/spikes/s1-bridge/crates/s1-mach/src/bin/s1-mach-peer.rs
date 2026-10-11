//! Test peer for the rendezvous integration tests. Plays the engine side.
//! Usage: s1-mach-peer <mode>; the service name comes from PM_S1_BRIDGE_NAME.
use s1_mach::rendezvous::{Client, ReceiveRight, SendRight};
use s1_mach::{ffi, wire};
use std::time::Duration;

fn fake_surface_right() -> SendRight {
    // A stand-in for IOSurfaceCreateMachPort: any send right exercises the transfer.
    let recv = ReceiveRight::allocate().unwrap();
    let name = recv.name();
    // SAFETY: inserting a send right on a receive right we own.
    let kr = unsafe { ffi::mach_port_insert_right(ffi::task_self(), name, name, ffi::MACH_MSG_TYPE_MAKE_SEND) };
    assert_eq!(kr, 0);
    std::mem::forget(recv); // keep the port alive for the life of the process
    SendRight::from_raw(name)
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let name = std::env::var("PM_S1_BRIDGE_NAME").expect("PM_S1_BRIDGE_NAME");
    let t = Duration::from_secs(3);
    match mode.as_str() {
        "session" => {
            let c = Client::check_in(&name, t).expect("check-in");
            let info = wire::SurfaceInfo {
                pool_index: 2,
                generation: 1,
                width: 3840,
                height: 2160,
                format: wire::PixelFormat::Rgba16F,
                bytes_per_row: 3840 * 8,
            };
            c.send_surface(fake_surface_right(), &info).unwrap();
            for seq in 0..100u64 {
                let d = wire::FrameDescriptor { pool_index: (seq % 8) as u32, seq, pts_us: seq as i64 * 16_667, ..Default::default() };
                c.send_frame(&d, Duration::from_millis(100)).unwrap();
            }
            c.goodbye().unwrap();
            println!("ok bridge-pid={}", c.bridge_peer().pid());
            // The fake surface right dies with this process; stay alive until the test
            // has dequeued everything (it closes our stdin).
            let mut sink = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut sink);
        }
        "twice" => {
            let _c = Client::check_in(&name, t).expect("first check-in");
            // The name must be gone after the first accepted check-in.
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
            let _c = Client::check_in(&name, t).expect("check-in");
            std::process::exit(0);
        }
        other => panic!("unknown mode {other}"),
    }
}
