//! Level 1 SVG rasters on the render thread (decision S1; RFC 16, "SVG
//! rasters"; docs/engine/vector.md, "Level 1 SVG images").
//!
//! The renderer that owns a document draws each level 1 SVG image's
//! bucket with today's code (Chromium's `<img>` into a canvas) and posts
//! it in row bands (`CompatMessage::SvgPixels`). This module decides what
//! to ask for and assembles what arrives; the pixels then go into the
//! render thread's renderer, which every session on the thread shares
//! (rasters are keyed by content and bucket, so one upload serves every
//! window and layer that draws them).
//!
//! - A viewer frame that drew without a raster names it
//!   (`Composed::svg_misses`); [`SvgState::wanted`] turns those into
//!   requests for the view's renderer, once per key while it is pending.
//! - A snapshot the renderer answered names the keys it posted
//!   (`SnapshotRequest::uploaded`, [`SvgRasterKey::upload_key`]):
//!   [`SvgState::expect`] marks them pending so the job waits for them.
//! - A viewer frame waits for the exact bucket (no stand-in), at most
//!   [`VIEW_WAIT_US`] from when it was asked. A failure
//!   (`CompatMessage::SvgFailed`), a refused band, or no answer within that
//!   wait ends the wait at once: the key is marked failed in the renderer
//!   (`Renderer::fail_svg_raster`) and the frame is drawn without that
//!   image, as today's compositor draws the frame without a layer whose
//!   `rasterSvgAsset` throws. The failure is logged once per key, and the
//!   key is asked again after [`RETRY_US`] while a view still draws
//!   without it ([`SvgState::retry_at`]); its raster arriving draws it.
//! - Pending asks the renderer never answered are asked again after
//!   [`ASK_AGAIN_US`]; a closed session's asks are dropped.

use std::collections::{BTreeMap, BTreeSet};

use pm_protocol::svg_raster::{SvgRasterKey, SvgRasterPixels, SvgRasterRequest};

use super::render::Key;

/// A pending ask is asked again after this long (host µs).
pub const ASK_AGAIN_US: i64 = 10_000_000;
/// A key the renderer could not draw is asked again after this long.
pub const RETRY_US: i64 = 5_000_000;
/// Longest a viewer frame waits for a bucket it asked for before it is
/// drawn without it (the ask stays open: its raster draws when it lands).
pub const VIEW_WAIT_US: i64 = RETRY_US;
/// Most rasters assembling at once, and their bytes.
pub const MAX_PENDING: usize = 256;
pub const MAX_PENDING_BYTES: u64 = 512 * 1024 * 1024;

struct Pending {
    /// The session that was asked (or that announced it).
    session: Key,
    asked_us: i64,
    /// Assembled rows (allocated on the first band).
    data: Vec<u8>,
    rows: Vec<bool>,
    missing: u32,
}

/// What a band did.
#[derive(Debug, PartialEq)]
pub enum Band {
    /// Stored; rows still missing.
    Partial,
    /// The raster is complete.
    Complete(SvgRasterKey, Vec<u8>),
    /// Refused (too many rasters assembling): this raster will not
    /// complete from the bands sent, so whoever waits for it stops.
    Refused(SvgRasterKey),
}

/// The render thread's SVG raster asks.
#[derive(Default)]
pub struct SvgState {
    pending: BTreeMap<SvgRasterKey, Pending>,
    bytes: u64,
    failed: BTreeMap<SvgRasterKey, i64>,
    /// Keys whose failure was logged.
    logged: BTreeSet<SvgRasterKey>,
    /// Rasters received complete (stats).
    pub received: u64,
    pub failures: u64,
}

impl SvgState {
    /// The requests to send for `needs` a frame of `session` missed:
    /// those not pending (or pending past [`ASK_AGAIN_US`]) and not failed
    /// within [`RETRY_US`]. `has` says what the renderer already stores
    /// (a raster that arrived after the frame was drawn).
    pub fn wanted(
        &mut self,
        session: Key,
        needs: &[pm_render::vector::svg_rasters::SvgRasterNeed],
        now: i64,
        has: impl Fn(&SvgRasterKey) -> bool,
    ) -> Vec<SvgRasterRequest> {
        let mut out = Vec::new();
        for need in needs {
            let key = &need.request.key;
            if has(key) || out.iter().any(|r: &SvgRasterRequest| &r.key == key) {
                continue;
            }
            if self.failed.get(key).is_some_and(|&at| now - at < RETRY_US) {
                continue;
            }
            if let Some(p) = self.pending.get_mut(key) {
                // Bands are arriving, or the ask is recent: wait for it.
                if !p.data.is_empty() || now - p.asked_us < ASK_AGAIN_US {
                    continue;
                }
                p.session = session;
                p.asked_us = now;
            } else {
                self.pending.insert(
                    key.clone(),
                    Pending {
                        session,
                        asked_us: now,
                        data: Vec::new(),
                        rows: Vec::new(),
                        missing: key.height,
                    },
                );
            }
            self.failed.remove(key);
            out.push(need.request.clone());
        }
        out
    }

