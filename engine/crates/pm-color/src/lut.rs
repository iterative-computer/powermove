//! Strict IRIDAS `.cube` and Autodesk `.3dl` readers and f64 evaluation; cube
//! rows are kept red-fastest. Every refusal is a sentence a person can read.
//! No implicit transfer function, gamut conversion, alpha handling or output clamp.
use std::fmt;

/// The largest finite half-float: a LUT stays within working storage.
const MAX_VALUE: f64 = 65504.0;
/// The largest `.cube` file; a 129-point cube is about 80 MiB of text.
pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
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

/// The bit depth of a `.3dl` table whose largest value is `largest`, by
/// OpenColorIO's rule: a table may overshoot its white by up to twice, so
/// up to 511 is 8 bits, 2047 is 10, 8191 is 12 and anything above is 16
/// (14-bit tables are not made). Below 128 it is not whole code values.
fn likely_depth(largest: f64) -> Option<u32> {
    match largest {
        v if v < 128. => None,
        v if v <= 511. => Some(8),
        v if v <= 2047. => Some(10),
        v if v <= 8191. => Some(12),
        _ => Some(16),
    }
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
            let fail = |why: &str| LutError(format!("Line {} {why}", line_index + 1));
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
                    return Err(fail("is a heading after the table’s rows began."));
                }
                if !seen.insert(key.to_owned()) {
                    return Err(fail("repeats a heading."));
                }
            }
            let numbers = |parts: &[&str]| -> Result<Vec<f64>, LutError> {
                parts
                    .iter()
                    .map(|s| {
                        s.parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .ok_or_else(|| fail("has a value that isn’t a number."))
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
                        return Err(fail("has a title that isn’t in quotes."));
                    }
                    result.title = Some(title[1..title.len() - 1].into());
                }
                "LUT_1D_SIZE" | "LUT_3D_SIZE" => {
                    let size = (fields.len() == 2)
                        .then(|| fields[1].parse::<usize>().ok())
                        .flatten()
                        .ok_or_else(|| fail("has a size that isn’t a whole number."))?;
                    let largest = if key == "LUT_1D_SIZE" { 65536 } else { 129 };
                    if size < 2 {
                        return Err(fail("asks for a table of fewer than 2 points."));
                    }
                    if size > largest {
                        return Err(fail(&format!(
                            "asks for a table of {size} points; the largest is {largest}."
                        )));
                    }
                    let n = size;
                    if key == "LUT_1D_SIZE" {
                        one = Some(n);
                    } else {
                        three = Some(n);
                    }
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    if fields.len() != 4 {
                        return Err(fail("needs three numbers: red, green and blue."));
                    }
                    let values = numbers(&fields[1..])?;
                    result.domain[usize::from(key == "DOMAIN_MAX")].copy_from_slice(&values);
                }
                "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                    if fields.len() != 3 {
                        return Err(fail("needs two numbers: the lowest and highest input."));
                    }
                    let v = numbers(&fields[1..])?;
                    if v[0] >= v[1] || !(v[1] - v[0]).is_finite() {
                        return Err(fail("has an empty input range."));
                    }
                    if key == "LUT_1D_INPUT_RANGE" {
                        one_range = Some([v[0], v[1]]);
                    } else {
                        three_range = Some([v[0], v[1]]);
                    }
                }
                _ => {
                    if fields.len() != 3 {
                        return Err(fail("isn’t a row of three numbers or a known heading."));
                    }
                    let values = numbers(&fields)?;
                    let expected = one.unwrap_or(0) + three.map_or(0, |n| n * n * n);
                    if expected == 0 {
                        return Err(fail("is a row before any LUT_1D_SIZE or LUT_3D_SIZE line."));
                    }
                    if rows.len() >= expected {
                        return Err(fail("is a row past the end of the table."));
                    }
                    rows.push([values[0], values[1], values[2]]);
                }
            }
        }
        let error = |s: &str| LutError(s.into());
        if one.is_none() && three.is_none() {
            return Err(error("The file has no LUT_1D_SIZE or LUT_3D_SIZE line."));
        }
        if one_range.is_some() && one.is_none() || three_range.is_some() && three.is_none() {
            return Err(error(
                "The file gives an input range for a table it doesn’t have.",
            ));
        }
        if (one_range.is_some() || three_range.is_some())
            && (seen.contains("DOMAIN_MIN") || seen.contains("DOMAIN_MAX"))
        {
            return Err(error(
                "The file gives both DOMAIN_MIN/MAX and an input range; a table has one or the other.",
            ));
        }
        if let Some(range) = one_range.or(if one.is_none() { three_range } else { None }) {
            result.domain = [[range[0]; 3], [range[1]; 3]];
        }
        for c in 0..3 {
            let width = result.domain[1][c] - result.domain[0][c];
            if width <= 0. || !width.is_finite() {
                return Err(error("The table’s input range is empty."));
            }
        }
        let count = one.unwrap_or(0) + three.map_or(0, |n| n * n * n);
        if rows.len() != count {
            return Err(LutError(format!(
                "The table has {} rows; its size needs {count}.",
                rows.len()
            )));
        }
        result.cube = rows.split_off(one.unwrap_or(0));
        result.shaper = rows;
        result.size = three.unwrap_or(0);
        if one.is_some() {
            result.cube_range = three_range.unwrap_or([0., 1.]);
        }
        Ok(result)
    }

    /// Reads a LUT file of `format`: `3dl`, or `.cube` for anything else.
    pub fn parse_format(format: &str, text: &str) -> Result<Self, LutError> {
        if format.eq_ignore_ascii_case("3dl") {
            Self::parse_3dl(text)
        } else {
            Self::parse(text)
        }
    }

    /// Reads an Autodesk `.3dl` cube as OpenColorIO does: one row of whole
    /// numbers per grid point, blue fastest, after an input line of more
    /// than three numbers. The cube's depth is `Mesh in out`'s output depth
    /// when the file has that line, and otherwise comes from the table's
    /// largest value ([`likely_depth`]). The input line is a 1D table applied
    /// before the cube, over 0..1, at its own depth from its own largest
    /// value; one within two codes of evenly spaced changes nothing and is
    /// dropped.
    pub fn parse_3dl(text: &str) -> Result<Self, LutError> {
        let mut input: Option<Vec<f64>> = None;
        let mut declared: Option<(u32, u32)> = None;
        let mut rows = Vec::new();
        for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let fail = |why: &str| LutError(format!("Line {} {why}", line_index + 1));
            let line = raw.split('#').next().unwrap_or("").trim();
            let fields: Vec<_> = line.split_whitespace().collect();
            let Some(key) = fields.first() else { continue };
            if key.eq_ignore_ascii_case("3DMESH")
                || key.eq_ignore_ascii_case("LUT8")
                || key.eq_ignore_ascii_case("gamma")
            {
                continue;
            }
            if key.eq_ignore_ascii_case("Mesh") {
                let bits = |i: usize| fields.get(i).and_then(|s| s.parse::<u32>().ok());
                declared = match (fields.len(), bits(1), bits(2)) {
                    (3, Some(input @ 1..=7), Some(output @ 8..=16)) => Some((input, output)),
                    _ => return Err(fail("needs the grid’s and the output’s bit depths.")),
                };
                continue;
            }
            let values = fields
                .iter()
                .map(|s| s.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.))
                .collect::<Option<Vec<f64>>>()
                .ok_or_else(|| fail("has a value that isn’t a number of zero or more."))?;
            match values.len() {
                3 => rows.push([values[0], values[1], values[2]]),
                4.. if input.is_none() => input = Some(values),
                4.. => return Err(fail("is a second input line; a .3dl has one.")),
                _ => return Err(fail("isn’t a row of three numbers.")),
            }
            if rows.len() > 129 * 129 * 129 + 1 {
                return Err(fail("is a row past the largest table of 129 points."));
            }
        }
        // A three-point table's input line reads as a row.
        if input.is_none() && rows.len() == 28 {
            input = Some(rows.remove(0).to_vec());
        }
        let size = match declared {
            Some((input, _)) => (1 << input) + 1,
            None => (rows.len() as f64).cbrt().round() as usize,
        };
        if !(2..=129).contains(&size) {
            return Err(LutError(
                "The file doesn’t give a table of 2 to 129 points.".into(),
            ));
        }
        if rows.len() != size * size * size {
            return Err(LutError(format!(
                "The table has {} rows; its size needs {}.",
                rows.len(),
                size * size * size
            )));
        }
        let largest = |values: &mut dyn Iterator<Item = f64>| values.fold(0f64, f64::max);
        let depth = |largest: f64, what: &str| {
            likely_depth(largest).ok_or_else(|| {
                LutError(format!(
                    "The {what}’s largest value is {largest}, too low for a .3dl: its values are whole code values of 8 bits or more."
                ))
            })
        };
        let bits = match declared {
            Some((_, output)) => output,
            None => depth(largest(&mut rows.iter().flatten().copied()), "table")?,
        };
        let scale = f64::from((1u32 << bits) - 1);
        let mut cube = vec![[0.; 3]; rows.len()];
        for r in 0..size {
            for g in 0..size {
                for b in 0..size {
                    cube[r + size * (g + size * b)] =
                        rows[b + size * (g + size * r)].map(|v| v / scale);
                }
            }
        }
        let mut shaper = Vec::new();
        if let Some(input) = input {
            let top =
                f64::from((1u32 << depth(largest(&mut input.iter().copied()), "input line")?) - 1);
            let step = top / (input.len() - 1) as f64;
            let even = |(i, v): (usize, &f64)| (i as f64 * step - v).abs() < 2.;
            if !input.iter().enumerate().all(even) {
                shaper = input.iter().map(|v| [v / top; 3]).collect();
            }
        }
        Ok(Self {
            title: None,
            domain: [[0.; 3], [1.; 3]],
            shaper,
            size,
            cube,
            cube_range: [0., 1.],
        })
    }

    /// The texture and layout the renderer samples, or why it can't hold
    /// this table: working targets stop at half-float's finite range.
    pub fn gpu(&self) -> Result<GpuLut, LutError> {
        if self
            .shaper
            .iter()
            .chain(&self.cube)
            .flatten()
            .any(|v| v.abs() > MAX_VALUE)
        {
            return Err(LutError(
                "The table has a value beyond ±65504, the most a frame can hold.".into(),
            ));
        }
        // The renderer divides by these spans in f32.
        let spans = (0..3)
            .map(|c| (self.domain[0][c], self.domain[1][c]))
            .chain([(self.cube_range[0], self.cube_range[1])]);
        for (lo, hi) in spans {
            let (lo, hi) = (lo as f32, hi as f32);
            if !(lo.is_finite() && hi.is_finite() && (hi - lo).is_finite() && hi > lo) {
                return Err(LutError(
                    "The table’s input range is too narrow or too large for a frame’s numbers."
                        .into(),
                ));
            }
        }
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
        Ok(GpuLut {
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
        })
    }

    /// Clamp input coordinates to declared domains; preserve extended table output.
    /// 1D interpolation is channel independent; 3D interpolation is tetrahedral.
    /// An infinite input is outside the domain like any other (the nearer
    /// end); one that is not a number reads as the domain's low end, as in
    /// OpenColorIO's lookups and the shader.
    pub fn apply(&self, rgb: [f64; 3]) -> Result<[f64; 3], LutError> {
        let unit = |v: f64, lo: f64, hi: f64| {
            if v.is_nan() {
                0.
            } else {
                ((v - lo) / (hi - lo)).clamp(0., 1.)
            }
        };
        let mut p = std::array::from_fn(|c| unit(rgb[c], self.domain[0][c], self.domain[1][c]));
        if !self.shaper.is_empty() {
            for (c, value) in p.iter_mut().enumerate() {
                let x = *value * (self.shaper.len() - 1) as f64;
                let lo = (x as usize).min(self.shaper.len() - 2);
                let t = x - lo as f64;
                *value = self.shaper[lo][c] * (1. - t) + self.shaper[lo + 1][c] * t;
            }
            if self.size > 0 {
                for v in &mut p {
                    *v = unit(*v, self.cube_range[0], self.cube_range[1]);
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
        let gpu = one.gpu().unwrap();
        assert_eq!((gpu.size, gpu.cube_size, gpu.shaper_size), (2, 0, 2));
        assert_eq!((gpu.width, gpu.height), (2, 1));
        assert_eq!(gpu.rgba, [0., 1., 2., 1., 3., 4., 5., 1.]);
        for bad in [
            "LUT_1D_SIZE 2\n0 0 0",
            "LUT_3D_SIZE 130",
            "LUT_1D_SIZE 65537",
            "LUT_1D_SIZE 2\n0 0 0\nNaN 0 0",
            "DOMAIN_MIN 1 1 1\nDOMAIN_MAX 1 1 1\nLUT_1D_SIZE 2\n0 0 0\n1 1 1",
        ] {
            assert!(CubeLut::parse(bad).is_err(), "{bad}");
        }
        let large = CubeLut::parse("LUT_1D_SIZE 2\n0 0 0\n65505 0 0").unwrap();
        assert!(large.gpu().is_err());
    }

    #[test]
    fn a_malformed_file_is_refused_with_a_plain_sentence() {
        for (text, sentence) in [
            ("", "The file has no LUT_1D_SIZE or LUT_3D_SIZE line."),
            (
                "LUT_3D_SIZE 2\n0 0 0\n1 1 1\n",
                "The table has 2 rows; its size needs 8.",
            ),
            (
                "LUT_3D_SIZE 2\n0 0 0\n0 0 zero\n",
                "Line 3 has a value that isn’t a number.",
            ),
            (
                "LUT_3D_SIZE 2\n0 0 0\n0 0\n",
                "Line 3 isn’t a row of three numbers or a known heading.",
            ),
            (
                "LUT_3D_SIZE 2\n0 0 0\nLUT_3D_SIZE 2\n",
                "Line 3 is a heading after the table’s rows began.",
            ),
            (
                "LUT_3D_SIZE 300\n",
                "Line 1 asks for a table of 300 points; the largest is 129.",
            ),
            (
                "LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 0 1 1\n0 0 0\n1 1 1\n",
                "The table’s input range is empty.",
            ),
            (
                "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n2 2 2\n",
                "Line 4 is a row past the end of the table.",
            ),
        ] {
            assert_eq!(CubeLut::parse(text).unwrap_err().to_string(), sentence);
        }
    }

    /// A `.3dl` whose cube is `axis` on each channel (blue fastest), after
    /// `head` lines.
    fn three_dl(head: &str, axis: &[u32]) -> String {
        let mut text = String::from(head);
        for r in axis {
            for g in axis {
                for b in axis {
                    text += &format!("{r} {g} {b}\n");
                }
            }
        }
        text
    }

    #[test]
    fn a_3dl_cube_is_read_blue_fastest_at_its_output_depth() {
        // Red is the slowest axis; each channel has its own slope.
        let mut text = String::from("# made for a test\n3DMESH\nMesh 1 12\n0 512 1023\n");
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
                    text += &format!("{} {} {}\n", r * 2047, g * 1000, b * 500);
                }
            }
        }
        text += "LUT8\ngamma 1.0\n";
        let lut = CubeLut::parse_format("3DL", &text).unwrap();
        let out = lut.apply([1., 0.5, 0.25]).unwrap();
        let expected = [4094. / 4095., 1000. / 4095., 250. / 4095.];
        for c in 0..3 {
            assert!((out[c] - expected[c]).abs() < 1e-12, "{out:?}");
        }
        // Without a Mesh line the depth comes from the table's largest value.
        let bare = text.replace("Mesh 1 12\n", "");
        assert_eq!(CubeLut::parse_3dl(&bare).unwrap(), lut);
        for (bad, sentence) in [
            (
                text.replace("Mesh 1 12", "Mesh 2 12"),
                "The table has 27 rows; its size needs 125.",
            ),
            (
                format!("{text}0 0 0\n"),
                "The table has 29 rows; its size needs 27.",
            ),
            (
                text.replace("0 512 1023\n", "0 256 512 768 1023\n0 256 512 768 1023\n"),
                "Line 5 is a second input line; a .3dl has one.",
            ),
            (
                text.replace("0 512 1023\n", "0 -5 1023\n"),
                "Line 4 has a value that isn’t a number of zero or more.",
            ),
        ] {
            assert_eq!(CubeLut::parse_3dl(&bad).unwrap_err().to_string(), sentence);
        }
    }

    /// OpenColorIO's rule (`GetLikelyLutBitDepth`): the largest value picks
    /// the depth with room for a table that overshoots its white, so a
    /// superwhite does not push the whole table down a depth.
    #[test]
    fn a_3dl_depth_follows_the_largest_value_with_room_for_overshoot() {
        for (largest, white) in [
            (128, 255.),
            (255, 255.),
            (511, 255.),
            (512, 1023.),
            (1023, 1023.),
            (1100, 1023.),
            (2047, 1023.),
            (2048, 4095.),
            (4095, 4095.),
            (4500, 4095.),
            (8191, 4095.),
            // 14-bit tables are not made: above 8191 is 16 bits.
            (8192, 65535.),
            (10000, 65535.),
            (16383, 65535.),
            (32767, 65535.),
            (32768, 65535.),
            (65535, 65535.),
            (70000, 65535.),
        ] {
            let lut = CubeLut::parse_3dl(&three_dl("", &[0, largest])).unwrap();
            let out = lut.apply([1., 0., 1.]).unwrap();
            let expected = f64::from(largest) / white;
            assert!(
                (out[0] - expected).abs() < 1e-12 && out[1] == 0.,
                "largest {largest}: {out:?}, expected {expected}"
            );
        }
        // A 10-bit table with a superwhite: its white stays 1.
        let mut text = three_dl("0 341 682 1023\n", &[0, 341, 682, 1023]);
        text = text.replacen("1023 1023 1023\n", "1100 1100 1100\n", 1);
        assert!(text.contains("1100 1100 1100") && text.contains("1023 1023 682"));
        let lut = CubeLut::parse_3dl(&text).unwrap();
        let white = lut.apply([1., 1., 1.]).unwrap();
        let below = lut.apply([1., 1., 2. / 3.]).unwrap();
        assert!((white[0] - 1100. / 1023.).abs() < 1e-12, "{white:?}");
        assert!(
            (below[0] - 1.).abs() < 1e-12 && (below[2] - 682. / 1023.).abs() < 1e-12,
            "{below:?}"
        );
        // An explicit output depth is the file's own statement.
        let declared = CubeLut::parse_3dl(&three_dl("Mesh 1 12\n", &[0, 512, 1023])).unwrap();
        assert!((declared.apply([1.; 3]).unwrap()[0] - 1023. / 4095.).abs() < 1e-12);
        // Values that are not code values are not a .3dl.
        assert_eq!(
            CubeLut::parse_3dl(&three_dl("", &[0, 1]))
                .unwrap_err()
                .to_string(),
            "The table’s largest value is 1, too low for a .3dl: its values are whole code values of 8 bits or more."
        );
    }

    /// The input line is a 1D table before the cube, at its own depth.
    #[test]
    fn a_3dl_input_line_is_applied_before_the_cube() {
        let axis = [0, 1024, 2048, 3071, 4095];
        let cube = |x: f64| {
            // The cube alone: linear between its five grid values.
            let at = x.clamp(0., 1.) * 4.;
            let lo = (at as usize).min(3);
            let t = at - lo as f64;
            (f64::from(axis[lo]) * (1. - t) + f64::from(axis[lo + 1]) * t) / 4095.
        };
        let shaped =
            CubeLut::parse_3dl(&three_dl("Mesh 2 12\n0 250 500 750 1000\n", &axis)).unwrap();
        let even = CubeLut::parse_3dl(&three_dl("Mesh 2 12\n0 256 512 767 1023\n", &axis)).unwrap();
        // 1000 is a 10-bit line: 0.5 reaches the cube at 500/1023.
        for (x, through) in [
            (0.5, 500. / 1023.),
            (1., 1000. / 1023.),
            (0.125, 125. / 1023.),
        ] {
            let out = shaped.apply([x; 3]).unwrap();
            assert!(
                (out[0] - cube(through)).abs() < 1e-12,
                "{x}: {out:?}, expected {}",
                cube(through)
            );
            let plain = even.apply([x; 3]).unwrap();
            assert!((plain[0] - cube(x)).abs() < 1e-12, "{x}: {plain:?}");
        }
        // An evenly spaced line (within two codes) changes nothing: no stage.
        let gpu = even.gpu().unwrap();
        assert_eq!((gpu.cube_size, gpu.shaper_size), (5, 0));
        let gpu = shaped.gpu().unwrap();
        assert_eq!((gpu.cube_size, gpu.shaper_size), (5, 5));
        // The line's depth is its own: a 12-bit line before a 10-bit cube,
        // and a line longer than the cube's edge.
        let mixed = CubeLut::parse_3dl(&three_dl(
            "0 300 600 900 1200 1500 2000 2500 3000 3500 4095\n",
            &[0, 512, 1023],
        ))
        .unwrap();
        let out = mixed.apply([0.25; 3]).unwrap();
        let through: f64 = 750. / 4095.;
        let expected = (through * 2. * 512.) / 1023.;
        assert!((out[0] - expected).abs() < 1e-12, "{out:?} {expected}");
        assert_eq!(
            CubeLut::parse_3dl(&three_dl("0 10 20 30 40\n", &axis))
                .unwrap_err()
                .to_string(),
            "The input line’s largest value is 40, too low for a .3dl: its values are whole code values of 8 bits or more."
        );
    }

    /// The renderer's spans are f32: a domain that is real in f64 but empty
    /// or infinite there is refused, never divided by.
    #[test]
    fn a_domain_the_renderer_cannot_hold_is_refused() {
        const SENTENCE: &str =
            "The table’s input range is too narrow or too large for a frame’s numbers.";
        for text in [
            "LUT_1D_SIZE 2\nDOMAIN_MIN 1 1 1\nDOMAIN_MAX 1.000000001 1.000000001 1.000000001\n0 0 0\n1 1 1\n",
            "LUT_1D_SIZE 2\nDOMAIN_MIN -1e100 -1e100 -1e100\nDOMAIN_MAX 1e100 1e100 1e100\n0 0 0\n1 1 1\n",
            "LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 -3e38\nDOMAIN_MAX 1 1 3e38\n0 0 0\n1 1 1\n",
            "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0 1e-50\n0 0 0\n1 1 1\n",
            "LUT_1D_SIZE 2\nLUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 16777216 16777217\n0 0 0\n1 1 1\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        ] {
            let lut = CubeLut::parse(text).expect("a real range in f64");
            assert_eq!(lut.gpu().unwrap_err().to_string(), SENTENCE, "{text}");
        }
        // The narrowest and widest a frame's numbers can tell apart are kept.
        for text in [
            "LUT_1D_SIZE 2\nDOMAIN_MIN 1 1 1\nDOMAIN_MAX 1.0000002 1.0000002 1.0000002\n0 0 0\n1 1 1\n",
            "LUT_1D_SIZE 2\nDOMAIN_MIN -1e38 -1e38 -1e38\nDOMAIN_MAX 1e38 1e38 1e38\n0 0 0\n1 1 1\n",
        ] {
            assert!(CubeLut::parse(text).unwrap().gpu().is_ok(), "{text}");
        }
    }

    #[test]
    fn a_value_that_is_not_finite_has_a_stated_lookup() {
        let lut = CubeLut::parse(
            "LUT_1D_SIZE 3\nDOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 1.5 1.5 1.5\n0.1 0.2 0.3\n0.4 0.5 0.6\n0.7 0.8 0.9\n",
        )
        .unwrap();
        // Infinite: the nearer end of the domain. Not a number: its low end.
        assert_eq!(
            lut.apply([f64::INFINITY, f64::NEG_INFINITY, f64::NAN])
                .unwrap(),
            [0.7, 0.2, 0.3]
        );
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
            let gpu = lut.gpu().unwrap();
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
        let gpu = CubeLut::parse(&text).unwrap().gpu().unwrap();
        assert_eq!((gpu.cube_size, gpu.shaper_size), (2, 3));
        assert_eq!(gpu.shaper_row, 2);
        let o = (gpu.shaper_row * gpu.width) as usize * 4;
        assert_eq!(gpu.rgba[o..o + 4], [0., 0., 0., 1.]);
    }
}
