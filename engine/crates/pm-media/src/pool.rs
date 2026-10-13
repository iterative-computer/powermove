//! Frame memory for software decoders.
//!
//! Each slot is page-aligned host memory that the GPU importer (`pm-gpu`)
//! wraps once in a shared-storage `MTLBuffer` (`newBufferWithBytesNoCopy`);
//! the wrapper is cached on the slot. Decoders with direct rendering write
//! their output planes straight into a slot (FFmpeg through `get_buffer2`,
//! dav1d through its picture allocator), and each plane becomes a linear
//! texture over that buffer: on Apple silicon's unified memory the GPU samples
//! the bytes the decoder wrote, with no upload and no copy.
//!
//! A slot returns to the pool when its last reference drops (a decoder's
//! buffer reference, a frame in a cache, a texture still in use on the GPU).

use crate::error::{MediaError, Result};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Apple silicon page size: `newBufferWithBytesNoCopy` needs page alignment.
pub const PAGE: usize = 16384;
/// Row and plane alignment: Metal's linear-texture alignment and every
/// FFmpeg `linesize_align` fit in it.
pub const ROW_ALIGN: usize = 256;

static NEXT_POOL: AtomicU64 = AtomicU64::new(1);

/// GPU objects cached on a slot by an importer, tagged with the device
/// they belong to.
pub type GpuHandle = Arc<dyn Any + Send + Sync>;

struct Slot {
    ptr: *mut u8,
    len: usize,
    /// (device id, importer object), set the first time a device imports it.
    gpu: Mutex<Option<(u64, GpuHandle)>>,
}

// The memory is owned by the slot and only freed in Drop.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

impl Drop for Slot {
    fn drop(&mut self) {
        // The GPU wrapper (an MTLBuffer over this memory) is released first.
        self.gpu.get_mut().unwrap().take();
        unsafe { libc::free(self.ptr as *mut libc::c_void) }
    }
}

struct Lease {
    slot: Option<Arc<Slot>>,
    pool: Weak<PoolInner>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let slot = self.slot.take().unwrap();
        if let Some(p) = self.pool.upgrade() {
            let mut g = p.state.lock().unwrap();
            g.live.remove(&(slot.ptr as usize));
            g.live_bytes -= slot.len;
            if g.free_bytes + slot.len <= g.max_free_bytes {
                g.free_bytes += slot.len;
                g.free.push(slot);
            } else {
                g.bytes -= slot.len;
                g.allocated -= 1;
            }
        }
    }
}

/// A leased slot; it returns to the pool when the last clone drops.
#[derive(Clone)]
pub struct SlotRef(Arc<Lease>);

impl SlotRef {
    fn slot(&self) -> &Slot {
        self.0.slot.as_ref().unwrap()
    }
    pub fn ptr(&self) -> *mut u8 {
        self.slot().ptr
    }
    pub fn len(&self) -> usize {
        self.slot().len
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Identifies the slot's memory (stable while any reference lives).
    pub fn id(&self) -> usize {
        self.slot().ptr as usize
    }
    /// The importer's cached object for `device`, or a new one from `make`.
    /// A slot imported by a second device gets an uncached object.
    pub fn gpu_handle(
        &self,
        device: u64,
        make: impl FnOnce() -> Result<GpuHandle>,
    ) -> Result<GpuHandle> {
        let mut g = self.slot().gpu.lock().unwrap();
        match &*g {
            Some((d, h)) if *d == device => Ok(h.clone()),
            Some(_) => make(),
            None => {
                let h = make()?;
                *g = Some((device, h.clone()));
                Ok(h)
            }
        }
    }
}

impl std::fmt::Debug for SlotRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlotRef({:#x}, {} bytes)", self.id(), self.len())
    }
}

#[derive(Default)]
struct State {
    free: Vec<Arc<Slot>>,
    live: BTreeMap<usize, Weak<Lease>>,
    allocated: usize,
    bytes: usize,
    live_bytes: usize,
    free_bytes: usize,
    max_free_bytes: usize,
}

struct PoolInner {
    id: u64,
    state: Mutex<State>,
    /// Zero slots when first allocated (decoders that require zeroed buffers).
    zero_new: bool,
}

/// Pool usage, for lease accounting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStats {
    pub slots: usize,
    /// Every slot the pool holds, live or free.
    pub bytes: usize,
    pub live_bytes: usize,
    pub free_bytes: usize,
}

/// Page-aligned frame memory shared by the software decoders of a process.
#[derive(Clone)]
pub struct FramePool(Arc<PoolInner>);

impl FramePool {
    /// A pool that keeps at most `max_free_bytes` of returned slots for reuse.
    pub fn new(max_free_bytes: usize) -> FramePool {
        Self::with_zeroing(max_free_bytes, false)
    }

