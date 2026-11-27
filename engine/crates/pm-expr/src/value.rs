//! The values an expression can produce or see.

use crate::string::JsStr;

/// A JavaScript value as the expression language sees it.
///
/// `O` is the host's handle to a document object or array (see
/// [`Host`](crate::Host)); everything else is created by the interpreter.
#[derive(Clone, Debug)]
pub enum Value<O> {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(JsStr),
    Object(Object<O>),
}

/// The objects an expression can hold.
#[derive(Clone, Debug)]
pub enum Object<O> {
    /// A document object or array supplied by the host (`layer`, `comp`,
    /// `value`, and anything reached from them).
    Doc(O),
    /// The `Math` namespace (a frozen plain object of constants and functions).
    Math,
    /// One of the `Math` functions, for example `Math.sin` as a value.
    MathFunction(MathFunction),
    /// `{ callable: name }`, what a bare helper or function name evaluates to
    /// (a fresh plain object each time).
    Callable(Callable),
    /// A `Number` wrapper object, the result of calling `constructor(n)`.
    NumberObject(f64),
    /// An empty plain object, the result of calling `constructor()`.
    PlainObject,
}

/// A function name that evaluates to `{ callable: name }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callable {
    Helper(Helper),
    Math(MathFunction),
}

impl Callable {
    pub fn name(self) -> &'static str {
        match self {
            Callable::Helper(h) => h.name(),
            Callable::Math(m) => m.name(),
        }
    }
}

macro_rules! names {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text),* }
            }

            pub fn from_name(name: &str) -> Option<$name> {
                match name { $($text => Some($name::$variant),)* _ => None }
            }
        }
    };
}

names! {
    /// The expression helpers, in `expression.ts` order.
    Helper {
        Clamp = "clamp",
        Lerp = "lerp",
        Linear = "linear",
        Ease = "ease",
        Random = "random",
        Wiggle = "wiggle",
        Bounce = "bounce",
        Loop = "loop",
        Pingpong = "pingpong",
        Param = "param",
        Sample = "sample",
        MotionPath = "motionPath",
    }
}

names! {
    /// The `Math` functions expressions may call, in `expression.ts` order.
    MathFunction {
        Abs = "abs",
        Acos = "acos",
        Asin = "asin",
        Atan = "atan",
        Atan2 = "atan2",
        Ceil = "ceil",
        Cos = "cos",
        Exp = "exp",
        Floor = "floor",
        Log = "log",
        Max = "max",
        Min = "min",
        Pow = "pow",
        Round = "round",
        Sign = "sign",
        Sin = "sin",
        Sqrt = "sqrt",
        Tan = "tan",
    }
}

/// The `Math` constants, in `expression.ts` order.
pub const MATH_CONSTANTS: [(&str, f64); 8] = [
    ("E", std::f64::consts::E),
    ("LN2", std::f64::consts::LN_2),
    ("LN10", std::f64::consts::LN_10),
    ("LOG2E", std::f64::consts::LOG2_E),
    ("LOG10E", std::f64::consts::LOG10_E),
    ("PI", std::f64::consts::PI),
    ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
    ("SQRT2", std::f64::consts::SQRT_2),
];

impl<O> Value<O> {
    /// Replaces document handles, keeping everything else.
    pub fn map_doc<P>(self, f: impl FnOnce(O) -> P) -> Value<P> {
        match self {
            Value::Undefined => Value::Undefined,
            Value::Null => Value::Null,
            Value::Bool(b) => Value::Bool(b),
            Value::Number(n) => Value::Number(n),
            Value::String(s) => Value::String(s),
            Value::Object(o) => Value::Object(match o {
                Object::Doc(d) => Object::Doc(f(d)),
                Object::Math => Object::Math,
                Object::MathFunction(m) => Object::MathFunction(m),
                Object::Callable(c) => Object::Callable(c),
                Object::NumberObject(n) => Object::NumberObject(n),
                Object::PlainObject => Object::PlainObject,
            }),
        }
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => !(n.is_nan() || *n == 0.0),
            Value::String(s) => !s.is_empty(),
            Value::Object(_) => true,
        }
    }

    /// `typeof value`.
    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Object(Object::MathFunction(_)) => "function",
            Value::Object(_) => "object",
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// `String(f64)`: ECMAScript `Number::toString`.
pub fn number_to_string(n: f64) -> JsStr {
    JsStr::from(pm_model::json::number_to_js_string(n))
}

/// `Number(string)`: ECMAScript `StringToNumber`.
pub fn string_to_number(s: &JsStr) -> f64 {
    match s.to_rust_string() {
        Some(text) => pm_model::json::string_to_number(&text),
        // A lone surrogate is neither whitespace nor part of a numeric literal.
        None => f64::NAN,
    }
}
