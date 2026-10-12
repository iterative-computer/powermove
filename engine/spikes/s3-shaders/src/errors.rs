//! Map compiler diagnostics to the author's line numbers (plan section 11, step 7).
//!
//! Normalization keeps line counts, so a glslang line is a program line. The
//! program is `preamble + generated declarations + user source`; `line_offset`
//! is the number of program lines before the user's first line.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Diagnostic {
    /// 1-based line in the author's source, or `None` when the problem is in
    /// Powermove's preamble or generated declarations.
    pub line: Option<u32>,
    pub message: String,
}

/// Parse glslang's `ERROR: 0:LINE: message` lines.
pub fn map(log: &str, line_offset: u32) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for raw in log.lines() {
        let raw = raw.trim();
        let Some(rest) = raw.strip_prefix("ERROR: ") else { continue };
        if rest.contains("compilation errors.") || rest.starts_with("Linking") {
            continue;
        }
        let mut fields = rest.splitn(3, ':');
        let (Some(_file), Some(line), Some(message)) = (fields.next(), fields.next(), fields.next()) else {
            out.push(Diagnostic { line: None, message: rest.to_string() });
            continue;
        };
        let Ok(line) = line.trim().parse::<u32>() else {
            out.push(Diagnostic { line: None, message: rest.to_string() });
            continue;
        };
        let user = line.checked_sub(line_offset).filter(|l| *l >= 1);
        let message = message.trim();
        if message.starts_with("compilation terminated") || message.contains("compilation errors.  No code generated") {
            continue;
        }
        out.push(Diagnostic { line: user, message: message.to_string() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_program_lines_to_user_lines() {
        let log = "ERROR: 0:42: 'foo' : undeclared identifier\nERROR: 0:3: 'bar' : syntax error\nERROR: 1 compilation errors.  No code generated.\n";
        let d = map(log, 40);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], Diagnostic { line: Some(2), message: "'foo' : undeclared identifier".into() });
        assert_eq!(d[1].line, None);
    }
}
