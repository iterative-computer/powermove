//! A typed plan for the numeric subset. Every uncertain type declines to the
//! complete bytecode evaluator; coercion and errors therefore keep one owner.
use crate::parser::{BinaryOp, Literal, Node, OBJECT_PROTOTYPE, UnaryOp};
use crate::{Context, Dialect, Host, JsStr, MathFunction, Object, Shape, Value};

#[derive(Clone, Debug)]
pub(crate) enum Numeric {
    Number(f64),
    Input(Input),
    Negate(Box<Self>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
    Field(Input, Vec<JsStr>),
    Param(JsStr, JsStr, JsStr),
    Math(MathFunction, Vec<Self>),
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

impl Numeric {
    pub fn fixed_is_layer_local(&self) -> bool {
        match self {
            Self::Input(Input::Fps) | Self::Field(Input::Comp, _) | Self::Param(..) => false,
            Self::Negate(n) => n.fixed_is_layer_local(),
            Self::Binary(_, l, r) => l.fixed_is_layer_local() && r.fixed_is_layer_local(),
            Self::Math(_, args) => args.iter().all(Self::fixed_is_layer_local),
            _ => true,
        }
    }

    /// Traverse fixed leaves in syntax order. A nonnumeric document leaf must
    /// decline the entire reusable binding, including if another leaf is
    /// numeric: partial folding could otherwise leave stale constants.
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
        match self {
            Self::Negate(n) => n.fixed_bits(host, ctx, bits)?,
            Self::Binary(_, l, r) => {
                l.fixed_bits(host, ctx, bits)?;
                r.fixed_bits(host, ctx, bits)?;
            }
            Self::Math(_, args) => {
                for arg in args {
                    arg.fixed_bits(host, ctx, bits)?;
                }
            }
            _ => {}
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
        match self {
            Self::Negate(n) => n.fixed_matches(host, ctx, bits, position),
            Self::Binary(_, l, r) => {
                l.fixed_matches(host, ctx, bits, position)
                    && r.fixed_matches(host, ctx, bits, position)
            }
            Self::Math(_, args) => args
                .iter()
                .all(|arg| arg.fixed_matches(host, ctx, bits, position)),
            _ => true,
        }
    }

    pub fn fixed_uses_layer(&self) -> bool {
        match self {
            Self::Field(Input::Layer, _) => true,
            Self::Negate(n) => n.fixed_uses_layer(),
            Self::Binary(_, l, r) => l.fixed_uses_layer() || r.fixed_uses_layer(),
            Self::Math(_, args) => args.iter().any(Self::fixed_uses_layer),
            _ => false,
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

    pub fn compile(node: &Node, dialect: Dialect) -> Option<Self> {
        Some(match node {
            Node::Literal(Literal::Number(n)) => Self::Number(*n),
            Node::Identifier(name) => Self::Input(match name.as_str() {
                "time" | "T" => Input::Time,
                "t" => Input::LocalTime,
                "fps" => Input::Fps,
                "idx" => Input::Index,
                "value" => Input::Value,
                "frame" if dialect == Dialect::Native => Input::Frame,
                "PI" => return Some(Self::Number(std::f64::consts::PI)),
                _ => return None,
            }),
            Node::Unary(UnaryOp::Plus, n) => Self::compile(n, dialect)?,
            Node::Unary(UnaryOp::Minus, n) => Self::Negate(Box::new(Self::compile(n, dialect)?)),
            Node::Binary(
                op @ (BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::Pow),
                l,
                r,
            ) => Self::Binary(
                *op,
                Box::new(Self::compile(l, dialect)?),
                Box::new(Self::compile(r, dialect)?),
            ),
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
                    _ => return None,
                };
                Self::Field(input, keys)
            }
            Node::Call(callee, args) => {
                if matches!(&**callee, Node::Identifier(n) if n == "param") && args.len() == 1 {
                    let Node::Literal(Literal::String(key)) = &args[0] else {
                        return None;
                    };
                    return Some(Self::Param(
                        key.clone(),
                        JsStr::from("params"),
                        JsStr::from("value"),
                    ));
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
                Self::Math(
                    function,
                    args.iter()
                        .map(|n| Self::compile(n, dialect))
                        .collect::<Option<Vec<_>>>()?,
                )
            }
            _ => return None,
        })
    }

    /// Revision-bound plans that need only the three changing numeric inputs.
    /// Any document read or unavailable input declines to the full evaluator.
    pub fn scalars(&self, t: f64, time: f64, value: f64) -> Option<f64> {
        Some(match self {
            Self::Number(n) => *n,
            Self::Input(Input::Time) => time,
            Self::Input(Input::LocalTime) => t,
            Self::Input(Input::Value) => value,
            Self::Negate(n) => -n.scalars(t, time, value)?,
            Self::Binary(op, l, r) => {
                let l = l.scalars(t, time, value)?;
                let r = r.scalars(t, time, value)?;
                binary(*op, l, r)
            }
            Self::Math(f, args) => {
                let mut values: smallvec::SmallVec<[f64; 12]> = smallvec::SmallVec::new();
                for arg in args {
                    values.push(arg.scalars(t, time, value)?);
                }
                crate::eval::apply_math(*f, &values)
            }
            _ => return None,
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
            Self::Negate(n) => {
                let n = n.bind(host, ctx);
                match n {
                    Self::Number(n) => Self::Number(-n),
                    n => Self::Negate(Box::new(n)),
                }
            }
            Self::Binary(op, l, r) => {
                let out = Self::Binary(
                    *op,
                    Box::new(l.bind(host, ctx)),
                    Box::new(r.bind(host, ctx)),
                );
                if matches!(&out, Self::Binary(_, l, r) if matches!(&**l, Self::Number(_)) && matches!(&**r, Self::Number(_)))
                {
                    Self::Number(out.evaluate(host, ctx).expect("numeric constants"))
                } else {
                    out
                }
            }
            Self::Math(f, args) => {
                let args: Vec<_> = args.iter().map(|n| n.bind(host, ctx)).collect();
                let constants = args.iter().all(|n| matches!(n, Self::Number(_)));
                let out = Self::Math(*f, args);
                if constants {
                    Self::Number(out.evaluate(host, ctx).expect("numeric constants"))
                } else {
                    out
                }
            }
            _ => self.clone(),
        }
    }

    pub fn evaluate<H: Host>(&self, host: &H, ctx: &Context<H::Obj>) -> Option<f64> {
        Some(match self {
            Self::Number(n) => *n,
            Self::Input(i) => Self::input(*i, ctx).as_number()?,
            Self::Negate(n) => -n.evaluate(host, ctx)?,
            Self::Binary(op, l, r) => {
                let l = l.evaluate(host, ctx)?;
                let r = r.evaluate(host, ctx)?;
                binary(*op, l, r)
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
            Self::Math(f, args) => {
                let mut values: smallvec::SmallVec<[f64; 12]> = smallvec::SmallVec::new();
                for arg in args {
                    values.push(arg.evaluate(host, ctx)?);
                }
                crate::eval::apply_math(*f, &values)
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
        _ => unreachable!("numeric compiler limits operators"),
    }
}
