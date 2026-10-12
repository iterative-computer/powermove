//! The tree-walking interpreter of `expression.ts`, with JavaScript's
//! conversions, and the dependency recorder.

use std::rc::Rc;

use crate::deps::{ContextReads, Dependencies, FieldRead, Root, Step};
use crate::host::{Host, Shape};
use crate::jsmath;
use crate::parser::{BinaryOp, Literal, Node, OBJECT_PROTOTYPE, UnaryOp};
use crate::string::JsStr;
use crate::value::{
    Callable, Helper, MATH_CONSTANTS, MathFunction, Object, Value, number_to_string,
    string_to_number,
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

#[derive(Debug)]
struct Path {
    root: Root,
    steps: Vec<Step>,
}

/// A document handle plus the path it was reached by.
#[derive(Clone, Debug)]
pub(crate) struct Tracked<O> {
    obj: O,
    path: Rc<Path>,
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
fn is_safe_integer(n: f64) -> bool {
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
enum MathLookup {
    Function(MathFunction),
    /// `Object.prototype[name]`; `None` for `__proto__`.
    Inherited(Option<&'static str>),
    Missing,
}

fn math_lookup(name: &JsStr) -> MathLookup {
    if let Some(text) = name.to_rust_string() {
        if let Some(f) = MathFunction::from_name(&text) {
            return MathLookup::Function(f);
        }
        for (key, function) in OBJECT_PROTOTYPE {
            if text == key {
                return MathLookup::Inherited(function.map(|_| key));
            }
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
    pub deps: Dependencies,
}

impl<'a, H: Host> Evaluator<'a, H> {
    pub fn new(host: &'a H, ctx: &'a Context<H::Obj>, dialect: Dialect) -> Self {
        Evaluator {
            host,
            ctx,
            dialect,
            deps: Dependencies::default(),
        }
    }

    // ── document reads ──────────────────────────────────────────────────

    fn root(&mut self, root: Root) -> V<H::Obj> {
        let value = match root {
            Root::Layer => &self.ctx.layer,
            Root::Comp => &self.ctx.comp,
            Root::Value => &self.ctx.value,
        };
        self.deps.fields.push(FieldRead {
            root,
            path: Vec::new(),
        });
        let path = Rc::new(Path {
            root,
            steps: Vec::new(),
        });
        value.clone().map_doc(|obj| Tracked { obj, path })
    }

    fn record(&mut self, parent: &Tracked<H::Obj>, step: Step) -> Rc<Path> {
        let mut steps = parent.path.steps.clone();
        steps.push(step);
        self.deps.fields.push(FieldRead {
            root: parent.path.root,
            path: steps.clone(),
        });
        Rc::new(Path {
            root: parent.path.root,
            steps,
        })
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
        let path = self.record(object, Step::Key(key.clone()));
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
        let path = Rc::new(Path {
            root: Root::Value,
            steps: Vec::new(),
        });
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
                    if self.property(t, &JsStr::from("toString")).is_some() {
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

    pub fn evaluate(&mut self, node: &Node) -> R<V<H::Obj>> {
        match node {
            Node::Literal(literal) => Ok(match literal {
                Literal::Number(n) => Value::Number(*n),
                Literal::String(s) => Value::String(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Null => Value::Null,
            }),
            Node::Identifier(name) => Ok(self.identifier(name)),
            Node::Unary(op, inner) => {
                let value = self.evaluate(inner)?;
                Ok(match op {
                    UnaryOp::Not => Value::Bool(!value.truthy()),
                    UnaryOp::Minus => Value::Number(-self.js_number(&value)?),
                    UnaryOp::Plus => Value::Number(self.js_number(&value)?),
                })
            }
            Node::Binary(op, left, right) => {
                let left = self.evaluate(left)?;
                match op {
                    BinaryOp::And => {
                        return if left.truthy() {
                            self.evaluate(right)
                        } else {
                            Ok(left)
                        };
                    }
                    BinaryOp::Or => {
                        return if left.truthy() {
                            Ok(left)
                        } else {
                            self.evaluate(right)
                        };
                    }
                    _ => {}
                }
                let right = self.evaluate(right)?;
                self.binary(*op, &left, &right)
            }
            Node::Conditional(test, yes, no) => {
                if self.evaluate(test)?.truthy() {
                    self.evaluate(yes)
                } else {
                    self.evaluate(no)
                }
            }
            Node::Member(object, property) => {
                let object = self.evaluate(object)?;
                let property = self.evaluate(property)?;
                self.member(&object, &property)
            }
            Node::Call(callee, args) => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.evaluate(arg)?);
                }
                match &**callee {
                    Node::Identifier(name) => {
                        if let Some(helper) = Helper::from_name(name) {
                            return self.helper(helper, &values);
                        }
                        self.call_math(math_lookup(&JsStr::from(name.as_str())), &values)
                    }
                    Node::Member(object, property) => match (&**object, &**property) {
                        (Node::Identifier(name), Node::Literal(literal)) if name == "Math" => {
                            let key = match literal {
                                Literal::Number(n) => number_to_string(*n),
                                Literal::String(s) => s.clone(),
                                Literal::Bool(b) => JsStr::from(if *b { "true" } else { "false" }),
                                Literal::Null => JsStr::from("null"),
                            };
                            self.call_math(math_lookup(&key), &values)
                        }
                        _ => Ok(Value::Undefined),
                    },
                    _ => Ok(Value::Undefined),
                }
            }
        }
    }

    fn identifier(&mut self, name: &str) -> V<H::Obj> {
        let ctx = self.ctx;
        match name {
            "time" | "T" => {
                self.deps.context.insert(ContextReads::TIME);
                Value::Number(ctx.time)
            }
            "PI" => Value::Number(std::f64::consts::PI),
            "Math" => Value::Object(Object::Math),
            "t" => {
                self.deps.context.insert(ContextReads::LOCAL_TIME);
                Value::Number(ctx.t)
            }
            "fps" => {
                self.deps.context.insert(ContextReads::FPS);
                Value::Number(ctx.fps)
            }
            "value" => {
                self.deps.context.insert(ContextReads::VALUE);
                self.root(Root::Value)
            }
            "layer" => self.root(Root::Layer),
            "comp" => self.root(Root::Comp),
            "ch" => {
                self.deps.context.insert(ContextReads::CHANNEL);
                Value::String(ctx.channel.clone())
            }
            "idx" => {
                self.deps.context.insert(ContextReads::INDEX);
                Value::Number(ctx.idx)
            }
            "frame" if self.dialect == Dialect::Native => {
                self.deps.context.insert(ContextReads::FRAME);
                Value::Number(ctx.frame)
            }
            _ => {
                if let Some(h) = Helper::from_name(name) {
                    Value::Object(Object::Callable(Callable::Helper(h)))
                } else if let Some(m) = MathFunction::from_name(name) {
                    Value::Object(Object::Callable(Callable::Math(m)))
                } else {
                    Value::Undefined
                }
            }
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

    fn call_math(&mut self, lookup: MathLookup, args: &[V<H::Obj>]) -> R<V<H::Obj>> {
        if matches!(lookup, MathLookup::Missing) {
            return Ok(Value::Undefined);
        }
        let mut numbers = Vec::with_capacity(args.len());
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

    // `6.2831` is the literal `expression.ts` uses, not τ.
    #[allow(clippy::approx_constant)]
    fn helper(&mut self, helper: Helper, args: &[V<H::Obj>]) -> R<V<H::Obj>> {
        let mut numbers: Vec<f64> = Vec::with_capacity(args.len());
        for arg in args {
            numbers.push(self.js_number(arg)?);
        }
        let n = |i: usize| numbers.get(i).copied();
        // An absent argument is `undefined`: NaN in arithmetic and comparisons,
        // but returned as `undefined` where the helper returns it unchanged.
        let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
        let opt = |v: Option<f64>| v.map_or(Value::Undefined, Value::Number);
        Ok(match helper {
            Helper::Clamp => opt(clamp_opt(n(0), nan(n(1)), nan(n(2)))),
            Helper::Lerp => Value::Number(nan(n(0)) + (nan(n(1)) - nan(n(0))) * nan(n(2))),
            Helper::Linear => {
                let (x, x0, x1, y0, y1) = (nan(n(0)), n(1), n(2), n(3), nan(n(4)));
                if strict_equal_opt(x1, x0) {
                    opt(y0)
                } else {
                    let (x0, x1, y0) = (nan(x0), nan(x1), nan(y0));
                    Value::Number(y0 + (y1 - y0) * clamp((x - x0) / (x1 - x0), 0.0, 1.0))
                }
            }
            Helper::Ease => {
                let (x, x0, x1, y0, y1) = (nan(n(0)), n(1), n(2), nan(n(3)), nan(n(4)));
                let ratio = if strict_equal_opt(x1, x0) {
                    0.0
                } else {
                    (x - nan(x0)) / (nan(x1) - nan(x0))
                };
                let amount = clamp(ratio, 0.0, 1.0);
                Value::Number(y0 + (y1 - y0) * amount * amount * (3.0 - 2.0 * amount))
            }
            Helper::Random => {
                let seed = n(0).unwrap_or(1.0) * 127.1;
                let value = jsmath::sin(seed) * 43758.5453;
                Value::Number(value - jsmath::floor(value))
            }
            Helper::Wiggle => {
                self.deps.context.insert(ContextReads::LOCAL_TIME);
                let t = self.ctx.t;
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
                Value::Number(value)
            }
            Helper::Bounce => {
                let mut x = clamp(n(0).unwrap_or(0.0), 0.0, 1.0);
                Value::Number(if x < 1.0 / 2.75 {
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
                })
            }
            Helper::Loop => {
                if nan(n(0)) <= 0.0 {
                    opt(n(1))
                } else {
                    Value::Number(nan(n(1)) % nan(n(0)))
                }
            }
            Helper::Pingpong => {
                let duration = n(0).unwrap_or(0.0);
                if duration <= 0.0 {
                    opt(n(1))
                } else {
                    let position = nan(n(1)) % (duration * 2.0);
                    Value::Number(if position < duration {
                        position
                    } else {
                        duration * 2.0 - position
                    })
                }
            }
            Helper::Param => {
                let name = match args.first() {
                    None | Some(Value::Undefined | Value::Null) => JsStr::default(),
                    Some(v) => self.js_string(v)?,
                };
                self.param(&name)?
            }
        })
    }

    /// `paramGet(name)` in `anim.ts`: `comp.params && comp.params[name]`, then
    /// `p ? p.value : 0`, with ordinary JavaScript property lookups.
    fn param(&mut self, name: &JsStr) -> R<V<H::Obj>> {
        let comp = self.root(Root::Comp);
        let params = match self.get(Got::Value(comp), &JsStr::from("params"))? {
            Got::Value(v) if v.truthy() => v,
            Got::Value(_) => return Ok(Value::Number(0.0)),
            Got::Inherited => unreachable!("\"params\" is no prototype member"),
        };
        let p = self.get(Got::Value(params), name)?;
        match p {
            Got::Value(v) if !v.truthy() => Ok(Value::Number(0.0)),
            p => match self.get(p, &JsStr::from("value"))? {
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
    let Some(text) = key.to_rust_string() else {
        return Value::Undefined;
    };
    if let Some((_, v)) = MATH_CONSTANTS.iter().find(|(name, _)| *name == text) {
        return Value::Number(*v);
    }
    match MathFunction::from_name(&text) {
        Some(f) => Value::Object(Object::MathFunction(f)),
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

fn apply_math(f: MathFunction, n: &[f64]) -> f64 {
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
