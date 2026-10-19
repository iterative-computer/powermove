//! Compiled once per source. Calls retain the language's syntactic callee
//! lookup: arguments run left to right, and the callee is never evaluated.
use crate::eval::{MathLookup, math_lookup};
use crate::parser::{BinaryOp, Literal, Node, UnaryOp};
use crate::value::{Callable, Helper, MathFunction};
use crate::{JsStr, number_to_string};

#[derive(Clone, Debug)]
pub(crate) enum Call {
    Helper(Helper),
    Math(MathLookup),
    Missing,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Input {
    Time,
    Pi,
    Math,
    LocalTime,
    Fps,
    Value,
    Layer,
    Comp,
    Channel,
    Index,
    Frame,
    Callable(Callable),
    Undefined,
}

impl Input {
    fn named(name: &str) -> Self {
        match name {
            "time" | "T" => Self::Time,
            "PI" => Self::Pi,
            "Math" => Self::Math,
            "t" => Self::LocalTime,
            "fps" => Self::Fps,
            "value" => Self::Value,
            "layer" => Self::Layer,
            "comp" => Self::Comp,
            "ch" => Self::Channel,
            "idx" => Self::Index,
            "frame" => Self::Frame,
            _ => Helper::from_name(name)
                .map(Callable::Helper)
                .or_else(|| MathFunction::from_name(name).map(Callable::Math))
                .map(Self::Callable)
                .unwrap_or(Self::Undefined),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Op {
    Literal(Literal),
    Identifier(Input),
    Unary(UnaryOp),
    Binary(BinaryOp),
    Member,
    Call(Call, usize),
    /// Keep the left value on the short circuit; otherwise pop it.
    Short(bool, usize),
    Branch(usize),
    Jump(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct Program {
    pub code: Vec<Op>,
    pub stack: usize,
}

impl Program {
    pub fn compile(tree: &Node) -> Self {
        let mut code = Vec::new();
        Self::emit(tree, &mut code);
        Self {
            code,
            stack: Self::stack(tree),
        }
    }

    fn stack(node: &Node) -> usize {
        match node {
            Node::Literal(_) | Node::Identifier(_) => 1,
            Node::Unary(_, n) => Self::stack(n),
            Node::Binary(BinaryOp::And | BinaryOp::Or, l, r) => Self::stack(l).max(Self::stack(r)),
            Node::Binary(_, l, r) | Node::Member(l, r) => Self::stack(l).max(1 + Self::stack(r)),
            Node::Conditional(t, y, n) => Self::stack(t).max(Self::stack(y)).max(Self::stack(n)),
            Node::Call(_, args) => args
                .iter()
                .enumerate()
                .map(|(i, n)| i + Self::stack(n))
                .max()
                .unwrap_or(1),
        }
    }

    fn emit(node: &Node, out: &mut Vec<Op>) {
        match node {
            Node::Literal(v) => out.push(Op::Literal(v.clone())),
            Node::Identifier(v) => out.push(Op::Identifier(Input::named(v))),
            Node::Unary(op, n) => {
                Self::emit(n, out);
                out.push(Op::Unary(*op));
            }
            Node::Binary(op @ (BinaryOp::And | BinaryOp::Or), l, r) => {
                Self::emit(l, out);
                let jump = out.len();
                out.push(Op::Short(*op == BinaryOp::Or, 0));
                Self::emit(r, out);
                out[jump] = Op::Short(*op == BinaryOp::Or, out.len());
            }
            Node::Binary(op, l, r) => {
                Self::emit(l, out);
                Self::emit(r, out);
                out.push(Op::Binary(*op));
            }
            Node::Conditional(test, yes, no) => {
                Self::emit(test, out);
                let branch = out.len();
                out.push(Op::Branch(0));
                Self::emit(yes, out);
                let jump = out.len();
                out.push(Op::Jump(0));
                out[branch] = Op::Branch(out.len());
                Self::emit(no, out);
                out[jump] = Op::Jump(out.len());
            }
            Node::Member(o, k) => {
                Self::emit(o, out);
                Self::emit(k, out);
                out.push(Op::Member);
            }
            Node::Call(callee, args) => {
                for arg in args {
                    Self::emit(arg, out);
                }
                let call = match &**callee {
                    Node::Identifier(name) => Helper::from_name(name)
                        .map(Call::Helper)
                        .unwrap_or_else(|| Call::Math(math_lookup(&JsStr::from(name.as_str())))),
                    Node::Member(o, k) => match (&**o, &**k) {
                        (Node::Identifier(name), Node::Literal(v)) if name == "Math" => {
                            let key = match v {
                                Literal::Number(n) => number_to_string(*n),
                                Literal::String(s) => s.clone(),
                                Literal::Bool(b) => JsStr::from(if *b { "true" } else { "false" }),
                                Literal::Null => JsStr::from("null"),
                            };
                            Call::Math(math_lookup(&key))
                        }
                        _ => Call::Missing,
                    },
                    _ => Call::Missing,
                };
                out.push(Op::Call(call, args.len()));
            }
        }
    }
}
