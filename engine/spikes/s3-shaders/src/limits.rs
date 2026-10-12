//! Resource limits for untrusted GLSL (plan section 11: "Store GLSL is
//! untrusted input: the pipeline is fuzzed and resource-limited").
//!
//! Two kinds of rule, both checked before the expensive stages:
//!
//! * WebGL's own validation limits, reproduced so the engine rejects what
//!   WebGL (ANGLE in Chromium) rejects: struct nesting deeper than 4, a call
//!   stack deeper than 256 (counting `main`), a private variable above 64 KiB,
//!   more than 1024 active fragment uniform vectors. Measured against WebGL with
//!   `scripts/limit-probes.ts`; see S3.md, "Resource limits".
//! * Budgets where WebGL has no rule but glslang or SPIRV-Cross would hang or
//!   exhaust memory: source size and preprocessor expansion. These are far
//!   above any real shader (the largest corpus program is 11.5 KB and expands
//!   to about 3,000 tokens) and are where the engine deliberately stops earlier
//!   than ANGLE, which keeps going for seconds.
//!
//! The source checks run on the author's program before glslang: a small
//! GLSL ES preprocessor counts expanded tokens with a budget (so a macro bomb
//! costs at most the budget), then scans the expanded tokens for struct
//! nesting. The SPIR-V checks (`check_spirv`) run on glslang's output, where
//! overloads are resolved and types are exact.

use std::collections::{HashMap, HashSet};

use rspirv::dr::{Module, Operand};
use rspirv::spirv::{Decoration, Op, StorageClass, Word};

/// Source bytes, bounding lexer memory only (comments are cheap; tokens are
/// the real budget). ANGLE accepted a 4 MiB program of comments in 11 s.
pub const MAX_SOURCE_BYTES: usize = 8 << 20;
/// Tokens after macro expansion. ANGLE accepted about 33 million (2^23 macro
/// doubling, 2.6 to 7 s) and failed near 67 million; glslang needs 5.6 s for
/// the 4.2 million of a 2^20 doubling, so the engine stops at 6 million (a
/// 2^21 doubling, 8.4 million, is refused).
pub const MAX_EXPANDED_TOKENS: usize = 6_000_000;
/// Statements (`;` after expansion). glslang's front end is quadratic in the
/// length of one function body: 55,000 statements (a 1 MiB program) take 4.4 s
/// per glslang pass and the pipeline runs two; 220,000 (4 MiB) did not finish
/// in 60 s. ANGLE accepted the 1 MiB program and rejected the 4 MiB one.
pub const MAX_STATEMENTS: usize = 65_536;
/// Nested macro expansions (ANGLE: `maxMacroExpansionDepth` 1000).
pub const MAX_MACRO_DEPTH: usize = 1000;
/// WebGL: "structure nesting level exceeds 4".
pub const MAX_STRUCT_NESTING: u32 = 4;
/// ANGLE in Chromium: call stack depth 256, counting `main`.
pub const MAX_CALL_DEPTH: u32 = 256;
/// ANGLE: "Size of declared private variable exceeds implementation-defined limit".
pub const MAX_PRIVATE_VARIABLE_BYTES: u64 = 64 * 1024;
/// `MAX_FRAGMENT_UNIFORM_VECTORS` reported by WebGL on Apple GPUs.
pub const MAX_FRAGMENT_UNIFORM_VECTORS: u64 = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitError {
    pub rule: &'static str,
    pub message: String,
    /// 1-based program line, when the rule points at one.
    pub line: Option<u32>,
}

impl LimitError {
    fn new(rule: &'static str, message: impl Into<String>, line: Option<u32>) -> Self {
        LimitError { rule, message: message.into(), line }
    }
}

impl std::fmt::Display for LimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "ERROR: 0:{line}: {} ({})", self.message, self.rule),
            None => write!(f, "ERROR: {} ({})", self.message, self.rule),
        }
    }
}

