//! `expressionDiagnostic` and `EXPRESSION_NAMES`.

use crate::Dialect;
use crate::parser::{Literal, MAX_SOURCE_CHARS, Node, parse};
use crate::value::{Helper, MathFunction};

/// The diagnostic for a source that is not a string or is too long.
pub const LENGTH_MESSAGE: &str = "Use an expression of at most 2000 characters.";

const CONTEXT_NAMES: [&str; 11] = ["time", "t", "T", "fps", "value", "layer", "comp", "ch", "idx", "PI", "Math"];

/// `EXPRESSION_NAMES`: context names, helpers, then `Math` functions. The
/// native dialect adds `frame` after `fps`.
pub fn expression_names(dialect: Dialect) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(40);
    for name in CONTEXT_NAMES {
        names.push(name);
        if name == "fps" && dialect == Dialect::Native {
            names.push("frame");
        }
    }
    names.extend(Helper::ALL.iter().map(|h| h.name()));
    names.extend(MathFunction::ALL.iter().map(|m| m.name()));
    names
}

fn is_known(name: &str, dialect: Dialect) -> bool {
    CONTEXT_NAMES.contains(&name)
        || (dialect == Dialect::Native && name == "frame")
        || Helper::from_name(name).is_some()
        || MathFunction::from_name(name).is_some()
}

/// `expressionDiagnostic(source)`: `None` when the editor accepts the
/// expression, otherwise the message it shows. Identifiers are checked on the
/// parsed tree (string literals and member keys are not names), in the order
/// `expression.ts` visits nodes, so the first problem reported is the same.
pub fn expression_diagnostic(source: &str, dialect: Dialect) -> Option<String> {
    let units: Vec<u16> = source.encode_utf16().collect();
    if units.len() > MAX_SOURCE_CHARS {
        return Some(LENGTH_MESSAGE.into());
    }
    match parse(&units) {
        Err(error) => Some(error.0),
        Ok(tree) => visit(&tree, dialect).err(),
    }
}

fn visit(node: &Node, dialect: Dialect) -> Result<(), String> {
    if let Node::Identifier(name) = node
        && !is_known(name, dialect)
    {
        return Err(format!("Unknown name “{name}”. Use time, value, or a supported function."));
    }
    if let Node::Call(callee, _) = node {
        if let Node::Identifier(name) = &**callee
            && Helper::from_name(name).is_none()
            && MathFunction::from_name(name).is_none()
        {
            return Err(format!("“{name}” is not a supported function."));
        }
        if let Node::Member(object, property) = &**callee {
            let math_object = matches!(&**object, Node::Identifier(n) if n == "Math");
            // `Object.hasOwn(MATH_FUNCTIONS, property.value)`: only a string
            // literal can name one of the functions.
            let documented = matches!(&**property, Node::Literal(Literal::String(s))
                if s.to_rust_string().is_some_and(|t| MathFunction::from_name(&t).is_some()));
            if !(math_object && documented) {
                return Err("Only the documented Math functions can be called.".into());
            }
        }
    }
    match node {
        Node::Literal(_) | Node::Identifier(_) => Ok(()),
        Node::Unary(_, value) => visit(value, dialect),
        Node::Binary(_, left, right) => {
            visit(left, dialect)?;
            visit(right, dialect)
        }
        Node::Conditional(test, yes, no) => {
            visit(test, dialect)?;
            visit(yes, dialect)?;
            visit(no, dialect)
        }
        Node::Member(object, property) => {
            visit(object, dialect)?;
            visit(property, dialect)
        }
        Node::Call(callee, args) => {
            visit(callee, dialect)?;
            args.iter().try_for_each(|arg| visit(arg, dialect))
        }
    }
}
