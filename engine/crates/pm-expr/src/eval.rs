//! The bytecode evaluator of `expression.ts`, with JavaScript's
//! conversions, and the dependency recorder.

use std::rc::Rc;

use crate::bytecode::{Call, Input, LiteralKey, MathMember, Op, Program};
use crate::deps::{ContextReads, Dependencies, FieldRead, Root, Step};
use crate::host::{Host, Shape};
use crate::jsmath;
use crate::parser::{BinaryOp, Literal, OBJECT_PROTOTYPE, UnaryOp};
use crate::string::JsStr;
use crate::value::{
    Helper, MATH_CONSTANTS, MathFunction, Object, Value, number_to_string, string_to_number,
};
use crate::{Context, Dialect};

/// A JavaScript exception thrown while evaluating, with V8's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    /// Converting a document object whose own `toString` property is not a
    /// function (JSON values never are).
    ToPrimitive,
    /// An `Object.prototype` method reached through the inherited lookups
    /// (`valueOf()`, `Math.hasOwnProperty(1)`, …) runs with `this` undefined.
    NullishToObject,
    /// `toLocaleString()` the same way.
    ToLocaleString,
    /// `__proto__()`: `Object.prototype` is not callable. The message names the
    /// local variable in `expression.ts` (`math`), as the unminified source does.
    NotAFunction,
    /// `param()` when the composition itself is `undefined` or `null`.
    ReadOfNullish { base: &'static str, key: String },
}

impl EvalError {
    pub fn message(&self) -> String {
        match self {
            EvalError::ToPrimitive => "Cannot convert object to primitive value".into(),
            EvalError::NullishToObject => "Cannot convert undefined or null to object".into(),
            EvalError::ToLocaleString => {
                "Object.prototype.toLocaleString called on null or undefined".into()
            }
            EvalError::NotAFunction => "math is not a function".into(),
            EvalError::ReadOfNullish { base, key } => {
                format!("Cannot read properties of {base} (reading '{key}')")
            }
        }
    }
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for EvalError {}

/// A read path as a linked list from the leaf up, so extending it is O(1);
/// flattened into [`FieldRead`]s once, when the evaluation finishes.
#[derive(Debug)]
struct Path {
    root: Root,
    parent: Option<Rc<Path>>,
    step: Option<Step>,
}

impl Path {
    fn root(root: Root) -> Rc<Path> {
        Rc::new(Path {
            root,
            parent: None,
            step: None,
        })
    }

    fn to_field_read(&self) -> FieldRead {
        let mut steps = Vec::new();
        let mut node = Some(self);
        while let Some(n) = node {
            if let Some(step) = &n.step {
                steps.push(step.clone());
            }
            node = n.parent.as_deref();
        }
        steps.reverse();
        FieldRead {
            root: self.root,
            path: steps,
        }
    }
}

/// Keys the interpreter itself looks up, allocated once.
struct Keys {
    to_string: JsStr,
    params: JsStr,
    value: JsStr,
}

fn keys() -> &'static Keys {
    static KEYS: std::sync::OnceLock<Keys> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| Keys {
        to_string: JsStr::from("toString"),
        params: JsStr::from("params"),
        value: JsStr::from("value"),
    })
}

/// A document handle plus the path it was reached by.
#[derive(Clone, Debug)]
pub(crate) struct Tracked<O> {
    obj: O,
    path: Option<Rc<Path>>,
}

impl<O> Tracked<O> {
    pub(crate) fn into_obj(self) -> O {
        self.obj
    }
}

impl<O: PartialEq> PartialEq for Tracked<O> {
    fn eq(&self, other: &Self) -> bool {
        self.obj == other.obj
    }
}

type V<O> = Value<Tracked<O>>;
type R<T> = Result<T, EvalError>;

/// A property reached through an inherited prototype member: a function (or
/// `Object.prototype`), truthy, whose `value` property is `undefined`.
enum Got<O> {
    Value(V<O>),
    Inherited,
}

const FORBIDDEN_MEMBERS: [&str; 3] = ["__proto__", "prototype", "constructor"];