/* ── tokens ─────────────────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ident,
    Number,
    Punct,
}

#[derive(Debug, Clone, Copy)]
struct Tok<'a> {
    text: &'a str,
    kind: Kind,
    line: u32,
    /// Whitespace precedes the token (decides function-like `#define`).
    space: bool,
}

/// One logical line: either a directive (`#` first) or text tokens.
struct Line<'a> {
    directive: bool,
    tokens: Vec<Tok<'a>>,
}

const PUNCT3: &[&str] = &["<<=", ">>="];
const PUNCT2: &[&str] = &["++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "^^", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^="];

/// Split the source into logical lines of tokens, dropping comments and
/// joining `\`-continued lines as GLSL ES 3.00 does.
fn lex(source: &str) -> Vec<Line<'_>> {
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut current = Line { directive: false, tokens: Vec::new() };
    let mut line = 1u32;
    let mut space = true;
    let mut i = 0;
    let mut at_line_start = true;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && bytes.get(i + 1) == Some(&b'\n') {
            i += 2;
            line += 1;
            space = true;
            continue;
        }
        if c == b'\n' {
            lines.push(std::mem::replace(&mut current, Line { directive: false, tokens: Vec::new() }));
            i += 1;
            line += 1;
            space = true;
            at_line_start = true;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            space = true;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            space = true;
            continue;
        }
        if c == b'#' && at_line_start && current.tokens.is_empty() {
            current.directive = true;
        }
        at_line_start = false;
        let start = i;
        let kind = if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            Kind::Ident
        } else if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let hex = c == b'0' && matches!(bytes.get(i + 1), Some(b'x' | b'X'));
            i += 1;
            while i < bytes.len() {
                let d = bytes[i];
                if d.is_ascii_alphanumeric() || d == b'.' || d == b'_' {
                    i += 1;
                } else if !hex && (d == b'+' || d == b'-') && matches!(bytes[i - 1], b'e' | b'E') {
                    i += 1;
                } else {
                    break;
                }
            }
            Kind::Number
        } else {
            let rest = &source[i..];
            let len = PUNCT3.iter().chain(PUNCT2).find(|p| rest.starts_with(**p)).map_or_else(|| rest.chars().next().map_or(1, char::len_utf8), |p| p.len());
            i += len;
            Kind::Punct
        };
        current.tokens.push(Tok { text: &source[start..i], kind, line, space });
        space = false;
    }
    lines.push(current);
    lines
}

/* ── preprocessor with a budget ─────────────────────────────────────── */

struct Macro<'a> {
    params: Option<Vec<&'a str>>,
    body: Vec<Tok<'a>>,
}

struct Expander<'a> {
    macros: HashMap<&'a str, Macro<'a>>,
    /// Tokens produced anywhere (arguments, nested bodies, output).
    work: usize,
}

impl<'a> Expander<'a> {
    fn charge(&mut self, n: usize, line: u32) -> Result<(), LimitError> {
        self.work += n;
        if self.work > 4 * MAX_EXPANDED_TOKENS {
            return Err(LimitError::new("preprocessor", format!("macro expansion exceeds {} tokens", MAX_EXPANDED_TOKENS), Some(line)));
        }
        Ok(())
    }

