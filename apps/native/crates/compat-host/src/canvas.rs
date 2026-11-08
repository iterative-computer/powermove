//! Native bitmap resources travel separately from the DOM mutation protocol.
//! The worker freezes/encodes pixels; AppKit only admits cached bytes and queues IDs.
use crate::Generation;
#[cfg(any(target_os = "macos", test))]
use crate::{Error, MAX_PENDING_BYTES, Result};
#[cfg(any(target_os = "macos", test))]
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct CanvasFrame {
    pub generation: Generation,
    pub node: u64,
    pub dom_sequence: u64,
    pub surface: String,
    pub version: u64,
    pub width: u32,
    pub height: u32,
    /// Encoded bytes: a canvas's PNG, or the file behind an `<img>`.
    pub png: Arc<[u8]>,
    /// `image/png` for a canvas; the image's own type for an `<img>`.
    pub mime: Arc<str>,
}

/// Per-host generation; resource IDs never repeat, including after consumption.
/// Failed queue admission rolls back the insertion, never evicts an older frame.
#[derive(Default)]
#[cfg(any(target_os = "macos", test))]
pub(crate) struct CanvasResources {
    next: u64,
    bytes: usize,
    entries: BTreeMap<u64, (Arc<[u8]>, Arc<str>)>,
}
#[cfg(any(target_os = "macos", test))]
impl CanvasResources {
    pub fn insert(&mut self, frame: &CanvasFrame) -> Result<u64> {
        if frame.node == 0
            || frame.node > pm_remote_dom::protocol::MAX_ID
            || frame.dom_sequence == 0
            || frame.surface.is_empty()
            || frame.png.len() > MAX_PENDING_BYTES - self.bytes
            || self.entries.len() >= 256
        {
            return Err(Error::Backpressure);
        }
        // Zero-size canvases clear the destination without allocating/encoding a bitmap.
        if (frame.width == 0 || frame.height == 0) != frame.png.is_empty() {
            return Err(Error::InvalidRequest);
        }
        self.next = self.next.checked_add(1).ok_or(Error::Backpressure)?;
        self.bytes += frame.png.len();
        self.entries
            .insert(self.next, (frame.png.clone(), frame.mime.clone()));
        Ok(self.next)
    }
    pub fn get(&self, id: u64) -> Result<(Arc<[u8]>, Arc<str>)> {
        self.entries.get(&id).cloned().ok_or(Error::InvalidRequest)
    }
    pub fn remove(&mut self, id: u64) -> Result<()> {
        let (bytes, _) = self.entries.remove(&id).ok_or(Error::InvalidRequest)?;
        self.bytes -= bytes.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(bytes: usize) -> CanvasFrame {
        CanvasFrame {
            generation: 1,
            node: 6,
            dom_sequence: 1,
            surface: "synthetic".into(),
            version: 1,
            width: 1,
            height: 1,
            png: vec![1; bytes].into(),
            mime: "image/png".into(),
        }
    }
    #[test]
    fn native_resources_remain_immutable_until_ack_and_pressure_never_evicts() {
        let mut resources = CanvasResources::default();
        let first = resources.insert(&frame(MAX_PENDING_BYTES)).unwrap();
        let (held, _) = resources.get(first).unwrap();
        assert_eq!(resources.insert(&frame(1)), Err(Error::Backpressure));
        assert_eq!(resources.get(first).unwrap().0.len(), MAX_PENDING_BYTES);
        resources.remove(first).unwrap();
        assert!(resources.get(first).is_err());
        assert_eq!(held.len(), MAX_PENDING_BYTES);
        assert!(resources.insert(&frame(1)).unwrap() > first);
    }
    #[test]
    fn empty_bitmaps_are_explicit_and_invalid_bindings_are_refused() {
        let mut resources = CanvasResources::default();
        assert_eq!(resources.insert(&frame(0)), Err(Error::InvalidRequest));
        let mut empty = frame(0);
        empty.width = 0;
        assert!(resources.insert(&empty).is_ok());
        empty.node = pm_remote_dom::protocol::MAX_ID + 1;
        assert!(resources.insert(&empty).is_err());
    }
}
