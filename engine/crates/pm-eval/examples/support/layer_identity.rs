//! A digest of every answer the layer replay (`tests/support/layer_replay.rs`)
//! or the captions replay (`tests/support/caption_replay.rs`) gives, for the
//! native/wasm32 bit-identity check of the layer queries (plan section 7,
//! determinism; Phase 1 gate).
//!
//! Numbers are hashed by their IEEE-754 bits, so `-0` and `0` differ; every
//! NaN hashes alike (JavaScript cannot tell NaNs apart, and wasm32 does not
//! fix the sign or payload of a NaN an operation produces).

#[path = "../../tests/support/caption_replay.rs"]
mod caption_replay;
#[path = "../../tests/support/layer_replay.rs"]
mod layer_replay;

use layer_replay::V;

/// FNV-1a over a tagged encoding of the answers.
pub struct Digest {
    hash: u64,
    count: u64,
}

impl Digest {
    fn new() -> Digest {
        Digest {
            hash: 0xcbf2_9ce4_8422_2325,
            count: 0,
        }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= u64::from(b);
            self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn text(&mut self, s: &str) {
        self.bytes(&(s.len() as u64).to_le_bytes());
        self.bytes(s.as_bytes());
    }

    fn value(&mut self, v: &V) {
        match v {
            V::U => self.bytes(b"u"),
            V::Z => self.bytes(b"z"),
            V::N(n) => {
                self.count += 1;
                self.bytes(b"n");
                let bits = if n.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    n.to_bits()
                };
                self.bytes(&bits.to_le_bytes());
            }
            V::S(s) => {
                self.bytes(b"s");
                self.text(s);
            }
            V::B(b) => self.bytes(if *b { b"t" } else { b"f" }),
            V::A(a) => {
                self.bytes(b"a");
                self.bytes(&(a.len() as u64).to_le_bytes());
                for x in a {
                    self.value(x);
                }
            }
            V::O(o) => {
                self.bytes(b"o");
                self.bytes(&(o.len() as u64).to_le_bytes());
                for (k, x) in o {
                    self.text(k);
                    self.value(x);
                }
            }
            V::Throw(m) | V::ThrowInternal(m) => {
                self.bytes(b"!");
                self.text(m);
            }
        }
    }

    /// The hash.
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// How many numbers the hash covers.
    pub fn numbers(&self) -> u64 {
        self.count
    }
}

/// The digest of the port's answers to the vectors in `text` (the JSON of
/// `layer-vectors.json.gz` or `caption-vectors.json.gz`, decompressed).
pub fn digest(text: &str) -> Digest {
    let vectors = pm_model::json::parse(text).expect("vectors JSON");
    let mut d = Digest::new();
    if layer_replay::get(&vectors, "docs").is_some() {
        layer_replay::replay(&vectors, &mut |op, _, ours, _| {
            d.text(op);
            d.value(&ours);
        });
    } else {
        caption_replay::replay(&vectors, &mut |op, _, ours, _| {
            d.text(op);
            d.value(&ours);
        });
    }
    d
}
