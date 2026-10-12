//! Distribution summaries.

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Dist {
    pub n: usize,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
    pub max: f64,
    pub min: f64,
}

/// Nearest-rank percentile on a sorted slice (`q` in 0..=1).
pub fn percentile_sorted(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub fn dist(values: &[f64]) -> Dist {
    if values.is_empty() {
        return Dist::default();
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let sum: f64 = v.iter().sum();
    Dist {
        n: v.len(),
        mean: sum / v.len() as f64,
        p50: percentile_sorted(&v, 0.50),
        p95: percentile_sorted(&v, 0.95),
        p99: percentile_sorted(&v, 0.99),
        p999: percentile_sorted(&v, 0.999),
        max: *v.last().unwrap(),
        min: v[0],
    }
}

/// Round to `digits` decimals for compact JSON.
pub fn r(x: f64, digits: i32) -> f64 {
    let m = 10f64.powi(digits);
    (x * m).round() / m
}

impl Dist {
    pub fn rounded(&self, digits: i32) -> Self {
        Self {
            n: self.n,
            mean: r(self.mean, digits),
            p50: r(self.p50, digits),
            p95: r(self.p95, digits),
            p99: r(self.p99, digits),
            p999: r(self.p999, digits),
            max: r(self.max, digits),
            min: r(self.min, digits),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        let d = dist(&v);
        assert_eq!(d.p50, 50.0);
        assert_eq!(d.p95, 95.0);
        assert_eq!(d.p99, 99.0);
        assert_eq!(d.max, 100.0);
        assert_eq!(d.min, 1.0);
        assert_eq!(d.mean, 50.5);
    }

    #[test]
    fn empty_and_single() {
        assert_eq!(dist(&[]).n, 0);
        let d = dist(&[3.0]);
        assert_eq!((d.p50, d.p99, d.max), (3.0, 3.0, 3.0));
    }
}
