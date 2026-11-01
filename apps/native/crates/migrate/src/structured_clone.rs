//! The data-only subset Chromium writes for Powermove media records. No V8 or
//! JavaScript executes. Unknown tags/versions, cycles, bad counts and out-of-range
//! references fail closed; Blob/File indices remain references, never byte arrays.
use crate::{
    Result,
    wire::{Cursor, MAX_VALUE, utf16},
};
use anyhow::{bail, ensure};
use serde_json::{Map, Value, json};

pub fn decode(bytes: &[u8]) -> Result<Value> {
    ensure!(
        bytes.len() <= MAX_VALUE,
        "serialized metadata exceeds bound"
    );
    let mut c = Cursor(bytes);
    ensure!(c.byte()? == 255, "missing Blink version");
    let blink = c.var()?;
    ensure!(
        (16..=21).contains(&blink),
        "unsupported Blink serialization version"
    );
    let v8_bytes = if blink >= 21 {
        ensure!(c.byte()? == 254, "missing Blink trailer header");
        let offset = u64::from_be_bytes(c.take(8)?.try_into()?) as usize;
        let size = u32::from_be_bytes(c.take(4)?.try_into()?) as usize;
        if offset != 0 {
            let header = bytes.len() - c.0.len();
            ensure!(
                offset >= header && offset.checked_add(size) == Some(bytes.len()),
                "invalid Blink trailer range"
            );
            // Trailer lists interfaces required to deserialize the host objects.
            // All host objects are independently checked by the decoder below.
            let mut t = Cursor(&bytes[offset..]);
            ensure!(t.byte()? == 0xa0, "unsupported Blink trailer");
            let n = u32::from_be_bytes(t.take(4)?.try_into()?) as usize;
            t.take(n)?;
            t.finish()?;
            &bytes[header..offset]
        } else {
            ensure!(size == 0, "invalid empty Blink trailer");
            c.0
        }
    } else {
        c.0
    };
    let mut parser = Parser {
        c: Cursor(v8_bytes),
        objects: Vec::new(),
        nodes: 0,
        expanded: 0,
    };
    ensure!(parser.c.byte()? == 255, "missing V8 header");
    ensure!(
        (13..=16).contains(&parser.c.var()?),
        "unsupported V8 serialization version"
    );
    let v = parser.value(0)?;
    while parser.c.0.first() == Some(&0) {
        parser.c.byte()?;
    }
    parser.c.finish()?;
    Ok(v)
}
struct Parser<'a> {
    c: Cursor<'a>,
    objects: Vec<Option<Value>>,
    nodes: usize,
    expanded: usize,
}
impl Parser<'_> {
    fn tag(&mut self) -> Result<u8> {
        loop {
            let tag = self.c.byte()?;
            if tag != 0 {
                return Ok(tag);
            }
        }
    }
    fn object(&mut self) -> usize {
        let id = self.objects.len();
        self.objects.push(None);
        id
    }
    fn done(&mut self, id: usize, value: Value) -> Result<Value> {
        self.charge(&value)?;
        self.objects[id] = Some(value.clone());
        Ok(value)
    }
    fn charge(&mut self, v: &Value) -> Result<()> {
        // Also bounds expanded references (otherwise a tiny stream could expand
        // an exponentially shared object graph into gigabytes of JSON).
        fn cost(v: &Value) -> usize {
            match v {
                Value::String(s) => s.len() + 32,
                Value::Array(a) => 32 + a.iter().map(cost).sum::<usize>(),
                Value::Object(o) => 32 + o.iter().map(|(k, v)| k.len() + cost(v)).sum::<usize>(),
                _ => 32,
            }
        }
        self.expanded = self.expanded.saturating_add(cost(v));
        ensure!(
            self.expanded <= MAX_VALUE,
            "expanded structured clone exceeds bound"
        );
        Ok(())
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.nodes += 1;
        ensure!(
            depth <= 64 && self.nodes <= 1_000_000,
            "structured clone complexity exceeds bound"
        );
        let tag = self.tag()?;
        match tag {
            b'?' => {
                self.c.var()?;
                self.value(depth + 1)
            }
            b'0' => Ok(Value::Null),
            b'_' => Ok(json!({"$undefined":true})),
            b'-' => Ok(json!({"$hole":true})),
            b'T' => Ok(Value::Bool(true)),
            b'F' => Ok(Value::Bool(false)),
            b'I' => {
                let n = u32::try_from(self.c.var()?)?;
                Ok(json!(((n >> 1) as i32) ^ -((n & 1) as i32)))
            }
            b'U' => Ok(json!(u32::try_from(self.c.var()?)?)),
            b'N' => {
                let n = f64::from_le_bytes(self.c.take(8)?.try_into()?);
                ensure!(n.is_finite(), "non-finite media metadata");
                Ok(json!(n))
            }
            b'"' => Ok(Value::String(
                self.c.bytes()?.iter().map(|b| char::from(*b)).collect(),
            )),
            b'S' => Ok(Value::String(String::from_utf8(self.c.bytes()?.to_vec())?)),
            b'c' => Ok(Value::String(utf16(self.c.bytes()?, false)?)),
            b'^' => {
                let id = usize::try_from(self.c.var()?)?;
                let v = self
                    .objects
                    .get(id)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| anyhow::anyhow!("invalid or cyclic V8 object reference"))?
                    .clone();
                self.charge(&v)?;
                Ok(v)
            }
            b'o' => {
                let id = self.object();
                let mut o = Map::new();
                let mut count = 0;
                loop {
                    if self.c.0.first() == Some(&0) {
                        self.c.byte()?;
                        continue;
                    }
                    if self.c.0.first() == Some(&b'{') {
                        self.c.byte()?;
                        break;
                    }
                    let key = self.value(depth + 1)?;
                    let key = key
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("non-string media object key"))?
                        .to_owned();
                    let v = self.value(depth + 1)?;
                    ensure!(o.insert(key, v).is_none(), "duplicate object property");
                    count += 1;
                }
                ensure!(self.c.var()? == count, "object property count mismatch");
                self.done(id, Value::Object(o))
            }
            b'A' => {
                let id = self.object();
                let n = self.c.len()?;
                ensure!(n <= 1_000_000, "array too long");
                let mut a = Vec::with_capacity(n);
                for _ in 0..n {
                    a.push(self.value(depth + 1)?);
                }
                ensure!(
                    self.tag()? == b'$' && self.c.var()? == 0 && self.c.var()? == n as u64,
                    "unsupported dense-array properties or invalid count"
                );
                self.done(id, Value::Array(a))
            }
            b'\\' => {
                let id = self.object();
                let kind = self.c.byte()?;
                ensure!(
                    kind == b'i' || kind == b'e',
                    "unsupported Blink host object (expected Blob/File index)"
                );
                let index = u32::try_from(self.c.var()?)?;
                self.done(id, json!({"$blob":index,"$file":kind==b'e'}))
            }
            _ => bail!("unsupported V8 value tag"),
        }
    }
}