const ARRAY_PROTOTYPE: [&str; 40] = [
    "length",
    "constructor",
    "at",
    "concat",
    "copyWithin",
    "fill",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "lastIndexOf",
    "pop",
    "push",
    "reverse",
    "shift",
    "unshift",
    "slice",
    "sort",
    "splice",
    "includes",
    "indexOf",
    "join",
    "keys",
    "entries",
    "values",
    "forEach",
    "filter",
    "flat",
    "flatMap",
    "map",
    "every",
    "some",
    "reduce",
    "reduceRight",
    "toReversed",
    "toSorted",
    "toSpliced",
    "with",
    "toLocaleString",
    "toString",
];
const STRING_PROTOTYPE: [&str; 52] = [
    "length",
    "constructor",
    "anchor",
    "at",
    "big",
    "blink",
    "bold",
    "charAt",
    "charCodeAt",
    "codePointAt",
    "concat",
    "endsWith",
    "fontcolor",
    "fontsize",
    "fixed",
    "includes",
    "indexOf",
    "isWellFormed",
    "italics",
    "lastIndexOf",
    "link",
    "localeCompare",
    "match",
    "matchAll",
    "normalize",
    "padEnd",
    "padStart",
    "repeat",
    "replace",
    "replaceAll",
    "search",
    "slice",
    "small",
    "split",
    "strike",
    "sub",
    "substr",
    "substring",
    "sup",
    "startsWith",
    "toString",
    "toWellFormed",
    "trim",
    "trimStart",
    "trimLeft",
    "trimEnd",
    "trimRight",
    "toLocaleLowerCase",
    "toLocaleUpperCase",
    "toLowerCase",
    "toUpperCase",
    "valueOf",
];
const NUMBER_PROTOTYPE: [&str; 7] = [
    "constructor",
    "toExponential",
    "toFixed",
    "toPrecision",
    "toString",
    "valueOf",
    "toLocaleString",
];
const BOOLEAN_PROTOTYPE: [&str; 3] = ["constructor", "toString", "valueOf"];
const FUNCTION_PROTOTYPE: [&str; 9] = [
    "length",
    "name",
    "constructor",
    "apply",
    "bind",
    "call",
    "toString",
    "arguments",
    "caller",
];

fn in_object_prototype(key: &JsStr) -> bool {
    OBJECT_PROTOTYPE.iter().any(|(name, _)| key.eq_str(name))
}

fn in_list(key: &JsStr, list: &[&str]) -> bool {
    list.iter().any(|name| key.eq_str(name)) || in_object_prototype(key)
}

/// `Number.isSafeInteger(n)`.
pub(crate) fn is_safe_integer(n: f64) -> bool {
    n.is_finite() && n.trunc() == n && n.abs() <= 9_007_199_254_740_991.0
}

/// A canonical array index string (`"0"`, `"12"`, not `"01"` or `"-0"`).
fn array_index(key: &JsStr) -> Option<u64> {
    let units = key.units();
    if units.is_empty() || units.len() > 16 || (units.len() > 1 && units[0] == u16::from(b'0')) {
        return None;
    }
    let mut n: u64 = 0;
    for &u in units {
        if !(0x30..=0x39).contains(&u) {
            return None;
        }
        n = n * 10 + u64::from(u - 0x30);
    }
    Some(n)
}

/// What `MATH_FUNCTIONS[name]` finds: an own function, an inherited
/// `Object.prototype` member, or nothing.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MathLookup {
    Function(MathFunction),
    /// `Object.prototype[name]`; `None` for `__proto__`.
    Inherited(Option<&'static str>),
    Missing,
}

pub(crate) fn math_lookup(name: &JsStr) -> MathLookup {
    if let Some(&f) = MathFunction::ALL.iter().find(|f| name.eq_str(f.name())) {
        return MathLookup::Function(f);
    }
    for (key, function) in OBJECT_PROTOTYPE {
        if name.eq_str(key) {
            return MathLookup::Inherited(function.map(|_| key));
        }
    }
    MathLookup::Missing
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hint {
    Default,
    Number,
    String,
}

pub(crate) struct Evaluator<'a, H: Host> {
    host: &'a H,
    ctx: &'a Context<H::Obj>,
    dialect: Dialect,
    context: ContextReads,
    reads: Vec<Rc<Path>>,
    tracking: bool,
}

impl<'a, H: Host> Evaluator<'a, H> {
    pub fn new(host: &'a H, ctx: &'a Context<H::Obj>, dialect: Dialect) -> Self {
        Evaluator {
            host,
            ctx,
            dialect,
            context: ContextReads::empty(),
            reads: Vec::new(),
            tracking: true,
        }
    }

    pub fn untracked(host: &'a H, ctx: &'a Context<H::Obj>, dialect: Dialect) -> Self {
        let mut out = Self::new(host, ctx, dialect);
        out.tracking = false;
        out
    }

    /// What the evaluation read, sorted and deduplicated.
    pub fn dependencies(&self) -> Dependencies {
        let mut fields: Vec<FieldRead> = self.reads.iter().map(|p| p.to_field_read()).collect();
        fields.sort();
        fields.dedup();
        Dependencies {
            context: self.context,
            fields,
        }
    }

