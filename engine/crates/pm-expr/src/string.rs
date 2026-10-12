//! JavaScript strings: sequences of UTF-16 code units.

use std::fmt;
use std::sync::Arc;

/// An immutable JavaScript string (UTF-16 code units, lone surrogates allowed).
///
/// Equality and ordering compare code units, as `===` and `<` do in
/// JavaScript. Cloning is cheap.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsStr(Arc<[u16]>);

impl JsStr {
    pub fn from_units(units: impl Into<Arc<[u16]>>) -> JsStr {
        JsStr(units.into())
    }

    pub fn units(&self) -> &[u16] {
        &self.0
    }

    /// `string.length`: the number of UTF-16 code units.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The string as Rust text, or `None` when it holds a lone surrogate.
    pub fn to_rust_string(&self) -> Option<String> {
        String::from_utf16(&self.0).ok()
    }

    /// The string as Rust text with lone surrogates replaced by U+FFFD.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    /// `a + b`.
    pub fn concat(&self, other: &JsStr) -> JsStr {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let mut units = Vec::with_capacity(self.len() + other.len());
        units.extend_from_slice(&self.0);
        units.extend_from_slice(&other.0);
        JsStr(units.into())
    }

    /// Whether the string equals the ASCII or Unicode text `s`.
    pub fn eq_str(&self, s: &str) -> bool {
        let mut units = self.0.iter();
        for u in s.encode_utf16() {
            if units.next() != Some(&u) {
                return false;
            }
        }
        units.next().is_none()
    }
}

impl Default for JsStr {
    fn default() -> JsStr {
        JsStr(Arc::from(Vec::new()))
    }
}

impl From<&str> for JsStr {
    fn from(s: &str) -> JsStr {
        JsStr(s.encode_utf16().collect::<Vec<u16>>().into())
    }
}

impl From<String> for JsStr {
    fn from(s: String) -> JsStr {
        JsStr::from(s.as_str())
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_string_lossy(), f)
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::JsStr;

    #[test]
    fn length_counts_code_units() {
        assert_eq!(JsStr::from("a😀").len(), 3);
        assert!(JsStr::from("abc").eq_str("abc"));
        assert!(!JsStr::from("abc").eq_str("ab"));
        assert!(JsStr::from("a") < JsStr::from("b"));
        assert!(JsStr::from("Z") < JsStr::from("a"));
        assert_eq!(
            JsStr::from("ab").concat(&JsStr::from("c")),
            JsStr::from("abc")
        );
    }
}
