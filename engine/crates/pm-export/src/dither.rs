//! Deterministic encoder-input dither (sections 9 and 17).
//!
//! Blue-noise RPDF of ±0.5 LSB, seeded by frame and pixel, added after the matrix in
//! the output domain. Offsets are `(rank + 0.5) / 4096 − 0.5` from a 64×64
//! void-and-cluster rank tile (Ulichney 1993), so |offset| < 0.5 and values already on
//! the output grid round back to themselves. Each frame and channel shifts the tile by
//! a PCG hash of `frame × 4 + channel`. Alpha is never dithered.
//!
//! The tile is generated in plain IEEE f64 arithmetic with `libm`'s software `exp`,
//! so it is identical on every platform; `tests::tile_is_pinned` fixes its hash. The
//! GPU encoder-input pass must use this tile and `dither_offset` bit for bit (S2's
//! prototype, `engine/spikes/s2-precision/src/pipeline.wgsl`, does).

use std::sync::OnceLock;

/// Tile side.
pub const SIZE: usize = 64;
const N: usize = SIZE * SIZE;
const SIGMA: f64 = 1.5;

fn kernel(exp: fn(f64) -> f64) -> Vec<f64> {
    let mut k = vec![0.0; N];
    for dy in 0..SIZE {
        for dx in 0..SIZE {
            let wx = dx.min(SIZE - dx) as f64;
            let wy = dy.min(SIZE - dy) as f64;
            k[dy * SIZE + dx] = exp(-(wx * wx + wy * wy) / (2.0 * SIGMA * SIGMA));
        }
    }
    k
}

struct Field<'k> {
    k: &'k [f64],
    energy: Vec<f64>,
}

impl<'k> Field<'k> {
    fn build(k: &'k [f64], bits: &[bool]) -> Self {
        let mut f = Field {
            k,
            energy: vec![0.0; N],
        };
        for (i, &b) in bits.iter().enumerate() {
            if b {
                f.splat(i, 1.0);
            }
        }
        f
    }

    fn splat(&mut self, i: usize, sign: f64) {
        let (x0, y0) = (i % SIZE, i / SIZE);
        for y in 0..SIZE {
            let dy = (y + SIZE - y0) % SIZE;
            for x in 0..SIZE {
                let dx = (x + SIZE - x0) % SIZE;
                self.energy[y * SIZE + x] += sign * self.k[dy * SIZE + dx];
            }
        }
    }

    /// The set pixel with the highest energy (first on ties).
    fn tightest(&self, bits: &[bool]) -> usize {
        let mut best = usize::MAX;
        let mut e = f64::NEG_INFINITY;
        for (i, (&set, &energy)) in bits.iter().zip(&self.energy).enumerate() {
            if set && energy > e {
                e = energy;
                best = i;
            }
        }
        best
    }

    /// The unset pixel with the lowest energy (first on ties).
    fn largest_void(&self, bits: &[bool]) -> usize {
        let mut best = usize::MAX;
        let mut e = f64::INFINITY;
        for (i, (&set, &energy)) in bits.iter().zip(&self.energy).enumerate() {
            if !set && energy < e {
                e = energy;
                best = i;
            }
        }
        best
    }
}

fn generate(exp: fn(f64) -> f64) -> Vec<u16> {
    let k = kernel(exp);
    // Initial pattern: 10% of pixels from a fixed xorshift sequence.
    let mut bits = vec![false; N];
    let mut s: u64 = 0x2545_F491_4F6C_DD1D;
    let ones = N / 10;
    let mut placed = 0;
    while placed < ones {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let i = (s % N as u64) as usize;
        if !bits[i] {
            bits[i] = true;
            placed += 1;
        }
    }
    // Relax: move the tightest cluster into the largest void until stable.
    let mut f = Field::build(&k, &bits);
    loop {
        let c = f.tightest(&bits);
        bits[c] = false;
        f.splat(c, -1.0);
        let v = f.largest_void(&bits);
        bits[v] = true;
        f.splat(v, 1.0);
        if v == c {
            break;
        }
    }
    let prototype = bits.clone();
    let mut rank = vec![0u16; N];
    // Phase 1: remove from the tightest clusters, ranks ones − 1 down to 0.
    let mut b = prototype.clone();
    let mut f1 = Field::build(&k, &b);
    for r in (0..ones).rev() {
        let c = f1.tightest(&b);
        b[c] = false;
        f1.splat(c, -1.0);
        rank[c] = r as u16;
    }
    // Phase 2: fill the largest voids up to half.
    let mut b = prototype;
    let mut f2 = Field::build(&k, &b);
    let mut r = ones;
    while r < N / 2 {
        let v = f2.largest_void(&b);
        b[v] = true;
        f2.splat(v, 1.0);
        rank[v] = r as u16;
        r += 1;
    }
    // Phase 3: the remaining zeros are the minority; fill their tightest clusters.
    let mut inv: Vec<bool> = b.iter().map(|x| !x).collect();
    let mut f3 = Field::build(&k, &inv);
    while r < N {
        let c = f3.tightest(&inv);
        inv[c] = false;
        f3.splat(c, -1.0);
        rank[c] = r as u16;
        r += 1;
    }
    rank
}

