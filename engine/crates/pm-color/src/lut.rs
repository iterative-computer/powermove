//! Strict IRIDAS .cube reader and f64 evaluation; RGB rows are red-fastest.
//! No implicit transfer function, gamut conversion, alpha handling or output clamp.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LutError(pub String);
impl fmt::Display for LutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for LutError {}

#[derive(Clone, Debug, PartialEq)]
pub struct CubeLut {
    pub title: Option<String>,
    domain: [[f64; 3]; 2],
    shaper: Vec<[f64; 3]>,
    size: usize,
    cube: Vec<[f64; 3]>,
    cube_range: [f64; 2],
}
impl CubeLut {
    /// Supports 1D, 3D, and combined shaper + 3D files. Bounded before allocation.
    /// DOMAIN_MIN/MAX applies to the first stage. Per-stage INPUT_RANGE directives
    /// may be used instead; mixing the two domain conventions is refused.
    pub fn parse(text: &str) -> Result<Self, LutError> {
        let mut result = Self {
            title: None,
            domain: [[0.; 3], [1.; 3]],
            shaper: Vec::new(),
            size: 0,
            cube: Vec::new(),
            cube_range: [0., 1.],
        };
        let mut one = None;
        let mut three = None;
        let mut seen = std::collections::BTreeSet::new();
        let mut rows = Vec::new();
        let mut one_range = None;
        let mut three_range = None;
        for (line_index, raw) in text.lines().enumerate() {
            let fail = |why: &str| LutError(format!("line {}: {why}", line_index + 1));
            // # inside a quoted TITLE is part of the title, not a comment.
            let mut quoted = false;
            let end = raw
                .char_indices()
                .find_map(|(i, c)| {
                    if c == '"' {
                        quoted = !quoted;
                    }
                    (c == '#' && !quoted).then_some(i)
                })
                .unwrap_or(raw.len());
            let line = raw[..end].trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<_> = line.split_whitespace().collect();
            let key = fields[0];
            let directive = matches!(
                key,
                "TITLE"
                    | "LUT_1D_SIZE"
                    | "LUT_3D_SIZE"
                    | "DOMAIN_MIN"
                    | "DOMAIN_MAX"
                    | "LUT_1D_INPUT_RANGE"
                    | "LUT_3D_INPUT_RANGE"
            );
            if directive {
                if !rows.is_empty() {
                    return Err(fail("header after table data"));
                }
                if !seen.insert(key.to_owned()) {
                    return Err(fail("duplicate header"));
                }
            }
            let numbers = |parts: &[&str]| -> Result<Vec<f64>, LutError> {
                parts
                    .iter()
                    .map(|s| {
                        s.parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .ok_or_else(|| fail("expected finite number"))
                    })
                    .collect()
            };
            match key {
                "TITLE" => {
                    let title = line[5..].trim();
                    if title.len() < 2
                        || !title.starts_with('"')
                        || !title.ends_with('"')
                        || title[1..title.len() - 1].contains('"')
                    {
                        return Err(fail("expected quoted title"));
                    }
                    result.title = Some(title[1..title.len() - 1].into());
                }
                "LUT_1D_SIZE" | "LUT_3D_SIZE" => {
                    if fields.len() != 2 {
                        return Err(fail("expected one integer size"));
                    }
                    let n = fields[1]
                        .parse::<usize>()
                        .map_err(|_| fail("invalid size"))?;
                    if !(2..=if key == "LUT_1D_SIZE" { 65536 } else { 129 }).contains(&n) {
                        return Err(fail("size outside supported bounds"));
                    }
                    if key == "LUT_1D_SIZE" {
                        one = Some(n);
                    } else {
                        three = Some(n);
                    }
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    if fields.len() != 4 {
                        return Err(fail("expected RGB domain"));
                    }
                    let values = numbers(&fields[1..])?;
                    result.domain[usize::from(key == "DOMAIN_MAX")].copy_from_slice(&values);
                }
                "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                    if fields.len() != 3 {
                        return Err(fail("expected min max range"));
                    }
                    let v = numbers(&fields[1..])?;
                    if v[0] >= v[1] || !(v[1] - v[0]).is_finite() {
                        return Err(fail("invalid input range"));
                    }
                    if key == "LUT_1D_INPUT_RANGE" {
                        one_range = Some([v[0], v[1]]);
                    } else {
                        three_range = Some([v[0], v[1]]);
                    }
                }
                _ => {
                    if fields.len() != 3 {
                        return Err(fail("expected RGB row or known header"));
                    }
                    let values = numbers(&fields)?;
                    let expected = one.unwrap_or(0) + three.map_or(0, |n| n * n * n);
                    if rows.len() >= expected {
                        return Err(fail("extra data or missing size header"));
                    }
                    rows.push([values[0], values[1], values[2]]);
                }
            }
        }
        let error = |s: &str| LutError(s.into());
        if one.is_none() && three.is_none() {
            return Err(error("missing LUT size"));
        }
        if one_range.is_some() && one.is_none() || three_range.is_some() && three.is_none() {
            return Err(error("input range without its table"));
        }
        if (one_range.is_some() || three_range.is_some())
            && (seen.contains("DOMAIN_MIN") || seen.contains("DOMAIN_MAX"))
        {
            return Err(error("mixed domain conventions"));
        }
        if let Some(range) = one_range.or(if one.is_none() { three_range } else { None }) {
            result.domain = [[range[0]; 3], [range[1]; 3]];
        }
        for c in 0..3 {
            let width = result.domain[1][c] - result.domain[0][c];
            if width <= 0. || !width.is_finite() {
                return Err(error("invalid domain"));
            }
        }
        let count = one.unwrap_or(0) + three.map_or(0, |n| n * n * n);
        if rows.len() != count {
            return Err(error("incomplete table"));
        }
        result.cube = rows.split_off(one.unwrap_or(0));
        result.shaper = rows;
        result.size = three.unwrap_or(0);
        if one.is_some() {
            result.cube_range = three_range.unwrap_or([0., 1.]);
        }
        Ok(result)
    }

    /// Clamp input coordinates to declared domains; preserve extended table output.
    /// 1D interpolation is channel independent; 3D interpolation is tetrahedral.
    pub fn apply(&self, rgb: [f64; 3]) -> Result<[f64; 3], LutError> {
        if rgb.iter().any(|v| !v.is_finite()) {
            return Err(LutError("nonfinite input".into()));
        }
        let mut p = std::array::from_fn(|c| {
            ((rgb[c] - self.domain[0][c]) / (self.domain[1][c] - self.domain[0][c])).clamp(0., 1.)
        });
        if !self.shaper.is_empty() {
            for (c, value) in p.iter_mut().enumerate() {
                let x = *value * (self.shaper.len() - 1) as f64;
                let lo = (x as usize).min(self.shaper.len() - 2);
                let t = x - lo as f64;
                *value = self.shaper[lo][c] * (1. - t) + self.shaper[lo + 1][c] * t;
            }
            if self.size > 0 {
                for v in &mut p {
                    *v = ((*v - self.cube_range[0]) / (self.cube_range[1] - self.cube_range[0]))
                        .clamp(0., 1.);
                }
            }
        }
        if self.size == 0 {
            return Ok(p);
        }
        let x = p.map(|v| v * (self.size - 1) as f64);
        let mut corner = x.map(|v| (v as usize).min(self.size - 2));
        let f: [f64; 3] = std::array::from_fn(|c| x[c] - corner[c] as f64);
        let mut order = [0, 1, 2];
        order.sort_by(|a, b| f[*b].total_cmp(&f[*a]));
        let at = |q: [usize; 3]| self.cube[q[0] + self.size * (q[1] + self.size * q[2])];
        let mut out = [0.; 3];
        let mut weight = 1. - f[order[0]];
        for i in 0..4 {
            let v = at(corner);
            for c in 0..3 {
                out[c] += weight * v[c];
            }
            if i < 3 {
                corner[order[i]] += 1;
                weight = if i == 2 {
                    f[order[2]]
                } else {
                    f[order[i]] - f[order[i + 1]]
                };
            }
        }
        if out.iter().any(|v| !v.is_finite()) {
            return Err(LutError("LUT arithmetic overflow".into()));
        }
        Ok(out)
    }
}
