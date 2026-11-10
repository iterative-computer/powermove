//! Figma paste (`legacy/core/figma`): the clipboard's `fig-kiwi` archive read
//! through its own embedded schema, then planned into the layers today's
//! converter creates. Nothing touches the document here; the caller applies
//! a clean plan as one Undo step, or offers the image fallback for its issues.
//!
//! Clipboard HTML is never interpreted: only the base64 between `(figma)`
//! markers is read, and the embedded schema is data (bounded interpreter, no
//! code). Decompression and decoding are bounded the way today's are.
use base64::Engine;
use pm_kernel::commands::layers::{AddLayer, BlockLayer};
use pm_model::{Map, Value};
use std::{collections::HashMap, io::Read};

pub const MAX_CLIPBOARD_CHARS: usize = 32 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SCHEMA_BYTES: usize = 4 * 1024 * 1024;
const TOO_LARGE: &str = "This Figma selection is too large. Copy fewer layers and try again.";
const INCOMPLETE: &str = "The copied Figma data is incomplete.";

/// A decoded kiwi value.
#[derive(Clone, Debug, PartialEq)]
pub enum K {
    Bool(bool),
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<K>),
    Obj(HashMap<String, K>),
}
impl K {
    pub fn get(&self, key: &str) -> Option<&K> {
        match self {
            K::Obj(m) => m.get(key),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            K::Num(n) => Some(*n),
            _ => None,
        }
    }
    fn text(&self) -> Option<&str> {
        match self {
            K::Str(s) => Some(s),
            _ => None,
        }
    }
    fn list(&self) -> &[K] {
        match self {
            K::List(l) => l,
            _ => &[],
        }
    }
    fn truthy(&self) -> bool {
        match self {
            K::Bool(b) => *b,
            K::Num(n) => *n != 0. && !n.is_nan(),
            K::Str(s) => !s.is_empty(),
            _ => true,
        }
    }
}
fn num(node: &K, key: &str) -> Option<f64> {
    node.get(key).and_then(K::num)
}
fn text<'a>(node: &'a K, key: &str) -> Option<&'a str> {
    node.get(key).and_then(K::text)
}

/// `FigmaSelection`: the node changes and their binary blobs.
#[derive(Debug, Default)]
pub struct Selection {
    pub nodes: Vec<K>,
    pub blobs: Vec<Vec<u8>>,
}

/// `decodeFigmaClipboard`.
pub fn decode(html: &str) -> Result<Selection, String> {
    if html.len() > MAX_CLIPBOARD_CHARS {
        return Err(TOO_LARGE.into());
    }
    let unreadable =
        || "Could not read the copied Figma layers. Copy them again and retry.".to_string();
    let start = html.find("(figma)").ok_or_else(unreadable)? + "(figma)".len();
    let end = html[start..].find("(/figma)").ok_or_else(unreadable)? + start;
    let encoded: String = html[start..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if encoded.is_empty() {
        return Err(unreadable());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| unreadable())?;
    if bytes.len() < 20 || &bytes[..8] != b"fig-kiwi" {
        return Err("This Figma clipboard format is not supported yet.".into());
    }
    let mut chunks: Vec<&[u8]> = vec![];
    let mut at = 12;
    while at < bytes.len() {
        if at + 4 > bytes.len() || chunks.len() >= 8 {
            return Err(INCOMPLETE.into());
        }
        let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        if size == 0 || at + size > bytes.len() {
            return Err(INCOMPLETE.into());
        }
        chunks.push(&bytes[at..at + size]);
        at += size;
    }
    if chunks.len() < 2 {
        return Err(INCOMPLETE.into());
    }
    let schema = Schema::decode(&expand(chunks[0], MAX_SCHEMA_BYTES)?)?;
    let message = schema.read_message(&expand(chunks[1], MAX_EXPANDED_BYTES)?)?;
    let nodes = match message.get("nodeChanges") {
        Some(K::List(nodes)) if !nodes.is_empty() => nodes.clone(),
        _ => return Err("No layers were found in the Figma selection.".into()),
    };
    if nodes.len() > 5000 {
        return Err("Copy up to 5,000 Figma layers at a time.".into());
    }
    let blobs = message
        .get("blobs")
        .map(K::list)
        .unwrap_or_default()
        .iter()
        .map(|b| match b.get("bytes") {
            Some(K::Bytes(bytes)) => bytes.clone(),
            _ => vec![],
        })
        .collect();
    Ok(Selection { nodes, blobs })
}

/// Raw deflate or a single zstd frame, at most `limit` bytes out.
fn expand(bytes: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    let read = |reader: &mut dyn Read, out: &mut Vec<u8>| -> Result<(), String> {
        let n = reader
            .take(limit as u64 + 1)
            .read_to_end(out)
            .map_err(|_| "Incomplete Figma compressed data.".to_string())?;
        if n > limit {
            return Err(TOO_LARGE.into());
        }
        Ok(())
    };
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        check_zstd_frame(bytes, limit)?;
        let mut decoder = zstd::stream::read::Decoder::with_buffer(bytes)
            .map_err(|_| "Incomplete Figma compressed data.".to_string())?
            .single_frame();
        read(&mut decoder, &mut out)?;
    } else {
        read(&mut flate2::read::DeflateDecoder::new(bytes), &mut out)?;
    }
    Ok(out)
}