    /// Expand `input` into `out`. Self-reference is blocked with `disabled`,
    /// arguments are fully expanded before substitution, and the result is
    /// rescanned. This is the C preprocessor model GLSL uses, simplified where
    /// it only changes token order, not counts (no `#`/`##` in GLSL ES).
    fn expand(&mut self, input: &[Tok<'a>], disabled: &mut Vec<&'a str>, out: &mut Vec<Tok<'a>>, depth: usize) -> Result<(), LimitError> {
        if depth > MAX_MACRO_DEPTH {
            let line = input.first().map(|t| t.line);
            return Err(LimitError::new("preprocessor", "macro expansion nested too deeply", line));
        }
        let mut i = 0;
        while i < input.len() {
            let t = input[i];
            if t.kind == Kind::Ident && !disabled.contains(&t.text) {
                if let Some(m) = self.macros.get(t.text) {
                    match &m.params {
                        None => {
                            let body = m.body.clone();
                            disabled.push(t.text);
                            self.expand(&body, disabled, out, depth + 1)?;
                            disabled.pop();
                            i += 1;
                            continue;
                        }
                        Some(params) if input.get(i + 1).is_some_and(|n| n.text == "(") => {
                            let params = params.clone();
                            let body = m.body.clone();
                            // Collect arguments up to the matching parenthesis.
                            let mut args: Vec<Vec<Tok<'a>>> = vec![Vec::new()];
                            let mut level = 0usize;
                            let mut j = i + 2;
                            let mut closed = false;
                            while j < input.len() {
                                let a = input[j];
                                match a.text {
                                    "(" => level += 1,
                                    ")" if level == 0 => {
                                        closed = true;
                                        break;
                                    }
                                    ")" => level -= 1,
                                    "," if level == 0 => {
                                        args.push(Vec::new());
                                        j += 1;
                                        continue;
                                    }
                                    _ => {}
                                }
                                args.last_mut().unwrap().push(a);
                                j += 1;
                            }
                            if !closed {
                                // Unterminated invocation: glslang reports it.
                                self.charge(1, t.line)?;
                                out.push(t);
                                i += 1;
                                continue;
                            }
                            let mut expanded_args = Vec::with_capacity(args.len());
                            for arg in &args {
                                let mut e = Vec::new();
                                self.expand(arg, disabled, &mut e, depth + 1)?;
                                expanded_args.push(e);
                            }
                            let mut substituted = Vec::new();
                            for b in &body {
                                if let Some(k) = params.iter().position(|p| *p == b.text) {
                                    let arg = expanded_args.get(k).map_or(&[][..], |a| a.as_slice());
                                    self.charge(arg.len(), b.line)?;
                                    substituted.extend_from_slice(arg);
                                } else {
                                    self.charge(1, b.line)?;
                                    substituted.push(*b);
                                }
                            }
                            disabled.push(t.text);
                            self.expand(&substituted, disabled, out, depth + 1)?;
                            disabled.pop();
                            i = j + 1;
                            continue;
                        }
                        Some(_) => {}
                    }
                }
            }
            self.charge(1, t.line)?;
            out.push(t);
            i += 1;
        }
        Ok(())
    }
}

/// Evaluate a `#if` expression (after `defined` and macro replacement).
/// Returns `None` when it cannot be evaluated; the caller then treats the
/// branch as active, which over-counts tokens but never under-counts.
fn eval_if(tokens: &[Tok<'_>]) -> Option<i64> {
    struct P<'t, 'a> {
        t: &'t [Tok<'a>],
        i: usize,
    }
    impl P<'_, '_> {
        fn peek(&self) -> Option<&str> {
            self.t.get(self.i).map(|t| t.text)
        }
        fn primary(&mut self) -> Option<i64> {
            let tok = *self.t.get(self.i)?;
            self.i += 1;
            match tok.text {
                "(" => {
                    let v = self.binary(0)?;
                    (self.peek() == Some(")")).then(|| self.i += 1)?;
                    Some(v)
                }
                "-" => self.primary().map(i64::wrapping_neg),
                "+" => self.primary(),
                "!" => self.primary().map(|v| i64::from(v == 0)),
                "~" => self.primary().map(|v| !v),
                _ if tok.kind == Kind::Number => {
                    let s = tok.text.trim_end_matches(['u', 'U']);
                    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                        i64::from_str_radix(h, 16).ok()
                    } else if s.len() > 1 && s.starts_with('0') {
                        i64::from_str_radix(&s[1..], 8).ok()
                    } else {
                        s.parse().ok()
                    }
                }
                // An identifier left after macro replacement is 0 in C; GLSL
                // makes it an error. Either way the value is not meaningful.
                _ => None,
            }
        }
        fn binary(&mut self, min: u8) -> Option<i64> {
            let mut left = self.primary()?;
            loop {
                let Some(op) = self.peek() else { break };
                let prec = match op {
                    "||" => 1,
                    "&&" => 2,
                    "|" => 3,
                    "^" => 4,
                    "&" => 5,
                    "==" | "!=" => 6,
                    "<" | ">" | "<=" | ">=" => 7,
                    "<<" | ">>" => 8,
                    "+" | "-" => 9,
                    "*" | "/" | "%" => 10,
                    _ => break,
                };
                if prec < min {
                    break;
                }
                let op = op.to_string();
                self.i += 1;
                let right = self.binary(prec + 1)?;
                left = match op.as_str() {
                    "||" => i64::from(left != 0 || right != 0),
                    "&&" => i64::from(left != 0 && right != 0),
                    "|" => left | right,
                    "^" => left ^ right,
                    "&" => left & right,
                    "==" => i64::from(left == right),
                    "!=" => i64::from(left != right),
                    "<" => i64::from(left < right),
                    ">" => i64::from(left > right),
                    "<=" => i64::from(left <= right),
                    ">=" => i64::from(left >= right),
                    "<<" => left.wrapping_shl(right as u32),
                    ">>" => left.wrapping_shr(right as u32),
                    "+" => left.wrapping_add(right),
                    "-" => left.wrapping_sub(right),
                    "*" => left.wrapping_mul(right),
                    "/" => left.checked_div(right)?,
                    _ => left.checked_rem(right)?,
                };
            }
            Some(left)
        }
    }
    let mut p = P { t: tokens, i: 0 };
    let v = p.binary(0)?;
    (p.i == tokens.len()).then_some(v)
}

