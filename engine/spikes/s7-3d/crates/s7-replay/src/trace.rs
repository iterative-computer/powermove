//! Recorded WebGL2 traces (see `ref/recorder.ts`): a JSON call list plus a binary blob.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::Path;

pub struct Trace {
    pub calls: Vec<Value>,
    pub blob: Vec<u8>,
}

impl Trace {
    pub fn load(dir: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(dir.join("trace.json")).with_context(|| format!("{}/trace.json", dir.display()))?;
        let calls: Vec<Value> = serde_json::from_str(&text)?;
        let blob = std::fs::read(dir.join("trace.bin")).with_context(|| format!("{}/trace.bin", dir.display()))?;
        Ok(Self { calls, blob })
    }
}

/// Accessors over one call's arguments (`[name, arg0, arg1, ..., {ret}?]`).
pub struct Call<'a> {
    pub name: &'a str,
    pub args: &'a [Value],
    pub ret: Option<&'a Value>,
    blob: &'a [u8],
}

impl<'a> Call<'a> {
    pub fn new(entry: &'a Value, blob: &'a [u8]) -> Result<Self> {
        let list = entry.as_array().ok_or_else(|| anyhow!("trace entry is not an array"))?;
        let name = list.first().and_then(|v| v.as_str()).ok_or_else(|| anyhow!("trace entry without name"))?;
        let mut args = &list[1..];
        let mut ret = None;
        if let Some(last) = args.last() {
            if let Some(obj) = last.as_object() {
                if obj.contains_key("ret") && obj.len() == 1 {
                    ret = obj.get("ret");
                    args = &args[..args.len() - 1];
                }
            }
        }
        Ok(Self { name, args, ret, blob })
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn f(&self, i: usize) -> Result<f64> {
        match self.args.get(i) {
            Some(Value::Number(n)) => n.as_f64().ok_or_else(|| anyhow!("bad number")),
            Some(Value::Bool(b)) => Ok(*b as u8 as f64),
            other => bail!("{}: arg {i} is not a number: {other:?}", self.name),
        }
    }
    pub fn u(&self, i: usize) -> Result<u32> {
        Ok(self.f(i)? as u32)
    }
    pub fn i(&self, i: usize) -> Result<i32> {
        Ok(self.f(i)? as i32)
    }
    pub fn b(&self, i: usize) -> Result<bool> {
        Ok(self.f(i)? != 0.0)
    }
    pub fn s(&self, i: usize) -> Result<&'a str> {
        self.args.get(i).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("{}: arg {i} is not a string", self.name))
    }
    /// GL object id (0 for null).
    pub fn o(&self, i: usize) -> Result<u32> {
        match self.args.get(i) {
            Some(Value::Null) | None => Ok(0),
            Some(Value::Object(m)) => m.get("o").and_then(|v| v.as_u64()).map(|v| v as u32).ok_or_else(|| anyhow!("{}: arg {i} is not an object", self.name)),
            other => bail!("{}: arg {i} is not an object: {other:?}", self.name),
        }
    }
    pub fn ret_id(&self) -> u32 {
        self.ret.and_then(|v| v.as_u64()).unwrap_or(0) as u32
    }
    pub fn is_null(&self, i: usize) -> bool {
        matches!(self.args.get(i), None | Some(Value::Null))
    }
    /// Typed array payload: (bytes, element tag).
    pub fn bytes(&self, i: usize) -> Result<(&'a [u8], &'a str)> {
        match self.args.get(i) {
            Some(Value::Object(m)) if m.contains_key("b") => {
                let off = m["b"].as_u64().unwrap() as usize;
                let n = m["n"].as_u64().unwrap() as usize;
                let t = m["t"].as_str().unwrap_or("u8");
                Ok((&self.blob[off..off + n], t))
            }
            other => bail!("{}: arg {i} is not a typed array: {other:?}", self.name),
        }
    }
    /// Snapshotted image source: (width, height, RGBA8 rows in GL order).
    pub fn image(&self, i: usize) -> Option<(u32, u32, &'a [u8])> {
        match self.args.get(i) {
            Some(Value::Object(m)) if m.contains_key("img") => {
                let off = m["img"].as_u64()? as usize;
                let w = m["w"].as_u64()? as u32;
                let h = m["h"].as_u64()? as u32;
                Some((w, h, &self.blob[off..off + (w * h * 4) as usize]))
            }
            _ => None,
        }
    }
    pub fn list(&self, i: usize) -> Result<Vec<u32>> {
        self.args.get(i).and_then(|v| v.as_array()).map(|a| a.iter().map(|v| v.as_u64().unwrap_or(0) as u32).collect()).ok_or_else(|| anyhow!("{}: arg {i} is not a list", self.name))
    }
}