/// `checkZstdFrame`: bound the history window before the decoder allocates
/// it, and refuse a second frame.
fn check_zstd_frame(bytes: &[u8], limit: usize) -> Result<(), String> {
    let incomplete = || "Incomplete Figma compressed data.".to_string();
    if bytes.len() < 6 {
        return Err(incomplete());
    }
    let descriptor = bytes[4];
    let single = descriptor & 32 != 0;
    let size_flag = descriptor >> 6;
    let mut at = if single { 5 } else { 6 };
    let window_byte = bytes[5];
    let window = if single {
        0.
    } else {
        2f64.powi(10 + i32::from(window_byte >> 3)) * (1. + f64::from(window_byte & 7) / 8.)
    };
    let dictionary = descriptor & 3;
    at += if dictionary == 3 {
        4
    } else {
        dictionary as usize
    };
    let size_bytes = if size_flag != 0 {
        1usize << size_flag
    } else if single {
        1
    } else {
        0
    };
    if at + size_bytes > bytes.len() {
        return Err(incomplete());
    }
    let mut content = 0f64;
    for i in 0..size_bytes {
        content += f64::from(bytes[at + i]) * 2f64.powi(8 * i as i32);
    }
    if size_flag == 1 {
        content += 256.;
    }
    if window > limit as f64 || content > limit as f64 {
        return Err(TOO_LARGE.into());
    }
    at += size_bytes;
    let mut last = false;
    while !last {
        if at + 3 > bytes.len() {
            return Err(incomplete());
        }
        let block =
            u32::from(bytes[at]) | u32::from(bytes[at + 1]) << 8 | u32::from(bytes[at + 2]) << 16;
        let kind = block >> 1 & 3;
        if kind == 3 {
            return Err("Invalid Figma compression block.".into());
        }
        last = block & 1 != 0;
        at += 3 + if kind == 1 { 1 } else { (block >> 3) as usize };
    }
    if descriptor & 4 != 0 {
        at += 4;
    }
    if at != bytes.len() {
        return Err("Unsupported Figma compression frame.".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Enum,
    Struct,
    Message,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Type {
    Bool,
    Byte,
    Int,
    Uint,
    Float,
    String,
    Int64,
    Uint64,
    Def(usize),
}
#[derive(Debug)]
struct Field {
    name: String,
    ty: Option<Type>,
    array: bool,
    value: u32,
}
#[derive(Debug)]
struct Definition {
    name: String,
    kind: Kind,
    fields: Vec<Field>,
}
/// The archive's own kiwi schema (`decodeBinarySchema`).
#[derive(Debug)]
struct Schema(Vec<Definition>);

/// A bounded reader over kiwi bytes.
struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Bytes<'a> {
    fn byte(&mut self, what: &str) -> Result<u8, String> {
        let b = *self.data.get(self.at).ok_or_else(|| what.to_string())?;
        self.at += 1;
        Ok(b)
    }
    /// At most five bytes, as today's `uint` reader.
    fn uint(&mut self, what: &str, invalid: &str) -> Result<u32, String> {
        let mut value = 0u32;
        for shift in (0..=28).step_by(7) {
            let b = self.byte(what)?;
            value |= u32::from(b & 127).wrapping_shl(shift);
            if b & 128 == 0 {
                return Ok(value);
            }
        }
        Err(invalid.into())
    }
    fn string(&mut self, what: &str) -> Result<String, String> {
        let end = self.data[self.at..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| what.to_string())?
            + self.at;
        let s = String::from_utf8_lossy(&self.data[self.at..end]).into_owned();
        self.at = end + 1;
        Ok(s)
    }
}

impl Schema {
    fn decode(data: &[u8]) -> Result<Schema, String> {
        const BAD: &str = "Incomplete Figma schema.";
        let mut b = Bytes { data, at: 0 };
        let count = b.uint(BAD, "Invalid Figma schema integer.")? as usize;
        if count > 4096 {
            return Err("This Figma schema is too large.".into());
        }
        let mut raw = Vec::with_capacity(count);
        let mut total = 0;
        for _ in 0..count {
            let name = b.string("Incomplete Figma schema text.")?;
            let kind = match b.byte(BAD)? {
                0 => Kind::Enum,
                1 => Kind::Struct,
                2 => Kind::Message,
                _ => return Err("Invalid Figma schema.".into()),
            };
            let fields = b.uint(BAD, "Invalid Figma schema integer.")? as usize;
            total += fields;
            if total > 65536 {
                return Err("This Figma schema is too large.".into());
            }
            let mut list = Vec::with_capacity(fields.min(1024));
            for _ in 0..fields {
                let name = b.string("Incomplete Figma schema text.")?;
                let ty = b.uint(BAD, "Invalid Figma schema integer.")?;
                let ty = if ty & 1 != 0 {
                    !i64::from(ty >> 1)
                } else {
                    i64::from(ty >> 1)
                };
                let array = b.byte(BAD)? & 1 != 0;
                let value = b.uint(BAD, "Invalid Figma schema integer.")?;
                list.push((name, ty, array, value));
            }
            raw.push((name, kind, list));
        }
        let mut definitions = Vec::with_capacity(count);
        for (name, kind, list) in raw {
            let mut fields = Vec::with_capacity(list.len());
            for (field, ty, array, value) in list {
                for n in [name.as_str(), field.as_str()] {
                    if ["__proto__", "prototype", "constructor"].contains(&n) {
                        return Err("Invalid Figma schema.".into());
                    }
                }
                let ty = match (kind, ty) {
                    (Kind::Enum, _) => None,
                    (_, t) if t < 0 => Some(match !t {
                        0 => Type::Bool,
                        1 => Type::Byte,
                        2 => Type::Int,
                        3 => Type::Uint,
                        4 => Type::Float,
                        5 => Type::String,
                        6 => Type::Int64,
                        7 => Type::Uint64,
                        _ => return Err("Invalid Figma schema.".into()),
                    }),
                    (_, t) if (t as usize) < count => Some(Type::Def(t as usize)),
                    _ => return Err("Invalid Figma schema.".into()),
                };
                fields.push(Field {
                    name: field,
                    ty,
                    array,
                    value,
                });
            }
            definitions.push(Definition { name, kind, fields });
        }
        Ok(Schema(definitions))
    }

    /// `readFigmaMessage`: the `Message` definition, every byte consumed.
    fn read_message(&self, data: &[u8]) -> Result<K, String> {
        let root = self
            .0
            .iter()
            .position(|d| d.name == "Message")
            .ok_or("Unknown Figma data type.")?;
        let mut reader = Reader {
            schema: self,
            b: Bytes { data, at: 0 },
            budget: 1_000_000,
        };
        let message = reader.read(Type::Def(root), 0)?;
        if reader.b.at != data.len() {
            return Err("Unexpected data after Figma layers.".into());
        }
        Ok(message)
    }
}

struct Reader<'s, 'd> {
    schema: &'s Schema,
    b: Bytes<'d>,
    budget: i64,
}
const LAYER_DATA: &str = "Incomplete Figma layer data.";
impl Reader<'_, '_> {
    fn spend(&mut self, depth: usize) -> Result<(), String> {
        self.budget -= 1;
        if self.budget < 0 || depth > 100 {
            return Err("This Figma selection is too complex. Copy fewer layers.".into());
        }
        Ok(())
    }
    fn uint(&mut self) -> Result<u32, String> {
        self.b.uint(LAYER_DATA, "Invalid Figma integer.")
    }
    fn read(&mut self, ty: Type, depth: usize) -> Result<K, String> {
        self.spend(depth)?;
        Ok(match ty {
            Type::Bool => K::Bool(self.b.byte(LAYER_DATA)? != 0),
            Type::Byte => K::Num(f64::from(self.b.byte(LAYER_DATA)?)),
            Type::Uint => K::Num(f64::from(self.uint()?)),
            Type::Int => {
                let v = self.uint()?;
                K::Num(f64::from(if v & 1 != 0 {
                    !((v >> 1) as i32)
                } else {
                    (v >> 1) as i32
                }))
            }
            Type::Float => {
                let first = self.b.byte(LAYER_DATA)?;
                if first == 0 {
                    K::Num(0.)
                } else {
                    let rest = [
                        self.b.byte(LAYER_DATA)?,
                        self.b.byte(LAYER_DATA)?,
                        self.b.byte(LAYER_DATA)?,
                    ];
                    let bits = u32::from_le_bytes([first, rest[0], rest[1], rest[2]]);
                    K::Num(f64::from(f32::from_bits(bits.rotate_left(23))))
                }
            }
            Type::String => K::Str(self.b.string("Incomplete Figma text.")?),
            Type::Uint64 | Type::Int64 => {
                let mut value = 0u64;
                let mut shift = 0;
                let mut byte = self.b.byte(LAYER_DATA)?;
                while byte & 128 != 0 && shift < 56 {
                    value |= u64::from(byte & 127) << shift;
                    shift += 7;
                    byte = self.b.byte(LAYER_DATA)?;
                }
                value |= u64::from(byte) << shift;
                K::Num(if ty == Type::Int64 {
                    let sign = value & 1 != 0;
                    let v = (value >> 1) as i64;
                    (if sign { !v } else { v }) as f64
                } else {
                    value as f64
                })
            }
            Type::Def(index) => self.definition(index, depth)?,
        })
    }
    fn definition(&mut self, index: usize, depth: usize) -> Result<K, String> {
        let definition = &self.schema.0[index];
        if definition.kind == Kind::Enum {
            let value = self.uint()?;
            let field = definition
                .fields
                .iter()
                .find(|f| f.value == value)
                .ok_or("Unknown Figma enum value.")?;
            return Ok(K::Str(field.name.clone()));
        }
        let mut out = HashMap::new();
        if definition.kind == Kind::Struct {
            for field in &definition.fields {
                out.insert(field.name.clone(), self.field(field, depth)?);
            }
        } else {
            loop {
                let tag = self.uint()?;
                if tag == 0 {
                    break;
                }
                self.budget -= 1;
                if self.budget < 0 {
                    return Err("This Figma selection is too complex.".into());
                }
                let field = definition
                    .fields
                    .iter()
                    .find(|f| f.value == tag)
                    .ok_or("Unknown Figma field.")?;
                out.insert(field.name.clone(), self.field(field, depth)?);
            }
        }
        Ok(K::Obj(out))
    }
    fn field(&mut self, field: &Field, depth: usize) -> Result<K, String> {
        let ty = field.ty.ok_or("Invalid Figma field.")?;
        if !field.array {
            return self.read(ty, depth + 1);
        }
        let length = self.uint()? as usize;
        if length as i64 > self.budget || length > self.b.data.len() - self.b.at {
            return Err("Invalid Figma array length.".into());
        }
        if ty == Type::Byte {
            let bytes = self.b.data[self.b.at..self.b.at + length].to_vec();
            self.b.at += length;
            return Ok(K::Bytes(bytes));
        }
        (0..length)
            .map(|_| self.read(ty, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(K::List)
    }
}

/// `ImportIssue`: a layer that needs the image fallback, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub name: String,
    pub reason: String,
}
/// `FigmaPlan`: the layers in stack order (top first), the top-level ones to
/// select, the issues, and the selection's size.
#[derive(Clone, Debug)]
pub struct Plan {
    pub layers: Vec<BlockLayer>,
    pub roots: Vec<String>,
    pub issues: Vec<Issue>,
    pub width: f64,
    pub height: f64,
}
/// The composition the plan lands in.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub w: f64,
    pub h: f64,
    pub fps: f64,
    pub dur: f64,
}

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1., 0., 0., 1., 0., 0.];
fn point(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}
fn transform(node: &K) -> Matrix {
    match node.get("transform") {
        Some(t) => {
            let f = |k| num(t, k).unwrap_or(0.);
            [f("m00"), f("m10"), f("m01"), f("m11"), f("m02"), f("m12")]
        }
        None => IDENTITY,
    }
}
fn guid(value: Option<&K>) -> String {
    match value {
        Some(g @ K::Obj(_)) => format!(
            "{}:{}",
            num(g, "sessionID").unwrap_or(0.),
            num(g, "localID").unwrap_or(0.)
        ),
        _ => String::new(),
    }
}
fn size(node: &K) -> (f64, f64) {
    let s = node.get("size");
    (
        s.and_then(|s| num(s, "x")).unwrap_or(0.),
        s.and_then(|s| num(s, "y")).unwrap_or(0.),
    )
}
fn position(node: &K) -> &str {
    node.get("parentIndex")
        .and_then(|p| text(p, "position"))
        .unwrap_or("")
}
fn visible_paints(paints: Option<&K>) -> Vec<&K> {
    paints
        .map(K::list)
        .unwrap_or_default()
        .iter()
        .filter(|p| p.get("visible").is_none_or(K::truthy) && num(p, "opacity").unwrap_or(1.) > 0.)
        .collect()
}
/// `#RRGGBBAA`, alpha times the paint's opacity.
fn colour(paint: Option<&&K>) -> String {
    let c = paint.and_then(|p| p.get("color"));
    let channel = |k: &str| c.and_then(|c| num(c, k)).unwrap_or(0.);
    let alpha = c.map_or(0., |c| num(c, "a").unwrap_or(1.))
        * paint.and_then(|p| num(p, "opacity")).unwrap_or(1.);
    let mut out = String::from("#");
    for v in [channel("r"), channel("g"), channel("b"), alpha] {
        out.push_str(&format!("{:02X}", (v.clamp(0., 1.) * 255.).round() as u8));
    }
    out
}
fn n(v: f64) -> Value {
    Value::Number(v)
}
fn s(v: &str) -> Value {
    Value::String(v.into())
}
fn property(v: Value) -> Value {
    let mut m = Map::new();
    m.insert("v", v);
    m.insert("kf", Value::Array(vec![]));
    m.insert("expr", Value::Null);
    Value::Object(m)
}
fn map(pairs: Vec<(&str, Value)>) -> Map {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(k, v);
    }
    m
}
/// `fontName.style` to a CSS weight, as today's table reads it.
fn style_weight(style: &str) -> f64 {
    let style = style.to_ascii_lowercase();
    // `a.?b`: `a`, at most one character, then `b`.
    let near = |a: &str, b: &str| {
        style.match_indices(a).any(|(i, _)| {
            let rest = &style[i + a.len()..];
            rest.starts_with(b)
                || rest
                    .chars()
                    .next()
                    .is_some_and(|c| rest[c.len_utf8()..].starts_with(b))
        })
    };
    let table: [(&dyn Fn() -> bool, f64); 8] = [
        (&|| style.contains("thin"), 100.),
        (&|| near("extra", "light"), 200.),
        (&|| style.contains("light"), 300.),
        (&|| style.contains("medium"), 500.),
        (&|| near("semi", "bold") || style.contains("demi"), 600.),
        (&|| near("extra", "bold") || near("ultra", "bold"), 800.),
        (&|| style.contains("black") || style.contains("heavy"), 900.),
        (&|| style.contains("bold"), 700.),
    ];
    table
        .iter()
        .find(|(test, _)| test())
        .map_or(400., |(_, w)| *w)
}

