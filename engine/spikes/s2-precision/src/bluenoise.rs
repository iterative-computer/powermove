//! A 64×64 blue-noise threshold tile by void-and-cluster (Ulichney 1993), generated
//! deterministically at startup. Used by the encoder-input dither (plan section 17).

use std::sync::OnceLock;

pub const SIZE: usize = 64;
const N: usize = SIZE * SIZE;
const SIGMA: f64 = 1.5;

/// Toroidal Gaussian kernel, indexed by wrapped (dx, dy).
fn kernel() -> Vec<f64> {
    let mut k = vec![0.0; N];
    for dy in 0..SIZE {
        for dx in 0..SIZE {
            let wx = dx.min(SIZE - dx) as f64;
            let wy = dy.min(SIZE - dy) as f64;
            k[dy * SIZE + dx] = (-(wx * wx + wy * wy) / (2.0 * SIGMA * SIGMA)).exp();
        }
    }
    k
}

struct Field {
    k: Vec<f64>,
    energy: Vec<f64>,
}

impl Field {
    fn new(k: Vec<f64>) -> Self {
        Self { k, energy: vec![0.0; N] }
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

    fn build(k: &[f64], bits: &[bool]) -> Self {
        let mut f = Field::new(k.to_vec());
        for (i, &b) in bits.iter().enumerate() {
            if b {
                f.splat(i, 1.0);
            }
        }
        f
    }

    /// Tightest cluster: the set pixel with the highest energy.
    fn tightest(&self, bits: &[bool], want: bool) -> usize {
        let mut best = usize::MAX;
        let mut e = f64::NEG_INFINITY;
        for i in 0..N {
            if bits[i] == want && self.energy[i] > e {
                e = self.energy[i];
                best = i;
            }
        }
        best
    }

    /// Largest void: the unset pixel with the lowest energy.
    fn largest_void(&self, bits: &[bool], want: bool) -> usize {
        let mut best = usize::MAX;
        let mut e = f64::INFINITY;
        for i in 0..N {
            if bits[i] != want && self.energy[i] < e {
                e = self.energy[i];
                best = i;
            }
        }
        best
    }
}

fn generate() -> Vec<u16> {
    let k = kernel();
    // Initial pattern: 10% of pixels from a fixed xorshift sequence.
    let mut bits = vec![false; N];
    let mut s: u64 = 0x2545F4914F6CDD1D;
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
        let c = f.tightest(&bits, true);
        bits[c] = false;
        f.splat(c, -1.0);
        let v = f.largest_void(&bits, true);
        bits[v] = true;
        f.splat(v, 1.0);
        if v == c {
            break;
        }
    }
    let prototype = bits.clone();
    let mut rank = vec![0u16; N];

    // Phase 1: remove from tightest clusters, ranks ones-1 down to 0.
    let mut b = prototype.clone();
    let mut f1 = Field::build(&k, &b);
    for r in (0..ones).rev() {
        let c = f1.tightest(&b, true);
        b[c] = false;
        f1.splat(c, -1.0);
        rank[c] = r as u16;
    }
    // Phase 2: fill largest voids up to half.
    let mut b = prototype;
    let mut f2 = Field::build(&k, &b);
    let mut r = ones;
    while r < N / 2 {
        let v = f2.largest_void(&b, true);
        b[v] = true;
        f2.splat(v, 1.0);
        rank[v] = r as u16;
        r += 1;
    }
    // Phase 3: the remaining zeros are the minority; fill their tightest clusters.
    let inv: Vec<bool> = b.iter().map(|x| !x).collect();
    let mut f3 = Field::build(&k, &inv);
    let mut inv = inv;
    while r < N {
        let c = f3.tightest(&inv, true);
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
    TILE.get_or_init(generate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_is_a_permutation() {
        let t = tile();
        let mut seen = vec![false; N];
        for &r in t {
            assert!(!seen[r as usize]);
            seen[r as usize] = true;
        }
    }

    /// Blue noise: low-frequency power is far below that of white noise.
    #[test]
    fn tile_is_high_pass() {
        let t = tile();
        let mean = (N as f64 - 1.0) / 2.0;
        // Power in the 3×3 lowest non-DC frequencies vs. the total (white noise ≈ 8/4095).
        let mut low = 0.0;
        let mut total = 0.0;
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
                let fu = u.min(SIZE - u);
                let fv = v.min(SIZE - v);
                if fu <= 1 && fv <= 1 {
                    low += p;
                }
            }
        }
        let frac = low / total;
        assert!(frac < 0.1 * 8.0 / 4095.0, "low-frequency fraction {frac}");
    }
}
