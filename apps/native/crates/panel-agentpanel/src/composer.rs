//! Source: v1.2.0 panels/agent/inline-prompt.ts (not Chromium DOM mechanics).
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{collections::HashMap, ops::Range, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment(Arc<Value>);
impl Attachment {
    /// Retains original encoded bytes, missing-payload markers and unknown fields.
    pub fn from_record(record: Value) -> Result<Self> {
        if record["id"].as_str().is_none_or(|s| s.is_empty()) || record["name"].as_str().is_none() {
            return Err(Error::Invalid("attachment record"));
        }
        Ok(Self(Arc::new(record)))
    }
    pub fn id(&self) -> &str {
        self.0["id"].as_str().expect("validated")
    }
    pub fn name(&self) -> &str {
        self.0["name"].as_str().expect("validated")
    }
    pub fn record(&self) -> &Value {
        &self.0
    }
    pub fn previewable(&self) -> bool {
        let mime = self.0["type"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        matches!(
            mime.as_str(),
            "image/png"
                | "image/jpeg"
                | "image/webp"
                | "image/gif"
                | "image/svg+xml"
                | "image/avif"
                | "image/bmp"
        ) && self.0["dataUrl"].is_string()
    }
    pub fn payload_available(&self) -> bool {
        ["dataUrl", "dataBase64", "content"]
            .iter()
            .any(|k| self.0[*k].is_string())
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Unit {
    Char(char),
    File(Attachment),
}
impl Unit {
    fn width(&self) -> usize {
        match self {
            Self::Char(c) => c.len_utf16(),
            Self::File(_) => 1,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    File(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    pub parts: Vec<Part>,
    pub plain: String,
}
#[derive(Clone, Debug, PartialEq)]
struct State {
    units: Vec<Unit>,
    selection: Range<usize>,
}
#[derive(Clone, Debug)]
pub struct Draft {
    state: State,
    catalog: HashMap<String, Attachment>,
    history: Vec<State>,
    cursor: usize,
    last_typing: Option<u64>,
    armed: Option<String>,
}
impl Default for Draft {
    fn default() -> Self {
        let state = State {
            units: vec![],
            selection: 0..0,
        };
        Self {
            state: state.clone(),
            catalog: HashMap::new(),
            history: vec![state],
            cursor: 0,
            last_typing: None,
            armed: None,
        }
    }
}
impl Draft {
    /// Legacy promptOffset counts UTF-16 text units and excludes file tokens.
    /// Invalid offsets are refused without rewriting the source archive (N4).
    pub fn from_legacy(text: &str, files: &[Value]) -> Result<Self> {
        let mut draft = Self::default();
        let chars: Vec<char> = text.chars().collect();
        let text_len = text.encode_utf16().count();
        let mut placed = Vec::new();
        for value in files {
            let file = Attachment::from_record(value.clone())?;
            if draft
                .catalog
                .insert(file.id().into(), file.clone())
                .is_some()
            {
                return Err(Error::Invalid("duplicate attachment id"));
            }
            let offset = match value.get("promptOffset") {
                None => text_len,
                Some(v) => v
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or(Error::Invalid("attachment offset"))?,
            };
            if offset > text_len {
                return Err(Error::Invalid("attachment offset"));
            }
            placed.push((offset, file));
        }
        placed.sort_by_key(|(offset, _)| *offset);
        let mut files = placed.into_iter().peekable();
        let mut offset = 0;
        for c in chars.into_iter().map(Some).chain(std::iter::once(None)) {
            while files.peek().is_some_and(|(at, _)| *at == offset) {
                draft
                    .state
                    .units
                    .push(Unit::File(files.next().expect("peeked").1));
            }
            if let Some(c) = c {
                offset += c.len_utf16();
                draft.state.units.push(Unit::Char(c));
                if files.peek().is_some_and(|(at, _)| *at < offset) {
                    return Err(Error::Invalid("split surrogate"));
                }
            }
        }
        draft.end();
        draft.history = vec![draft.state.clone()];
        Ok(draft)
    }
    /// Text-only worker projection; does not clone encoded attachment bodies.
    pub fn text(&self) -> String {
        self.state
            .units
            .iter()
            .filter_map(|unit| match unit {
                Unit::Char(c) => Some(*c),
                Unit::File(_) => None,
            })
            .collect()
    }
    pub fn legacy(&self) -> (String, Vec<Value>) {
        let mut text = String::new();
        let mut offset = 0;
        let mut files = vec![];
        for unit in &self.state.units {
            match unit {
                Unit::Char(c) => {
                    text.push(*c);
                    offset += c.len_utf16();
                }
                Unit::File(f) => {
                    let mut record = f.record().clone();
                    record["promptOffset"] = json!(offset);
                    files.push(record);
                }
            }
        }
        (text, files)
    }
    pub fn request_text(&self) -> String {
        self.state
            .units
            .iter()
            .map(|u| match u {
                Unit::Char(c) => c.to_string(),
                Unit::File(f) => format!("[Attachment: {}]", f.name()),
            })
            .collect()
    }
    pub fn sendable(&self) -> bool {
        self.state.units.iter().any(|u| match u {
            Unit::File(_) => true,
            Unit::Char(c) => !c.is_whitespace(),
        })
    }
    pub fn attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.state
            .units
            .iter()
            .filter_map(|u| if let Unit::File(f) = u { Some(f) } else { None })
    }
    pub fn attachment(&self, id: &str) -> Option<&Attachment> {
        self.attachments().find(|f| f.id() == id)
    }
    pub fn selection(&self) -> Range<usize> {
        let pos = |n: usize| self.state.units[..n].iter().map(Unit::width).sum();
        pos(self.state.selection.start)..pos(self.state.selection.end)
    }
    /// The selection in `text()` coordinates, where attachments take no room
    /// (the native composer shows text only; attachments sit above it).
    pub fn text_selection(&self) -> Range<usize> {
        let pos = |n: usize| -> usize {
            self.state.units[..n]
                .iter()
                .map(|u| match u {
                    Unit::Char(c) => c.len_utf16(),
                    Unit::File(_) => 0,
                })
                .sum()
        };
        pos(self.state.selection.start)..pos(self.state.selection.end)
    }
    /// Selects a `text()` range; a boundary next to attachments lands after them.
    pub fn select_text(&mut self, range: Range<usize>) -> Result<()> {
        if range.start > range.end {
            return Err(Error::Invalid("selection order"));
        }
        let unit = |offset: usize| -> Result<usize> {
            let mut at = 0;
            let mut found = None;
            for (i, u) in self.state.units.iter().enumerate() {
                if at == offset {
                    found = Some(i);
                }
                if let Unit::Char(c) = u {
                    if at == offset {
                        return Ok(i);
                    }
                    at += c.len_utf16();
                }
            }
            match (at == offset, found) {
                (true, _) => Ok(self.state.units.len()),
                (false, Some(i)) => Ok(i),
                _ => Err(Error::Invalid("v4 text range")),
            }
        };
        self.state.selection = unit(range.start)?..unit(range.end)?;
        self.disarm();
        Ok(())
    }
    fn index(&self, offset: usize) -> Result<usize> {
        let mut at = 0;
        for (i, u) in self.state.units.iter().enumerate() {
            if at == offset {
                return Ok(i);
            }
            at += u.width();
            if at > offset {
                return Err(Error::Invalid("split surrogate"));
            }
        }
        if at == offset {
            Ok(self.state.units.len())
        } else {
            Err(Error::Invalid("caret offset"))
        }
    }
    pub fn select(&mut self, range: Range<usize>) -> Result<()> {
        if range.start > range.end {
            return Err(Error::Invalid("selection order"));
        }
        self.state.selection = self.index(range.start)?..self.index(range.end)?;
        self.disarm();
        Ok(())
    }
    pub fn select_all(&mut self) {
        self.state.selection = 0..self.state.units.len();
        self.disarm();
    }
    pub fn disarm(&mut self) {
        self.armed = None;
    }
    pub fn armed(&self) -> Option<&str> {
        self.armed.as_deref()
    }
    fn end(&mut self) {
        let end = self.state.units.len();
        self.state.selection = end..end;
    }
    fn preserve_selected_files(&mut self) {
        if self.state.units[self.state.selection.clone()]
            .iter()
            .any(|u| matches!(u, Unit::File(_)))
        {
            self.state.selection.start = self.state.selection.end;
        }
    }
    fn insert(&mut self, units: Vec<Unit>) {
        let at = self.state.selection.start;
        let end = at + units.len();
        self.state.units.splice(self.state.selection.clone(), units);
        self.state.selection = end..end;
    }
    fn commit(&mut self, now: u64, typing: bool) {
        self.armed = None;
        self.history.truncate(self.cursor + 1);
        if typing && self.cursor > 0 && self.last_typing.is_some_and(|t| now >= t && now - t < 600)
        {
            self.history[self.cursor] = self.state.clone();
        } else {
            self.history.push(self.state.clone());
            self.cursor += 1;
            if self.history.len() > 100 {
                self.history.remove(0);
                self.cursor -= 1;
            }
        }
        self.last_typing = typing.then_some(now);
    }
    pub fn insert_text(&mut self, text: &str, now: u64, typing: bool) {
        self.preserve_selected_files();
        self.insert(text.chars().map(Unit::Char).collect());
        self.commit(now, typing);
    }
    pub fn add_files(&mut self, files: Vec<Attachment>, now: u64) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let mut seen = std::collections::HashSet::new();
        for f in &files {
            if !seen.insert(f.id()) || self.catalog.get(f.id()).is_some_and(|old| old != f) {
                return Err(Error::Invalid("attachment identity conflict"));
            }
        }
        self.preserve_selected_files();
        // Existing tokens move, never duplicate the same attachment id.
        let mut caret = self.state.selection.start;
        let mut i = 0;
        self.state.units.retain(|u| {
            let keep = !matches!(u, Unit::File(f) if seen.contains(f.id()));
            if !keep && i < self.state.selection.start {
                caret -= 1;
            }
            i += 1;
            keep
        });
        self.state.selection = caret..caret;
        for f in &files {
            self.catalog.insert(f.id().into(), f.clone());
        }
        self.insert(files.into_iter().map(Unit::File).collect());
        self.commit(now, false);
        Ok(())
    }
    pub fn remove_file(&mut self, id: &str, now: u64) -> Result<()> {
        let index = self
            .state
            .units
            .iter()
            .position(|u| matches!(u, Unit::File(f) if f.id() == id))
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        self.state.units.remove(index);
        self.state.selection = index..index;
        self.commit(now, false);
        Ok(())
    }
    pub fn move_file(&mut self, id: &str, offset: usize, now: u64) -> Result<()> {
        let target = self.index(offset)?;
        let from = self
            .state
            .units
            .iter()
            .position(|u| matches!(u, Unit::File(f) if f.id() == id))
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        let unit = self.state.units.remove(from);
        let target = target - usize::from(from < target);
        self.state.units.insert(target, unit);
        self.state.selection = target + 1..target + 1;
        self.commit(now, false);
        Ok(())
    }
    /// Keyboard interception concerns tokens only. Ordinary character, word and
    /// line deletion remains native and supplies its exact UTF-16 edit range.
    pub fn delete_token(&mut self, backward: bool, repeat: bool, now: u64) -> bool {
        let mut range = self.state.selection.clone();
        if range.is_empty() {
            if backward && range.start > 0 {
                range.start -= 1;
            } else if !backward && range.end < self.state.units.len() {
                range.end += 1;
            }
        }
        if !self.state.units[range]
            .iter()
            .any(|u| matches!(u, Unit::File(_)))
        {
            self.disarm();
            return false;
        }
        self.delete(backward, repeat, now)
    }
    /// Returns true for both arm and removal; repeated token deletion is inert.
    pub fn delete(&mut self, backward: bool, repeat: bool, now: u64) -> bool {
        let mut range = self.state.selection.clone();
        if range.is_empty() {
            if backward && range.start > 0 {
                range.start -= 1;
            } else if !backward && range.end < self.state.units.len() {
                range.end += 1;
            }
        }
        if range.is_empty() {
            self.disarm();
            return false;
        }
        let file = self.state.units[range.clone()].iter().find_map(|u| {
            if let Unit::File(f) = u {
                Some(f.id().to_owned())
            } else {
                None
            }
        });
        if let Some(id) = file {
            if repeat {
                return true;
            }
            if self.armed.as_ref() != Some(&id) {
                self.armed = Some(id);
                self.state.selection = range;
                return true;
            }
        }
        self.state.units.drain(range.clone());
        self.state.selection = range.start..range.start;
        self.commit(now, false);
        true
    }
    /// Native word/line deletion supplies its exact UTF-16 range; no placeholder
    /// newline or zero-width DOM sentinel is synthesized (N3 quirk policy).
    pub fn delete_range(&mut self, range: Range<usize>, now: u64) -> Result<()> {
        let start = self.index(range.start)?;
        let end = self.index(range.end)?;
        if start > end
            || self.state.units[start..end]
                .iter()
                .any(|u| matches!(u, Unit::File(_)))
        {
            return Err(Error::Invalid("deletion intersects attachment"));
        }
        self.state.units.drain(start..end);
        self.state.selection = start..start;
        self.commit(now, false);
        Ok(())
    }
    pub fn undo(&mut self, redo: bool) -> bool {
        let next = if redo {
            self.cursor.checked_add(1)
        } else {
            self.cursor.checked_sub(1)
        };
        let Some(next) = next.filter(|n| *n < self.history.len()) else {
            return false;
        };
        self.cursor = next;
        self.state = self.history[next].clone();
        self.end();
        self.disarm();
        self.last_typing = None;
        true
    }
    pub fn clipboard(&mut self, cut: bool, now: u64) -> Clipboard {
        let mut parts = vec![];
        let mut plain = String::new();
        for unit in &self.state.units[self.state.selection.clone()] {
            match unit {
                Unit::Char(c) => {
                    plain.push(*c);
                    if let Some(Part::Text(text)) = parts.last_mut() {
                        text.push(*c);
                    } else {
                        parts.push(Part::Text(c.to_string()));
                    }
                }
                Unit::File(f) => {
                    plain.push_str(f.name());
                    parts.push(Part::File(f.id().into()));
                }
            }
        }
        if cut && !self.state.selection.is_empty() {
            let start = self.state.selection.start;
            self.state.units.drain(self.state.selection.clone());
            self.state.selection = start..start;
            self.commit(now, false);
        }
        Clipboard { parts, plain }
    }
    pub fn paste_parts(&mut self, parts: &[Part], now: u64) -> Result<()> {
        if parts.len() > 1000 {
            return Err(Error::Invalid("clipboard parts limit"));
        }
        for part in parts {
            if let Part::File(id) = part
                && !self.catalog.contains_key(id)
            {
                return Err(Error::UnknownId(id.clone()));
            }
        }
        self.preserve_selected_files();
        // Remove the selected text, then move known tokens through add_files.
        let mut staged = self.clone();
        staged.insert(vec![]);
        let mut seen = std::collections::HashSet::new();
        for part in parts {
            match part {
                Part::Text(text) => staged.insert(text.chars().map(Unit::Char).collect()),
                Part::File(id) if seen.insert(id) => {
                    let file = staged.catalog[id].clone();
                    staged.add_files(vec![file], now)?;
                }
                Part::File(_) => {}
            }
        }
        self.state = staged.state;
        self.commit(now, false);
        Ok(())
    }
    /// Programmatic suggestion/slash drafts retain attachments and local Undo.
    pub fn set_text(&mut self, text: &str, now: u64) -> Result<()> {
        let (_, mut files) = self.legacy();
        let end = text.encode_utf16().count();
        let mut boundaries = vec![0];
        let mut offset = 0;
        for c in text.chars() {
            offset += c.len_utf16();
            boundaries.push(offset);
        }
        for f in &mut files {
            let old = f["promptOffset"].as_u64().unwrap_or(end as u64) as usize;
            let at = old.min(end);
            f["promptOffset"] = json!(
                boundaries
                    .iter()
                    .copied()
                    .take_while(|p| *p <= at)
                    .last()
                    .unwrap_or(0)
            );
        }
        let next = Self::from_legacy(text, &files)?;
        self.state = next.state;
        self.commit(now, false);
        Ok(())
    }
    pub fn clear_after_send(&mut self) {
        *self = Self::default();
    }
}