/// One path contour: vertices `(x, y, inX, inY, outX, outY)`.
type Contour = (Vec<[f64; 6]>, bool);
/// `readGeometry` + `parseSvgPathData`: Figma's opcode blob (0 Z, 1 M, 2 L,
/// 3 Q, 4 C; little-endian f32 points) into contours.
fn geometry(bytes: &[u8]) -> Result<Vec<Contour>, &'static str> {
    let mut out: Vec<Contour> = vec![];
    let mut vertices: Vec<[f64; 6]> = vec![];
    let (mut x, mut y, mut start) = (0., 0., (0., 0.));
    let mut at = 0;
    let mut commands = 0;
    let mut moved = false;
    let finish = |vertices: &mut Vec<[f64; 6]>, out: &mut Vec<Contour>, closed: bool| {
        if closed && vertices.len() > 1 {
            let (first, last) = (vertices[0], *vertices.last().unwrap());
            if (first[0] - last[0]).abs() < 1e-8 && (first[1] - last[1]).abs() < 1e-8 {
                vertices[0][2] = last[2];
                vertices[0][3] = last[3];
                vertices.pop();
            }
        }
        if !vertices.is_empty() {
            out.push((std::mem::take(vertices), closed));
        }
    };
    let cubic = |vertices: &mut Vec<[f64; 6]>, c1: (f64, f64), c2: (f64, f64), end: (f64, f64)| {
        let Some(start) = vertices.last_mut() else {
            vertices.push([end.0, end.1, 0., 0., 0., 0.]);
            return;
        };
        start[4] = c1.0 - start[0];
        start[5] = c1.1 - start[1];
        vertices.push([end.0, end.1, c2.0 - end.0, c2.1 - end.1, 0., 0.]);
    };
    while at < bytes.len() {
        commands += 1;
        if commands > 100_000 {
            return Err("Vector geometry is too large");
        }
        let opcode = bytes[at];
        at += 1;
        let count = match opcode {
            0 => 0,
            1 | 2 => 2,
            3 => 4,
            4 => 6,
            _ => return Err("Unsupported vector geometry"),
        };
        if at + count * 4 > bytes.len() {
            return Err("Unsupported vector geometry");
        }
        let mut v = [0f64; 6];
        for slot in v.iter_mut().take(count) {
            let value = f64::from(f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()));
            at += 4;
            if !value.is_finite() || value.abs() > 1e7 {
                return Err("Invalid vector coordinates");
            }
            *slot = value;
        }
        match opcode {
            0 => {
                finish(&mut vertices, &mut out, true);
                (x, y) = start;
                moved = false;
            }
            1 => {
                if !vertices.is_empty() {
                    finish(&mut vertices, &mut out, false);
                }
                vertices.push([v[0], v[1], 0., 0., 0., 0.]);
                (x, y) = (v[0], v[1]);
                start = (x, y);
                moved = true;
            }
            _ if !moved || vertices.is_empty() => return Err("SVG path command before move"),
            2 => {
                vertices.push([v[0], v[1], 0., 0., 0., 0.]);
                (x, y) = (v[0], v[1]);
            }
            3 => {
                let (q, p) = ((v[0], v[1]), (v[2], v[3]));
                let c1 = (x + 2. / 3. * (q.0 - x), y + 2. / 3. * (q.1 - y));
                let c2 = (p.0 + 2. / 3. * (q.0 - p.0), p.1 + 2. / 3. * (q.1 - p.1));
                cubic(&mut vertices, c1, c2, p);
                (x, y) = p;
            }
            _ => {
                cubic(&mut vertices, (v[0], v[1]), (v[2], v[3]), (v[4], v[5]));
                (x, y) = (v[4], v[5]);
            }
        }
    }
    finish(&mut vertices, &mut out, false);
    Ok(out)
}