    // ── document reads ──────────────────────────────────────────────────

    fn root(&mut self, root: Root) -> V<H::Obj> {
        let value = match root {
            Root::Layer => &self.ctx.layer,
            Root::Comp => &self.ctx.comp,
            Root::Value => &self.ctx.value,
        };
        let path = self.tracking.then(|| Path::root(root));
        if let Some(path) = &path {
            self.reads.push(path.clone());
        }
        value.clone().map_doc(|obj| Tracked { obj, path })
    }

    fn record(&mut self, parent: &Tracked<H::Obj>, step: Step) -> Option<Rc<Path>> {
        let parent_path = parent.path.as_ref()?;
        let path = Rc::new(Path {
            root: parent_path.root,
            parent: Some(parent_path.clone()),
            step: Some(step),
        });
        self.reads.push(path.clone());
        Some(path)
    }

    fn length(&mut self, array: &Tracked<H::Obj>) -> usize {
        self.record(array, Step::Length);
        match self.host.shape(&array.obj) {
            Shape::Array { len } => len,
            Shape::Object => 0,
        }
    }

    fn element(&mut self, array: &Tracked<H::Obj>, index: u64) -> V<H::Obj> {
        let path = self.record(array, Step::Index(index));
        let len = match self.host.shape(&array.obj) {
            Shape::Array { len } => len,
            Shape::Object => 0,
        };
        if index >= len as u64 {
            return Value::Undefined;
        }
        self.host
            .element(&array.obj, index as usize)
            .map_doc(|obj| Tracked { obj, path })
    }

    fn property(&mut self, object: &Tracked<H::Obj>, key: &JsStr) -> Option<V<H::Obj>> {
        let path = if self.tracking {
            self.record(object, Step::Key(key.clone()))
        } else {
            None
        };
        self.host
            .property(&object.obj, key)
            .map(|v| v.map_doc(|obj| Tracked { obj, path }))
    }

    fn is_array(&self, t: &Tracked<H::Obj>) -> Option<usize> {
        match self.host.shape(&t.obj) {
            Shape::Array { len } => Some(len),
            Shape::Object => None,
        }
    }

    // ── conversions ─────────────────────────────────────────────────────

    /// `String(value)` for a value from outside an evaluation.
    pub fn string_of(&mut self, value: &Value<H::Obj>) -> R<JsStr> {
        let path = Some(Path::root(Root::Value));
        let value = value.clone().map_doc(|obj| Tracked { obj, path });
        self.js_string(&value)
    }

    /// `OrdinaryToPrimitive` for the objects expressions can hold.
    fn js_primitive(&mut self, value: &V<H::Obj>, hint: Hint) -> R<V<H::Obj>> {
        let Value::Object(object) = value else {
            return Ok(value.clone());
        };
        Ok(match object {
            Object::Doc(t) => {
                if self.is_array(t).is_some() {
                    Value::String(self.join(t)?)
                } else {
                    // A non-callable own `toString` (every JSON value) makes
                    // both conversion orders fail; otherwise the inherited
                    // `Object.prototype.toString` answers.
                    if self.property(t, &keys().to_string).is_some() {
                        return Err(EvalError::ToPrimitive);
                    }
                    Value::String(JsStr::from("[object Object]"))
                }
            }
            Object::Math | Object::Callable(_) | Object::PlainObject => {
                Value::String(JsStr::from("[object Object]"))
            }
            Object::MathFunction(f) => Value::String(JsStr::from(format!(
                "function {}() {{ [native code] }}",
                f.name()
            ))),
            Object::NumberObject(n) => match hint {
                Hint::String => Value::String(number_to_string(*n)),
                Hint::Number | Hint::Default => Value::Number(*n),
            },
        })
    }

    /// `Array.prototype.join(",")` on a document array.
    fn join(&mut self, array: &Tracked<H::Obj>) -> R<JsStr> {
        let len = self.length(array);
        let mut units: Vec<u16> = Vec::new();
        for i in 0..len {
            if i > 0 {
                units.push(u16::from(b','));
            }
            let element = self.element(array, i as u64);
            if matches!(element, Value::Undefined | Value::Null) {
                continue;
            }
            units.extend_from_slice(self.js_string(&element)?.units());
        }
        Ok(JsStr::from_units(units))
    }

    /// `Number(value)`.
    fn js_number(&mut self, value: &V<H::Obj>) -> R<f64> {
        Ok(match value {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Number(n) => *n,
            Value::String(s) => string_to_number(s),
            Value::Object(_) => {
                let primitive = self.js_primitive(value, Hint::Number)?;
                self.js_number(&primitive)?
            }
        })
    }

