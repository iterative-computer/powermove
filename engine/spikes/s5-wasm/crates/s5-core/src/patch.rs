//! Leaf patches (`apps/desktop/src/shared/patch.ts`) and the binary framing
//! the JS fast path uses to hand them to the mirror.
//!
//! Framing (little-endian):
//!
//! ```text
//! u32 count
//! per patch:
//!   u8  kind            0 = delete, 1 = set, 2 = layer order (id-keyed structural)
//!   u32 len; bytes      path as a JSON array
//!   kind 1: u32 len; bytes   value as JSON
//!   kind 2: u32 len; bytes   ids as a JSON array of strings
//!           u32 n; n x (u32 len; bytes)   changed or new layers as JSON
//! ```
//!
//! Framing lets a structural patch carry its 1 MB value without the mirror
//! scanning it once to find its end and again to reconcile it.

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PathPart {
    Index(u64),
    Key(String),
}

impl PathPart {
    pub fn as_key(&self) -> Option<&str> {
        match self {
            PathPart::Key(k) => Some(k),
            PathPart::Index(_) => None,
        }
    }
    pub fn as_index(&self) -> Option<usize> {
        match self {
            PathPart::Index(i) => usize::try_from(*i).ok(),
            PathPart::Key(_) => None,
        }
    }
    /// JS property keys are strings; a numeric part on an object is its decimal form.
    fn object_key(&self) -> std::borrow::Cow<'_, str> {
        match self {
            PathPart::Key(k) => std::borrow::Cow::Borrowed(k),
            PathPart::Index(i) => std::borrow::Cow::Owned(i.to_string()),
        }
    }
}

#[derive(Debug)]
pub enum FramedPatch<'a> {
    Delete {
        path: Vec<PathPart>,
    },
    Set {
        path: Vec<PathPart>,
        value: &'a [u8],
    },
    LayerOrder {
        path: Vec<PathPart>,
        ids: Vec<String>,
        layers: Vec<&'a [u8]>,
    },
}

#[derive(Debug)]
pub struct PatchError(pub String);

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PatchError {}

pub fn perr<T>(message: impl Into<String>) -> Result<T, PatchError> {
    Err(PatchError(message.into()))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, PatchError> {
        let b = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| PatchError("truncated frame".into()))?;
        self.at += 1;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, PatchError> {
        let s = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| PatchError("truncated frame".into()))?;
        self.at += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn chunk(&mut self) -> Result<&'a [u8], PatchError> {
        let len = self.u32()? as usize;
        let s = self
            .bytes
            .get(self.at..self.at + len)
            .ok_or_else(|| PatchError("truncated frame".into()))?;
        self.at += len;
        Ok(s)
    }
}

pub fn decode_frames(bytes: &[u8]) -> Result<Vec<FramedPatch<'_>>, PatchError> {
    let mut r = Reader { bytes, at: 0 };
    let count = r.u32()? as usize;
    let mut out = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let kind = r.u8()?;
        let path: Vec<PathPart> =
            serde_json::from_slice(r.chunk()?).map_err(|e| PatchError(format!("bad path: {e}")))?;
        out.push(match kind {
            0 => FramedPatch::Delete { path },
            1 => FramedPatch::Set {
                path,
                value: r.chunk()?,
            },
            2 => {
                let ids: Vec<String> = serde_json::from_slice(r.chunk()?)
                    .map_err(|e| PatchError(format!("bad ids: {e}")))?;
                let n = r.u32()? as usize;
                let mut layers = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    layers.push(r.chunk()?);
                }
                FramedPatch::LayerOrder { path, ids, layers }
            }
            other => return perr(format!("unknown patch kind {other}")),
        });
    }
    if r.at != bytes.len() {
        return perr("trailing bytes after frames");
    }
    Ok(out)
}

/// Encode patches the way the JS writer does (tests and native benchmarks).
pub fn encode_frames(patches: &[(u8, String, Option<String>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(patches.len() as u32).to_le_bytes());
    for (kind, path, value) in patches {
        out.push(*kind);
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        if let Some(value) = value {
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value.as_bytes());
        }
    }
    out
}