struct Entry<'a> {
    node: &'a K,
    children: Vec<usize>,
}
struct Planner<'a> {
    entries: Vec<Entry<'a>>,
    blobs: &'a [Vec<u8>],
    fonts: &'a [String],
    from: f64,
    dur: f64,
    offset: (f64, f64),
    layers: Vec<BlockLayer>,
    roots: Vec<String>,
    issues: Vec<Issue>,
    visited: Vec<bool>,
    vertices: usize,
}

/// `planFigmaImport`: converts first, without touching the document, so the
/// fallback decision is all or nothing. `fonts` are the installed families
/// (empty: not checked).
pub fn plan(
    selection: &Selection,
    target: Target,
    at: f64,
    fonts: &[String],
) -> Result<Plan, String> {
    let skip = [
        "DOCUMENT",
        "CANVAS",
        "STYLE",
        "VARIABLE",
        "VARIABLE_SET",
        "SLICE",
    ];
    let mut entries: Vec<Entry> = vec![];
    let mut index: HashMap<String, usize> = HashMap::new();
    for node in &selection.nodes {
        if node.get("guid").is_none()
            || text(node, "phase") == Some("REMOVED")
            || skip.contains(&text(node, "type").unwrap_or(""))
        {
            continue;
        }
        let id = guid(node.get("guid"));
        if index.insert(id, entries.len()).is_some() {
            return Err("The copied Figma selection contains duplicate layers.".into());
        }
        entries.push(Entry {
            node,
            children: vec![],
        });
    }
    let mut roots = vec![];
    for i in 0..entries.len() {
        let parent = guid(
            entries[i]
                .node
                .get("parentIndex")
                .and_then(|p| p.get("guid")),
        );
        match index.get(&parent) {
            Some(&p) => entries[p].children.push(i),
            None => roots.push(i),
        }
    }
    if roots.is_empty() {
        return Err("No importable layers were found in the Figma selection.".into());
    }
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &r in &roots {
        let node = entries[r].node;
        let (m, (w, h)) = (transform(node), size(node));
        for (x, y) in [(0., 0.), (w, 0.), (0., h), (w, h)] {
            let p = point(&m, x, y);
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
    }
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return Err("The copied Figma selection has invalid bounds.".into());
    }
    let (width, height) = ((x1 - x0).max(1.), (y1 - y0).max(1.));
    let from = (at * target.fps).round() / target.fps;
    let count = entries.len();
    let mut planner = Planner {
        entries,
        blobs: &selection.blobs,
        fonts,
        from,
        dur: (1. / target.fps).max(target.dur - from),
        offset: ((target.w - width) / 2. - x0, (target.h - height) / 2. - y0),
        layers: vec![],
        roots: vec![],
        issues: vec![],
        visited: vec![false; count],
        vertices: 0,
    };
    // Topmost first: Figma's last sibling paints on top.
    roots
        .sort_by(|&a, &b| position(planner.entries[b].node).cmp(position(planner.entries[a].node)));
    for r in roots {
        planner.build(r, None, 0)?;
    }
    if planner.visited.iter().any(|v| !v) {
        return Err("The copied Figma layer hierarchy is invalid.".into());
    }
    Ok(Plan {
        layers: planner.layers,
        roots: planner.roots,
        issues: planner.issues,
        width,
        height,
    })
}

