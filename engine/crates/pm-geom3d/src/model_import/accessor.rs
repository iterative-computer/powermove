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

    fn checked_layout(&self, index: usize) -> Result<Layout> {
        let layout = self.layout(index)?;
        match layout.component_type {
            5120 | 5121 | 5122 | 5123 | 5125 | 5126 => Ok(layout),
            c => err(format!("component type {c}")),
        }
    }

    /// Reads an accessor as f64 components: normalized integers mapped as
    /// three.js's getters map them (`denormalize`), others as stored.
    pub fn read(&self, index: usize) -> Result<(Vec<f64>, usize)> {
        let layout = self.checked_layout(index)?;
        let mut out = vec![0.0; layout.count * layout.comps];
        let comps = self.visit(index, |row, component, value| {
            out[row * layout.comps + component] = value;
        })?;
        Ok((out, comps))
    }

    /// Decode components into the caller's storage. Keeping the f64 getter
    /// result scalar preserves normalized rounding without a full f64 array.
    fn visit(&self, index: usize, mut set: impl FnMut(usize, usize, f64)) -> Result<usize> {
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
                    set(i, c, decode(&buffer[base + offset..]));
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
                // `setX` past the end of the typed array writes nothing.
                if target >= count {
                    continue;
                }
                for c in 0..comps {
                    set(target, c, decode(&vb[(k * comps + c) * csize..]));
                }
            }
        }
        Ok(comps)
    }

    /// An accessor as f32 tuples of `N` components (as three.js stores
    /// attributes in Float32Arrays, or reads normalized integers through its
    /// getters).
    pub fn read_n<const N: usize>(&self, index: usize) -> Result<Vec<[f32; N]>> {
        let layout = self.checked_layout(index)?;
        if layout.comps != N {
            // Preserve the old path's error ordering for malformed accessors:
            // it validated the buffer before reporting a tuple-size mismatch.
            let (_, comps) = self.read(index)?;
            return err(format!("accessor {index} has {comps} components, not {N}"));
        }
        let mut out = vec![[0.0; N]; layout.count];
        self.visit(index, |row, component, value| {
            out[row][component] = value as f32;
        })?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_tuples_match_f64_getters_for_sparse_normalized_values() {
        let bytes = [0u8, 127, 128, 255, 0, 1, 1, 127, 0, 128];
        let json = json!({
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":6},
                {"buffer":0,"byteOffset":6,"byteLength":1},
                {"buffer":0,"byteOffset":7,"byteLength":3}
            ],
            "accessors":[{"bufferView":0,"componentType":5120,"normalized":true,
                "count":2,"type":"VEC3","sparse":{"count":1,
                    "indices":{"bufferView":1,"componentType":5121},
                    "values":{"bufferView":2}}}]
        });
        let data = Data {
            json: &json,
            buffers: vec![&bytes],
        };
        let (old, _) = data.read(0).unwrap();
        let direct = data.read_n::<3>(0).unwrap();
        let old_bits: Vec<_> = old.iter().map(|v| (*v as f32).to_bits()).collect();
        let direct_bits: Vec<_> = direct.iter().flatten().map(|v| v.to_bits()).collect();
        assert_eq!(direct_bits, old_bits);
        assert_eq!(direct[0][0], 0.0);
        assert_eq!(direct[0][1], 1.0);
        assert_eq!(direct[0][2], -1.0);
        assert_eq!(direct[1], [1.0, 0.0, -1.0]);
    }

    #[test]
    fn direct_matrix_tuples_honor_column_padding() {
        let bytes = [1u8, 2, 3, 99, 4, 5, 6, 99, 7, 8, 9, 99];
        let json = json!({
            "bufferViews":[{"buffer":0,"byteLength":12}],
            "accessors":[{"bufferView":0,"componentType":5121,"count":1,"type":"MAT3"}]
        });
        let data = Data {
            json: &json,
            buffers: vec![&bytes],
        };
        assert_eq!(
            data.read_n::<9>(0).unwrap(),
            [[1., 2., 3., 4., 5., 6., 7., 8., 9.]]
        );
    }

    #[test]
    fn wide_integer_and_malformed_tuple_keep_old_semantics() {
        let bytes = u32::MAX.to_le_bytes();
        let json = json!({
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5125,"count":1,"type":"SCALAR"}]
        });
        let data = Data {
            json: &json,
            buffers: vec![&bytes],
        };
        assert_eq!(data.read(0).unwrap().0, [u32::MAX as f64]);
        assert_eq!(data.read_n::<1>(0).unwrap(), [[u32::MAX as f32]]);
        assert_eq!(
            data.read_n::<3>(0).unwrap_err().0,
            "accessor 0 has 1 components, not 3"
        );
    }

    #[test]
    fn direct_float_tuples_keep_signed_zero_and_nan_round_trip() {
        let values = [f32::from_bits(0x7f80_0001), -0.0, 1.25];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let json = json!({
            "bufferViews":[{"buffer":0,"byteLength":12}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}]
        });
        let data = Data {
            json: &json,
            buffers: vec![&bytes],
        };
        let (old, _) = data.read(0).unwrap();
        let direct = data.read_n::<3>(0).unwrap();
        for i in 0..3 {
            assert_eq!(direct[0][i].to_bits(), (old[i] as f32).to_bits());
        }
        assert_eq!(direct[0][1].to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn unsupported_component_type_precedes_large_output_allocation() {
        // A VEC3 output at this count would overflow its length before visit
        // could reject the component type.
        let json = json!({"accessors":[{
            "componentType":9999,"count":(usize::MAX / 2) as u64,"type":"VEC3"
        }]});
        let data = Data {
            json: &json,
            buffers: vec![],
        };
        assert_eq!(data.read(0).unwrap_err().0, "component type 9999");

        // The direct tuple path must validate before allocating as well.
        let json = json!({"accessors":[{
            "componentType":9999,"count":0,"type":"VEC3"
        }]});
        let data = Data {
            json: &json,
            buffers: vec![],
        };
        assert_eq!(data.read_n::<3>(0).unwrap_err().0, "component type 9999");
    }
}
