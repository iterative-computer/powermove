//! Main's bounded standalone .cube GPU layout (red-fastest, RGBA32F).
use crate::catalog::CatalogError;
#[derive(Debug, Clone, PartialEq)]
pub struct Cube {
    pub size: u32,
    pub width: u32,
    pub height: u32,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub rgba: Vec<f32>,
}
pub fn parse(text: &str) -> Result<Cube, CatalogError> {
    let error = |s: &str| CatalogError(s.into());
    if text.len() > 16 * 1024 * 1024 {
        return Err(error("LUT exceeds the 16 MiB text limit"));
    }
    let mut size = 0u32;
    let mut dimensions = 3;
    let mut domain_min = [0.; 3];
    let mut domain_max = [1.; 3];
    let mut rgba = vec![];
    let mut seen = std::collections::BTreeSet::new();
    let numbers = |s: &str, n: usize| -> Result<Vec<f64>, CatalogError> {
        let values = s
            .split_whitespace()
            .map(|word| {
                word.parse::<f64>()
                    .map_err(|_| error("Invalid LUT numeric row"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != n || values.iter().any(|v| !v.is_finite() || v.abs() > 65504.) {
            return Err(error("Invalid LUT numeric row"));
        }
        Ok(values)
    };
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let mut quote = false;
        let end = raw
            .char_indices()
            .find_map(|(i, c)| {
                if c == '"' {
                    quote = !quote;
                }
                (c == '#' && !quote).then_some(i)
            })
            .unwrap_or(raw.len());
        let line = raw[..end].trim();
        if line.is_empty() {
            continue;
        }
        let first = line.split_whitespace().next().unwrap();
        if first.bytes().next().is_some_and(|c| c.is_ascii_uppercase()) {
            if !seen.insert(first.to_owned()) {
                return Err(error("Duplicate LUT directive"));
            }
            if !rgba.is_empty() {
                return Err(error("LUT headers must precede data"));
            }
            let value = line[first.len()..].trim();
            match first {
                "TITLE" => {
                    if value.len() < 2
                        || !value.starts_with('"')
                        || !value.ends_with('"')
                        || value[1..value.len() - 1].contains('"')
                    {
                        return Err(error("Invalid LUT title"));
                    }
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    let v = numbers(value, 3)?;
                    let v = std::array::from_fn(|i| v[i] as f32);
                    if first == "DOMAIN_MIN" {
                        domain_min = v;
                    } else {
                        domain_max = v;
                    }
                }
                "LUT_1D_SIZE" | "LUT_3D_SIZE" => {
                    if size != 0 {
                        return Err(error("Combined 1D and 3D LUTs are not supported"));
                    }
                    dimensions = if first == "LUT_1D_SIZE" { 1 } else { 3 };
                    let n = numbers(value, 1)?[0];
                    if n.fract() != 0. || n < 2. || n > if dimensions == 1 { 4096. } else { 64. } {
                        return Err(error("LUT size outside supported bounds"));
                    }
                    size = n as u32;
                }
                _ => return Err(error("Unsupported LUT directive")),
            }
        } else {
            if size == 0 {
                return Err(error("LUT size must precede data"));
            }
            if rgba.len() / 4
                >= if dimensions == 1 {
                    size as usize
                } else {
                    size.pow(3) as usize
                }
            {
                return Err(error("Too many LUT rows"));
            }
            rgba.extend(numbers(line, 3)?.iter().map(|v| *v as f32));
            rgba.push(1.);
        }
    }
    if size == 0
        || rgba.len() / 4
            != if dimensions == 1 {
                size as usize
            } else {
                size.pow(3) as usize
            }
    {
        return Err(error("LUT row count does not match its declared size"));
    }
    if (0..3).any(|i| domain_max[i] - domain_min[i] < 1e-30) {
        return Err(error("LUT domain must increase at GPU precision"));
    }
    Ok(Cube {
        size,
        width: if dimensions == 1 { size } else { size * size },
        height: if dimensions == 1 { 1 } else { size },
        domain_min,
        domain_max,
        rgba,
    })
}
