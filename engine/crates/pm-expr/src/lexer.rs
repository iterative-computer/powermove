//! The tokenizer of `expression.ts`, over UTF-16 code units so that error
//! positions and the length limit count what JavaScript counts.

use crate::string::JsStr;

#[derive(Clone, Debug)]
pub(crate) enum Tok {
    Number(f64),
    Str(JsStr),
    Ident(String),
    Op(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub tok: Tok,
    /// UTF-16 index of the token's first code unit.
    pub at: usize,
}

impl Token {
    /// `token.value === s` in `expression.ts`: compares the token's value
    /// regardless of its kind (a string literal `"("` equals `'('`).
    pub fn is(&self, s: &str) -> bool {
        match &self.tok {
            Tok::Number(_) => false,
            Tok::Str(v) => v.eq_str(s),
            Tok::Ident(v) => v == s,
            Tok::Op(v) => *v == s,
            Tok::Eof => s.is_empty(),
        }
    }

    /// `typeof token.value === 'string'`.
    pub fn has_string_value(&self) -> bool {
        !matches!(self.tok, Tok::Number(_))
    }
}

/// A parse failure: the `SyntaxError` message `expression.ts` throws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError(pub String);

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SyntaxError {}

/// `/\s/` in JavaScript: WhiteSpace and LineTerminator code points.
pub(crate) fn is_js_space(u: u16) -> bool {
    matches!(
        u,
        0x09..=0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
    )
}

fn is_digit(u: Option<u16>) -> bool {
    matches!(u, Some(0x30..=0x39))
}

fn is_ident_start(u: u16) -> bool {
    matches!(u, 0x41..=0x5a | 0x61..=0x7a | 0x5f | 0x24)
}

fn is_ident_part(u: u16) -> bool {
    is_ident_start(u) || matches!(u, 0x30..=0x39)
}

const MULTI_OPERATORS: [&str; 9] = ["===", "!==", "**", "&&", "||", "==", "!=", "<=", ">="];
const SINGLE_OPERATORS: [&str; 16] = [
    "+", "-", "*", "/", "%", "<", ">", "(", ")", "[", "]", "!", ",", ".", "?", ":",
];

pub(crate) struct Lexer<'s> {
    source: &'s [u16],
    at: usize,
}