/// `#if`/`#elif`: replace `defined X` and `defined(X)`, expand macros, evaluate.
fn condition<'a>(ex: &mut Expander<'a>, rest: &[Tok<'a>]) -> Result<bool, LimitError> {
    let mut replaced = Vec::new();
    let mut k = 0;
    while k < rest.len() {
        if rest[k].text == "defined" {
            let (id, skip) = if rest.get(k + 1).is_some_and(|t| t.text == "(") { (rest.get(k + 2), 4) } else { (rest.get(k + 1), 2) };
            let value = if id.is_some_and(|id| ex.macros.contains_key(id.text)) { "1" } else { "0" };
            replaced.push(Tok { text: value, kind: Kind::Number, line: rest[k].line, space: true });
            k += skip;
            continue;
        }
        replaced.push(rest[k]);
        k += 1;
    }
    let mut expanded = Vec::new();
    ex.expand(&replaced, &mut Vec::new(), &mut expanded, 0)?;
    Ok(eval_if(&expanded).is_none_or(|v| v != 0))
}

/// Run the preprocessor over `source` within the budget; returns the
/// expanded tokens of the active code.
fn preprocess(source: &str) -> Result<Vec<Tok<'_>>, LimitError> {
    let mut ex = Expander { macros: HashMap::new(), work: 0 };
    let mut out = Vec::new();
    // Conditional stack: (this branch active, some branch already taken, parent active).
    let mut stack: Vec<(bool, bool, bool)> = Vec::new();
    let active = |stack: &Vec<(bool, bool, bool)>| stack.last().is_none_or(|s| s.0);
    let mut pending: Vec<Tok<'_>> = Vec::new();
    let lines = lex(source);
    for line in &lines {
        if !line.directive {
            if active(&stack) {
                pending.extend_from_slice(&line.tokens);
            }
            continue;
        }
        // Text before a directive is complete (invocations cannot span directives).
        if !pending.is_empty() {
            ex.expand(&pending, &mut Vec::new(), &mut out, 0)?;
            pending.clear();
            if out.len() > MAX_EXPANDED_TOKENS {
                return Err(LimitError::new("preprocessor", format!("program exceeds {MAX_EXPANDED_TOKENS} tokens after macro expansion"), out.last().map(|t| t.line)));
            }
        }
        let t = &line.tokens;
        let name = t.get(1).map_or("", |t| t.text);
        match name {
            "define" if active(&stack) => {
                let Some(id) = t.get(2) else { continue };
                let function_like = t.get(3).is_some_and(|p| p.text == "(" && !p.space);
                let (params, body_start) = if function_like {
                    let close = t[3..].iter().position(|p| p.text == ")").map_or(t.len(), |p| p + 3);
                    (Some(t[4..close.min(t.len())].iter().filter(|p| p.text != ",").map(|p| p.text).collect()), close + 1)
                } else {
                    (None, 3)
                };
                let body = t.get(body_start..).map_or_else(Vec::new, <[Tok<'_>]>::to_vec);
                ex.macros.insert(id.text, Macro { params, body });
            }
            "undef" if active(&stack) => {
                if let Some(id) = t.get(2) {
                    ex.macros.remove(id.text);
                }
            }
            "ifdef" | "ifndef" => {
                let parent = active(&stack);
                let defined = t.get(2).is_some_and(|id| ex.macros.contains_key(id.text));
                let on = parent && (defined == (name == "ifdef"));
                stack.push((on, on, parent));
            }
            "if" => {
                let parent = active(&stack);
                let on = parent && condition(&mut ex, t.get(2..).unwrap_or_default())?;
                stack.push((on, on, parent));
            }
            "elif" => {
                if let Some(&(_, taken, parent)) = stack.last() {
                    let on = parent && !taken && condition(&mut ex, t.get(2..).unwrap_or_default())?;
                    *stack.last_mut().unwrap() = (on, taken || on, parent);
                }
            }
            "else" => {
                if let Some(&(_, taken, parent)) = stack.last() {
                    *stack.last_mut().unwrap() = (parent && !taken, true, parent);
                }
            }
            "endif" => {
                stack.pop();
            }
            // GLSL ES 3.00 (and ANGLE): `#extension` must precede every
            // non-preprocessor token. glslang's ES front end does not check
            // this; the fuzz found it.
            "extension" if active(&stack) && !out.is_empty() => {
                return Err(LimitError::new("es-rule", "extension directive must occur before any non-preprocessor tokens in ES", t.first().map(|t| t.line)));
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        ex.expand(&pending, &mut Vec::new(), &mut out, 0)?;
    }
    if out.len() > MAX_EXPANDED_TOKENS {
        return Err(LimitError::new("preprocessor", format!("program exceeds {MAX_EXPANDED_TOKENS} tokens after macro expansion"), out.last().map(|t| t.line)));
    }
    Ok(out)
}

/// WebGL's struct nesting rule over expanded tokens: a struct whose member is
/// a struct of nesting level 4 is rejected.
fn check_struct_nesting(tokens: &[Tok<'_>]) -> Result<(), LimitError> {
    let mut levels: HashMap<&str, u32> = HashMap::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].text != "struct" {
            i += 1;
            continue;
        }
        let (name, open) = match (tokens.get(i + 1), tokens.get(i + 2)) {
            (Some(n), Some(o)) if n.kind == Kind::Ident && o.text == "{" => (Some(n.text), i + 2),
            (Some(o), _) if o.text == "{" => (None, i + 1),
            _ => {
                i += 1;
                continue;
            }
        };
        let mut level = 1u32;
        let mut j = open + 1;
        let mut declaration_start = true;
        while j < tokens.len() && tokens[j].text != "}" {
            let tok = tokens[j];
            if declaration_start && tok.kind == Kind::Ident && !matches!(tok.text, "lowp" | "mediump" | "highp" | "const" | "invariant") {
                if let Some(inner) = levels.get(tok.text) {
                    if inner + 1 > MAX_STRUCT_NESTING {
                        return Err(LimitError::new(
                            "struct-nesting",
                            format!("Reference of struct type {} exceeds maximum allowed nesting level of {MAX_STRUCT_NESTING}", tok.text),
                            Some(tok.line),
                        ));
                    }
                    level = level.max(inner + 1);
                }
                declaration_start = false;
            }
            if tok.text == ";" {
                declaration_start = true;
            }
            j += 1;
        }
        if let Some(name) = name {
            levels.insert(name, level);
        }
        i = j + 1;
    }
    Ok(())
}

/// Source-level limits on the author's program, before glslang.
pub fn check_source(source: &str) -> Result<(), LimitError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(LimitError::new("source-size", format!("program is {} bytes; the limit is {MAX_SOURCE_BYTES}", source.len()), None));
    }
    let tokens = preprocess(source)?;
    let statements = tokens.iter().filter(|t| t.text == ";").count();
    if statements > MAX_STATEMENTS {
        return Err(LimitError::new("statements", format!("program has {statements} statements; the limit is {MAX_STATEMENTS}"), None));
    }
    check_struct_nesting(&tokens)
}

