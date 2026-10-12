//! The precedence-climbing parser of `expression.ts`, including its limits
//! and its quirks.
//!
//! Two JavaScript behaviours of the original are part of the language and are
//! reproduced here:
//!
//! * Tokens are compared by value, whatever their kind: a string literal
//!   `"+"` in operator position is the `+` operator, and `"("` after a value
//!   opens a call.
//! * The operator table is a plain object, so a token whose value names an
//!   `Object.prototype` member (`constructor`, `toString`, `__proto__`, …)
//!   finds a function (or `Object.prototype`) as its precedence. Such a token
//!   is consumed as an inert binary operator, and the minimum precedence of
//!   its right operand becomes a string (`function toString() { [native
//!   code] }1`), which then compares with later precedences as JavaScript's
//!   `<` does.

use crate::lexer::{Lexer, SyntaxError, Tok, Token};
use crate::string::JsStr;

pub const MAX_SOURCE_CHARS: usize = 2_000;
pub const MAX_NODES: usize = 160;
pub const MAX_DEPTH: usize = 24;
pub const MAX_CALL_ARGS: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Literal {
    Number(f64),
    String(JsStr),
    Bool(bool),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Plus,
    Minus,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    /// An `Object.prototype` member name used as an operator: both operands
    /// are evaluated and the result is `undefined`.
    Inert,
}

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Literal(Literal),
    Identifier(String),
    Unary(UnaryOp, Box<Node>),
    Binary(BinaryOp, Box<Node>, Box<Node>),
    Conditional(Box<Node>, Box<Node>, Box<Node>),
    Member(Box<Node>, Box<Node>),
    Call(Box<Node>, Vec<Node>),
}

/// `Object.prototype` member names, in V8's property order, with the name of
/// the function each one holds (`None` for `__proto__`, which holds
/// `Object.prototype` itself).
pub(crate) const OBJECT_PROTOTYPE: [(&str, Option<&str>); 12] = [
    ("constructor", Some("Object")),
    ("__defineGetter__", Some("__defineGetter__")),
    ("__defineSetter__", Some("__defineSetter__")),
    ("hasOwnProperty", Some("hasOwnProperty")),
    ("__lookupGetter__", Some("__lookupGetter__")),
    ("__lookupSetter__", Some("__lookupSetter__")),
    ("isPrototypeOf", Some("isPrototypeOf")),
    ("propertyIsEnumerable", Some("propertyIsEnumerable")),
    ("toString", Some("toString")),
    ("valueOf", Some("valueOf")),
    ("__proto__", None),
    ("toLocaleString", Some("toLocaleString")),
];

const PRECEDENCE: [(&str, i32, BinaryOp); 16] = [
    ("||", 1, BinaryOp::Or),
    ("&&", 2, BinaryOp::And),
    ("==", 3, BinaryOp::Eq),
    ("!=", 3, BinaryOp::Ne),
    ("===", 3, BinaryOp::StrictEq),
    ("!==", 3, BinaryOp::StrictNe),
    ("<", 4, BinaryOp::Lt),
    ("<=", 4, BinaryOp::Le),
    (">", 4, BinaryOp::Gt),
    (">=", 4, BinaryOp::Ge),
    ("+", 5, BinaryOp::Add),
    ("-", 5, BinaryOp::Sub),
    ("*", 6, BinaryOp::Mul),
    ("/", 6, BinaryOp::Div),
    ("%", 6, BinaryOp::Rem),
    ("**", 7, BinaryOp::Pow),
];

