//! Accessor reading (glTF 2.0 section 3.6): strides, normalized integers as
//! three.js's `BufferAttribute` getters return them, matrix column padding,
//! and sparse accessors. Ported from the S7 prototype (`s7-geom3d`).

use super::{Error, Result, err};
use serde_json::Value;

pub(crate) struct Data<'a> {
    pub json: &'a Value,
    pub buffers: Vec<&'a [u8]>,
}

/// An accessor's component layout.
pub(crate) struct Layout {
    pub count: usize,
    pub comps: usize,
    pub component_type: u32,
    pub normalized: bool,
}

pub(crate) fn components(t: &str) -> Result<usize> {
    Ok(match t {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT2" => 4,
        "MAT3" => 9,
        "MAT4" => 16,
        t => return err(format!("accessor type {t}")),
    })
}

impl<'a> Data<'a> {
    pub fn accessor(&self, index: usize) -> Result<&'a Value> {
        self.json["accessors"]
            .get(index)
            .ok_or_else(|| Error(format!("accessor {index} missing")))
    }

    pub fn layout(&self, index: usize) -> Result<Layout> {
        let a = self.accessor(index)?;
        Ok(Layout {
            count: a["count"]
                .as_u64()
                .ok_or_else(|| Error("accessor count".into()))? as usize,
            comps: components(a["type"].as_str().unwrap_or(""))?,
            component_type: a["componentType"].as_u64().unwrap_or(0) as u32,
            normalized: a["normalized"].as_bool().unwrap_or(false),
        })
    }

    /// Reads an accessor as f64 components: normalized integers mapped as
    /// three.js's getters map them (`denormalize`), others as stored.
    pub fn read(&self, index: usize) -> Result<(Vec<f64>, usize)> {
        let a = self.accessor(index)?;
        let Layout {
            count,
            comps,
            component_type: ctype,
            normalized,
        } = self.layout(index)?;
        let csize = match ctype {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            c => return err(format!("component type {c}")),
        };
        let decode = |b: &[u8]| -> f64 {
            match ctype {
                5120 => {
                    let v = b[0] as i8 as f64;
                    if normalized { (v / 127.0).max(-1.0) } else { v }
                }
                5121 => {
                    let v = b[0] as f64;
                    if normalized { v / 255.0 } else { v }
                }
                5122 => {
                    let v = i16::from_le_bytes([b[0], b[1]]) as f64;
                    if normalized {
                        (v / 32767.0).max(-1.0)
                    } else {
                        v
                    }
                }
                5123 => {
                    let v = u16::from_le_bytes([b[0], b[1]]) as f64;
                    if normalized { v / 65535.0 } else { v }
                }
                5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            }
        };
        let mut out = vec![0.0; count * comps];
        let kind = a["type"].as_str().unwrap_or("");
        if let Some(view_index) = a["bufferView"].as_u64() {
            let view = &self.json["bufferViews"][view_index as usize];
            let buffer = self
                .buffers
                .get(view["buffer"].as_u64().unwrap_or(0) as usize)
                .ok_or_else(|| Error("buffer missing".into()))?;
            let view_start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let view_end = view_start + view["byteLength"].as_u64().unwrap_or(0) as usize;
            let start = view_start + a["byteOffset"].as_u64().unwrap_or(0) as usize;
            // Matrix columns of 1- and 2-byte components are 4-byte aligned.
            let column_pad = |rows: usize| -> usize { (rows * csize).div_ceil(4) * 4 };
            let element = match (comps, csize) {
                (4, 1) if kind == "MAT2" => 2 * column_pad(2),
                (9, 1 | 2) => 3 * column_pad(3),
                (16, 1 | 2) => 4 * column_pad(4),
                _ => comps * csize,
            };
            let stride = view["byteStride"]
                .as_u64()
                .map(|s| s as usize)
                .unwrap_or(element);
            let needed = if count == 0 {
                0
            } else {
                start + stride * (count - 1) + element
            };
            if needed > buffer.len() || needed > view_end {
                return err(format!("accessor {index} reads past its buffer view"));
            }
            for i in 0..count {
                let base = start + i * stride;
                for c in 0..comps {
                    let offset = match comps {
                        9 if csize < 4 => (c / 3) * column_pad(3) + (c % 3) * csize,
                        16 if csize < 4 => (c / 4) * column_pad(4) + (c % 4) * csize,
                        4 if csize < 4 && kind == "MAT2" => {
                            (c / 2) * column_pad(2) + (c % 2) * csize
                        }
                        _ => c * csize,
                    };
                    out[i * comps + c] = decode(&buffer[base + offset..]);
                }
            }
        }
        if let Some(sparse) = a.get("sparse") {
            let n = sparse["count"].as_u64().unwrap_or(0) as usize;
            let idx = &sparse["indices"];
            let val = &sparse["values"];
            let read_view = |v: &Value| -> Result<&'a [u8]> {
                let view = &self.json["bufferViews"][v["bufferView"]
                    .as_u64()
                    .ok_or_else(|| Error("sparse view".into()))?
                    as usize];
                let buffer = self
                    .buffers
                    .get(view["buffer"].as_u64().unwrap_or(0) as usize)
                    .ok_or_else(|| Error("buffer missing".into()))?;
                let start = view["byteOffset"].as_u64().unwrap_or(0) as usize
                    + v["byteOffset"].as_u64().unwrap_or(0) as usize;
                buffer
                    .get(start..)
                    .ok_or_else(|| Error("sparse range".into()))
            };
            let ib = read_view(idx)?;
            let vb = read_view(val)?;
            let isize = match idx["componentType"].as_u64().unwrap_or(0) {
                5121 => 1,
                5123 => 2,
                5125 => 4,
                c => return err(format!("sparse index type {c}")),
            };
            if ib.len() < n * isize || vb.len() < n * comps * csize {
                return err("sparse data past its buffer view");
            }
            for k in 0..n {
                let target = match isize {
                    1 => ib[k] as usize,
                    2 => u16::from_le_bytes([ib[2 * k], ib[2 * k + 1]]) as usize,
                    _ => {
                        u32::from_le_bytes([ib[4 * k], ib[4 * k + 1], ib[4 * k + 2], ib[4 * k + 3]])
                            as usize
                    }
                };
                if target >= count {
                    return err("sparse index out of range");
                }
                for c in 0..comps {
                    out[target * comps + c] = decode(&vb[(k * comps + c) * csize..]);
                }
            }
        }
        Ok((out, comps))
    }

    /// An accessor as f32 tuples of `N` components (as three.js stores
    /// attributes in Float32Arrays, or reads normalized integers through its
    /// getters).
    pub fn read_n<const N: usize>(&self, index: usize) -> Result<Vec<[f32; N]>> {
        let (v, comps) = self.read(index)?;
        if comps != N {
            return err(format!("accessor {index} has {comps} components, not {N}"));
        }
        Ok(v.chunks_exact(N)
            .map(|c| std::array::from_fn(|i| c[i] as f32))
            .collect())
    }
}
