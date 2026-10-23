//! A typed plan for the numeric subset. Every uncertain type declines to the
//! complete bytecode evaluator; coercion and errors therefore keep one owner.
//!
//! The subset covers what channel expressions are made of: arithmetic,
//! comparisons, `!`, `&&`/`||`, the conditional operator, literal member reads
//! of `layer`/`comp`/`value`, `param("name")`, `Math` constants and functions,
//! and every numeric helper (`clamp`, `lerp`, `linear`, `ease`, `random`,
//! `wiggle`, `bounce`, `loop`, `pingpong`).
//!
//! Booleans are carried as `0.0`/`1.0`. The compiler tracks each node's static
//! type, so this encoding is only observable where JavaScript's own conversion
//! gives the same number (`Number(true)` is 1, `-false` is `-0`, `true == 1`)
//! or where only truthiness is read. A plan whose result may be a boolean, or
//! a strict equality between a number and a boolean, is not compiled. At run
//! time a plan never errors: an input that is not a number (a string field, an
//! object, a helper returning `undefined`) makes it decline, and the bytecode
//! evaluator answers instead, with every conversion and error of V8.
use crate::eval::{apply_helper, apply_math};
use crate::parser::{BinaryOp, Literal, Node, OBJECT_PROTOTYPE, UnaryOp};
use crate::{
    Context, Dialect, Helper, Host, JsStr, MATH_CONSTANTS, MathFunction, Object, Shape, Value,
};