/// `PRECEDENCE[op]`: a number, an inherited function, or `Object.prototype`.
#[derive(Clone, Copy, Debug)]
enum Precedence {
    Number(i32, BinaryOp),
    /// The `String()` of the inherited value: `function NAME() { [native
    /// code] }` or `[object Object]`.
    Inherited(&'static str),
}

/// The minimum precedence argument: a number or, after an inherited
/// "operator", a string.
#[derive(Clone, Debug)]
enum Minimum {
    Number(i32),
    String(String),
}

fn inherited_text(function: Option<&str>) -> &'static str {
    match function {
        None => "[object Object]",
        Some("Object") => "function Object() { [native code] }",
        Some("__defineGetter__") => "function __defineGetter__() { [native code] }",
        Some("__defineSetter__") => "function __defineSetter__() { [native code] }",
        Some("hasOwnProperty") => "function hasOwnProperty() { [native code] }",
        Some("__lookupGetter__") => "function __lookupGetter__() { [native code] }",
        Some("__lookupSetter__") => "function __lookupSetter__() { [native code] }",
        Some("isPrototypeOf") => "function isPrototypeOf() { [native code] }",
        Some("propertyIsEnumerable") => "function propertyIsEnumerable() { [native code] }",
        Some("toString") => "function toString() { [native code] }",
        Some("valueOf") => "function valueOf() { [native code] }",
        Some("toLocaleString") => "function toLocaleString() { [native code] }",
        Some(_) => unreachable!("every Object.prototype function is listed"),
    }
}

fn precedence(token: &Token) -> Option<Precedence> {
    for (op, p, bin) in PRECEDENCE {
        if token.is(op) {
            return Some(Precedence::Number(p, bin));
        }
    }
    for (name, function) in OBJECT_PROTOTYPE {
        if token.is(name) {
            return Some(Precedence::Inherited(inherited_text(function)));
        }
    }
    None
}

/// `precedence < minimum` under JavaScript's abstract relational comparison.
fn less_than(precedence: Precedence, minimum: &Minimum) -> bool {
    match (precedence, minimum) {
        (Precedence::Number(p, _), Minimum::Number(m)) => p < *m,
        // A number against a non-numeric string compares with NaN.
        (Precedence::Number(..), Minimum::String(_)) => false,
        // An object becomes its string, which is NaN against a number.
        (Precedence::Inherited(_), Minimum::Number(_)) => false,
        // Two strings compare by code units (all ASCII here).
        (Precedence::Inherited(text), Minimum::String(m)) => text < m.as_str(),
    }
}

/// `precedence + (op === '**' ? 0 : 1)`.
fn next_minimum(precedence: Precedence, is_pow: bool) -> Minimum {
    match precedence {
        Precedence::Number(p, _) => Minimum::Number(p + if is_pow { 0 } else { 1 }),
        Precedence::Inherited(text) => Minimum::String(format!("{text}1")),
    }
}

pub(crate) struct Parser<'s> {
    lexer: Lexer<'s>,
    token: Token,
    nodes: usize,
}

