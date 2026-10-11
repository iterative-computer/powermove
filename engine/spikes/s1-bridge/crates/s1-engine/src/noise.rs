//! Blue-noise threshold matrix by Ulichney's void-and-cluster method (toroidal
//! Gaussian energy, sigma 1.5). Used for the RPDF dither of the 8-bit display path.

const SIGMA: f64 = 1.5;

/// Returns `n * n` ranks in `0..n*n`, each exactly once.
pub fn void_and_cluster(n: usize) -> Vec<u32> {
    let len = n * n;
    // Toroidal Gaussian kernel indexed by (dx, dy).
    let mut kernel = vec![0f64; len];
    for dy in 0..n {
        for dx in 0..n {
            let wx = dx.min(n - dx) as f64;
            let wy = dy.min(n - dy) as f64;
            kernel[dy * n + dx] = (-(wx * wx + wy * wy) / (2.0 * SIGMA * SIGMA)).exp();
        }
    }
    let add = |energy: &mut [f64], at: usize, sign: f64| {
        let (ax, ay) = (at % n, at / n);
        for y in 0..n {
            let dy = (y + n - ay) % n;
            for x in 0..n {
                let dx = (x + n - ax) % n;
                energy[y * n + x] += sign * kernel[dy * n + dx];
            }
        }
    };

    // Initial binary pattern: ~10% of pixels from a deterministic hash, then relaxed
    // by repeatedly moving the tightest cluster into the largest void.
    let mut on = vec![false; len];
    let mut energy = vec![0f64; len];
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let initial = len / 10;
    let mut placed = 0;
    while placed < initial {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let at = (state % len as u64) as usize;
        if !on[at] {
            on[at] = true;
            add(&mut energy, at, 1.0);
            placed += 1;
        }
    }
    let tightest = |on: &[bool], energy: &[f64]| {
        (0..len).filter(|&i| on[i]).max_by(|&a, &b| energy[a].total_cmp(&energy[b])).unwrap()
    };
    let largest_void = |on: &[bool], energy: &[f64]| {
        (0..len).filter(|&i| !on[i]).min_by(|&a, &b| energy[a].total_cmp(&energy[b])).unwrap()
    };
    loop {
        let c = tightest(&on, &energy);
        on[c] = false;
        add(&mut energy, c, -1.0);
        let v = largest_void(&on, &energy);
        if v == c {
            on[c] = true;
            add(&mut energy, c, 1.0);
            break;
        }
        on[v] = true;
        add(&mut energy, v, 1.0);
    }

    let mut rank = vec![0u32; len];
    // Phase 1: remove points from the initial pattern, tightest cluster first.
    {
        let mut on1 = on.clone();
        let mut e1 = energy.clone();
        for r in (0..initial).rev() {
            let c = tightest(&on1, &e1);
            on1[c] = false;
            add(&mut e1, c, -1.0);
            rank[c] = r as u32;
        }
    }
    // Phase 2 and 3: fill the largest voids until every pixel has a rank.
    for r in initial..len {
        let v = largest_void(&on, &energy);
        on[v] = true;
        add(&mut energy, v, 1.0);
        rank[v] = r as u32;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_a_permutation() {
        let n = 32;
        let r = void_and_cluster(n);
        let mut seen = vec![false; n * n];
        for &v in &r {
            assert!(!seen[v as usize]);
            seen[v as usize] = true;
        }
    }

    /// Blue noise has little low-frequency energy: the share of spectral power at
    /// radial frequency below n/8 is far below white noise's share (the band's area).
    #[test]
    fn low_frequency_energy_is_small() {
        let n = 64usize;
        let r = void_and_cluster(n);
        let v: Vec<f64> = r.iter().map(|&x| (x as f64 + 0.5) / (n * n) as f64 - 0.5).collect();
        let (mut low, mut total, mut low_bins, mut bins) = (0.0, 0.0, 0usize, 0usize);
        for fy in 0..n {
            for fx in 0..n {
                if fx == 0 && fy == 0 {
                    continue;
                }
                let (mut re, mut im) = (0.0, 0.0);
                for y in 0..n {
                    for x in 0..n {
                        let a = -2.0 * std::f64::consts::PI * ((fx * x + fy * y) as f64) / n as f64;
                        re += v[y * n + x] * a.cos();
                        im += v[y * n + x] * a.sin();
                    }
                }
                let p = re * re + im * im;
                let wx = fx.min(n - fx) as f64;
                let wy = fy.min(n - fy) as f64;
                total += p;
                bins += 1;
                if (wx * wx + wy * wy).sqrt() < n as f64 / 8.0 {
                    low += p;
                    low_bins += 1;
                }
            }
        }
        let share = low / total;
        let white_share = low_bins as f64 / bins as f64;
        assert!(share < white_share / 10.0, "low-band share {share:.5} vs white {white_share:.5}");
    }
}