#[derive(Clone, Debug)]
pub(crate) enum Numeric {
    Number(f64),
    Input(Input),
    Field(Input, Vec<JsStr>),
    Param(JsStr, JsStr, JsStr),
    /// An operator over child plans (one array, so every traversal sees every
    /// child: binding identity depends on visiting all fixed leaves).
    Node(Op, Vec<Numeric>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    /// `-x`.
    Negate,
    /// `!x`: a boolean.
    Not,
    /// Arithmetic (a number) or a comparison (a boolean).
    Binary(BinaryOp),
    /// `&&` (`false`) or `||` (`true`): the deciding operand itself.
    Logic(bool),
    /// `test ? yes : no`.
    Cond,
    Math(MathFunction),
    /// Any helper except `param`.
    Helper(Helper),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Input {
    Time,
    LocalTime,
    Fps,
    Frame,
    Index,
    Value,
    Layer,
    Comp,
}

/// The static type of a plan node's JavaScript value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Number,
    Bool,
    /// `a && b`, `c ? a : b`, … with a number on one side and a boolean on
    /// the other.
    Either,
}

impl Ty {
    fn merge(self, other: Ty) -> Ty {
        if self == other { self } else { Ty::Either }
    }
}

/// `ToBoolean` of a number, or of a boolean encoded as 0/1.
fn truthy(n: f64) -> bool {
    !(n.is_nan() || n == 0.0)
}

fn bool_number(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

impl Numeric {
    fn children(&self) -> &[Numeric] {
        match self {
            Self::Node(_, children) => children,
            _ => &[],
        }
    }

    pub fn fixed_is_layer_local(&self) -> bool {
        match self {
            Self::Input(Input::Fps) | Self::Field(Input::Comp, _) | Self::Param(..) => false,
            _ => self.children().iter().all(Self::fixed_is_layer_local),
        }
    }

    /// Traverse fixed leaves in syntax order. A nonnumeric document leaf must
    /// decline the entire reusable binding, including if another leaf is
    /// numeric: partial folding could otherwise leave stale constants. Leaves
    /// in branches the current inputs do not take count too.
    pub fn fixed_bits<H: Host>(
        &self,
        host: &H,
        ctx: &Context<H::Obj>,
        bits: &mut Vec<u64>,
    ) -> Option<()> {
        if self.is_fixed_leaf() {
            bits.push(self.evaluate(host, ctx)?.to_bits());
            return Some(());
        }
        for child in self.children() {
            child.fixed_bits(host, ctx, bits)?;
        }
        Some(())
    }

    pub fn fixed_matches<H: Host>(
        &self,
        host: &H,
        ctx: &Context<H::Obj>,
        bits: &[u64],
        position: &mut usize,
    ) -> bool {
        if self.is_fixed_leaf() {
            let expected = bits.get(*position);
            *position += 1;
            return self
                .evaluate(host, ctx)
                .is_some_and(|n| expected == Some(&n.to_bits()));
        }
        self.children()
            .iter()
            .all(|child| child.fixed_matches(host, ctx, bits, position))
    }

    pub fn fixed_uses_layer(&self) -> bool {
        match self {
            Self::Field(Input::Layer, _) => true,
            _ => self.children().iter().any(Self::fixed_uses_layer),
        }
    }

    fn is_fixed_leaf(&self) -> bool {
        matches!(
            self,
            Self::Input(Input::Fps | Input::Index)
                | Self::Field(Input::Layer | Input::Comp, _)
                | Self::Param(..)
        )
    }

    /// A plan for an expression whose value is always a number, or `None`.
    pub fn compile(node: &Node, dialect: Dialect) -> Option<Self> {
        match Self::typed(node, dialect)? {
            (plan, Ty::Number) => Some(plan),
            _ => None,
        }
    }

    fn typed(node: &Node, dialect: Dialect) -> Option<(Self, Ty)> {
        let node_of = |op: Op, children: Vec<Numeric>| Self::Node(op, children);
        Some(match node {
            Node::Literal(Literal::Number(n)) => (Self::Number(*n), Ty::Number),
            Node::Literal(Literal::Bool(b)) => (Self::Number(bool_number(*b)), Ty::Bool),
            Node::Literal(_) => return None,
            Node::Identifier(name) => (
                Self::Input(match name.as_str() {
                    "time" | "T" => Input::Time,
                    "t" => Input::LocalTime,
                    "fps" => Input::Fps,
                    "idx" => Input::Index,
                    "value" => Input::Value,
                    "frame" if dialect == Dialect::Native => Input::Frame,
                    "PI" => return Some((Self::Number(std::f64::consts::PI), Ty::Number)),
                    _ => return None,
                }),
                Ty::Number,
            ),
            // `+x` is `Number(x)`: a boolean's 0/1 is already its number.
            Node::Unary(UnaryOp::Plus, n) => (Self::typed(n, dialect)?.0, Ty::Number),
            Node::Unary(UnaryOp::Minus, n) => (
                node_of(Op::Negate, vec![Self::typed(n, dialect)?.0]),
                Ty::Number,
            ),
            Node::Unary(UnaryOp::Not, n) => {
                (node_of(Op::Not, vec![Self::typed(n, dialect)?.0]), Ty::Bool)
            }
            Node::Binary(op @ (BinaryOp::And | BinaryOp::Or), l, r) => {
                let (l, lt) = Self::typed(l, dialect)?;
                let (r, rt) = Self::typed(r, dialect)?;
                (
                    node_of(Op::Logic(*op == BinaryOp::Or), vec![l, r]),
                    lt.merge(rt),
                )
            }
            Node::Binary(op, l, r) => {
                let (l, lt) = Self::typed(l, dialect)?;
                let (r, rt) = Self::typed(r, dialect)?;
                let ty = match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem
                    | BinaryOp::Pow => Ty::Number,
                    // `ToNumber` on both sides; `==` between numbers and
                    // booleans converts the boolean to its number.
                    BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::Eq
                    | BinaryOp::Ne => Ty::Bool,
                    // `===` compares the encoding only between equal types.
                    BinaryOp::StrictEq | BinaryOp::StrictNe if lt == rt && lt != Ty::Either => {
                        Ty::Bool
                    }
                    _ => return None,
                };
                (node_of(Op::Binary(*op), vec![l, r]), ty)
            }
            Node::Conditional(test, yes, no) => {
                let (test, _) = Self::typed(test, dialect)?;
                let (yes, yt) = Self::typed(yes, dialect)?;
                let (no, nt) = Self::typed(no, dialect)?;
                (node_of(Op::Cond, vec![test, yes, no]), yt.merge(nt))
            }
            Node::Member(_, _) => {
                let mut keys = Vec::new();
                let mut root = node;
                while let Node::Member(base, key) = root {
                    let Node::Literal(Literal::String(key)) = &**key else {
                        return None;
                    };
                    if ["__proto__", "prototype", "constructor"]
                        .iter()
                        .any(|name| key.eq_str(name))
                    {
                        return None;
                    }
                    keys.push(key.clone());
                    root = base;
                }
                keys.reverse();
                let Node::Identifier(name) = root else {
                    return None;
                };
                let input = match name.as_str() {
                    "layer" => Input::Layer,
                    "comp" => Input::Comp,
                    "value" => Input::Value,
                    // `Math.PI`, `Math.E`, …: own constants of the Math value.
                    "Math" => {
                        let [key] = keys.as_slice() else {
                            return None;
                        };
                        let (_, n) = MATH_CONSTANTS.iter().find(|(name, _)| key.eq_str(name))?;
                        return Some((Self::Number(*n), Ty::Number));
                    }
                    _ => return None,
                };
                (Self::Field(input, keys), Ty::Number)
            }
            Node::Call(callee, args) => {
                if let Node::Identifier(name) = &**callee
                    && let Some(helper) = Helper::from_name(name)
                {
                    // `sample` and `motionPath` ask the host; only the
                    // complete evaluator has it.
                    if helper == Helper::Sample || helper == Helper::MotionPath {
                        return None;
                    }
                    if helper == Helper::Param {
                        let [Node::Literal(Literal::String(key))] = args.as_slice() else {
                            return None;
                        };
                        return Some((
                            Self::Param(key.clone(), JsStr::from("params"), JsStr::from("value")),
                            Ty::Number,
                        ));
                    }
                    let args = Self::arguments(args, dialect)?;
                    return Some((node_of(Op::Helper(helper), args), Ty::Number));
                }
                let function = match &**callee {
                    Node::Identifier(n) => MathFunction::from_name(n)?,
                    Node::Member(base, key) if matches!(&**base, Node::Identifier(n) if n == "Math") =>
                    {
                        let Node::Literal(Literal::String(key)) = &**key else {
                            return None;
                        };
                        MathFunction::ALL
                            .iter()
                            .find(|f| key.eq_str(f.name()))
                            .copied()?
                    }
                    _ => return None,
                };
                (
                    node_of(Op::Math(function), Self::arguments(args, dialect)?),
                    Ty::Number,
                )
            }
        })
    }

    /// Call arguments go through `Number()`, so booleans are their 0/1.
    fn arguments(args: &[Node], dialect: Dialect) -> Option<Vec<Self>> {
        args.iter()
            .map(|n| Self::typed(n, dialect).map(|(plan, _)| plan))
            .collect()
    }

    /// Revision-bound plans that need only the three changing numeric inputs.
    /// Any document read or unavailable input declines to the full evaluator.
    pub fn scalars(&self, t: f64, time: f64, value: f64) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Input(Input::Time) => Some(time),
            Self::Input(Input::LocalTime) => Some(t),
            Self::Input(Input::Value) => Some(value),
            Self::Node(op, children) => {
                Self::apply(*op, children, t, &mut |n| n.scalars(t, time, value))
            }
            _ => None,
        }
    }

    /// One operator over children evaluated by `eval` (the scalar or the
    /// document form). Short-circuiting operators evaluate only the operands
    /// JavaScript evaluates.
    #[inline]
    fn apply(
        op: Op,
        children: &[Numeric],
        t: f64,
        eval: &mut impl FnMut(&Numeric) -> Option<f64>,
    ) -> Option<f64> {
        Some(match op {
            Op::Negate => -eval(&children[0])?,
            Op::Not => bool_number(!truthy(eval(&children[0])?)),
            Op::Binary(op) => {
                let l = eval(&children[0])?;
                let r = eval(&children[1])?;
                binary(op, l, r)
            }
            Op::Logic(or) => {
                let l = eval(&children[0])?;
                if truthy(l) == or {
                    l
                } else {
                    eval(&children[1])?
                }
            }
            Op::Cond => {
                if truthy(eval(&children[0])?) {
                    eval(&children[1])?
                } else {
                    eval(&children[2])?
                }
            }
            Op::Math(f) => {
                let mut values: smallvec::SmallVec<[f64; 12]> = smallvec::SmallVec::new();
                for arg in children {
                    values.push(eval(arg)?);
                }
                apply_math(f, &values)
            }
            Op::Helper(h) => {
                let mut values: smallvec::SmallVec<[f64; 12]> = smallvec::SmallVec::new();
                for arg in children {
                    values.push(eval(arg)?);
                }
                apply_helper(h, &values, t)?
            }
        })
    }

    fn input<O: Clone>(input: Input, ctx: &Context<O>) -> Value<O> {
        match input {
            Input::Time => Value::Number(ctx.time),
            Input::LocalTime => Value::Number(ctx.t),
            Input::Fps => Value::Number(ctx.fps),
            Input::Frame => Value::Number(ctx.frame),
            Input::Index => Value::Number(ctx.idx),
            Input::Value => ctx.value.clone(),
            Input::Layer => ctx.layer.clone(),
            Input::Comp => ctx.comp.clone(),
        }
    }

    /// Folds document/fps/index inputs (the binding context's time, local
    /// time, frame and value are placeholders and never read).
    pub fn bind<H: Host>(&self, host: &H, ctx: &Context<H::Obj>) -> Self {
        match self {
            Self::Field(Input::Value, _)
            | Self::Input(Input::Value | Input::Time | Input::LocalTime | Input::Frame) => {
                self.clone()
            }
            Self::Field(_, _) | Self::Param(..) | Self::Input(Input::Fps | Input::Index) => self
                .evaluate(host, ctx)
                .map(Self::Number)
                .unwrap_or_else(|| self.clone()),
            Self::Node(op, children) => {
                let constant = |n: &Numeric| match n {
                    Self::Number(n) => Some(*n),
                    _ => None,
                };
                match op {
                    // A decided test or left operand removes the operator
                    // (the untaken operand is never evaluated by JavaScript).
                    Op::Cond => {
                        let test = children[0].bind(host, ctx);
                        if let Some(n) = constant(&test) {
                            return children[if truthy(n) { 1 } else { 2 }].bind(host, ctx);
                        }
                        let mut out = vec![test];
                        out.extend(children[1..].iter().map(|n| n.bind(host, ctx)));
                        Self::Node(*op, out)
                    }
                    Op::Logic(or) => {
                        let left = children[0].bind(host, ctx);
                        match constant(&left) {
                            Some(n) if truthy(n) == *or => left,
                            Some(_) => children[1].bind(host, ctx),
                            None => Self::Node(*op, vec![left, children[1].bind(host, ctx)]),
                        }
                    }
                    _ => {
                        let out: Vec<_> = children.iter().map(|n| n.bind(host, ctx)).collect();
                        // `wiggle` reads the local time, so it never folds.
                        // A helper answering `undefined` stays for the
                        // bytecode evaluator to report.
                        let folded = (*op != Op::Helper(Helper::Wiggle)
                            && out.iter().all(|n| constant(n).is_some()))
                        .then(|| Self::apply(*op, &out, f64::NAN, &mut |n| constant(n)))
                        .flatten();
                        folded.map_or(Self::Node(*op, out), Self::Number)
                    }
                }
            }
            _ => self.clone(),
        }
    }

    pub fn evaluate<H: Host>(&self, host: &H, ctx: &Context<H::Obj>) -> Option<f64> {
        Some(match self {
            Self::Number(n) => *n,
            Self::Input(i) => Self::input(*i, ctx).as_number()?,
            Self::Node(op, children) => {
                return Self::apply(*op, children, ctx.t, &mut |n| n.evaluate(host, ctx));
            }
            Self::Field(input, keys) => {
                let mut value = Self::input(*input, ctx);
                for key in keys {
                    value = match value {
                        Value::Object(Object::Doc(obj)) => match host.shape(&obj) {
                            Shape::Array { len } if key.eq_str("length") => {
                                Value::Number(len as f64)
                            }
                            Shape::Array { .. } => return None,
                            Shape::Object => host.property(&obj, key)?,
                        },
                        Value::String(s) if key.eq_str("length") => Value::Number(s.len() as f64),
                        _ => return None,
                    };
                }
                value.as_number()?
            }
            Self::Param(key, params_key, value_key) => {
                let Value::Object(Object::Doc(comp)) = &ctx.comp else {
                    return None;
                };
                let params = host.property(comp, params_key).unwrap_or(Value::Undefined);
                if !params.truthy() {
                    return Some(0.0);
                }
                let Value::Object(Object::Doc(params)) = params else {
                    return None;
                };
                if host.shape(&params) != Shape::Object {
                    return None;
                }
                let parameter = match host.property(&params, key) {
                    Some(p) => p,
                    None if OBJECT_PROTOTYPE.iter().any(|(name, _)| key.eq_str(name)) => {
                        return None;
                    }
                    None => return Some(0.0),
                };
                if !parameter.truthy() {
                    return Some(0.0);
                }
                let Value::Object(Object::Doc(parameter)) = parameter else {
                    return None;
                };
                if host.shape(&parameter) != Shape::Object {
                    return None;
                }
                host.property(&parameter, value_key)?.as_number()?
            }
        })
    }
}

fn binary(op: BinaryOp, l: f64, r: f64) -> f64 {
    match op {
        BinaryOp::Add => l + r,
        BinaryOp::Sub => l - r,
        BinaryOp::Mul => l * r,
        BinaryOp::Div => l / r,
        BinaryOp::Rem => l % r,
        BinaryOp::Pow => crate::jsmath::pow(l, r),
        BinaryOp::Lt => bool_number(l < r),
        BinaryOp::Le => bool_number(l <= r),
        BinaryOp::Gt => bool_number(l > r),
        BinaryOp::Ge => bool_number(l >= r),
        BinaryOp::Eq | BinaryOp::StrictEq => bool_number(l == r),
        BinaryOp::Ne | BinaryOp::StrictNe => bool_number(l != r),
        _ => unreachable!("numeric compiler limits operators"),
    }
}