    /// The renderer posted `key` before asking again (a snapshot): it is
    /// pending until its bands arrive.
    pub fn expect(&mut self, session: Key, key: &SvgRasterKey, now: i64) {
        self.pending.entry(key.clone()).or_insert(Pending {
            session,
            asked_us: now,
            data: Vec::new(),
            rows: Vec::new(),
            missing: key.height,
        });
    }

    /// Whether a snapshot waiting for `upload_key` must keep waiting.
    pub fn outstanding(&self, upload_key: &str) -> bool {
        upload_key.starts_with("svg:") && self.pending.keys().any(|k| k.upload_key() == upload_key)
    }

    /// One band (validated by the protocol). Bands nobody asked for are
    /// accepted too: a snapshot's renderer posts before the engine hears
    /// that it did.
    pub fn band(&mut self, session: Key, band: SvgRasterPixels, now: i64) -> Band {
        let bytes = band.key.bytes();
        if !self.pending.contains_key(&band.key) {
            if self.pending.len() >= MAX_PENDING {
                return Band::Refused(band.key);
            }
            self.expect(session, &band.key, now);
        }
        let p = self.pending.get_mut(&band.key).expect("inserted above");
        if p.data.is_empty() {
            if self.bytes + bytes > MAX_PENDING_BYTES {
                return Band::Refused(band.key);
            }
            self.bytes += bytes;
            p.data = vec![0; bytes as usize];
            p.rows = vec![false; band.key.height as usize];
            p.missing = band.key.height;
        }
        let row_bytes = band.key.width as usize * 4;
        let start = band.row as usize * row_bytes;
        p.data[start..start + band.data.0.len()].copy_from_slice(&band.data.0);
        for r in band.row..band.row + band.rows {
            let seen = &mut p.rows[r as usize];
            if !*seen {
                *seen = true;
                p.missing -= 1;
            }
        }
        if p.missing > 0 {
            return Band::Partial;
        }
        let done = self.pending.remove(&band.key).expect("present");
        self.bytes -= done.data.len() as u64;
        self.failed.remove(&band.key);
        self.logged.remove(&band.key);
        self.received += 1;
        Band::Complete(band.key, done.data)
    }

    /// The renderer could not draw `key`. Returns whether to log it (once
    /// per key until it succeeds).
    pub fn failed(&mut self, key: &SvgRasterKey, now: i64) -> bool {
        if let Some(p) = self.pending.remove(key) {
            self.bytes -= p.data.len() as u64;
        }
        self.failed.insert(key.clone(), now);
        self.failures += 1;
        self.logged.insert(key.clone())
    }

    /// Whether the renderer said it could not draw `key` (and it has not
    /// arrived since).
    pub fn has_failed(&self, key: &SvgRasterKey) -> bool {
        self.failed.contains_key(key)
    }

    /// When `key` was last asked for, while it is pending.
    pub fn asked_at(&self, key: &SvgRasterKey) -> Option<i64> {
        self.pending.get(key).map(|p| p.asked_us)
    }

    /// When a frame that drew without `key` should ask for it again: a
    /// failure after [`RETRY_US`], an ask nothing arrived for after
    /// [`ASK_AGAIN_US`] ([`SvgState::wanted`] decides then).
    pub fn retry_at(&self, key: &SvgRasterKey) -> Option<i64> {
        if let Some(at) = self.failed.get(key) {
            return Some(at + RETRY_US);
        }
        self.pending
            .get(key)
            .filter(|p| p.data.is_empty())
            .map(|p| p.asked_us + ASK_AGAIN_US)
    }

    /// A session closed: its asks and partial rasters go.
    pub fn forget_session(&mut self, session: Key) {
        let mut freed = 0;
        self.pending.retain(|_, p| {
            let keep = p.session != session;
            if !keep {
                freed += p.data.len() as u64;
            }
            keep
        });
        self.bytes -= freed;
    }

    /// Rasters asked for or assembling.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{Bytes, SessionId};
    use pm_protocol::host::PortId;
    use pm_render::vector::svg_rasters::SvgRasterNeed;

    fn session(n: u64) -> Key {
        (PortId(n as u32), SessionId::new(n).unwrap())
    }

    fn key(w: u32, h: u32) -> SvgRasterKey {
        SvgRasterKey {
            content: "ab".repeat(32),
            width: w,
            height: h,
        }
    }

    fn need(k: SvgRasterKey) -> SvgRasterNeed {
        SvgRasterNeed {
            request: SvgRasterRequest {
                key: k,
                asset: "a1".into(),
            },
            path: "/a.svg".into(),
            composition: "c".into(),
            layer: "l".into(),
        }
    }