impl<'s> Lexer<'s> {
    pub fn new(source: &'s [u16]) -> Lexer<'s> {
        Lexer { source, at: 0 }
    }

    fn unit(&self, i: usize) -> Option<u16> {
        self.source.get(i).copied()
    }

    pub fn next_token(&mut self) -> Result<Token, SyntaxError> {
        while self.unit(self.at).is_some_and(is_js_space) {
            self.at += 1;
        }
        let at = self.at;
        let Some(first) = self.unit(at) else {
            return Ok(Token { tok: Tok::Eof, at });
        };

        if is_digit(Some(first)) || first == u16::from(b'.') && is_digit(self.unit(at + 1)) {
            return self.number(at);
        }

        if first == u16::from(b'"') || first == u16::from(b'\'') {
            return self.string(at, first);
        }

        if is_ident_start(first) {
            let mut end = at + 1;
            while self.unit(end).is_some_and(is_ident_part) {
                end += 1;
            }
            self.at = end;
            let name: String = self.source[at..end]
                .iter()
                .map(|&u| u as u8 as char)
                .collect();
            return Ok(Token {
                tok: Tok::Ident(name),
                at,
            });
        }

        for op in MULTI_OPERATORS {
            if self.starts_with(at, op) {
                self.at += op.len();
                return Ok(Token {
                    tok: Tok::Op(op),
                    at,
                });
            }
        }
        for op in SINGLE_OPERATORS {
            if u16::from(op.as_bytes()[0]) == first {
                self.at += 1;
                return Ok(Token {
                    tok: Tok::Op(op),
                    at,
                });
            }
        }
        Err(SyntaxError(format!("Unsupported token at {at}")))
    }

    fn starts_with(&self, at: usize, op: &str) -> bool {
        op.bytes()
            .enumerate()
            .all(|(i, b)| self.unit(at + i) == Some(u16::from(b)))
    }

    /// `/^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/` then `Number(match)`.
    fn number(&mut self, at: usize) -> Result<Token, SyntaxError> {
        let mut end = at;
        if is_digit(self.unit(end)) {
            while is_digit(self.unit(end)) {
                end += 1;
            }
            if self.unit(end) == Some(u16::from(b'.')) {
                end += 1;
                while is_digit(self.unit(end)) {
                    end += 1;
                }
            }
        } else {
            // '.' followed by a digit
            end += 1;
            while is_digit(self.unit(end)) {
                end += 1;
            }
        }
        if matches!(self.unit(end), Some(0x65 | 0x45)) {
            let mut e = end + 1;
            if matches!(self.unit(e), Some(0x2b | 0x2d)) {
                e += 1;
            }
            if is_digit(self.unit(e)) {
                while is_digit(self.unit(e)) {
                    e += 1;
                }
                end = e;
            }
        }
        let text: String = self.source[at..end]
            .iter()
            .map(|&u| u as u8 as char)
            .collect();
        self.at = end;
        let value = parse_decimal_literal(&text);
        if !value.is_finite() {
            return Err(SyntaxError(format!("Invalid number at {at}")));
        }
        Ok(Token {
            tok: Tok::Number(value),
            at,
        })
    }

    fn string(&mut self, at: usize, quote: u16) -> Result<Token, SyntaxError> {
        let mut value: Vec<u16> = Vec::new();
        self.at += 1;
        while self.at < self.source.len() {
            let c = self.source[self.at];
            self.at += 1;
            if c == quote {
                return Ok(Token {
                    tok: Tok::Str(JsStr::from_units(value)),
                    at,
                });
            }
            if c == u16::from(b'\n') || c == u16::from(b'\r') {
                return Err(SyntaxError(format!("Unterminated string at {at}")));
            }
            if c != u16::from(b'\\') {
                value.push(c);
                continue;
            }
            let Some(escaped) = self.unit(self.at) else {
                break;
            };
            self.at += 1;
            let replaced = match escaped {
                0x6e => 0x0a,   // n
                0x72 => 0x0d,   // r
                0x74 => 0x09,   // t
                0x62 => 0x08,   // b
                0x66 => 0x0c,   // f
                0x76 => 0x0b,   // v
                other => other, // \\ " ' and every unknown escape keep the character
            };
            value.push(replaced);
        }
        Err(SyntaxError(format!("Unterminated string at {at}")))
    }
}

/// `Number(text)` for text matched by the number pattern above. Rust's float
/// parser is correctly rounded like ECMAScript's and accepts `1.`, `.5`, and
/// `1.e5`.
fn parse_decimal_literal(text: &str) -> f64 {
    text.parse::<f64>().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> Vec<Tok> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut lexer = Lexer::new(&units);
        let mut out = Vec::new();
        loop {
            let t = lexer.next_token().unwrap();
            if matches!(t.tok, Tok::Eof) {
                return out;
            }
            out.push(t.tok);
        }
    }

    #[test]
    fn numbers_match_the_regex() {
        assert!(matches!(lex("1.")[..], [Tok::Number(v)] if v == 1.0));
        assert!(matches!(lex(".5")[..], [Tok::Number(v)] if v == 0.5));
        assert!(matches!(lex("1.e2")[..], [Tok::Number(v)] if v == 100.0));
        assert!(matches!(lex("1e")[..], [Tok::Number(v), Tok::Ident(_)] if v == 1.0));
        assert!(
            matches!(lex("1.5.5")[..], [Tok::Number(a), Tok::Number(b)] if a == 1.5 && b == 0.5)
        );
        assert!(matches!(lex("0.1")[..], [Tok::Number(v)] if v == 0.1));
    }

    #[test]
    fn errors_report_utf16_positions() {
        let units: Vec<u16> = "'😀' @".encode_utf16().collect();
        let mut lexer = Lexer::new(&units);
        lexer.next_token().unwrap();
        assert_eq!(lexer.next_token().unwrap_err().0, "Unsupported token at 5");
        let units: Vec<u16> = "1e999".encode_utf16().collect();
        assert_eq!(
            Lexer::new(&units).next_token().unwrap_err().0,
            "Invalid number at 0"
        );
    }
}