/// Ranks 0..4095, each exactly once, row-major.
pub fn tile() -> &'static [u16] {
    static TILE: OnceLock<Vec<u16>> = OnceLock::new();
    TILE.get_or_init(|| generate(libm::exp))
}

/// The tile as offsets `(rank + 0.5) / 4096 − 0.5`.
fn offsets() -> &'static [f64] {
    static OFFSETS: OnceLock<Vec<f64>> = OnceLock::new();
    OFFSETS.get_or_init(|| {
        tile()
            .iter()
            .map(|&r| (r as f64 + 0.5) / N as f64 - 0.5)
            .collect()
    })
}

/// PCG output permutation (O'Neill): deterministic on every device.
pub fn hash_u32(x: u32) -> u32 {
    let state = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// The dither offset in (−0.5, 0.5) LSB for pixel (`x`, `y`) of `channel` (0 = Y′,
/// 1 = Cb, 2 = Cr, or R, G, B) in frame `frame`. Chroma planes pass chroma-site
/// coordinates.
pub fn dither_offset(frame: u32, x: u32, y: u32, channel: u32) -> f64 {
    let h = hash_u32(frame.wrapping_mul(4).wrapping_add(channel));
    let ox = h & 63;
    let oy = (h >> 6) & 63;
    let qx = (x.wrapping_add(ox) & 63) as usize;
    let qy = (y.wrapping_add(oy) & 63) as usize;
    offsets()[qy * SIZE + qx]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn tile_is_a_permutation() {
        let mut seen = vec![false; N];
        for &r in tile() {
            assert!(!seen[r as usize]);
            seen[r as usize] = true;
        }
    }

    /// Blue noise: low-frequency power is far below white noise's share (8/4095).
    #[test]
    fn tile_is_high_pass() {
        let t = tile();
        let mean = (N as f64 - 1.0) / 2.0;
        let (mut low, mut total) = (0.0, 0.0);
        for v in 0..SIZE {
            for u in 0..SIZE {
                if u == 0 && v == 0 {
                    continue;
                }
                let (mut re, mut im) = (0.0, 0.0);
                for y in 0..SIZE {
                    for x in 0..SIZE {
                        let a = 2.0 * std::f64::consts::PI * ((u * x + v * y) as f64) / SIZE as f64;
                        let s = t[y * SIZE + x] as f64 - mean;
                        re += s * a.cos();
                        im -= s * a.sin();
                    }
                }
                let p = re * re + im * im;
                total += p;
                if u.min(SIZE - u) <= 1 && v.min(SIZE - v) <= 1 {
                    low += p;
                }
            }
        }
        assert!(
            low / total < 0.1 * 8.0 / 4095.0,
            "low-frequency fraction {}",
            low / total
        );
    }

    /// Pins the tile: any change to the generator changes every dithered export.
    #[test]
    fn tile_is_pinned() {
        let bytes: Vec<u8> = tile().iter().flat_map(|r| r.to_le_bytes()).collect();
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, PINNED_TILE_SHA256);
    }

    pub(crate) const PINNED_TILE_SHA256: &str =
        "007e1ea0161e008f838588ebdb239ebfca6b4f4338f30e5c6c1c54e27c943443";

    /// The tile depends on `exp` to the last bit: macOS's `exp` (which S2's prototype
    /// used) and libm's give different tiles. libm keeps it identical on every platform;
    /// both are valid blue noise.
    #[test]
    fn tile_is_generated_with_the_portable_exp() {
        let platform = generate(f64::exp);
        let differing = platform.iter().zip(tile()).filter(|(a, b)| a != b).count();
        if differing > 0 {
            let mut seen = vec![false; N];
            platform.iter().for_each(|&r| seen[r as usize] = true);
            assert!(
                seen.iter().all(|&s| s),
                "the platform tile is a permutation too"
            );
        }
        eprintln!("{differing} of {N} ranks differ between the platform exp and libm");
    }

    #[test]
    fn offsets_are_rpdf_within_half_an_lsb_and_deterministic() {
        let mut sum = 0.0;
        let mut max: f64 = 0.0;
        for y in 0..64 {
            for x in 0..64 {
                let o = dither_offset(5, x, y, 1);
                assert_eq!(o, dither_offset(5, x, y, 1));
                sum += o;
                max = max.max(o.abs());
            }
        }
        assert!(max < 0.5 && max > 0.499, "max |offset| {max}");
        assert!(sum.abs() < 1e-9, "mean offset {}", sum / 4096.0);
        // Frames and channels shift the tile.
        let differs = (0..64)
            .filter(|&x| dither_offset(0, x, 0, 0) != dither_offset(1, x, 0, 0))
            .count();
        assert!(differs > 32);
        let differs = (0..64)
            .filter(|&x| dither_offset(0, x, 0, 0) != dither_offset(0, x, 0, 1))
            .count();
        assert!(differs > 32);
    }

    #[test]
    fn pcg_hash_matches_reference_values() {
        // pcg_hash from Jarzynski and Olano, "Hash Functions for GPU Rendering" (2020).
        assert_eq!(hash_u32(0), 129_708_002);
        assert_eq!(hash_u32(1), 2_831_084_092);
    }
}