    /// `String(value)`.
    fn js_string(&mut self, value: &V<H::Obj>) -> R<JsStr> {
        Ok(match value {
            Value::Undefined => JsStr::from("undefined"),
            Value::Null => JsStr::from("null"),
            Value::Bool(b) => JsStr::from(if *b { "true" } else { "false" }),
            Value::Number(n) => number_to_string(*n),
            Value::String(s) => s.clone(),
            Value::Object(_) => {
                let primitive = self.js_primitive(value, Hint::String)?;
                self.js_string(&primitive)?
            }
        })
    }

    fn strict_equal(a: &V<H::Obj>, b: &V<H::Obj>) -> bool {
        match (a, b) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Number(x), Value::Number(y)) => x == y,
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Object(x), Value::Object(y)) => match (x, y) {
                (Object::Doc(p), Object::Doc(q)) => p == q,
                (Object::Math, Object::Math) => true,
                (Object::MathFunction(p), Object::MathFunction(q)) => p == q,
                // Callable, Number, and plain objects are created fresh by
                // each evaluation of their node, so no two are ever the same.
                _ => false,
            },
            _ => false,
        }
    }

    /// `IsLooselyEqual(a, b)`.
    fn loose_equal(&mut self, a: &V<H::Obj>, b: &V<H::Obj>) -> R<bool> {
        use Value as J;
        Ok(match (a, b) {
            (J::Undefined | J::Null, J::Undefined | J::Null) => true,
            (J::Bool(_), J::Bool(_))
            | (J::Number(_), J::Number(_))
            | (J::String(_), J::String(_))
            | (J::Object(_), J::Object(_)) => Self::strict_equal(a, b),
            (J::Number(x), J::String(y)) => *x == string_to_number(y),
            (J::String(x), J::Number(y)) => string_to_number(x) == *y,
            (J::Bool(x), _) => return self.loose_equal(&J::Number(f64::from(u8::from(*x))), b),
            (_, J::Bool(y)) => return self.loose_equal(a, &J::Number(f64::from(u8::from(*y)))),
            (J::Number(_) | J::String(_), J::Object(_)) => {
                let primitive = self.js_primitive(b, Hint::Default)?;
                return self.loose_equal(a, &primitive);
            }
            (J::Object(_), J::Number(_) | J::String(_)) => {
                let primitive = self.js_primitive(a, Hint::Default)?;
                return self.loose_equal(&primitive, b);
            }
            _ => false,
        })
    }

    // ── evaluation ──────────────────────────────────────────────────────

    pub fn run(&mut self, program: &Program) -> R<V<H::Obj>> {
        // The compiler computes the maximum active stack, bounded by the parser.
        let mut stack: smallvec::SmallVec<[V<H::Obj>; 8]> =
            smallvec::SmallVec::with_capacity(program.stack);
        let pop = |stack: &mut smallvec::SmallVec<[V<H::Obj>; 8]>| {
            stack.pop().expect("the compiler balances the stack")
        };
        let mut pc = 0;
        while let Some(op) = program.code.get(pc) {
            pc += 1;
            let value = match op {
                Op::Literal(v) => match v {
                    Literal::Number(n) => Value::Number(*n),
                    Literal::String(s) => Value::String(s.clone()),
                    Literal::Bool(b) => Value::Bool(*b),
                    Literal::Null => Value::Null,
                },
                Op::Identifier(name) => self.identifier(*name),
                Op::Unary(op) => {
                    let v = pop(&mut stack);
                    match op {
                        UnaryOp::Not => Value::Bool(!v.truthy()),
                        UnaryOp::Minus => Value::Number(-self.js_number(&v)?),
                        UnaryOp::Plus => Value::Number(self.js_number(&v)?),
                    }
                }
                Op::Binary(op) => {
                    let r = pop(&mut stack);
                    let l = pop(&mut stack);
                    self.binary(*op, &l, &r)?
                }
                Op::Member => {
                    let k = pop(&mut stack);
                    let o = pop(&mut stack);
                    self.member(&o, &k)?
                }
                Op::MemberKey(key) => {
                    let o = pop(&mut stack);
                    self.member_key(&o, key)
                }
                Op::Call(call, count) => {
                    let start = stack.len() - count;
                    let args = &stack[start..];
                    let result = match call {
                        Call::Helper(h) => self.helper(*h, args),
                        Call::Math(m) => self.call_math(*m, args),
                        Call::Missing => Ok(Value::Undefined),
                    };
                    stack.truncate(start);
                    result?
                }
                Op::Short(truthy, target) => {
                    if stack.last().expect("operand").truthy() == *truthy {
                        pc = *target;
                    } else {
                        stack.pop();
                    }
                    continue;
                }
                Op::Branch(target) => {
                    if !pop(&mut stack).truthy() {
                        pc = *target;
                    }
                    continue;
                }
                Op::Jump(target) => {
                    pc = *target;
                    continue;
                }
            };
            stack.push(value);
        }
        debug_assert_eq!(stack.len(), 1);
        Ok(pop(&mut stack))
    }

    fn identifier(&mut self, name: Input) -> V<H::Obj> {
        let ctx = self.ctx;
        match name {
            Input::Time => {
                self.context.insert(ContextReads::TIME);
                Value::Number(ctx.time)
            }
            Input::Pi => Value::Number(std::f64::consts::PI),
            Input::Math => Value::Object(Object::Math),
            Input::LocalTime => {
                self.context.insert(ContextReads::LOCAL_TIME);
                Value::Number(ctx.t)
            }
            Input::Fps => {
                self.context.insert(ContextReads::FPS);
                Value::Number(ctx.fps)
            }
            Input::Value => {
                self.context.insert(ContextReads::VALUE);
                self.root(Root::Value)
            }
            Input::Layer => self.root(Root::Layer),
            Input::Comp => self.root(Root::Comp),
            Input::Channel => {
                self.context.insert(ContextReads::CHANNEL);
                Value::String(ctx.channel.clone())
            }
            Input::Index => {
                self.context.insert(ContextReads::INDEX);
                Value::Number(ctx.idx)
            }
            Input::Frame if self.dialect == Dialect::Native => {
                self.context.insert(ContextReads::FRAME);
                Value::Number(ctx.frame)
            }
            Input::Callable(c) => Value::Object(Object::Callable(c)),
            _ => Value::Undefined,
        }
    }

    fn binary(&mut self, op: BinaryOp, left: &V<H::Obj>, right: &V<H::Obj>) -> R<V<H::Obj>> {
        use BinaryOp as B;
        Ok(match op {
            B::Add => {
                if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                    let l = self.js_string(left)?;
                    let r = self.js_string(right)?;
                    Value::String(l.concat(&r))
                } else {
                    let l = self.js_number(left)?;
                    let r = self.js_number(right)?;
                    Value::Number(l + r)
                }
            }
            B::Sub | B::Mul | B::Div | B::Rem | B::Pow | B::Lt | B::Le | B::Gt | B::Ge => {
                let l = self.js_number(left)?;
                let r = self.js_number(right)?;
                match op {
                    B::Sub => Value::Number(l - r),
                    B::Mul => Value::Number(l * r),
                    B::Div => Value::Number(l / r),
                    B::Rem => Value::Number(l % r),
                    B::Pow => Value::Number(jsmath::pow(l, r)),
                    B::Lt => Value::Bool(l < r),
                    B::Le => Value::Bool(l <= r),
                    B::Gt => Value::Bool(l > r),
                    _ => Value::Bool(l >= r),
                }
            }
            B::Eq => Value::Bool(self.loose_equal(left, right)?),
            B::Ne => Value::Bool(!self.loose_equal(left, right)?),
            B::StrictEq => Value::Bool(Self::strict_equal(left, right)),
            B::StrictNe => Value::Bool(!Self::strict_equal(left, right)),
            B::Inert | B::And | B::Or => Value::Undefined,
        })
    }

    /// `member(object, property)` in `expression.ts`.
    fn member(&mut self, object: &V<H::Obj>, property: &V<H::Obj>) -> R<V<H::Obj>> {
        enum Key {
            Number(f64),
            String(JsStr),
        }
        let key = match property {
            Value::Number(n) => Key::Number(*n),
            other => Key::String(self.js_string(other)?),
        };
        if let Key::String(s) = &key {
            if FORBIDDEN_MEMBERS.iter().any(|f| s.eq_str(f)) {
                return Ok(Value::Undefined);
            }
            if matches!(object, Value::Object(Object::Math)) {
                return Ok(math_member(s));
            }
        }
        if let Value::Object(Object::Doc(t)) = object
            && self.is_array(t).is_some()
        {
            if let Key::String(s) = &key
                && s.eq_str("length")
            {
                return Ok(Value::Number(self.length(t) as f64));
            }
            let n = match &key {
                Key::Number(n) => *n,
                Key::String(s) => string_to_number(s),
            };
            if !is_safe_integer(n) || n < 0.0 {
                // Negative indices are ordinary properties no JSON array has.
                return Ok(Value::Undefined);
            }
            return Ok(self.element(t, n as u64));
        }
        if let (Value::String(s), Key::String(k)) = (object, &key)
            && k.eq_str("length")
        {
            return Ok(Value::Number(s.len() as f64));
        }
        let Value::Object(object) = object else {
            return Ok(Value::Undefined);
        };
        let key = match key {
            Key::Number(n) => number_to_string(n),
            Key::String(s) => s,
        };
        Ok(match object {
            Object::Doc(t) => self.property(t, &key).unwrap_or(Value::Undefined),
            Object::Callable(c) if key.eq_str("callable") => Value::String(JsStr::from(c.name())),
            // `typeof` a Math function is "function": no members.
            Object::MathFunction(_) => Value::Undefined,
            // No own properties (Math was handled above for string keys, and
            // no Math key is a number's string form).
            _ => Value::Undefined,
        })
    }

    /// `member(object, key)` for a literal key: the same answers and reads as
    /// [`member`](Self::member), with the key's conversions made at compile
    /// time (a literal key never throws).
    fn member_key(&mut self, object: &V<H::Obj>, key: &LiteralKey) -> V<H::Obj> {
        if !key.number {
            if key.forbidden {
                return Value::Undefined;
            }
            if matches!(object, Value::Object(Object::Math)) {
                return match key.math {
                    MathMember::Constant(n) => Value::Number(n),
                    MathMember::Function(f) => Value::Object(Object::MathFunction(f)),
                    MathMember::Undefined => Value::Undefined,
                };
            }
        }
        if let Value::Object(Object::Doc(t)) = object
            && self.is_array(t).is_some()
        {
            if key.length {
                return Value::Number(self.length(t) as f64);
            }
            // Negative indices are ordinary properties no JSON array has.
            return key.index.map_or(Value::Undefined, |i| self.element(t, i));
        }
        if let Value::String(s) = object
            && key.length
        {
            return Value::Number(s.len() as f64);
        }
        match object {
            Value::Object(Object::Doc(t)) => {
                self.property(t, &key.string).unwrap_or(Value::Undefined)
            }
            Value::Object(Object::Callable(c)) if key.callable => {
                Value::String(JsStr::from(c.name()))
            }
            _ => Value::Undefined,
        }
    }

    fn call_math(&mut self, lookup: MathLookup, args: &[V<H::Obj>]) -> R<V<H::Obj>> {
        if matches!(lookup, MathLookup::Missing) {
            return Ok(Value::Undefined);
        }
        let mut numbers: smallvec::SmallVec<[f64; 12]> = smallvec::SmallVec::new();
        for arg in args {
            numbers.push(self.js_number(arg)?);
        }
        match lookup {
            MathLookup::Missing => Ok(Value::Undefined),
            MathLookup::Function(f) => Ok(Value::Number(apply_math(f, &numbers))),
            MathLookup::Inherited(None) => Err(EvalError::NotAFunction),
            MathLookup::Inherited(Some(name)) => match name {
                // `Object(n)`: a Number wrapper, or `{}` without arguments.
                "constructor" => Ok(Value::Object(match numbers.first() {
                    Some(n) => Object::NumberObject(*n),
                    None => Object::PlainObject,
                })),
                "toString" => Ok(Value::String(JsStr::from("[object Undefined]"))),
                "toLocaleString" => Err(EvalError::ToLocaleString),
                // Its argument is a number, never an object.
                "isPrototypeOf" => Ok(Value::Bool(false)),
                _ => Err(EvalError::NullishToObject),
            },
        }
    }

    fn helper(&mut self, helper: Helper, args: &[V<H::Obj>]) -> R<V<H::Obj>> {
        let mut numbers: smallvec::SmallVec<[f64; 12]> = smallvec::SmallVec::new();
        for arg in args {
            numbers.push(self.js_number(arg)?);
        }
        if helper == Helper::Param {
            let name = match args.first() {
                None | Some(Value::Undefined | Value::Null) => JsStr::default(),
                Some(v) => self.js_string(v)?,
            };
            return self.param(&name);
        }
        if helper == Helper::Wiggle {
            self.context.insert(ContextReads::LOCAL_TIME);
        }
        Ok(apply_helper(helper, &numbers, self.ctx.t).map_or(Value::Undefined, Value::Number))
    }

    /// `paramGet(name)` in `anim.ts`: `comp.params && comp.params[name]`, then
    /// `p ? p.value : 0`, with ordinary JavaScript property lookups.
    fn param(&mut self, name: &JsStr) -> R<V<H::Obj>> {
        let comp = self.root(Root::Comp);
        let params = match self.get(Got::Value(comp), &keys().params)? {
            Got::Value(v) if v.truthy() => v,
            Got::Value(_) => return Ok(Value::Number(0.0)),
            Got::Inherited => unreachable!("\"params\" is no prototype member"),
        };
        let p = self.get(Got::Value(params), name)?;
        match p {
            Got::Value(v) if !v.truthy() => Ok(Value::Number(0.0)),
            p => match self.get(p, &keys().value)? {
                Got::Value(v) => Ok(v),
                Got::Inherited => unreachable!("\"value\" is no prototype member"),
            },
        }
    }

    /// `base[key]` (`[[Get]]`) for the values `param` walks through.
    fn get(&mut self, base: Got<H::Obj>, key: &JsStr) -> R<Got<H::Obj>> {
        let inherited = |hit: bool| {
            if hit {
                Got::Inherited
            } else {
                Got::Value(Value::Undefined)
            }
        };
        let Got::Value(base) = base else {
            // Inherited members are functions or Object.prototype; `param`
            // only asks them for `value`, which none has.
            return Ok(Got::Value(Value::Undefined));
        };
        Ok(match &base {
            Value::Undefined => {
                return Err(EvalError::ReadOfNullish {
                    base: "undefined",
                    key: key.to_string_lossy(),
                });
            }
            Value::Null => {
                return Err(EvalError::ReadOfNullish {
                    base: "null",
                    key: key.to_string_lossy(),
                });
            }
            Value::Bool(_) => inherited(in_list(key, &BOOLEAN_PROTOTYPE)),
            Value::Number(_) | Value::Object(Object::NumberObject(_)) => {
                inherited(in_list(key, &NUMBER_PROTOTYPE))
            }
            Value::String(s) => {
                if let Some(i) = array_index(key).filter(|&i| i < s.len() as u64) {
                    Got::Value(Value::String(JsStr::from_units(vec![
                        s.units()[i as usize],
                    ])))
                } else if key.eq_str("length") {
                    Got::Value(Value::Number(s.len() as f64))
                } else {
                    inherited(in_list(key, &STRING_PROTOTYPE))
                }
            }
            Value::Object(Object::Doc(t)) => {
                if self.is_array(t).is_some() {
                    if let Some(i) = array_index(key) {
                        let v = self.element(t, i);
                        if !matches!(v, Value::Undefined) {
                            return Ok(Got::Value(v));
                        }
                    }
                    if key.eq_str("length") {
                        Got::Value(Value::Number(self.length(t) as f64))
                    } else {
                        inherited(in_list(key, &ARRAY_PROTOTYPE))
                    }
                } else if let Some(v) = self.property(t, key) {
                    Got::Value(v)
                } else {
                    inherited(in_object_prototype(key))
                }
            }
            Value::Object(Object::Math) => {
                let own = math_member(key);
                if matches!(own, Value::Undefined) {
                    inherited(in_object_prototype(key))
                } else {
                    Got::Value(own)
                }
            }
            Value::Object(Object::MathFunction(_)) => inherited(in_list(key, &FUNCTION_PROTOTYPE)),
            Value::Object(Object::Callable(c)) => {
                if key.eq_str("callable") {
                    Got::Value(Value::String(JsStr::from(c.name())))
                } else {
                    inherited(in_object_prototype(key))
                }
            }
            Value::Object(Object::PlainObject) => inherited(in_object_prototype(key)),
        })
    }
}

