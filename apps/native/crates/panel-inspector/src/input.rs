//! Platform-independent input reducers. Host coordinates/focus never enter the
//! document. Constants below are shipped behavior, not native visual decisions.
use crate::InspectorError;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub ctrl: bool,
}
impl Modifiers {
    pub fn command(self) -> bool {
        self.meta || self.ctrl
    }
    pub fn scrub_multiplier(self) -> f64 {
        if self.shift {
            10.0
        } else if self.alt {
            0.1
        } else {
            1.0
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericSpec {
    pub step: f64,
    pub speed: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
}
impl Default for NumericSpec {
    fn default() -> Self {
        Self {
            step: 1.0,
            speed: 0.5,
            min: None,
            max: None,
        }
    }
}
impl NumericSpec {
    pub fn validate(self) -> Result<Self, InspectorError> {
        if !self.step.is_finite()
            || self.step <= 0.0
            || !self.speed.is_finite()
            || self.speed <= 0.0
            || self.min.is_some_and(|n| !n.is_finite())
            || self.max.is_some_and(|n| !n.is_finite())
            || matches!((self.min,self.max),(Some(a),Some(b)) if a>b)
        {
            Err(InspectorError::InvalidInput)
        } else {
            Ok(self)
        }
    }
    fn clamp(self, value: f64, places: i32) -> f64 {
        let mut v = value;
        if let Some(min) = self.min {
            v = v.max(min);
        }
        if let Some(max) = self.max {
            v = v.min(max);
        }
        round(v, places)
    }
    pub fn step_value(self, current: f64, up: bool, mods: Modifiers) -> f64 {
        self.clamp(
            if current.is_finite() { current } else { 0.0 }
                + if up { self.step } else { -self.step } * if mods.shift { 10.0 } else { 1.0 },
            4,
        )
    }
    pub fn slider_key(self, current: f64, key: &str, mods: Modifiers) -> Option<f64> {
        match key {
            "ArrowUp" | "ArrowRight" => Some(self.step_value(current, true, mods)),
            "ArrowDown" | "ArrowLeft" => Some(self.step_value(current, false, mods)),
            "Home" => self.min,
            "End" => self.max,
            _ => None,
        }
    }
}
/// Values a native event adapter turns into edit begin/step/finish calls.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEffect {
    Decline,
    Begin,
    Consume,
    OpenEditor,
    Write(f64),
    Commit,
    Cancel,
}
#[derive(Clone, Debug)]
pub struct NumericInput {
    spec: NumericSpec,
    start: f64,
    current: f64,
    moved: bool,
    pub draft: Option<String>,
    initial_draft: Option<String>,
    active: bool,
}
impl NumericInput {
    pub fn new(spec: NumericSpec, current: f64) -> Result<Self, InspectorError> {
        spec.validate()?;
        if !current.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        Ok(Self {
            spec,
            start: current,
            current,
            moved: false,
            draft: None,
            initial_draft: None,
            active: false,
        })
    }
    pub fn pointer_begin(&mut self, primary_button: bool) -> InputEffect {
        if !primary_button || self.draft.is_some() || self.active {
            return InputEffect::Decline;
        }
        self.start = self.current;
        self.moved = false;
        self.active = true;
        InputEffect::Begin
    }
    /// Cumulative movement from gesture start; right/up increase the value.
    pub fn pointer_move(&mut self, dx: f64, dy: f64, mods: Modifiers) -> InputEffect {
        if !self.active || !dx.is_finite() || !dy.is_finite() || (!self.moved && dx.hypot(dy) < 3.0)
        {
            return InputEffect::Decline;
        }
        self.moved = true;
        self.current = self.spec.clamp(
            self.start + (dx - dy) * self.spec.step * self.spec.speed * mods.scrub_multiplier(),
            3,
        );
        InputEffect::Write(self.current)
    }
    pub fn pointer_end(&mut self) -> InputEffect {
        if !self.active {
            return InputEffect::Decline;
        }
        self.active = false;
        if self.moved {
            InputEffect::Commit
        } else {
            self.open();
            InputEffect::OpenEditor
        }
    }
    pub fn wheel_begin(&mut self) -> bool {
        if self.active || self.draft.is_some() {
            return false;
        }
        self.start = self.current;
        self.active = true;
        true
    }
    pub fn wheel(&mut self, delta: f64, mods: Modifiers) -> InputEffect {
        if !self.active || !delta.is_finite() {
            return InputEffect::Decline;
        }
        let before = self.spec.clamp(self.current, 4);
        let raw = self.current + delta * self.spec.step * self.spec.speed * mods.scrub_multiplier();
        self.current = raw
            .max(self.spec.min.unwrap_or(f64::NEG_INFINITY))
            .min(self.spec.max.unwrap_or(f64::INFINITY));
        let after = self.spec.clamp(self.current, 4);
        if before == after {
            InputEffect::Decline
        } else {
            InputEffect::Write(after)
        }
    }
    pub fn wheel_end(&mut self) -> InputEffect {
        if !self.active {
            return InputEffect::Decline;
        }
        self.active = false;
        InputEffect::Commit
    }
    pub fn cancel(&mut self) -> InputEffect {
        if self.active {
            self.active = false;
            self.current = self.start;
            InputEffect::Cancel
        } else if self.draft.take().is_some() {
            self.initial_draft = None;
            InputEffect::Cancel
        } else {
            InputEffect::Decline
        }
    }
    pub fn open(&mut self) {
        let draft = round(self.current, 3).to_string();
        self.draft = Some(draft.clone());
        self.initial_draft = Some(draft);
    }
    pub fn set_draft(&mut self, draft: String) {
        if self.draft.is_some() {
            self.draft = Some(draft);
        }
    }
    /// Like the tag, typed values are validated by the semantic command; min /
    /// max clamps apply to stepping and scrubbing, not to typed arithmetic.
    pub fn finish_draft(&mut self, commit: bool) -> InputEffect {
        let Some(draft) = self.draft.take() else {
            return InputEffect::Decline;
        };
        let changed = self
            .initial_draft
            .take()
            .is_some_and(|initial| draft.trim() != initial);
        if commit
            && changed
            && let Ok(value) = parse_draft(&draft, self.current)
        {
            let value = round(value, 4);
            self.current = value;
            return InputEffect::Write(value);
        }
        if commit {
            InputEffect::Commit
        } else {
            InputEffect::Cancel
        }
    }
    pub fn key(&mut self, key: &str, mods: Modifiers) -> InputEffect {
        if self.active && key == "Escape" {
            return self.cancel();
        }
        if let Some(draft) = self.draft.as_mut() {
            match key {
                "Enter" => return self.finish_draft(true),
                "Escape" => return self.finish_draft(false),
                "ArrowUp" | "ArrowDown" => {
                    let current = parse_float_prefix(draft).unwrap_or(0.0);
                    let value = current
                        + if key == "ArrowUp" {
                            self.spec.step
                        } else {
                            -self.spec.step
                        } * if mods.shift { 10.0 } else { 1.0 };
                    *draft = round(value, 4).to_string();
                    return InputEffect::Consume;
                }
                _ => return InputEffect::Consume,
            }
        }
        match key {
            "Enter" | "F2" => {
                self.open();
                InputEffect::OpenEditor
            }
            "ArrowUp" | "ArrowDown" => {
                self.current = self.spec.step_value(self.current, key == "ArrowUp", mods);
                InputEffect::Write(self.current)
            }
            _ => InputEffect::Consume,
        }
    }
}
fn parse_float_prefix(s: &str) -> Option<f64> {
    let s = s.trim_start();
    (1..=s.len())
        .rev()
        .filter(|i| s.is_char_boundary(*i))
        .find_map(|i| s[..i].parse::<f64>().ok())
        .filter(|n| n.is_finite())
}
fn round(n: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    (n * factor + 0.5).floor() / factor
}
pub fn toggle_key(current: bool, key: &str, disabled: bool) -> Option<bool> {
    toggle_key_mixed(current, key, disabled, false)
}
/// The tag resolves a mixed switch to On for Space/Enter.
pub fn toggle_key_mixed(current: bool, key: &str, disabled: bool, mixed: bool) -> Option<bool> {
    if disabled {
        return None;
    }
    match key {
        "ArrowLeft" | "ArrowUp" => Some(false),
        "ArrowRight" | "ArrowDown" => Some(true),
        " " | "Enter" => Some(mixed || !current),
        _ => None,
    }
}
/// Safe recursive-descent arithmetic, never JavaScript evaluation.
pub fn parse_draft(source: &str, current: f64) -> Result<f64, InspectorError> {
    let raw = source.trim();
    let expanded = if let Some(rest) = raw.strip_prefix("-=") {
        format!("{current}-{rest}")
    } else if raw.starts_with(['+', '*', '/']) {
        format!("{current}{raw}")
    } else {
        raw.to_owned()
    };
    if expanded.len() > 4096 {
        return Err(InspectorError::InvalidInput);
    }
    let mut parser = Arithmetic {
        bytes: expanded.as_bytes(),
        at: 0,
        depth: 0,
    };
    let value = parser.expr()?;
    parser.ws();
    if parser.at != parser.bytes.len() || !value.is_finite() {
        Err(InspectorError::InvalidInput)
    } else {
        Ok(value)
    }
}
struct Arithmetic<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}
impl Arithmetic<'_> {
    fn ws(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn take(&mut self, c: u8) -> bool {
        self.ws();
        if self.bytes.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<f64, InspectorError> {
        let mut n = self.term()?;
        loop {
            if self.take(b'+') {
                n += self.term()?;
            } else if self.take(b'-') {
                n -= self.term()?;
            } else {
                return Ok(n);
            }
        }
    }
    fn term(&mut self) -> Result<f64, InspectorError> {
        let mut n = self.atom()?;
        loop {
            if self.take(b'*') {
                n *= self.atom()?;
            } else if self.take(b'/') {
                n /= self.atom()?;
            } else {
                return Ok(n);
            }
        }
    }
    fn atom(&mut self) -> Result<f64, InspectorError> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(InspectorError::InvalidInput);
        }
        let result = self.atom_inner();
        self.depth -= 1;
        result
    }
    fn atom_inner(&mut self) -> Result<f64, InspectorError> {
        if self.take(b'+') {
            return self.atom();
        }
        if self.take(b'-') {
            return Ok(-self.atom()?);
        }
        if self.take(b'(') {
            let n = self.expr()?;
            return if self.take(b')') {
                Ok(n)
            } else {
                Err(InspectorError::InvalidInput)
            };
        }
        self.ws();
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.at += 1;
        }
        if self
            .bytes
            .get(self.at)
            .is_some_and(|b| *b == b'e' || *b == b'E')
        {
            self.at += 1;
            if self
                .bytes
                .get(self.at)
                .is_some_and(|b| *b == b'+' || *b == b'-')
            {
                self.at += 1;
            }
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .ok_or(InspectorError::InvalidInput)
    }
}