    pub fn with_zeroing(max_free_bytes: usize, zero_new: bool) -> FramePool {
        FramePool(Arc::new(PoolInner {
            id: NEXT_POOL.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(State {
                max_free_bytes,
                ..Default::default()
            }),
            zero_new,
        }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// Leases a slot of at least `len` bytes (rounded up to whole pages).
    pub fn acquire(&self, len: usize) -> Result<SlotRef> {
        let len = len.max(1).div_ceil(PAGE) * PAGE;
        let mut g = self.0.state.lock().unwrap();
        let slot = match g.free.iter().position(|s| s.len == len) {
            Some(i) => {
                g.free_bytes -= len;
                g.free.swap_remove(i)
            }
            None => {
                let mut p: *mut libc::c_void = std::ptr::null_mut();
                if unsafe { libc::posix_memalign(&mut p, PAGE, len) } != 0 || p.is_null() {
                    return Err(MediaError::OutOfMemory(len));
                }
                if self.0.zero_new {
                    unsafe { std::ptr::write_bytes(p as *mut u8, 0, len) };
                }
                g.allocated += 1;
                g.bytes += len;
                Arc::new(Slot {
                    ptr: p as *mut u8,
                    len,
                    gpu: Mutex::new(None),
                })
            }
        };
        g.live_bytes += len;
        let key = slot.ptr as usize;
        let lease = Arc::new(Lease {
            slot: Some(slot),
            pool: Arc::downgrade(&self.0),
        });
        g.live.insert(key, Arc::downgrade(&lease));
        Ok(SlotRef(lease))
    }

    /// The live slot containing `addr`. Decoders hand back plane pointers,
    /// and a plane's buffer reference may start inside a slot.
    pub fn find_containing(&self, addr: *const u8) -> Option<SlotRef> {
        let a = addr as usize;
        let g = self.0.state.lock().unwrap();
        let (base, w) = g.live.range(..=a).next_back()?;
        w.upgrade()
            .filter(|l| a < *base + l.slot.as_ref().unwrap().len)
            .map(SlotRef)
    }

    /// Releases free slots beyond `keep_bytes` (a lease shrink).
    pub fn trim(&self, keep_bytes: usize) {
        let mut g = self.0.state.lock().unwrap();
        g.max_free_bytes = keep_bytes;
        while g.free_bytes > keep_bytes {
            let s = g.free.pop().unwrap();
            g.free_bytes -= s.len;
            g.bytes -= s.len;
            g.allocated -= 1;
        }
    }

    pub fn stats(&self) -> PoolStats {
        let g = self.0.state.lock().unwrap();
        PoolStats {
            slots: g.allocated,
            bytes: g.bytes,
            live_bytes: g.live_bytes,
            free_bytes: g.free_bytes,
        }
    }
}

pub fn ceil_rshift(v: usize, s: u8) -> usize {
    (v + (1 << s) - 1) >> s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_page_aligned_reused_and_trimmed() {
        let pool = FramePool::new(1 << 20);
        let a = pool.acquire(1000).unwrap();
        assert_eq!(a.ptr() as usize % PAGE, 0);
        assert_eq!(a.len(), PAGE);
        let id = a.id();
        let inner = unsafe { a.ptr().add(500) };
        assert_eq!(pool.find_containing(inner).map(|s| s.id()), Some(id));
        assert!(pool.find_containing(unsafe { a.ptr().add(PAGE) }).is_none());
        drop(a);
        assert_eq!(
            pool.stats(),
            PoolStats {
                slots: 1,
                bytes: PAGE,
                live_bytes: 0,
                free_bytes: PAGE
            }
        );
        let b = pool.acquire(PAGE).unwrap();
        assert_eq!(b.id(), id, "a free slot of the same size is reused");
        drop(b);
        pool.trim(0);
        assert_eq!(pool.stats(), PoolStats::default());
    }

    #[test]
    fn gpu_handles_are_cached_per_device() {
        let pool = FramePool::new(0);
        let s = pool.acquire(10).unwrap();
        let h1 = s.gpu_handle(7, || Ok(Arc::new(1u32))).unwrap();
        let h2 = s.gpu_handle(7, || panic!("cached")).unwrap();
        assert!(Arc::ptr_eq(&h1, &h2));
        let h3 = s.gpu_handle(8, || Ok(Arc::new(2u32))).unwrap();
        assert_eq!(h3.downcast_ref::<u32>(), Some(&2));
        // A full free list drops returned slots instead of keeping them.
        drop(s);
        assert_eq!(pool.stats().slots, 0);
    }
}
