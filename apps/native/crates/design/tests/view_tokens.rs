//! The Settings window, the sign-in sheet, the welcome window, the visual
//! pickers, and the Agent composer (its field, its pill, the model picker
//! and the reasoning menu) and panel name no colour and no text size of
//! their own: every one comes from pm-design (`Colors`, `Settings`,
//! `Welcome`, `TextSize`, `Ink`, the token table). This fails on a raw hex
//! colour, an RGB literal, an ink alpha written as a byte, or a numeric font
//! size in those files; in the panel's TypeScript, on a style or metric
//! written as a number or a colour rather than a token.
use std::path::Path;

const VIEWS: &[&str] = &[
    "../shell/src/settings_window.rs",
    "../shell/src/welcome_view.rs",
    "../app/src/settings_window/form.rs",
    "../app/src/settings_window/form/previews.rs",
    "../app/src/settings_window/form/agent_app_icons.rs",
    "../app/src/sign_in.rs",
    "../shell/src/agent_chooser.rs",
    "../shell/src/pill.rs",
    "../shell/src/text.rs",
];

/// The Agent panel's TypeScript views (the composer, the pill, the empty
/// state and the thread).
const PANELS: &[&str] = &["../../js/v4/panels/agent/view.tsx"];

/// Style keys whose value is a size, a space, a radius, a colour or type.
const VISUAL: &[&str] = &[
    "fontSize",
    "fontWeight",
    "lineHeight",
    "symbolSize",
    "color",
    "background",
    "borderColor",
    "borderRadius",
    "borderWidth",
    "width",
    "height",
    "minWidth",
    "minHeight",
    "maxWidth",
    "maxHeight",
    "gap",
    "padding",
    "paddingLeft",
    "paddingRight",
    "paddingTop",
    "paddingBottom",
    "margin",
    "marginLeft",
    "marginRight",
    "marginTop",
    "marginBottom",
];

/// What on a TypeScript `line` is a style written as a literal: a visual key
/// set to a number other than zero, or to a colour string.
fn panel_violation(line: &str) -> Option<String> {
    let code = line.trim_start();
    if code.starts_with("//") || code.starts_with('*') || code.starts_with("/*") {
        return None;
    }
    for key in VISUAL {
        let mut rest = line;
        while let Some(at) = rest.find(key) {
            let before = rest[..at].chars().next_back();
            let after = rest[at + key.len()..].trim_start();
            rest = &rest[at + key.len()..];
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let Some(value) = after.strip_prefix(':') else {
                continue;
            };
            let value = value.trim_start();
            let number: String = value
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !number.is_empty() && number.parse::<f64>().is_ok_and(|n| n != 0.) {
                return Some(format!("{key} written as {number}"));
            }
            if value.starts_with("\"#") || value.starts_with("\"rgb") || value.starts_with("'#") {
                return Some(format!("{key} written as a colour"));
            }
        }
    }
    None
}

/// Calls whose size or colour argument must be a token, never a number.
const SIZED: &[&str] = &[
    "text_size(px(",
    "systemFontOfSize(",
    "systemFontOfSize_weight(",
    "configurationWithPointSize_weight_scale(",
    "colorWithSRGBRed_green_blue_alpha(",
    "colorWithRed_green_blue_alpha(",
    "rgb(",
    "rgba(",
];

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// What on `line` is a literal colour or size, if anything.
fn violation(line: &str) -> Option<String> {
    let code = line.trim_start();
    if code.starts_with("//") {
        return None;
    }
    let chars: Vec<char> = line.chars().collect();
    for i in 0..chars.len() {
        // 0xRRGGBB (six or more hex digits in a row).
        if chars[i] == '0'
            && chars.get(i + 1) == Some(&'x')
            && chars[i + 2..].iter().take_while(|c| is_hex(**c)).count() >= 6
        {
            return Some("hex colour (0x…)".into());
        }
        // #rgb, #rrggbb, #rrggbbaa in a string.
        if chars[i] == '#' {
            let run = chars[i + 1..].iter().take_while(|c| is_hex(**c)).count();
            let after = chars.get(i + 1 + run).copied().unwrap_or(' ');
            if matches!(run, 3 | 6 | 8) && !after.is_ascii_alphanumeric() && after != '_' {
                return Some("hex colour (#…)".into());
            }
        }
    }
    if line.contains("<< 8) | 0x") {
        return Some("ink alpha as a byte (use an `Ink` step)".into());
    }
    for call in SIZED {
        let mut rest = line;
        while let Some(at) = rest.find(call) {
            let arg = rest[at + call.len()..].trim_start();
            if arg.starts_with(|c: char| c.is_ascii_digit()) {
                return Some(format!("literal in {call}…)"));
            }
            rest = &rest[at + call.len()..];
        }
    }
    None
}

#[test]
fn views_take_colours_and_text_sizes_from_tokens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for file in VIEWS {
        let text = std::fs::read_to_string(root.join(file)).expect(file);
        // A file's tests may spell out what they check.
        let body = text.split("#[cfg(test)]").next().unwrap_or(&text);
        let lines: Vec<&str> = body.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            // A call whose argument starts on the next line.
            let joined = match lines.get(n + 1) {
                Some(next) if SIZED.iter().any(|c| line.trim_end().ends_with(c)) => {
                    format!("{line}{}", next.trim_start())
                }
                _ => line.to_string(),
            };
            if let Some(what) = violation(&joined) {
                found.push(format!("{file}:{}: {what}: {}", n + 1, line.trim()));
            }
        }
    }
    for file in PANELS {
        let text = std::fs::read_to_string(root.join(file)).expect(file);
        for (n, line) in text.lines().enumerate() {
            if let Some(what) = panel_violation(line) {
                found.push(format!("{file}:{}: {what}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "colours and text sizes come from pm-design tokens:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_panel_lint_catches_what_it_should() {
    for bad in [
        "fontSize: 15,",
        "style={{ gap: 6, color: t(\"colors.text\") }}",
        "background: \"#2E2E2E\",",
        "borderRadius: 8",
    ] {
        assert!(panel_violation(bad).is_some(), "{bad}");
    }
    for good in [
        "fontSize: t(\"typography.body\"),",
        "left: 0, right: 0, bottom: 0,",
        "marginLeft: t(\"space.xs\"),",
        "// fontSize: 15 was the old composer size",
        "const latency = { tokenWidth: 4 };",
        "width: 0,",
    ] {
        assert!(panel_violation(good).is_none(), "{good}");
    }
}

#[test]
fn the_lint_catches_what_it_should() {
    for bad in [
        "let c = 0xff7a2e;",
        r##"<stop stop-color="#ffffff"/>"##,
        ".bg(rgba((c.text << 8) | 0x1f))",
        ".text_size(px(13.))",
        "NSFont::systemFontOfSize(14.)",
        "window.paint_path(path, rgb(0xffffff));",
    ] {
        assert!(violation(bad).is_some(), "{bad}");
    }
    for good in [
        ".text_size(px(ts().base))",
        "#[derive(Clone)]",
        "const KEY: u64 = 0x5160_0000;",
        "// near #2E2E2E",
        "rgba(alpha(c.text, ink().edge))",
    ] {
        assert!(violation(good).is_none(), "{good}");
    }
}