/// `Object.hasOwn(MATH_VALUE, key) ? MATH_VALUE[key] : undefined`.
fn math_member<O>(key: &JsStr) -> Value<O> {
    if let Some((_, v)) = MATH_CONSTANTS.iter().find(|(name, _)| key.eq_str(name)) {
        return Value::Number(*v);
    }
    match MathFunction::ALL.iter().find(|f| key.eq_str(f.name())) {
        Some(&f) => Value::Object(Object::MathFunction(f)),
        None => Value::Undefined,
    }
}

/// `clamp(value, minimum, maximum)` in `expression.ts`.
fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

/// The same with a possibly absent value, which comes back unchanged.
fn clamp_opt(value: Option<f64>, minimum: f64, maximum: f64) -> Option<f64> {
    value.map(|v| clamp(v, minimum, maximum))
}

/// `a === b` where either may be `undefined`.
fn strict_equal_opt(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// The numeric helpers of `expression.ts` (every helper except `param`), on
/// arguments already converted with `Number()`. `t` is the layer-local time
/// `wiggle` reads. `None` is `undefined`: an absent argument returned
/// unchanged. The bytecode evaluator and the typed numeric plan share this one
/// implementation.
// `6.2831` is the literal `expression.ts` uses, not τ.
#[allow(clippy::approx_constant)]
pub(crate) fn apply_helper(helper: Helper, numbers: &[f64], t: f64) -> Option<f64> {
    let n = |i: usize| numbers.get(i).copied();
    // An absent argument is `undefined`: NaN in arithmetic and comparisons,
    // but returned as `undefined` where the helper returns it unchanged.
    let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
    Some(match helper {
        Helper::Clamp => return clamp_opt(n(0), nan(n(1)), nan(n(2))),
        Helper::Lerp => nan(n(0)) + (nan(n(1)) - nan(n(0))) * nan(n(2)),
        Helper::Linear => {
            let (x, x0, x1, y0, y1) = (nan(n(0)), n(1), n(2), n(3), nan(n(4)));
            if strict_equal_opt(x1, x0) {
                return y0;
            }
            let (x0, x1, y0) = (nan(x0), nan(x1), nan(y0));
            y0 + (y1 - y0) * clamp((x - x0) / (x1 - x0), 0.0, 1.0)
        }
        Helper::Ease => {
            let (x, x0, x1, y0, y1) = (nan(n(0)), n(1), n(2), nan(n(3)), nan(n(4)));
            let ratio = if strict_equal_opt(x1, x0) {
                0.0
            } else {
                (x - nan(x0)) / (nan(x1) - nan(x0))
            };
            let amount = clamp(ratio, 0.0, 1.0);
            y0 + (y1 - y0) * amount * amount * (3.0 - 2.0 * amount)
        }
        Helper::Random => {
            let seed = n(0).unwrap_or(1.0) * 127.1;
            let value = jsmath::sin(seed) * 43758.5453;
            value - jsmath::floor(value)
        }
        Helper::Wiggle => {
            let frequency = n(0).unwrap_or(0.0);
            let mut amplitude = n(1).unwrap_or(0.0);
            let seed = n(2).unwrap_or(0.0) * 17.3;
            let mut value = 0.0;
            let mut current = frequency;
            for index in 0..3 {
                let i = f64::from(index);
                value += amplitude
                    * (jsmath::sin(t * current * 6.2831 + seed + i * 2.4)
                        + jsmath::sin(t * current * 3.94 + seed * 1.7 + i))
                    * 0.5;
                amplitude *= 0.5;
                current *= 2.03;
            }
            value
        }
        Helper::Bounce => {
            let mut x = clamp(n(0).unwrap_or(0.0), 0.0, 1.0);
            if x < 1.0 / 2.75 {
                7.5625 * x * x
            } else if x < 2.0 / 2.75 {
                x -= 1.5 / 2.75;
                7.5625 * x * x + 0.75
            } else if x < 2.5 / 2.75 {
                x -= 2.25 / 2.75;
                7.5625 * x * x + 0.9375
            } else {
                x -= 2.625 / 2.75;
                7.5625 * x * x + 0.984375
            }
        }
        Helper::Loop => {
            if nan(n(0)) <= 0.0 {
                return n(1);
            }
            nan(n(1)) % nan(n(0))
        }
        Helper::Pingpong => {
            let duration = n(0).unwrap_or(0.0);
            if duration <= 0.0 {
                return n(1);
            }
            let position = nan(n(1)) % (duration * 2.0);
            if position < duration {
                position
            } else {
                duration * 2.0 - position
            }
        }
        // `param` reads the document; the evaluators handle it themselves.
        Helper::Param => return None,
    })
}

pub(crate) fn apply_math(f: MathFunction, n: &[f64]) -> f64 {
    let a = n.first().copied().unwrap_or(f64::NAN);
    let b = n.get(1).copied().unwrap_or(f64::NAN);
    match f {
        MathFunction::Abs => jsmath::abs(a),
        MathFunction::Acos => jsmath::acos(a),
        MathFunction::Asin => jsmath::asin(a),
        MathFunction::Atan => jsmath::atan(a),
        MathFunction::Atan2 => jsmath::atan2(a, b),
        MathFunction::Ceil => jsmath::ceil(a),
        MathFunction::Cos => jsmath::cos(a),
        MathFunction::Exp => jsmath::exp(a),
        MathFunction::Floor => jsmath::floor(a),
        MathFunction::Log => jsmath::log(a),
        MathFunction::Max => jsmath::max(n),
        MathFunction::Min => jsmath::min(n),
        MathFunction::Pow => jsmath::pow(a, b),
        MathFunction::Round => jsmath::round(a),
        MathFunction::Sign => jsmath::sign(a),
        MathFunction::Sin => jsmath::sin(a),
        MathFunction::Sqrt => jsmath::sqrt(a),
        MathFunction::Tan => jsmath::tan(a),
    }
}
