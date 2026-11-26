//! The Settings window, the sign-in sheet, the welcome window and the visual
//! pickers name no colour and no text size of their own: every one comes from
//! pm-design (`Colors`, `Settings`, `Welcome`, `TextSize`, `Ink`). This fails
//! on a raw hex colour, an RGB literal, an ink alpha written as a byte, or a
//! numeric font size in those files.
use std::path::Path;

const VIEWS: &[&str] = &[
    "../shell/src/settings_window.rs",
    "../shell/src/welcome_view.rs",
    "../app/src/settings_window/form.rs",
    "../app/src/settings_window/form/previews.rs",
    "../app/src/settings_window/form/agent_app_icons.rs",
    "../app/src/sign_in.rs",
];

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
    assert!(
        found.is_empty(),
        "colours and text sizes come from pm-design tokens:\n{}",
        found.join("\n")
    );
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
