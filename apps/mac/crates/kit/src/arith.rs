//! The numeric field's deliberately small, non-executable arithmetic language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidExpression;

pub fn parse(source: &str) -> Result<f64, InvalidExpression> {
    struct Parser<'a> {
        s: &'a [u8],
        i: usize,
        depth: usize,
    }
    impl Parser<'_> {
        fn ws(&mut self) {
            while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
                self.i += 1;
            }
        }
        fn take(&mut self, c: u8) -> bool {
            self.ws();
            if self.s.get(self.i) == Some(&c) {
                self.i += 1;
                true
            } else {
                false
            }
        }
        fn finite(n: f64) -> Result<f64, InvalidExpression> {
            if n.is_finite() {
                Ok(n)
            } else {
                Err(InvalidExpression)
            }
        }
        fn unary(&mut self) -> Result<f64, InvalidExpression> {
            self.depth += 1;
            if self.depth > 128 {
                return Err(InvalidExpression);
            }
            let result = if self.take(b'+') {
                self.unary()
            } else if self.take(b'-') {
                self.unary().map(|n| -n)
            } else if self.take(b'(') {
                let n = self.expr()?;
                if !self.take(b')') {
                    return Err(InvalidExpression);
                }
                Ok(n)
            } else {
                self.ws();
                let start = self.i;
                while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
                if self.s.get(self.i) == Some(&b'.') {
                    self.i += 1;
                    while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                }
                if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
                    self.i += 1;
                    if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                        self.i += 1;
                    }
                    while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                }
                let raw =
                    std::str::from_utf8(&self.s[start..self.i]).map_err(|_| InvalidExpression)?;
                Self::finite(raw.parse().map_err(|_| InvalidExpression)?)
            };
            self.depth -= 1;
            result
        }
        fn product(&mut self) -> Result<f64, InvalidExpression> {
            let mut n = self.unary()?;
            loop {
                if self.take(b'*') {
                    n = Self::finite(n * self.unary()?)?;
                } else if self.take(b'/') {
                    n = Self::finite(n / self.unary()?)?;
                } else {
                    return Ok(n);
                }
            }
        }
        fn expr(&mut self) -> Result<f64, InvalidExpression> {
            let mut n = self.product()?;
            loop {
                if self.take(b'+') {
                    n = Self::finite(n + self.product()?)?;
                } else if self.take(b'-') {
                    n = Self::finite(n - self.product()?)?;
                } else {
                    return Ok(n);
                }
            }
        }
    }
    let mut p = Parser {
        s: source.as_bytes(),
        i: 0,
        depth: 0,
    };
    let n = p.expr()?;
    p.ws();
    if p.i == p.s.len() {
        Ok(n)
    } else {
        Err(InvalidExpression)
    }
}

/// Relative operators use the live value, rather than the last playback readout.
pub fn draft(source: &str, current: f64) -> Result<f64, InvalidExpression> {
    let raw = source.trim();
    if raw.starts_with(['+', '*', '/']) || raw.starts_with("-=") {
        let raw = raw.replacen("+=", "+", 1).replacen("-=", "-", 1);
        parse(&format!("{current}{raw}"))
    } else {
        parse(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic() {
        for (s, n) in [
            ("1 + 2 * 3", 7.),
            ("(1+2)*3", 9.),
            ("-4/2 + +3", 1.),
            (".5*8", 4.),
            ("1e2+2.5e-1", 100.25),
            ("--2", 2.),
            (" 42 ", 42.),
        ] {
            assert_eq!(parse(s), Ok(n), "{s}");
        }
    }
    #[test]
    fn rejects_invalid_and_executable_input() {
        for s in [
            "1 / 1e999",
            "1/(1e308*1e308)",
            "",
            "1+",
            "(1+2",
            "1 2",
            "2**3",
            "1/0",
            "NaN",
            "Infinity",
            "globalThis.alert(1)",
            "constructor.constructor(\"return 1\")()",
            "1; globalThis.pwned = true",
            "${globalThis.process}",
        ] {
            assert!(parse(s).is_err(), "{s}");
        }
        assert!(parse(&"(".repeat(1000)).is_err());
    }
    #[test]
    fn relative_and_negative() {
        for (s, n) in [
            ("+5", 25.),
            ("*2", 40.),
            ("/4", 5.),
            ("-=3", 17.),
            ("-2", -2.),
            ("+=3", 23.),
        ] {
            assert_eq!(draft(s, 20.), Ok(n));
        }
    }
}