    fn band(k: &SvgRasterKey, row: u32, rows: u32, fill: u8) -> SvgRasterPixels {
        SvgRasterPixels {
            key: k.clone(),
            row,
            rows,
            data: Bytes(vec![fill; (k.width * rows * 4) as usize]),
        }
    }

    #[test]
    fn each_key_is_asked_once_while_pending_and_again_after_a_while() {
        let mut s = SvgState::default();
        let needs = vec![need(key(2, 2)), need(key(2, 2)), need(key(4, 2))];
        assert_eq!(s.wanted(session(1), &needs, 0, |_| false).len(), 2);
        assert!(s.wanted(session(1), &needs, 1_000, |_| false).is_empty());
        assert!(
            s.wanted(session(2), &needs, 1_000, |_| false).is_empty(),
            "pending for any session"
        );
        assert_eq!(
            s.wanted(session(1), &needs, ASK_AGAIN_US + 1, |_| false)
                .len(),
            2
        );
        // What the renderer already stores is never asked for.
        let mut fresh = SvgState::default();
        assert!(fresh.wanted(session(1), &needs, 0, |_| true).is_empty());
    }

    #[test]
    fn bands_assemble_in_any_order_into_one_raster() {
        let mut s = SvgState::default();
        let k = key(2, 3);
        s.wanted(session(1), &[need(k.clone())], 0, |_| false);
        assert!(s.outstanding(&k.upload_key()));
        assert_eq!(s.band(session(1), band(&k, 2, 1, 9), 1), Band::Partial);
        assert_eq!(s.band(session(1), band(&k, 0, 1, 7), 1), Band::Partial);
        match s.band(session(1), band(&k, 1, 1, 8), 2) {
            Band::Complete(done, data) => {
                assert_eq!(done, k);
                assert_eq!(&data[..8], &[7; 8]);
                assert_eq!(&data[8..16], &[8; 8]);
                assert_eq!(&data[16..], &[9; 8]);
            }
            other => panic!("{other:?}"),
        }
        assert!(!s.outstanding(&k.upload_key()));
        assert_eq!((s.pending(), s.received, s.bytes), (0, 1, 0));
    }

    #[test]
    fn snapshots_wait_for_what_the_renderer_posted() {
        let mut s = SvgState::default();
        let k = key(1, 1);
        // The bands may arrive before the snapshot says they were posted.
        assert!(matches!(
            s.band(session(3), band(&k, 0, 1, 1), 0),
            Band::Complete(..)
        ));
        assert!(!s.outstanding(&k.upload_key()));
        let k2 = key(1, 2);
        s.expect(session(3), &k2, 0);
        assert!(s.outstanding(&k2.upload_key()));
        assert!(!s.outstanding("fallback-key"));
        s.forget_session(session(3));
        assert!(!s.outstanding(&k2.upload_key()));
    }

    #[test]
    fn failures_are_logged_once_and_retried_later() {
        let mut s = SvgState::default();
        let k = key(2, 2);
        s.wanted(session(1), &[need(k.clone())], 0, |_| false);
        assert!(s.failed(&k, 10));
        assert!(!s.failed(&k, 11), "logged once");
        assert!(
            s.wanted(session(1), &[need(k.clone())], 20, |_| false)
                .is_empty()
        );
        assert_eq!(
            s.wanted(session(1), &[need(k.clone())], 11 + RETRY_US, |_| false)
                .len(),
            1
        );
        assert_eq!(s.failures, 2);
    }

    #[test]
    fn a_failed_or_unanswered_key_says_when_to_ask_again() {
        let mut s = SvgState::default();
        let k = key(2, 2);
        assert_eq!(s.retry_at(&k), None, "never asked");
        s.wanted(session(1), &[need(k.clone())], 100, |_| false);
        assert_eq!(s.asked_at(&k), Some(100));
        assert_eq!(s.retry_at(&k), Some(100 + ASK_AGAIN_US), "unanswered");
        // Bands arriving: it is on its way.
        s.band(session(1), band(&k, 0, 1, 1), 200);
        assert_eq!(s.retry_at(&k), None);
        s.failed(&k, 300);
        assert_eq!(s.asked_at(&k), None, "no longer pending");
        assert_eq!(s.retry_at(&k), Some(300 + RETRY_US));
        assert!(
            s.wanted(session(1), &[need(k.clone())], 300 + RETRY_US - 1, |_| {
                false
            })
            .is_empty()
        );
        assert_eq!(
            s.wanted(session(1), &[need(k.clone())], 300 + RETRY_US, |_| false)
                .len(),
            1,
            "asked again once due"
        );
    }

    #[test]
    fn a_refused_band_names_its_raster() {
        let mut s = SvgState::default();
        for i in 0..MAX_PENDING as u32 {
            assert_eq!(
                s.band(session(1), band(&key(1, 2 + i), 0, 1, 0), 0),
                Band::Partial
            );
        }
        let k = key(3, 3);
        assert_eq!(
            s.band(session(1), band(&k, 0, 1, 0), 0),
            Band::Refused(k.clone())
        );
    }
}