impl Planner<'_> {
    fn issue(&mut self, node: &K, reason: &str) {
        let name = text(node, "name")
            .filter(|s| !s.is_empty())
            .or(text(node, "type"))
            .unwrap_or("Layer")
            .to_owned();
        if self.issues.len() < 30
            && !self
                .issues
                .iter()
                .any(|i| i.name == name && i.reason == reason)
        {
            self.issues.push(Issue {
                name,
                reason: reason.into(),
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        &mut self,
        node: &K,
        kind: &str,
        content: Map,
        mut properties: Map,
        name: String,
        group: Option<&str>,
        (w, h): (f64, f64),
    ) -> String {
        // Primitive shapes render around their centre; Figma's origin is
        // the top-left corner. Editable vector paths use authored coordinates.
        if kind == "shape"
            && content
                .get("paths")
                .is_none_or(|p| p.as_array().is_none_or(Vec::is_empty))
        {
            properties.insert("anchor.x", n(-w / 2.));
            properties.insert("anchor.y", n(-h / 2.));
        }
        let id = pm_kernel::new_id("L");
        self.layers.push(BlockLayer {
            add: AddLayer {
                id: id.clone(),
                layer_type: kind.into(),
                comp: None,
                name: Some(name),
                from: Some(self.from),
                duration: Some(self.dur),
                content,
                properties,
                color: None,
                index: None,
                parent: None,
                blend: None,
                motion_blur: None,
                visible: Some(node.get("visible").is_none_or(K::truthy)),
                shy: None,
                collapsed: Some(false),
                time: self.from,
            },
            group: group.map(str::to_owned),
            locked: node.get("locked").is_some_and(K::truthy),
        });
        id
    }

    fn build(&mut self, entry: usize, group: Option<&str>, depth: usize) -> Result<(), String> {
        if self.visited[entry] || depth > 100 {
            return Err("The copied Figma layer hierarchy is invalid.".into());
        }
        self.visited[entry] = true;
        let node = self.entries[entry].node;
        let kind = text(node, "type").unwrap_or("");
        let m = transform(node);
        if m.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
            return Err("A copied Figma layer has an invalid transform.".into());
        }
        let (w, h) = size(node);
        if ![w, h]
            .iter()
            .all(|v| v.is_finite() && (0. ..=1e6).contains(v))
        {
            return Err("A copied Figma layer has an invalid size.".into());
        }
        for key in [
            "opacity",
            "strokeWeight",
            "cornerRadius",
            "cornerSmoothing",
            "rectangleTopLeftCornerRadius",
            "rectangleTopRightCornerRadius",
            "rectangleBottomLeftCornerRadius",
            "rectangleBottomRightCornerRadius",
        ] {
            if num(node, key).is_some_and(|v| !v.is_finite() || v < 0.) {
                return Err("A copied Figma layer has invalid properties.".into());
            }
        }
        let sx = m[0].hypot(m[1]);
        let det = m[0] * m[3] - m[1] * m[2];
        let sy = if sx != 0. { det / sx } else { 0. };
        let skew = if sy != 0. {
            ((m[0] * m[2] + m[1] * m[3]) / (sx * sy))
                .atan()
                .to_degrees()
        } else {
            0.
        };
        if sx == 0. || sy == 0. || skew.abs() > 80. {
            self.issue(node, "unsupported transform");
        }
        let (ox, oy) = if group.is_some() {
            (0., 0.)
        } else {
            self.offset
        };
        let p = map(vec![
            ("position.x", n(m[4] + ox)),
            ("position.y", n(m[5] + oy)),
            ("anchor.x", n(0.)),
            ("anchor.y", n(0.)),
            ("scale.x", n(sx * 100.)),
            ("scale.y", n(sy * 100.)),
            ("rotation", n(m[1].atan2(m[0]).to_degrees())),
            ("skew", n(skew)),
            ("opacity", n(num(node, "opacity").unwrap_or(1.) * 100.)),
        ]);
        let name = text(node, "name")
            .filter(|s| !s.is_empty())
            .or(Some(kind).filter(|k| !k.is_empty()))
            .unwrap_or("Figma layer")
            .to_owned();
        if node.get("mask").is_some_and(K::truthy) {
            self.issue(node, "mask");
        }
        if node
            .get("effects")
            .map(K::list)
            .unwrap_or_default()
            .iter()
            .any(|e| e.get("visible").is_none_or(K::truthy))
        {
            self.issue(node, "effects");
        }
        if text(node, "blendMode").is_some_and(|b| !["NORMAL", "PASS_THROUGH"].contains(&b)) {
            self.issue(node, "blend mode");
        }
        let fills = visible_paints(node.get("fillPaints").or(node.get("backgroundPaints")));
        let strokes = visible_paints(node.get("strokePaints"));
        if fills.len() > 1 || strokes.len() > 1 {
            self.issue(node, "multiple fills or strokes");
        }
        for paint in fills.iter().chain(&strokes) {
            let c = paint.get("color");
            if num(paint, "opacity").is_some_and(|v| !v.is_finite())
                || ["r", "g", "b", "a"]
                    .iter()
                    .any(|k| c.and_then(|c| num(c, k)).is_some_and(|v| !v.is_finite()))
            {
                return Err("A Figma layer has an invalid fill.".into());
            }
            match text(paint, "type") {
                Some("SOLID") => {}
                Some("IMAGE") => self.issue(node, "image fill"),
                _ => self.issue(node, "gradient or special fill"),
            }
            if text(paint, "blendMode").is_some_and(|b| b != "NORMAL") {
                self.issue(node, "fill blend mode");
            }
        }
        if !node
            .get("dashPattern")
            .map(K::list)
            .unwrap_or_default()
            .is_empty()
        {
            self.issue(node, "dashed stroke");
        }
        if !strokes.is_empty() && text(node, "strokeCap").is_some_and(|c| c != "NONE") {
            self.issue(node, "stroke end caps");
        }
        let join = if kind == "VECTOR" { "MITER" } else { "ROUND" };
        if !strokes.is_empty() && text(node, "strokeJoin").is_some_and(|j| j != join) {
            self.issue(node, "stroke joins");
        }
        if kind == "INSTANCE" {
            self.issue(node, "component instance");
        }
        let radius = num(node, "cornerRadius").unwrap_or(0.);
        let corner = |k: &str| n(num(node, k).unwrap_or(radius));
        let independent = node
            .get("rectangleCornerRadiiIndependent")
            .is_some_and(K::truthy);
        let stroke_weight = if strokes.is_empty() {
            0.
        } else {
            num(node, "strokeWeight").unwrap_or(1.)
        };
        let shape = map(vec![
            (
                "shape",
                s(if kind == "ELLIPSE" { "ellipse" } else { "rect" }),
            ),
            ("w", n(w)),
            ("h", n(h)),
            ("color", s(&colour(fills.first()))),
            ("radius", n(radius)),
            (
                "smoothing",
                n(num(node, "cornerSmoothing").unwrap_or(0.) * 100.),
            ),
            ("independentCorners", Value::Bool(independent)),
            ("radiusTL", corner("rectangleTopLeftCornerRadius")),
            ("radiusTR", corner("rectangleTopRightCornerRadius")),
            ("radiusBR", corner("rectangleBottomRightCornerRadius")),
            ("radiusBL", corner("rectangleBottomLeftCornerRadius")),
            ("stroke", n(stroke_weight)),
            ("strokeColor", s(&colour(strokes.first()))),
        ]);
        if !strokes.is_empty() && text(node, "strokeAlign") != Some("CENTER") {
            self.issue(node, "inside or outside stroke");
        }
        let children = self.entries[entry].children.clone();
        if ["GROUP", "FRAME", "SYMBOL", "INSTANCE"].contains(&kind) {
            if !strokes.is_empty() {
                self.issue(node, "frame stroke");
            }
            let id = self.create(
                node,
                "group",
                Map::new(),
                p.clone(),
                name.clone(),
                group,
                (w, h),
            );
            if group.is_none() {
                self.roots.push(id.clone());
            }
            let mut ordered = children.clone();
            ordered.sort_by(|&a, &b| {
                position(self.entries[a].node).cmp(position(self.entries[b].node))
            });
            // Figma siblings paint bottom to top; Powermove's stack is top to bottom.
            for &child in ordered.iter().rev() {
                self.build(child, Some(&id), depth + 1)?;
            }
            if kind != "GROUP" && (!fills.is_empty() || !strokes.is_empty()) {
                let mut base = p.clone();
                for (k, v) in [
                    ("position.x", 0.),
                    ("position.y", 0.),
                    ("scale.x", 100.),
                    ("scale.y", 100.),
                    ("rotation", 0.),
                    ("skew", 0.),
                    ("opacity", 100.),
                ] {
                    base.insert(k, n(v));
                }
                let label = format!(
                    "{} background",
                    text(node, "name")
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Frame")
                );
                let background =
                    self.create(node, "shape", shape.clone(), base, label, Some(&id), (w, h));
                if let Some(record) = self.layers.iter_mut().find(|l| l.add.id == background) {
                    record.add.visible = Some(true);
                }
            }
            if kind != "GROUP"
                && !node
                    .get("frameMaskDisabled")
                    .is_some_and(|v| *v == K::Bool(true))
                && !children.is_empty()
            {
                // No mask is needed when every descendant lies inside a square
                // frame; rounded clipping and overflow need the image fallback.
                let overflows = children.iter().any(|&c| {
                    let child = self.entries[c].node;
                    let (cm, (cw, ch)) = (transform(child), size(child));
                    [(0., 0.), (cw, 0.), (0., ch), (cw, ch)]
                        .iter()
                        .any(|&(x, y)| {
                            let v = point(&cm, x, y);
                            v.0 < 0. || v.1 < 0. || v.0 > w || v.1 > h
                        })
                        || !self.entries[c].children.is_empty()
                        || !visible_paints(child.get("strokePaints")).is_empty()
                });
                if radius != 0. || independent || overflows {
                    self.issue(node, "clipped frame");
                }
            }
            return Ok(());
        }
        let id = match kind {
            "TEXT" => self.text_layer(node, &m, &p, &fills, &strokes, name, group, (w, h))?,
            "RECTANGLE" | "ROUNDED_RECTANGLE" | "ELLIPSE" => {
                if let Some(arc) = node.get("arcData") {
                    let start = num(arc, "startingAngle").unwrap_or(0.);
                    let end = num(arc, "endingAngle").unwrap_or(std::f64::consts::TAU);
                    if num(arc, "innerRadius").unwrap_or(0.) > 0.
                        || (end - start - std::f64::consts::TAU).abs() > 1e-5
                    {
                        self.issue(node, "partial ellipse");
                    }
                }
                self.create(node, "shape", shape, p, name, group, (w, h))
            }
            "VECTOR" | "BOOLEAN_OPERATION" | "LINE" | "REGULAR_POLYGON" | "STAR" => {
                let paths = match self.vector_paths(node, &fills, &strokes) {
                    Ok(paths) => paths,
                    Err(()) => {
                        self.issue(node, "complex vector geometry");
                        vec![]
                    }
                };
                let content = map(vec![("paths", Value::Array(paths))]);
                self.create(node, "shape", content, p, name, group, (w, h))
            }
            other => {
                let reason = format!(
                    "unsupported {}",
                    if other.is_empty() {
                        "layer".into()
                    } else {
                        other.to_ascii_lowercase()
                    }
                );
                self.issue(node, &reason);
                self.create(node, "group", Map::new(), p, name, group, (w, h))
            }
        };
        if group.is_none() {
            self.roots.push(id);
        }
        if !children.is_empty() {
            self.issue(node, "unsupported layer hierarchy");
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn text_layer(
        &mut self,
        node: &K,
        m: &Matrix,
        p: &Map,
        fills: &[&K],
        strokes: &[&K],
        name: String,
        group: Option<&str>,
        (w, h): (f64, f64),
    ) -> Result<String, String> {
        let data = node.get("textData");
        let characters = data.and_then(|d| text(d, "characters")).unwrap_or("");
        let size = num(node, "fontSize").unwrap_or(16.);
        let font_name = node.get("fontName");
        let font = font_name
            .and_then(|f| text(f, "family"))
            .filter(|f| !f.is_empty())
            .unwrap_or("Arial");
        if !size.is_finite() || size <= 0. || size > 16384. {
            return Err("A Figma text layer has an invalid font size.".into());
        }
        if !self.fonts.is_empty() && !self.fonts.iter().any(|f| f == font) {
            self.issue(node, &format!("missing font: {font}"));
        }
        if data
            .and_then(|d| d.get("characterStyleIDs"))
            .map(K::list)
            .unwrap_or_default()
            .iter()
            .any(|id| id.num() != Some(0.))
        {
            self.issue(node, "mixed text styles");
        }
        let plain_lines = data
            .and_then(|d| d.get("lines"))
            .map(K::list)
            .unwrap_or_default()
            .iter()
            .all(|l| text(l, "lineType").is_none_or(|t| t.is_empty() || t == "PLAIN"));
        if text(node, "textDecoration").is_some_and(|d| d != "NONE")
            || text(node, "textCase").is_some_and(|c| c != "ORIGINAL")
            || num(node, "paragraphSpacing").is_some_and(|v| v != 0.)
            || num(node, "paragraphIndent").is_some_and(|v| v != 0.)
            || !node
                .get("fontVariations")
                .map(K::list)
                .unwrap_or_default()
                .is_empty()
            || !plain_lines
        {
            self.issue(node, "advanced text formatting");
        }
        let align = text(node, "textAlignHorizontal");
        if !matches!(align, None | Some("LEFT" | "CENTER" | "RIGHT")) {
            self.issue(node, "justified text");
        }
        let baseline = node
            .get("derivedTextData")
            .and_then(|d| d.get("baselines"))
            .and_then(|b| b.list().first());
        let line_height = node.get("lineHeight");
        let units = line_height.and_then(|l| text(l, "units"));
        let value = line_height
            .and_then(|l| num(l, "value"))
            .unwrap_or(f64::NAN);
        let leading = match units {
            Some("PIXELS") => value,
            Some("PERCENT") => size * value / 100.,
            _ => baseline
                .and_then(|b| num(b, "lineHeight"))
                .unwrap_or(size * 1.2),
        };
        if !leading.is_finite() || leading <= 0. {
            return Err("A Figma text layer has invalid line spacing.".into());
        }
        let spacing = node.get("letterSpacing");
        let spacing_value = spacing.and_then(|l| num(l, "value"));
        if spacing.is_some() && spacing_value.is_some_and(|v| !v.is_finite()) {
            return Err("A Figma text layer has invalid letter spacing.".into());
        }
        let align = align.unwrap_or("LEFT").to_ascii_lowercase();
        let ox = match align.as_str() {
            "center" => w / 2.,
            "right" => w,
            _ => 0.,
        };
        let oy = baseline
            .and_then(|b| b.get("position"))
            .and_then(|p| num(p, "y"))
            .map_or(0., |y| y - size * 0.82);
        let origin = point(m, ox, oy);
        let (gx, gy) = if group.is_some() {
            (0., 0.)
        } else {
            self.offset
        };
        let mut props = p.clone();
        props.insert("position.x", n(origin.0 + gx));
        props.insert("position.y", n(origin.1 + gy));
        let style = font_name.and_then(|f| text(f, "style")).unwrap_or("");
        let weight = num(node, "fontWeight").unwrap_or_else(|| style_weight(style));
        let lower = style.to_ascii_lowercase();
        let auto = text(node, "textAutoResize");
        let tracking = match spacing.and_then(|l| text(l, "units")) {
            Some("PERCENT") => size * spacing_value.unwrap_or(0.) / 100.,
            _ => spacing_value.unwrap_or(0.),
        };
        let content = map(vec![
            ("text", s(characters)),
            ("font", s(font)),
            ("size", n(size)),
            ("weight", n(weight)),
            (
                "italic",
                Value::Bool(lower.contains("italic") || lower.contains("oblique")),
            ),
            ("color", s(&colour(fills.first()))),
            ("align", s(&align)),
            (
                "boxWidth",
                n(if auto == Some("WIDTH_AND_HEIGHT") {
                    0.
                } else {
                    w
                }),
            ),
            ("boxHeight", n(if auto == Some("NONE") { h } else { 0. })),
            ("tracking", n(tracking)),
            ("leading", n(leading / size)),
        ]);
        let id = self.create(node, "text", content, props, name, group, (w, h));
        if !strokes.is_empty() {
            self.issue(node, "text stroke");
        }
        Ok(id)
    }

    /// `materializeSvgPaths` of the node's fill geometry; `Err` asks for the
    /// image fallback.
    fn vector_paths(&mut self, node: &K, fills: &[&K], strokes: &[&K]) -> Result<Vec<Value>, ()> {
        let mut paths = vec![];
        let name = text(node, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or("Path");
        let opacity = |paint: Option<&&K>| {
            paint.and_then(|p| num(p, "opacity")).unwrap_or(1.)
                * paint
                    .and_then(|p| p.get("color"))
                    .and_then(|c| num(c, "a"))
                    .unwrap_or(1.)
                * 100.
        };
        for entry in node.get("fillGeometry").map(K::list).unwrap_or_default() {
            let blob = num(entry, "commandsBlob")
                .and_then(|i| self.blobs.get(i as usize))
                .ok_or(())?;
            let contours = geometry(blob).map_err(|_| ())?;
            self.vertices += contours.iter().map(|c| c.0.len()).sum::<usize>();
            // Powermove fills each editable path separately: compound contours
            // could hold holes, so never fill those in silently.
            if self.vertices > 50_000
                || (!fills.is_empty() && (contours.len() > 1 || contours.iter().any(|c| !c.1)))
                || num(entry, "styleID").is_some_and(|v| v != 0.)
            {
                return Err(());
            }
            for (vertices, closed) in contours {
                let mut p = Map::new();
                for (k, v) in [
                    ("x", 0.),
                    ("y", 0.),
                    ("rotation", 0.),
                    ("scaleX", 100.),
                    ("scaleY", 100.),
                    ("trimStart", 0.),
                    ("trimEnd", 100.),
                    ("trimOffset", 0.),
                    ("copies", 1.),
                    ("repeatX", 30.),
                    ("repeatY", 0.),
                    ("repeatRotation", 0.),
                ] {
                    p.insert(k, property(n(v)));
                }
                p.insert("closed", property(Value::Bool(closed)));
                p.insert("fill", property(s(&colour(fills.first())[..7])));
                p.insert("fillEnabled", property(Value::Bool(!fills.is_empty())));
                p.insert("fillOpacity", property(n(opacity(fills.first()))));
                p.insert("stroke", property(s(&colour(strokes.first())[..7])));
                let width = if strokes.is_empty() {
                    0.
                } else {
                    num(node, "strokeWeight").unwrap_or(1.)
                };
                p.insert("strokeWidth", property(n(width)));
                p.insert("strokeOpacity", property(n(opacity(strokes.first()))));
                let vertices = vertices
                    .iter()
                    .map(|v| {
                        let mut vp = Map::new();
                        for (k, value) in ["x", "y", "inX", "inY", "outX", "outY"].iter().zip(v) {
                            vp.insert(*k, property(n(*value)));
                        }
                        Value::Object(map(vec![
                            ("id", s(&pm_kernel::new_id("v"))),
                            ("p", Value::Object(vp)),
                        ]))
                    })
                    .collect();
                paths.push(Value::Object(map(vec![
                    ("id", s(&pm_kernel::new_id("path"))),
                    ("name", s(name)),
                    ("parent", Value::Null),
                    ("vertices", Value::Array(vertices)),
                    ("p", Value::Object(p)),
                ])));
            }
        }
        if paths.is_empty() {
            return Err(());
        }
        Ok(paths)
    }
}

/// The fallback prompt's reasons (`runFigmaPaste`): a decoding problem, or
/// the first six issues, each kept short for the sheet.
pub fn reasons(problem: Option<&str>, issues: &[Issue]) -> String {
    let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    match problem {
        Some(problem) => clip(problem, 600),
        None => issues
            .iter()
            .take(6)
            .map(|i| format!("“{}”: {}", clip(&i.name, 80), clip(&i.reason, 120)))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// `inlinePng`: a data-URI PNG preview in the clipboard HTML (never a URL).
pub fn inline_png(html: &str) -> Option<Vec<u8>> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<img") {
        let tag_start = from + i;
        let tag_end = lower[tag_start..]
            .find('>')
            .map_or(lower.len(), |e| tag_start + e);
        let tag = &lower[tag_start..tag_end];
        if let Some(src) = tag.find("src") {
            let rest = tag[src + 3..].trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start();
                let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'');
                if let Some(quote) = quote
                    && let Some(data) = rest[1..].strip_prefix("data:image/png;base64,")
                {
                    let start = tag_start + (tag.len() - data.len());
                    let len = data.find(quote).unwrap_or(data.len());
                    let encoded = &html[start..start + len];
                    if encoded.len() > MAX_CLIPBOARD_CHARS
                        || !encoded
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                    {
                        return None;
                    }
                    return base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .ok();
                }
            }
        }
        from = tag_end;
    }
    None
}

#[cfg(test)]
#[path = "figma_tests.rs"]
mod tests;