/* ── SPIR-V checks (exact types and resolved overloads) ─────────────── */

struct Types<'m> {
    defs: HashMap<Word, &'m rspirv::dr::Instruction>,
    constants: HashMap<Word, u64>,
}

impl<'m> Types<'m> {
    fn new(module: &'m Module) -> Self {
        let mut defs = HashMap::new();
        let mut constants = HashMap::new();
        for inst in &module.types_global_values {
            if let Some(id) = inst.result_id {
                defs.insert(id, inst);
                if inst.class.opcode == Op::Constant {
                    if let Some(Operand::LiteralBit32(v)) = inst.operands.first() {
                        constants.insert(id, u64::from(*v));
                    }
                }
            }
        }
        Types { defs, constants }
    }

    fn id(inst: &rspirv::dr::Instruction, k: usize) -> Option<Word> {
        match inst.operands.get(k) {
            Some(Operand::IdRef(id)) => Some(*id),
            _ => None,
        }
    }

    /// Bytes as ANGLE counts a variable: 4 per scalar component, arrays and
    /// structs summed (a `mat4[1024]` is exactly 64 KiB).
    fn bytes(&self, ty: Word) -> u64 {
        let Some(inst) = self.defs.get(&ty) else { return 0 };
        match inst.class.opcode {
            Op::TypeBool | Op::TypeInt | Op::TypeFloat => 4,
            Op::TypeVector | Op::TypeMatrix => {
                let n = match inst.operands.get(1) {
                    Some(Operand::LiteralBit32(n)) => u64::from(*n),
                    _ => 1,
                };
                Self::id(inst, 0).map_or(0, |e| self.bytes(e)).saturating_mul(n)
            }
            Op::TypeArray => {
                let len = Self::id(inst, 1).and_then(|c| self.constants.get(&c)).copied().unwrap_or(1);
                Self::id(inst, 0).map_or(0, |e| self.bytes(e)).saturating_mul(len)
            }
            Op::TypeStruct => inst.operands.iter().filter_map(|o| if let Operand::IdRef(m) = o { Some(self.bytes(*m)) } else { None }).fold(0u64, u64::saturating_add),
            Op::TypePointer => Self::id(inst, 1).map_or(0, |p| self.bytes(p)),
            _ => 0,
        }
    }

