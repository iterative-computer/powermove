//! Partial evaluation for sources outside the typed numeric plan.
//!
//! A subexpression that reads no per-evaluation input (`time`, `T`, `t`,
//! `value`, `ch`, the native `frame`, or `wiggle`'s implicit local time) has
//! the same answer at every time of one document revision: it reads only
//! `layer`, `comp`, `fps`, `idx` and literals, and nothing an expression does
//! has side effects. Binding evaluates each maximal such subexpression (a
//! *leaf*) once with the complete bytecode evaluator and, when the answer is a
//! primitive, substitutes it as a literal. A leaf that throws or yields an
//! object stays in the program and is evaluated in place, so its errors and
//! identities happen exactly where they did. A conditional, `&&` or `||`
//! whose deciding operand became a literal keeps only the operand JavaScript
//! evaluates. The bound tree is then compiled again, so a source like
//! `comp.layers[0].p.x.v + time` binds to the numeric plan's scalar path.
//!
//! The embedding owns validity exactly as for the numeric binding: a bound
//! expression is used only while [`GeneralBinding::matches`] holds, which
//! re-evaluates every substituted leaf and compares its primitive answer.

use crate::bytecode::Program;
use crate::eval::Evaluator;
use crate::parser::{BinaryOp, Literal, Node};
use crate::{Context, Dialect, Helper, Host, JsStr, Value};

#[derive(Debug)]
pub(crate) struct General {
    tree: Node,
    /// The leaves in syntax order, compiled.
    leaves: Vec<Program>,
    uses_layer: bool,
    layer_local: bool,
}

/// A primitive a leaf answered. Numbers compare by bits (signed zero, NaN).
#[derive(Clone, Debug)]
pub(crate) enum Primitive {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(JsStr),
}

impl PartialEq for Primitive {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Primitive::Undefined, Primitive::Undefined) | (Primitive::Null, Primitive::Null) => {
                true
            }
            (Primitive::Bool(a), Primitive::Bool(b)) => a == b,
            (Primitive::Number(a), Primitive::Number(b)) => a.to_bits() == b.to_bits(),
            (Primitive::String(a), Primitive::String(b)) => a == b,
            _ => false,
        }
    }
}

impl Primitive {
    fn of<O>(value: &Value<O>) -> Option<Primitive> {
        Some(match value {
            Value::Undefined => Primitive::Undefined,
            Value::Null => Primitive::Null,
            Value::Bool(b) => Primitive::Bool(*b),
            Value::Number(n) => Primitive::Number(*n),
            Value::String(s) => Primitive::String(s.clone()),
            Value::Object(_) => return None,
        })
    }

    fn node(&self) -> Node {
        match self {
            // An unknown name is `undefined` in both dialects.
            Primitive::Undefined => Node::Identifier("undefined".into()),
            Primitive::Null => Node::Literal(Literal::Null),
            Primitive::Bool(b) => Node::Literal(Literal::Bool(*b)),
            Primitive::Number(n) => Node::Literal(Literal::Number(*n)),
            Primitive::String(s) => Node::Literal(Literal::String(s.clone())),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Primitive::Undefined | Primitive::Null => false,
            Primitive::Bool(b) => *b,
            Primitive::Number(n) => !(n.is_nan() || *n == 0.0),
            Primitive::String(s) => !s.is_empty(),
        }
    }
}

/// Which leaves were substituted, and with what (`None`: kept in place).
#[derive(Clone, Debug)]
pub struct GeneralBinding {
    values: Vec<Option<Primitive>>,
}

fn is_static(node: &Node, dialect: Dialect) -> bool {
    match node {
        Node::Literal(_) => true,
        Node::Identifier(name) => {
            let dynamic = matches!(name.as_str(), "time" | "T" | "t" | "value" | "ch")
                || (name == "frame" && dialect == Dialect::Native);
            !dynamic
        }
        Node::Unary(_, n) => is_static(n, dialect),
        Node::Binary(_, l, r) | Node::Member(l, r) => {
            is_static(l, dialect) && is_static(r, dialect)
        }
        Node::Conditional(t, y, n) => {
            is_static(t, dialect) && is_static(y, dialect) && is_static(n, dialect)
        }
        // The callee is looked up by name, never evaluated.
        Node::Call(callee, args) => {
            !matches!(&**callee, Node::Identifier(name) if Helper::from_name(name) == Some(Helper::Wiggle))
                && args.iter().all(|a| is_static(a, dialect))
        }
    }
}

/// Nodes worth evaluating once (a literal is already one).
fn foldable(node: &Node) -> bool {
    !matches!(node, Node::Literal(_))
}

/// Maximal static subtrees in syntax order.
fn collect<'a>(node: &'a Node, dialect: Dialect, out: &mut Vec<&'a Node>) {
    if is_static(node, dialect) {
        if foldable(node) {
            out.push(node);
        }
        return;
    }
    match node {
        Node::Literal(_) | Node::Identifier(_) => {}
        Node::Unary(_, n) => collect(n, dialect, out),
        Node::Binary(_, l, r) | Node::Member(l, r) => {
            collect(l, dialect, out);
            collect(r, dialect, out);
        }
        Node::Conditional(t, y, n) => {
            collect(t, dialect, out);
            collect(y, dialect, out);
            collect(n, dialect, out);
        }
        Node::Call(_, args) => {
            for a in args {
                collect(a, dialect, out);
            }
        }
    }
}