pub enum Op<'a> {
    Delete,
    Set(&'a Value),
}

/// `applyPatch` for one patch below `root`, with JS semantics: a missing
/// parent skips the patch; deleting an array index splices.
pub fn apply_value_patch(
    root: &mut Value,
    path: &[PathPart],
    op: Op<'_>,
) -> Result<(), PatchError> {
    let Some((last, parents)) = path.split_last() else {
        return perr("empty path below root");
    };
    let mut parent = root;
    for part in parents {
        let next = match (parent, part) {
            (Value::Object(map), part) => map.get_mut(&*part.object_key()),
            (Value::Array(items), PathPart::Index(i)) => items.get_mut(*i as usize),
            _ => None,
        };
        match next {
            Some(v) if !v.is_null() => parent = v,
            _ => return Ok(()),
        }
    }
    match (parent, op) {
        (Value::Object(map), Op::Set(value)) => {
            let key = last.object_key();
            match map.get_mut(&*key) {
                Some(slot) => *slot = value.clone(),
                None => {
                    map.insert(key.into_owned(), value.clone());
                }
            }
        }
        (Value::Object(map), Op::Delete) => {
            map.shift_remove(&*last.object_key());
        }
        (Value::Array(items), Op::Set(value)) => match last {
            PathPart::Index(i) => {
                let i = *i as usize;
                if i < items.len() {
                    items[i] = value.clone();
                } else {
                    // JS leaves holes, which serialize as null.
                    items.resize(i, Value::Null);
                    items.push(value.clone());
                }
            }
            PathPart::Key(_) => {} // a named property on an array never serializes
        },
        (Value::Array(items), Op::Delete) => {
            if let PathPart::Index(i) = last
                && (*i as usize) < items.len()
            {
                items.remove(*i as usize);
            }
        }
        (_, Op::Set(_)) => return perr("cannot set a property on a primitive"),
        (_, Op::Delete) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(parts: Value) -> Vec<PathPart> {
        serde_json::from_value(parts).unwrap()
    }

    #[test]
    fn js_semantics() {
        let mut doc = json!({"a": {"b": [1, 2, 3]}, "c": null});
        apply_value_patch(&mut doc, &p(json!(["a", "b", 1])), Op::Delete).unwrap();
        assert_eq!(doc["a"]["b"], json!([1, 3]));
        apply_value_patch(&mut doc, &p(json!(["a", "b", 4])), Op::Set(&json!(9))).unwrap();
        assert_eq!(doc["a"]["b"], json!([1, 3, null, null, 9]));
        apply_value_patch(&mut doc, &p(json!(["c", "x"])), Op::Set(&json!(1))).unwrap();
        assert_eq!(doc["c"], Value::Null);
        apply_value_patch(&mut doc, &p(json!(["missing", "x"])), Op::Set(&json!(1))).unwrap();
        assert!(doc.get("missing").is_none());
        apply_value_patch(&mut doc, &p(json!(["a", 0])), Op::Set(&json!(1))).unwrap();
        assert_eq!(doc["a"]["0"], json!(1));
    }

    #[test]
    fn delete_keeps_key_order() {
        let mut doc = json!({"x": 1, "y": 2, "z": 3});
        apply_value_patch(&mut doc, &p(json!(["x"])), Op::Delete).unwrap();
        assert_eq!(serde_json::to_string(&doc).unwrap(), r#"{"y":2,"z":3}"#);
    }

    #[test]
    fn frames_round_trip() {
        let bytes = encode_frames(&[
            (
                1,
                r#"["layers",3,"p","opacity","v"]"#.into(),
                Some("50".into()),
            ),
            (0, r#"["revision"]"#.into(), None),
        ]);
        let frames = decode_frames(&bytes).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(matches!(&frames[0], FramedPatch::Set { value, .. } if *value == b"50"));
        assert!(decode_frames(&bytes[..bytes.len() - 1]).is_err());
    }
}