    /// Lower bound on the uniform vectors a type needs under GLSL ES packing
    /// (Appendix A.7): components packed perfectly into 4-wide rows, except
    /// that every array element and matrix column starts a new row.
    fn vectors(&self, ty: Word) -> f64 {
        let Some(inst) = self.defs.get(&ty) else { return 0.0 };
        match inst.class.opcode {
            Op::TypeBool | Op::TypeInt | Op::TypeFloat => 0.25,
            Op::TypeVector => match inst.operands.get(1) {
                Some(Operand::LiteralBit32(n)) => f64::from(*n) / 4.0,
                _ => 1.0,
            },
            Op::TypeMatrix => match inst.operands.get(1) {
                Some(Operand::LiteralBit32(n)) => f64::from(*n),
                _ => 1.0,
            },
            Op::TypeArray => {
                let len = Self::id(inst, 1).and_then(|c| self.constants.get(&c)).copied().unwrap_or(1) as f64;
                let element = Self::id(inst, 0).map_or(0.0, |e| self.vectors(e));
                element.ceil().max(1.0) * len
            }
            Op::TypeStruct => inst.operands.iter().filter_map(|o| if let Operand::IdRef(m) = o { Some(self.vectors(*m)) } else { None }).sum(),
            _ => 0.0,
        }
    }
}