impl<'s> Parser<'s> {
    pub fn new(source: &'s [u16]) -> Result<Parser<'s>, SyntaxError> {
        let mut lexer = Lexer::new(source);
        let token = lexer.next_token()?;
        Ok(Parser {
            lexer,
            token,
            nodes: 0,
        })
    }

    pub fn parse(mut self) -> Result<Node, SyntaxError> {
        let value = self.expression(Minimum::Number(0), 0)?;
        if !matches!(self.token.tok, Tok::Eof) {
            return Err(SyntaxError(format!(
                "Unexpected token at {}",
                self.token.at
            )));
        }
        Ok(value)
    }

    fn node(&mut self, value: Node, depth: usize) -> Result<Node, SyntaxError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(SyntaxError("Expression is too complex".into()));
        }
        Ok(value)
    }

    fn advance(&mut self) -> Result<Token, SyntaxError> {
        let next = self.lexer.next_token()?;
        Ok(std::mem::replace(&mut self.token, next))
    }

    fn take(&mut self, value: &str) -> Result<bool, SyntaxError> {
        if !self.token.is(value) {
            return Ok(false);
        }
        self.advance()?;
        Ok(true)
    }

    fn require(&mut self, value: &str) -> Result<(), SyntaxError> {
        if !self.take(value)? {
            return Err(SyntaxError(format!(
                "Expected {value} at {}",
                self.token.at
            )));
        }
        Ok(())
    }

    fn expression(&mut self, minimum: Minimum, depth: usize) -> Result<Node, SyntaxError> {
        let mut left = self.prefix(depth + 1)?;
        while self.token.has_string_value() {
            let Some(precedence) = precedence(&self.token) else {
                break;
            };
            if less_than(precedence, &minimum) {
                break;
            }
            let is_pow = self.token.is("**");
            self.advance()?;
            let right = self.expression(next_minimum(precedence, is_pow), depth + 1)?;
            let op = match precedence {
                Precedence::Number(_, op) => op,
                Precedence::Inherited(_) => BinaryOp::Inert,
            };
            left = self.node(Node::Binary(op, Box::new(left), Box::new(right)), depth)?;
        }
        if matches!(minimum, Minimum::Number(0)) && self.take("?")? {
            let yes = self.expression(Minimum::Number(0), depth + 1)?;
            self.require(":")?;
            let no = self.expression(Minimum::Number(0), depth + 1)?;
            left = self.node(
                Node::Conditional(Box::new(left), Box::new(yes), Box::new(no)),
                depth,
            )?;
        }
        Ok(left)
    }

    fn prefix(&mut self, depth: usize) -> Result<Node, SyntaxError> {
        let unary = if self.token.is("+") {
            Some(UnaryOp::Plus)
        } else if self.token.is("-") {
            Some(UnaryOp::Minus)
        } else if self.token.is("!") {
            Some(UnaryOp::Not)
        } else {
            None
        };
        if let Some(op) = unary {
            self.advance()?;
            let value = self.prefix(depth + 1)?;
            return self.node(Node::Unary(op, Box::new(value)), depth);
        }

        let mut value = match &self.token.tok {
            Tok::Number(_) | Tok::Str(_) => {
                let literal = match self.advance()?.tok {
                    Tok::Number(n) => Literal::Number(n),
                    Tok::Str(s) => Literal::String(s),
                    _ => unreachable!(),
                };
                self.node(Node::Literal(literal), depth)?
            }
            Tok::Ident(_) => {
                let Tok::Ident(name) = self.advance()?.tok else {
                    unreachable!()
                };
                let node = match name.as_str() {
                    "true" => Node::Literal(Literal::Bool(true)),
                    "false" => Node::Literal(Literal::Bool(false)),
                    "null" => Node::Literal(Literal::Null),
                    _ => Node::Identifier(name),
                };
                self.node(node, depth)?
            }
            _ => {
                if self.take("(")? {
                    let inner = self.expression(Minimum::Number(0), depth + 1)?;
                    self.require(")")?;
                    inner
                } else {
                    return Err(SyntaxError(format!(
                        "Expected a value at {}",
                        self.token.at
                    )));
                }
            }
        };

        loop {
            if self.take(".")? {
                let Tok::Ident(_) = &self.token.tok else {
                    return Err(SyntaxError(format!(
                        "Expected a property at {}",
                        self.token.at
                    )));
                };
                let Tok::Ident(name) = self.advance()?.tok else {
                    unreachable!()
                };
                let property = self.node(
                    Node::Literal(Literal::String(JsStr::from(name.as_str()))),
                    depth + 1,
                )?;
                value = self.node(Node::Member(Box::new(value), Box::new(property)), depth)?;
            } else if self.take("[")? {
                let property = self.expression(Minimum::Number(0), depth + 1)?;
                self.require("]")?;
                value = self.node(Node::Member(Box::new(value), Box::new(property)), depth)?;
            } else if self.take("(")? {
                let mut args = Vec::new();
                if !self.take(")")? {
                    loop {
                        if args.len() >= MAX_CALL_ARGS {
                            return Err(SyntaxError(
                                "Expression call has too many arguments".into(),
                            ));
                        }
                        args.push(self.expression(Minimum::Number(0), depth + 1)?);
                        if !self.take(",")? {
                            break;
                        }
                    }
                    self.require(")")?;
                }
                value = self.node(Node::Call(Box::new(value), args), depth)?;
            } else {
                break;
            }
        }
        Ok(value)
    }
}

/// Parses UTF-16 source with the limits of `expression.ts` (the length limit
/// is checked by the callers, which differ in how they report it).
pub(crate) fn parse(source: &[u16]) -> Result<Node, SyntaxError> {
    Parser::new(source)?.parse()
}
