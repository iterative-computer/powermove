//! Strict IRIDAS .cube reader and f64 evaluation; RGB rows are red-fastest.
//! No implicit transfer function, gamut conversion, alpha handling or output clamp.
use std::fmt;

/// The largest finite half-float: a LUT stays within working storage.
const MAX_VALUE: f64 = 65504.0;
/// The widest texture row a table may use (Metal's 2D limit).
const MAX_ROW: usize = 16384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LutError(pub String);
impl fmt::Display for LutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for LutError {}

/// A LUT as one RGBA32F texture for the Apply Color LUT shader, its layout in
/// the shader's uniforms. Cube rows come first, red fastest; the 1D stage
/// follows in rows of `width`. `alpha` is 1.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuLut {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    /// Cube edge, or the 1D length of a 1D LUT (the shipped uniform).
    pub size: u32,
    /// Cube edge; 0 without a cube.
    pub cube_size: u32,
    /// Green steps in one atlas row of the cube.
    pub cube_columns: u32,
    /// 1D length; 0 without a 1D stage.
    pub shaper_size: u32,
    /// The texture row the 1D stage starts on.
    pub shaper_row: u32,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// The cube's input range after a 1D stage.
    pub cube_range: [f32; 2],
}

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
        for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
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
                    if values.iter().any(|v| v.abs() > MAX_VALUE) {
                        return Err(fail("value outside half-float range"));
                    }
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

    /// The texture and layout the renderer samples.
    pub fn gpu(&self) -> GpuLut {
        let n = self.size;
        let columns = if n == 0 { 1 } else { n.min(MAX_ROW / n) };
        let rows_per_blue = if n == 0 { 0 } else { n.div_ceil(columns) };
        let cube_width = n * columns;
        let shaper = self.shaper.len();
        let width = cube_width.max(shaper.min(MAX_ROW));
        let cube_rows = n * rows_per_blue;
        let height = cube_rows + shaper.div_ceil(width.max(1));
        let mut rgba = vec![0f32; width * height * 4];
        let mut put = |x: usize, y: usize, v: [f64; 3]| {
            let o = (y * width + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[v[0] as f32, v[1] as f32, v[2] as f32, 1.0]);
        };
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    put(
                        r + n * (g % columns),
                        b * rows_per_blue + g / columns,
                        self.cube[r + n * (g + n * b)],
                    );
                }
            }
        }
        for (i, v) in self.shaper.iter().enumerate() {
            put(i % width, cube_rows + i / width, *v);
        }
        let to_f32 = |v: [f64; 3]| v.map(|c| c as f32);
        GpuLut {
            width: width as u32,
            height: height as u32,
            rgba,
            size: if n > 0 { n as u32 } else { shaper as u32 },
            cube_size: n as u32,
            cube_columns: columns as u32,
            shaper_size: shaper as u32,
            shaper_row: cube_rows as u32,
            domain_min: to_f32(self.domain[0]),
            domain_max: to_f32(self.domain[1]),
            cube_range: self.cube_range.map(|v| v as f32),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_cube(n: usize) -> String {
        let mut text = format!("LUT_3D_SIZE {n}\n");
        let v = |i: usize| i as f64 / (n - 1) as f64;
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    text += &format!("{} {} {}\n", v(r), v(g), v(b));
                }
            }
        }
        text
    }

    #[test]
    fn domains_titles_and_rejections_match_the_shipped_reader() {
        let one = CubeLut::parse(
            "\u{feff}TITLE \"A # title\"\nLUT_1D_SIZE 2\nDOMAIN_MIN -1 -2 -3\nDOMAIN_MAX 1 2 3\n0 1 2\n3 4 5\n",
        )
        .unwrap();
        assert_eq!(one.title.as_deref(), Some("A # title"));
        let gpu = one.gpu();
        assert_eq!((gpu.size, gpu.cube_size, gpu.shaper_size), (2, 0, 2));
        assert_eq!((gpu.width, gpu.height), (2, 1));
        assert_eq!(gpu.rgba, [0., 1., 2., 1., 3., 4., 5., 1.]);
        for bad in [
            "LUT_1D_SIZE 2\n0 0 0",
            "LUT_3D_SIZE 130",
            "LUT_1D_SIZE 65537",
            "LUT_1D_SIZE 2\n0 0 0\nNaN 0 0",
            "LUT_1D_SIZE 2\n0 0 0\n65505 0 0",
            "DOMAIN_MIN 1 1 1\nDOMAIN_MAX 1 1 1\nLUT_1D_SIZE 2\n0 0 0\n1 1 1",
        ] {
            assert!(CubeLut::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pro_cube_sizes_are_read() {
        for n in [17, 33, 65] {
            let lut = CubeLut::parse(&identity_cube(n)).unwrap();
            let out = lut.apply([0.25, 0.5, 0.75]).unwrap();
            for (a, b) in out.iter().zip([0.25, 0.5, 0.75]) {
                assert!((a - b).abs() < 1e-12, "{n}");
            }
        }
    }

    #[test]
    fn the_atlas_addresses_every_cube_texel_and_fits_a_metal_texture() {
        for n in [2usize, 3, 65, 129] {
            let mut text = format!("LUT_3D_SIZE {n}\n");
            for i in 0..n * n * n {
                text += &format!("{} {} 0\n", i % 60000, i % 7);
            }
            let lut = CubeLut::parse(&text).unwrap();
            let gpu = lut.gpu();
            assert!(gpu.width as usize <= MAX_ROW, "{n}");
            let columns = gpu.cube_columns as usize;
            let rows = n.div_ceil(columns);
            for (r, g, b) in [
                (0, 0, 0),
                (n - 1, 0, 0),
                (0, n - 1, 0),
                (0, 0, n - 1),
                (1, 1, 1),
            ] {
                let (x, y) = (r + n * (g % columns), b * rows + g / columns);
                let o = (y * gpu.width as usize + x) * 4;
                let index = (r + n * (g + n * b)) % 60000;
                assert_eq!(gpu.rgba[o], index as f32, "{n} {r} {g} {b}");
            }
        }
    }

    #[test]
    fn a_shaper_follows_the_cube_rows() {
        let text = format!(
            "LUT_1D_SIZE 3\nLUT_3D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n1 1 1\n{}",
            identity_cube(2).split_once('\n').unwrap().1
        );
        let gpu = CubeLut::parse(&text).unwrap().gpu();
        assert_eq!((gpu.cube_size, gpu.shaper_size), (2, 3));
        assert_eq!(gpu.shaper_row, 2);
        let o = (gpu.shaper_row * gpu.width) as usize * 4;
        assert_eq!(gpu.rgba[o..o + 4], [0., 0., 0., 1.]);
    }
}