fn mentions(node: &Node, names: &[&str]) -> bool {
    match node {
        Node::Literal(_) => false,
        Node::Identifier(name) => names.contains(&name.as_str()),
        Node::Unary(_, n) => mentions(n, names),
        Node::Binary(_, l, r) | Node::Member(l, r) => mentions(l, names) || mentions(r, names),
        Node::Conditional(t, y, n) => {
            mentions(t, names) || mentions(y, names) || mentions(n, names)
        }
        Node::Call(callee, args) => {
            matches!(&**callee, Node::Identifier(name) if names.contains(&name.as_str()))
                || args.iter().any(|a| mentions(a, names))
        }
    }
}

impl General {
    /// `None` when the source has nothing to evaluate once.
    pub fn compile(tree: &Node, dialect: Dialect) -> Option<General> {
        let mut leaves = Vec::new();
        collect(tree, dialect, &mut leaves);
        if leaves.is_empty() {
            return None;
        }
        // `param(…)` reads the composition like `comp` does.
        let comp_reads = leaves
            .iter()
            .any(|leaf| mentions(leaf, &["comp", "fps", "param"]));
        Some(General {
            uses_layer: leaves.iter().any(|leaf| mentions(leaf, &["layer"])),
            layer_local: !comp_reads,
            leaves: leaves.iter().map(|leaf| Program::compile(leaf)).collect(),
            tree: tree.clone(),
        })
    }

    pub fn uses_layer(&self) -> bool {
        self.uses_layer
    }

    pub fn layer_local(&self) -> bool {
        self.layer_local
    }

    fn evaluate_leaves<H: Host>(
        &self,
        host: &H,
        ctx: &Context<H::Obj>,
        dialect: Dialect,
    ) -> Vec<Option<Primitive>> {
        self.leaves
            .iter()
            .map(|leaf| {
                Evaluator::untracked(host, ctx, dialect)
                    .run(leaf)
                    .ok()
                    .and_then(|v| Primitive::of(&v.map_doc(|_| ())))
            })
            .collect()
    }

    /// The substituted tree and its binding; `None` when no leaf answered a
    /// primitive (the source itself is as good).
    pub fn bind<H: Host>(
        &self,
        host: &H,
        ctx: &Context<H::Obj>,
        dialect: Dialect,
    ) -> Option<(Node, GeneralBinding)> {
        let values = self.evaluate_leaves(host, ctx, dialect);
        if values.iter().all(Option::is_none) {
            return None;
        }
        let mut next = values.iter();
        let tree = substitute(&self.tree, dialect, &mut next);
        Some((tree, GeneralBinding { values }))
    }
}

impl GeneralBinding {
    /// Whether every substituted leaf still answers its primitive. Kept
    /// leaves are evaluated in place anyway.
    pub(crate) fn matches<H: Host>(
        &self,
        general: &General,
        host: &H,
        ctx: &Context<H::Obj>,
        dialect: Dialect,
    ) -> bool {
        general.leaves.len() == self.values.len()
            && general
                .leaves
                .iter()
                .zip(&self.values)
                .all(|(leaf, expected)| match expected {
                    None => true,
                    Some(expected) => Evaluator::untracked(host, ctx, dialect)
                        .run(leaf)
                        .ok()
                        .and_then(|v| Primitive::of(&v.map_doc(|_| ())))
                        .is_some_and(|v| v == *expected),
                })
    }
}

/// The tree with leaves replaced, walking leaves in the order `collect` did.
fn substitute<'v>(
    node: &Node,
    dialect: Dialect,
    values: &mut impl Iterator<Item = &'v Option<Primitive>>,
) -> Node {
    if is_static(node, dialect) {
        if !foldable(node) {
            return node.clone();
        }
        return match values.next().expect("one value per leaf") {
            Some(p) => p.node(),
            None => node.clone(),
        };
    }
    let constant = |n: &Node| match n {
        Node::Literal(Literal::Number(n)) => Some(Primitive::Number(*n)),
        Node::Literal(Literal::String(s)) => Some(Primitive::String(s.clone())),
        Node::Literal(Literal::Bool(b)) => Some(Primitive::Bool(*b)),
        Node::Literal(Literal::Null) => Some(Primitive::Null),
        _ => None,
    };
    let mut sub = |n: &Node| substitute(n, dialect, values);
    match node {
        Node::Literal(_) | Node::Identifier(_) => node.clone(),
        Node::Unary(op, n) => Node::Unary(*op, Box::new(sub(n))),
        Node::Binary(op @ (BinaryOp::And | BinaryOp::Or), l, r) => {
            let left = sub(l);
            // Every leaf of the right operand is consumed, taken or not.
            let right = sub(r);
            match constant(&left) {
                // `a && b` / `a || b` keep `a` when it decides.
                Some(p) if p.truthy() == (*op == BinaryOp::Or) => left,
                Some(_) => right,
                None => Node::Binary(*op, Box::new(left), Box::new(right)),
            }
        }
        Node::Binary(op, l, r) => {
            let left = sub(l);
            Node::Binary(*op, Box::new(left), Box::new(sub(r)))
        }
        Node::Member(o, k) => {
            let object = sub(o);
            Node::Member(Box::new(object), Box::new(sub(k)))
        }
        Node::Conditional(t, y, n) => {
            let test = sub(t);
            let yes = sub(y);
            let no = sub(n);
            match constant(&test) {
                Some(p) if p.truthy() => yes,
                Some(_) => no,
                None => Node::Conditional(Box::new(test), Box::new(yes), Box::new(no)),
            }
        }
        Node::Call(callee, args) => Node::Call(callee.clone(), args.iter().map(sub).collect()),
    }
}