/// WebGL limits that need resolved types and overloads: private variable
/// size, call stack depth and recursion, active uniform vectors.
///
/// `active_uniform_members` lists the default-block member indices the
/// program statically uses (see `reflect`).
pub fn check_spirv(module: &Module, active_uniform_members: &HashSet<u32>) -> Result<(), LimitError> {
    let types = Types::new(module);
    let names = crate::rewrite::names(module);
    let name_of = |id: Word| names.get(&id).cloned().unwrap_or_else(|| format!("%{id}"));
    // Private variables: module-scope `Private` and function-scope `Function`.
    let function_variables = module.functions.iter().flat_map(|f| f.blocks.iter()).flat_map(|b| b.instructions.iter());
    for inst in module.types_global_values.iter().chain(function_variables) {
        if inst.class.opcode != Op::Variable {
            continue;
        }
        if !matches!(inst.operands.first(), Some(Operand::StorageClass(StorageClass::Private | StorageClass::Function))) {
            continue;
        }
        let (Some(ty), Some(id)) = (inst.result_type, inst.result_id) else { continue };
        let size = types.bytes(ty);
        if size > MAX_PRIVATE_VARIABLE_BYTES {
            return Err(LimitError::new("private-variable-size", format!("'{}' : Size of declared private variable ({size} bytes) exceeds implementation-defined limit ({MAX_PRIVATE_VARIABLE_BYTES})", name_of(id)), None));
        }
    }
    // Call graph: depth from the entry point, counting it; recursion.
    let mut calls: HashMap<Word, Vec<Word>> = HashMap::new();
    for function in &module.functions {
        let Some(id) = function.def.as_ref().and_then(|d| d.result_id) else { continue };
        let callees = function.blocks.iter().flat_map(|b| b.instructions.iter()).filter(|i| i.class.opcode == Op::FunctionCall).filter_map(|i| Types::id(i, 0)).collect::<HashSet<_>>();
        calls.insert(id, callees.into_iter().collect());
    }
    let entry = module.entry_points.first().and_then(|e| Types::id(e, 1));
    if let Some(entry) = entry {
        let mut depth: HashMap<Word, u32> = HashMap::new();
        // Iterative DFS with an explicit stack; `0` marks "on the stack".
        let mut stack = vec![(entry, 0usize)];
        depth.insert(entry, 0);
        while let Some(&(f, k)) = stack.last() {
            let callees = calls.get(&f).map_or(&[][..], Vec::as_slice);
            if k < callees.len() {
                stack.last_mut().unwrap().1 += 1;
                let g = callees[k];
                match depth.get(&g) {
                    Some(0) => return Err(LimitError::new("recursion", format!("Recursive function call in the call chain through {}", name_of(g)), None)),
                    Some(_) => {}
                    None => {
                        depth.insert(g, 0);
                        stack.push((g, 0));
                    }
                }
            } else {
                let d = 1 + callees.iter().map(|g| depth[g]).max().unwrap_or(0);
                depth.insert(f, d);
                stack.pop();
            }
        }
        if depth[&entry] > MAX_CALL_DEPTH {
            return Err(LimitError::new("call-depth", format!("Call stack too deep (larger than {MAX_CALL_DEPTH})"), None));
        }
    }
    // Active uniform vectors: default-block members the program uses, plus samplers do not count.
    let block_struct = module.types_global_values.iter().find_map(|inst| {
        if inst.class.opcode != Op::TypeStruct {
            return None;
        }
        let id = inst.result_id?;
        let is_block = module.annotations.iter().any(|a| a.class.opcode == Op::Decorate && Types::id(a, 0) == Some(id) && matches!(a.operands.get(1), Some(Operand::Decoration(Decoration::Block))));
        (is_block && names.get(&id).is_some_and(|n| n == "gl_DefaultUniformBlock")).then_some(inst)
    });
    if let Some(block) = block_struct {
        let used: f64 = block
            .operands
            .iter()
            .enumerate()
            .filter(|(k, _)| active_uniform_members.contains(&(*k as u32)))
            .filter_map(|(_, o)| if let Operand::IdRef(m) = o { Some(types.vectors(*m)) } else { None })
            .sum();
        if used.ceil() as u64 > MAX_FRAGMENT_UNIFORM_VECTORS {
            return Err(LimitError::new("uniform-vectors", format!("FRAGMENT shader uniforms count exceeds MAX_FRAGMENT_UNIFORM_VECTORS({MAX_FRAGMENT_UNIFORM_VECTORS})"), None));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "#version 300 es\nprecision highp float;\nout vec4 o;\n";

    fn doubling(n: usize) -> String {
        let mut s = String::from(H);
        s.push_str("#define A0 1.0\n");
        for i in 1..=n {
            s.push_str(&format!("#define A{i} (A{p} + A{p})\n", p = i - 1));
        }
        s.push_str(&format!("void main(){{ o = vec4(A{n}); }}\n"));
        s
    }

    #[test]
    fn macro_bombs_stop_at_the_budget_quickly() {
        assert!(check_source(&doubling(20)).is_ok(), "ANGLE accepts a 2^20 doubling in 0.6 s");
        let started = std::time::Instant::now();
        let error = check_source(&doubling(40)).unwrap_err();
        assert_eq!(error.rule, "preprocessor");
        assert!(started.elapsed().as_secs_f64() < 2.0, "{:?}", started.elapsed());
        let mut f = String::from(H);
        f.push_str("#define F0(x) (x)\n");
        for i in 1..=40 {
            f.push_str(&format!("#define F{i}(x) F{p}(x) + F{p}(x)\n", p = i - 1));
        }
        f.push_str("void main(){ o = vec4(F40(1.0)); }\n");
        assert_eq!(check_source(&f).unwrap_err().rule, "preprocessor");
    }

    #[test]
    fn dead_branches_and_conditionals_are_honored() {
        let mut s = String::from(H);
        s.push_str("#if 0\n");
        s.push_str(&doubling(40)[H.len()..].replace("void main(){ o = vec4(A40); }\n", "float x = A40;\n"));
        s.push_str("#endif\n#define ON 1\n#if defined(ON) && ON > 0\nvoid main(){ o = vec4(1.0); }\n#else\nstruct A{float x;}; struct B{A a;}; struct C{B b;}; struct D{C c;}; struct E{D d;};\n#endif\n");
        check_source(&s).expect("dead code is not expanded or checked");
    }

    #[test]
    fn function_like_needs_an_adjacent_parenthesis() {
        // `#define P (x)` is object-like: P expands to `(x)`.
        let s = format!("{H}#define P (1.0)\n#define F(a) a * a\nvoid main(){{ o = vec4(F(P) + F(2.0)); }}\n");
        let tokens = preprocess(&s).unwrap();
        let text: Vec<&str> = tokens.iter().map(|t| t.text).collect();
        let joined = text.join(" ");
        assert!(joined.contains("( 1.0 ) * ( 1.0 ) + 2.0 * 2.0"), "{joined}");
    }

    #[test]
    fn struct_nesting_follows_webgl() {
        let nest = |n: usize| {
            let mut s = String::from(H);
            s.push_str("struct S0 { float x; };\n");
            for i in 1..n {
                s.push_str(&format!("struct S{i} {{ highp S{p} a; float b; }};\n", p = i - 1));
            }
            s.push_str("void main(){ o = vec4(1.0); }\n");
            s
        };
        check_source(&nest(4)).expect("4 levels");
        let e = check_source(&nest(5)).unwrap_err();
        assert_eq!(e.rule, "struct-nesting");
        assert_eq!(e.line, Some(8));
    }

    #[test]
    fn statements_are_capped() {
        let many = |n: usize| format!("{H}void main(){{ float f = 0.0;\n{} o = vec4(f); }}\n", "f += 1.0;\n".repeat(n));
        check_source(&many(MAX_STATEMENTS - 4)).expect("4 statements around the loop body");
        assert_eq!(check_source(&many(MAX_STATEMENTS - 3)).unwrap_err().rule, "statements");
    }

    #[test]
    fn extension_directives_must_come_first() {
        check_source("#version 300 es\n#extension GL_EXT_shader_texture_lod : enable\nprecision highp float;\nout vec4 o;\nvoid main(){ o = vec4(1.0); }\n").expect("before code");
        let late = format!("{H}#extension GL_EXT_shader_texture_lod : enable\nvoid main(){{ o = vec4(1.0); }}\n");
        let e = check_source(&late).unwrap_err();
        assert_eq!((e.rule, e.line), ("es-rule", Some(4)));
    }

    #[test]
    fn source_size_is_capped() {
        let comments = format!("{H}// {}\nvoid main(){{ o = vec4(1.0); }}\n", "x".repeat(4 << 20));
        check_source(&comments).expect("4 MiB of comments (ANGLE accepts it)");
        let big = format!("{H}// {}\nvoid main(){{ o = vec4(1.0); }}\n", "x".repeat(MAX_SOURCE_BYTES));
        assert_eq!(check_source(&big).unwrap_err().rule, "source-size");
    }

    fn spirv(source: &str) -> Module {
        let spv = crate::compile::compile(source, crate::compile::Stage::Fragment).expect("compiles");
        crate::rewrite::load(&spv.words).unwrap()
    }

    fn all_members() -> HashSet<u32> {
        (0..64).collect()
    }

    #[test]
    fn private_variables_above_64_kib_are_rejected() {
        let ok = spirv("#version 450\nuniform vec2 u; out vec4 o; void main(){ float a[16384]; a[int(u.x)] = 1.0; o = vec4(a[int(u.y)]); }\n");
        check_spirv(&ok, &all_members()).expect("64 KiB");
        let big = spirv("#version 450\nuniform vec2 u; out vec4 o; void main(){ float a[16385]; a[int(u.x)] = 1.0; o = vec4(a[int(u.y)]); }\n");
        assert_eq!(check_spirv(&big, &all_members()).unwrap_err().rule, "private-variable-size");
        let mat = spirv("#version 450\nuniform vec2 u; out vec4 o; mat4 g[1025]; void main(){ g[int(u.x)] = mat4(1.0); o = g[int(u.y)][0]; }\n");
        assert_eq!(check_spirv(&mat, &all_members()).unwrap_err().rule, "private-variable-size");
    }

    #[test]
    fn call_depth_counts_main_and_shares_subtrees() {
        let chain = |d: usize| {
            let mut s = String::from("#version 450\nuniform vec2 u; out vec4 o;\nfloat g0(float x){ return x + 1.0; }\n");
            for i in 1..d {
                s.push_str(&format!("float g{i}(float x){{ return g{p}(x) + g{p}(x * 0.5); }}\n", p = i - 1));
            }
            s.push_str(&format!("void main(){{ o = vec4(g{}(u.x)); }}\n", d - 1));
            s
        };
        check_spirv(&spirv(&chain(255)), &all_members()).expect("255 functions plus main");
        assert_eq!(check_spirv(&spirv(&chain(256)), &all_members()).unwrap_err().rule, "call-depth");
    }

    #[test]
    fn active_uniform_vectors_are_limited() {
        let used = |n: usize| spirv(&format!("#version 450\nuniform vec4 big[{n}]; uniform vec2 u_res; out vec4 o; void main(){{ o = big[int(u_res.x)]; }}\n"));
        check_spirv(&used(1023), &all_members()).expect("1023 + 1");
        assert_eq!(check_spirv(&used(1024), &all_members()).unwrap_err().rule, "uniform-vectors");
        check_spirv(&used(5000), &HashSet::from([1])).expect("an inactive array does not count");
    }
}
