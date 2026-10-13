//! Hardware decode session limits. Main arbitrates hardware sessions across
//! engine processes (section 13 "Decode service", section 8 "Device"); an
//! engine asks its arbiter before creating a hardware session and holds the
//! permit for the session's lifetime. [`LocalArbiter`] enforces per-codec
//! limits inside one process (tests, the CLI host); the engine host plugs in
//! one backed by main.

use crate::apple::cf::fcc;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Hardware decoder families with separate session limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HwCodec {
    H264,
    Hevc,
    ProRes,
    Av1,
    Other,
}

impl HwCodec {
    pub fn from_fourcc(f: u32) -> HwCodec {
        match f {
            x if x == fcc(b"avc1") => HwCodec::H264,
            x if x == fcc(b"hvc1") => HwCodec::Hevc,
            x if x == fcc(b"av01") => HwCodec::Av1,
            x if crate::vt::is_prores_fourcc(&x.to_be_bytes()) => HwCodec::ProRes,
            _ => HwCodec::Other,
        }
    }
}

/// Grants hardware decode sessions.
pub trait HardwareArbiter: Send + Sync {
    /// A permit for one hardware session, or None when the limit is reached.
    fn try_acquire(&self, codec: HwCodec) -> Option<HwPermit>;
    /// Sessions currently granted, per codec.
    fn in_use(&self) -> HashMap<HwCodec, usize>;
}

/// Held for the lifetime of a hardware session; dropping it returns the slot.
pub struct HwPermit {
    release: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl HwPermit {
    pub fn new(release: impl FnOnce() + Send + Sync + 'static) -> HwPermit {
        HwPermit { release: Some(Box::new(release)) }
    }
}

impl Drop for HwPermit {
    fn drop(&mut self) {
        if let Some(r) = self.release.take() {
            r();
        }
    }
}

impl std::fmt::Debug for HwPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HwPermit")
    }
}

/// Per-codec session limits within one process.
#[derive(Clone)]
pub struct LocalArbiter {
    inner: Arc<Mutex<LocalState>>,
}

struct LocalState {
    limits: HashMap<HwCodec, usize>,
    default_limit: usize,
    used: HashMap<HwCodec, usize>,
}

impl LocalArbiter {
    /// `default_limit` sessions per codec unless `limits` says otherwise.
    pub fn new(default_limit: usize, limits: &[(HwCodec, usize)]) -> LocalArbiter {
        LocalArbiter {
            inner: Arc::new(Mutex::new(LocalState {
                limits: limits.iter().copied().collect(),
                default_limit,
                used: HashMap::new(),
            })),
        }
    }

    /// No limit (every request is granted).
    pub fn unlimited() -> LocalArbiter {
        Self::new(usize::MAX, &[])
    }
}

impl HardwareArbiter for LocalArbiter {
    fn try_acquire(&self, codec: HwCodec) -> Option<HwPermit> {
        let mut g = self.inner.lock().unwrap();
        let limit = g.limits.get(&codec).copied().unwrap_or(g.default_limit);
        let used = g.used.entry(codec).or_insert(0);
        if *used >= limit {
            return None;
        }
        *used += 1;
        let inner = self.inner.clone();
        Some(HwPermit::new(move || {
            let mut g = inner.lock().unwrap();
            if let Some(u) = g.used.get_mut(&codec) {
                *u -= 1;
            }
        }))
    }

    fn in_use(&self) -> HashMap<HwCodec, usize> {
        self.inner.lock().unwrap().used.iter().filter(|(_, n)| **n > 0).map(|(k, v)| (*k, *v)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_are_limited_and_returned() {
        let a = LocalArbiter::new(2, &[(HwCodec::ProRes, 1)]);
        let p1 = a.try_acquire(HwCodec::ProRes).unwrap();
        assert!(a.try_acquire(HwCodec::ProRes).is_none());
        let h1 = a.try_acquire(HwCodec::Hevc).unwrap();
        let _h2 = a.try_acquire(HwCodec::Hevc).unwrap();
        assert!(a.try_acquire(HwCodec::Hevc).is_none());
        drop(p1);
        drop(h1);
        assert!(a.try_acquire(HwCodec::ProRes).is_some());
        assert_eq!(a.in_use().get(&HwCodec::Hevc), Some(&1));
        assert_eq!(HwCodec::from_fourcc(fcc(b"ap4x")), HwCodec::ProRes);
    }
}
