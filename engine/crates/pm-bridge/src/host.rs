//! Main's side of the bridge, independent of Node so it runs under tests:
//! the receive loop for one engine launch and the table of surfaces main
//! holds for Chromium.
//!
//! The loop accepts the engine's one check-in, then turns each surface
//! message into an `IOSurfaceRef` main keeps a reference to until JavaScript
//! reports `allReferencesReleased` for its import ([`SurfaceTable::release`]).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;

use crate::audit::ExpectedPeer;
use crate::rendezvous::{Event, Rendezvous};
use crate::wire::{PixelFormat, SurfaceInfo};

/// Most surfaces main holds for one engine launch: four live pools of the
/// largest size. A retired pool stays held until Chromium lets go of it.
pub const MAX_HELD: usize = 4 * crate::wire::MAX_POOL as usize;

/// An IOSurface main holds while Chromium may import it.
struct HeldSurface {
    surface: CFRetained<IOSurfaceRef>,
    info: SurfaceInfo,
}

// SAFETY: IOSurface references are atomically reference counted and the
// IOSurface API is thread-safe; main never touches the pixels.
unsafe impl Send for HeldSurface {}

/// What main holds, keyed by (generation, pool index).
#[derive(Default)]
pub struct SurfaceTable {
    held: BTreeMap<(u32, u32), HeldSurface>,
}

impl SurfaceTable {
    /// Drops main's reference once every import of the surface is released.
    pub fn release(&mut self, generation: u32, pool_index: u32) -> bool {
        self.held.remove(&(generation, pool_index)).is_some()
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Bytes of the surfaces held, as IOSurface allocated them.
    pub fn bytes(&self) -> u64 {
        self.held
            .values()
            .map(|s| s.surface.alloc_size() as u64)
            .sum()
    }

    pub fn clear(&mut self) {
        self.held.clear();
    }
}

/// One thing the receive loop reports, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Accepted {
        pid: i32,
        pidversion: i32,
    },
    Rejected {
        pid: i32,
        reason: String,
    },
    /// A pool surface main now holds. `io_surface` is the `IOSurfaceRef`
    /// pointer Electron's `SharedTextureHandle.ioSurface` takes; it stays
    /// valid until [`SurfaceTable::release`] for the same key.
    Surface {
        info: SurfaceInfo,
        io_surface: usize,
        alloc_bytes: u64,
    },
    Retire {
        client_port: u32,
        generation: u32,
    },
    Goodbye,
    /// The engine exited or crashed (its last send right went away).
    Gone,
    /// The receive loop ended for another reason (accept timed out, a Mach
    /// failure). No further events follow.
    Error {
        reason: String,
    },
}

/// Runs one launch's receive loop on the calling thread until the engine is
/// gone, `stop` is set, or a fatal error. Events go to `emit` in order.
pub fn run(
    rendezvous: Rendezvous,
    expected: &ExpectedPeer,
    accept_timeout: Duration,
    table: &Arc<Mutex<SurfaceTable>>,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(HostEvent),
) {
    let (mut connection, rejections) = match rendezvous.accept(expected, accept_timeout) {
        Ok(v) => v,
        Err((error, rejections)) => {
            for r in rejections {
                emit(HostEvent::Rejected {
                    pid: r.pid,
                    reason: r.reason,
                });
            }
            emit(HostEvent::Error {
                reason: format!("check-in: {error}"),
            });
            return;
        }
    };
    for r in rejections {
        emit(HostEvent::Rejected {
            pid: r.pid,
            reason: r.reason,
        });
    }
    let peer = connection.peer();
    emit(HostEvent::Accepted {
        pid: peer.pid(),
        pidversion: peer.pidversion(),
    });
    while !stop.load(Ordering::SeqCst) {
        let event = match connection.recv(Duration::from_millis(100)) {
            Ok(None) => continue,
            Ok(Some(event)) => event,
            Err(crate::rendezvous::BridgeError::Rejected(reason)) => {
                emit(HostEvent::Rejected { pid: -1, reason });
                continue;
            }
            Err(error) => {
                emit(HostEvent::Error {
                    reason: error.to_string(),
                });
                return;
            }
        };
        match event {
            Event::Surface { surface, info } => {
                // Takes main's own +1 reference; the port right is released
                // when `surface` drops at the end of this arm.
                let Some(io) = IOSurfaceRef::lookup_from_mach_port(surface.name()) else {
                    emit(HostEvent::Rejected {
                        pid: peer.pid(),
                        reason: "IOSurfaceLookupFromMachPort failed".into(),
                    });
                    continue;
                };
                if let Err(reason) = matches_surface(&io, &info) {
                    emit(HostEvent::Rejected {
                        pid: peer.pid(),
                        reason,
                    });
                    continue;
                }
                let pointer = CFRetained::as_ptr(&io).as_ptr() as usize;
                let alloc_bytes = io.alloc_size() as u64;
                {
                    let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
                    let key = (info.generation, info.pool_index);
                    if !table.held.contains_key(&key) && table.held.len() >= MAX_HELD {
                        drop(table);
                        emit(HostEvent::Rejected {
                            pid: peer.pid(),
                            reason: format!("more than {MAX_HELD} surfaces held"),
                        });
                        continue;
                    }
                    // A repeated key replaces the older surface; its import in
                    // JavaScript is replaced the same way.
                    table.held.insert(key, HeldSurface { surface: io, info });
                }
                emit(HostEvent::Surface {
                    info,
                    io_surface: pointer,
                    alloc_bytes,
                });
            }
            Event::Retire {
                client_port,
                generation,
            } => emit(HostEvent::Retire {
                client_port,
                generation,
            }),
            Event::Goodbye => emit(HostEvent::Goodbye),
            Event::PeerGone => {
                emit(HostEvent::Gone);
                return;
            }
        }
    }
}

/// The surface must be what the message says: Chromium imports it with the
/// message's size and format.
fn matches_surface(io: &IOSurfaceRef, info: &SurfaceInfo) -> Result<(), String> {
    let (w, h, bpr) = (io.width(), io.height(), io.bytes_per_row());
    if w != info.width as usize || h != info.height as usize {
        return Err(format!(
            "surface is {w}x{h}, the message says {}x{}",
            info.width, info.height
        ));
    }
    if bpr != info.bytes_per_row as usize {
        return Err(format!(
            "surface rows are {bpr} bytes, the message says {}",
            info.bytes_per_row
        ));
    }
    let fourcc = io.pixel_format();
    let want = match info.format {
        PixelFormat::Bgra8 => u32::from_be_bytes(*b"BGRA"),
        PixelFormat::Rgba16F => u32::from_be_bytes(*b"RGhA"),
    };
    if fourcc != want {
        return Err(format!(
            "surface pixel format {fourcc:#x} is not {:?}",
            info.format
        ));
    }
    Ok(())
}

/// Main's own record of a held surface's size, for diagnostics.
pub fn held_infos(table: &SurfaceTable) -> Vec<SurfaceInfo> {
    table.held.values().map(|s| s.info).collect()
}
