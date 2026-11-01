//! Engine-held synthetic frame; no actual engine renderer is linked in this spike.
use serde_json::{Value, json};
use std::collections::HashMap;
pub const BYTES: usize = 1920 * 1080 * 4;
pub fn expected(i: usize) -> u8 {
    if i % 4 == 3 {
        255
    } else {
        (i.wrapping_mul(17).wrapping_add(23) & 255) as u8
    }
}
pub fn frame() -> Vec<u8> {
    (0..BYTES).map(|i| expected(i)).collect()
}
pub fn valid(bytes: &[u8]) -> bool {
    bytes.len() == BYTES && bytes.iter().enumerate().all(|(i, b)| *b == expected(i))
}
#[derive(Default)]
pub struct Renders {
    next: u64,
    pending: HashMap<u64, (u64, Option<Vec<u8>>)>,
    pub output: Option<Vec<u8>>,
}
impl Renders {
    pub fn begin(&mut self, nested: bool) -> Value {
        self.next += 1;
        let id = self.next;
        if nested {
            json!({"status":"rendered","id":"frame0","revision":7})
        } else {
            self.pending.insert(id, (7, None));
            json!({"status":"needs_dependency","render":id,"revision":7,"override":"raster0"})
        }
    }
    pub fn upload(&mut self, id: u64, revision: u64, rgba: &[u8]) -> bool {
        if rgba.len() != 16 * 16 * 4 || !rgba.chunks_exact(4).all(|p| p == [35, 180, 90, 255]) {
            return false;
        }
        if let Some((r, pixels)) = self.pending.get_mut(&id) {
            if *r == revision {
                *pixels = Some(rgba.to_vec());
                return true;
            }
        }
        false
    }
    pub fn resume(&mut self, id: u64, revision: u64, dependency: bool) -> Value {
        match self.pending.remove(&id) {
            Some((r, Some(override_pixels))) if r == revision && dependency => {
                let mut out = frame();
                for row in 0..16 {
                    out[row * 1920 * 4..row * 1920 * 4 + 64]
                        .copy_from_slice(&override_pixels[row * 64..row * 64 + 64]);
                }
                self.output = Some(out);
                json!({"status":"rendered","id":"outer0","revision":r})
            }
            _ => json!({"status":"failed","code":"dependency_cycle_or_stale"}),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_is_pinned_single_use_and_rejects_stale() {
        let mut r = Renders::default();
        let a = r.begin(false);
        let id = a["render"].as_u64().unwrap();
        assert_eq!(r.resume(id, 8, true)["status"], "failed");
        let b = r.begin(false);
        let id = b["render"].as_u64().unwrap();
        assert!(r.upload(id, 7, &[35, 180, 90, 255].repeat(256)));
        assert_eq!(r.resume(id, 7, true)["status"], "rendered");
        assert_eq!(r.resume(id, 7, true)["status"], "failed");
    }
    #[test]
    fn all_pixel_bytes_roundtrip_and_corruption_is_rejected() {
        let mut a = frame();
        assert!(valid(&a));
        a[BYTES - 1] ^= 1;
        assert!(!valid(&a));
        assert!(!valid(&a[..BYTES - 1]));
    }
}
