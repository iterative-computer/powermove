//! Streaming agent prose -> inert semantic blocks/runs. No HTML is rendered by
//! this parser, and no file read or remote request occurs while parsing text.
use crate::{error, media};
use regex::Regex;
use serde_json::{Value, json};
use std::io;
fn regex(pattern: &str) -> &'static Regex {
    static PATTERNS: std::sync::LazyLock<std::collections::BTreeMap<&'static str, Regex>> =
        std::sync::LazyLock::new(|| {
            [
r"(?i)^<br\s*/?>\n?",
r#"(?i)^((?:~/|/)[^\s<>()\[\]`'"]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|mp4|m4v|mov|webm|mp3|m4a|wav|aac|ogg|flac))(?:[^A-Za-z0-9_/]|$)"#,
r"(?i)^<(b|strong|i|em|s|del|strike|code|kbd|u|mark|sub|sup|small|span)(?:\s[^<>]*)?>",
r#"(?i)^<img\s[^<>]*?src=["']([^"']+)["'][^<>]*>"#,
r#"(?i)alt=["']([^"']*)["']"#,
r"(?i)^<(https?://[^\s<>]+)>",
r"^( {0,3})(`{3,}|~{3,})\s*([^\s`]*)[^`]*$",
r"^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$",
r"^( {0,3})([-*+]|\d{1,9}[.)])(?:([ \t]+)(.*))?$",
r"^:?-{3,}:?$",
r"^ {0,3}(=+|-{3,})[ \t]*$",
r"^\[([ xX])\](?:[ \t]+|$)"
    ].into_iter().map(|p|(p,Regex::new(p).expect("constant markdown regex"))).collect()
        });
    PATTERNS.get(pattern).expect("registered markdown regex")
}

fn name(path: &str) -> &str {
    path.split(['?', '#'])
        .next()
        .unwrap_or(path)
        .split('/')
        .rfind(|v| !v.is_empty())
        .unwrap_or(path)
}
fn media_run(target: &str, alt: &str, image: bool) -> Option<Value> {
    if let Some(local) = media::local_path(target) {
        return Some(
            json!({"text":"","media":{"kind":media::kind(&local),"src":media::file_url(&local),"path":local,"alt":if alt.is_empty(){name(&local)}else{alt}}}),
        );
    }
    if image && target.to_ascii_lowercase().starts_with("https://") {
        return Some(
            json!({"text":"","media":{"kind":media::kind(target).unwrap_or("image"),"src":target,"path":target,"alt":if alt.is_empty(){name(target)}else{alt},"remote":true}}),
        );
    }
    None
}
fn text_run(text: &str, flags: &Value) -> Value {
    let mut run = flags.clone();
    run["text"] = json!(text);
    run
}
fn flush(plain: &mut String, runs: &mut Vec<Value>, flags: &Value) {
    if !plain.is_empty() {
        runs.push(text_run(plain, flags));
        plain.clear();
    }
}
fn boundary(c: char) -> bool {
    c.is_whitespace() || "([{'\"".contains(c)
}
fn safe_link(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}
fn emphasis(rest: &str, marker: &str) -> Option<(usize, String)> {
    let body = rest.strip_prefix(marker)?;
    if body.chars().next().is_none_or(char::is_whitespace) {
        return None;
    }
    let mut at = 0;
    while let Some(position) = body[at..].find(marker) {
        let position = at + position;
        let inner = &body[..position];
        if !inner.is_empty()
            && inner
                .chars()
                .next_back()
                .is_some_and(|v| !v.is_whitespace())
            && (!marker.contains('*') || !inner.ends_with('*'))
            && (!marker.contains('_') || !inner.ends_with('_'))
        {
            return Some((marker.len() * 2 + position, inner.into()));
        }
        at = position + marker.len();
        if at >= body.len() {
            break;
        }
    }
    None
}
fn link(rest: &str) -> Option<(usize, bool, String, String)> {
    let image = rest.starts_with("![");
    let open = usize::from(image);
    if rest.as_bytes().get(open) != Some(&b'[') {
        return None;
    }
    let mut depth = 0;
    let mut end = None;
    for (i, c) in rest[open..].char_indices() {
        if c == '[' {
            depth += 1;
        }
        if c == ']' {
            depth -= 1;
            if depth == 0 {
                end = Some(open + i);
                break;
            }
        }
    }
    let end = end?;
    if rest.as_bytes().get(end + 1) != Some(&b'(') {
        return None;
    }
    let start = end + 2;
    let mut nesting = 0;
    let mut close = None;
    for (i, c) in rest[start..].char_indices() {
        if c == '(' {
            nesting += 1;
        }
        if c == ')' {
            if nesting == 0 {
                close = Some(start + i);
                break;
            }
            nesting -= 1;
        }
    }
    let close = close?;
    let target = &rest[start..close];
    let target = if let Some(angle) = target.strip_prefix('<') {
        angle.split_once('>')?.0
    } else {
        target.split(" \"").next().unwrap_or(target)
    };
    if target.is_empty() {
        return None;
    }
    Some((close + 1, image, rest[open + 1..end].into(), target.into()))
}
fn inline(text: &str, flags: &Value, depth: usize) -> io::Result<Vec<Value>> {
    if depth > 64 {
        return Err(error("Markdown nesting exceeds 64 levels"));
    }
    let mut runs = vec![];
    let mut plain = String::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let previous = text[..i].chars().next_back().unwrap_or(' ');
        let special = |c: char| {
            matches!(
                c,
                '\\' | '<' | '`' | '~' | '*' | '_' | '!' | '[' | '/' | 'h' | 'H'
            )
        };
        if !rest.chars().next().is_some_and(special) {
            let end = rest.find(special).unwrap_or(rest.len());
            plain.push_str(&rest[..end]);
            i += end;
            continue;
        }

        if rest.starts_with('\\')
            && let Some(next) = rest[1..].chars().next()
            && next.is_ascii_punctuation()
        {
            plain.push(next);
            i += 1 + next.len_utf8();
            continue;
        }
        if let Some(m) = regex(r"(?i)^<br\s*/?>\n?").find(rest) {
            plain.push('\n');
            i += m.end();
            continue;
        }
        if rest.starts_with('`') {
            let count = rest.bytes().take_while(|c| *c == b'`').count();
            let marker = "`".repeat(count);
            let body = &rest[count..];
            if let Some(end) = body.find(&marker)
                && end > 0
                && !body[..end].ends_with('`')
                && !body[end + count..].starts_with('`')
            {
                flush(&mut plain, &mut runs, flags);
                runs.push(json!({"text":body[..end].replace('\n'," "),"c":true}));
                if let Some(media) = media_run(body[..end].trim(), "", false) {
                    runs.push(media);
                }
                i += count * 2 + end;
                continue;
            }
        }
        let mut styled = false;
        for (marker, extra, allowed) in [
            ("~~", json!({"s":true}), flags["s"] != true),
            (
                "***",
                json!({"b":true,"i":true}),
                flags["b"] != true && flags["i"] != true,
            ),
            ("**", json!({"b":true}), flags["b"] != true),
            (
                "__",
                json!({"b":true}),
                flags["b"] != true && boundary(previous),
            ),
            ("*", json!({"i":true}), flags["i"] != true),
            (
                "_",
                json!({"i":true}),
                flags["i"] != true && boundary(previous),
            ),
        ] {
            if !allowed {
                continue;
            }
            if let Some((used, inner)) = emphasis(rest, marker) {
                if marker.contains('_')
                    && rest[used..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric())
                {
                    continue;
                }
                if marker == "*" && inner.contains('*') || marker == "_" && inner.contains('_') {
                    continue;
                }
                flush(&mut plain, &mut runs, flags);
                let mut nested = flags.clone();
                for (k, v) in extra.as_object().unwrap() {
                    nested[k] = v.clone();
                }
                runs.extend(inline(&inner, &nested, depth + 1)?);
                i += used;
                styled = true;
                break;
            }
        }
        if styled {
            continue;
        }
        if let Some(m) = regex(
            r"(?i)^<(b|strong|i|em|s|del|strike|code|kbd|u|mark|sub|sup|small|span)(?:\s[^<>]*)?>",
        )
        .captures(rest)
        {
            let tag = m[1].to_ascii_lowercase();
            let end_tag = format!("</{tag}>");
            let start = m[0].len();
            if let Some(end) = rest[start..].to_ascii_lowercase().find(&end_tag) {
                let inner = &rest[start..start + end];
                flush(&mut plain, &mut runs, flags);
                if matches!(tag.as_str(), "code" | "kbd") {
                    runs.push(json!({"text":inner,"c":true}));
                } else {
                    let mut next = flags.clone();
                    let key = match tag.as_str() {
                        "b" | "strong" => Some("b"),
                        "i" | "em" => Some("i"),
                        "s" | "del" | "strike" => Some("s"),
                        _ => None,
                    };
                    if let Some(key) = key {
                        next[key] = json!(true);
                    }
                    runs.extend(inline(inner, &next, depth + 1)?);
                }
                i += start + end + end_tag.len();
                continue;
            }
        }
        if let Some(m) = regex(r#"(?i)^<img\s[^<>]*?src=["']([^"']+)["'][^<>]*>"#).captures(rest) {
            let alt = regex(r#"(?i)alt=["']([^"']*)["']"#)
                .captures(&m[0])
                .map(|m| m[1].to_owned())
                .unwrap_or_default();
            flush(&mut plain, &mut runs, flags);
            runs.push(media_run(&m[1], &alt, true).unwrap_or_else(|| {
                text_run(if alt.is_empty() { name(&m[1]) } else { &alt }, flags)
            }));
            i += m[0].len();
            continue;
        }
        if let Some((used, image, label, href)) = link(rest) {
            flush(&mut plain, &mut runs, flags);
            let media = media_run(&href, if image { &label } else { "" }, image);
            if image && let Some(media) = media.as_ref() {
                runs.push(media.clone());
                i += used;
                continue;
            }
            let label = if label.is_empty() {
                if image {
                    name(&href).into()
                } else {
                    href.clone()
                }
            } else {
                label
            };
            let inner = inline(&label, flags, depth + 1)?;
            let mut labels = inner
                .iter()
                .filter(|r| r.get("media").is_none())
                .cloned()
                .collect::<Vec<_>>();
            if labels.is_empty() {
                labels.push(text_run(
                    inner
                        .iter()
                        .find_map(|r| r["media"]["alt"].as_str())
                        .unwrap_or_else(|| name(&href)),
                    flags,
                ));
            }
            for mut label in labels {
                if safe_link(&href) && label["c"] != true {
                    label["href"] = json!(href);
                }
                runs.push(label);
            }
            runs.extend(inner.into_iter().filter(|r| r.get("media").is_some()));
            if let Some(media) = media {
                runs.push(media);
            }
            i += used;
            continue;
        }
        if let Some(m) = regex(r"(?i)^<(https?://[^\s<>]+)>").captures(rest) {
            flush(&mut plain, &mut runs, flags);
            let mut run = text_run(&m[1], flags);
            run["href"] = json!(&m[1]);
            runs.push(run);
            i += m[0].len();
            continue;
        }
        if (i == 0 || !previous.is_alphanumeric() && previous != '_' && previous != '/')
            && safe_link(rest)
        {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '<' || c == '>')
                .unwrap_or(rest.len());
            let mut url = &rest[..end];
            url = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"', '*', '_', '~']);
            while url.ends_with(')')
                && url.chars().filter(|c| *c == ')').count()
                    > url.chars().filter(|c| *c == '(').count()
            {
                url = &url[..url.len() - 1];
            }
            if !url.is_empty() {
                flush(&mut plain, &mut runs, flags);
                let mut run = text_run(url, flags);
                run["href"] = json!(url);
                runs.push(run);
                i += url.len();
                continue;
            }
        }
        if boundary(previous) && (rest.starts_with('/') || rest.starts_with("~/"))
            && let Some(path)=regex(r#"(?i)^((?:~/|/)[^\s<>()\[\]`'"]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|mp4|m4v|mov|webm|mp3|m4a|wav|aac|ogg|flac))(?:[^A-Za-z0-9_/]|$)"#).captures(rest)
            && let Some(media)=media_run(&path[1],"",false) {
            plain.push_str(&path[1]);flush(&mut plain,&mut runs,flags);runs.push(media);i+=path[1].len();continue;
        }
        let c = rest.chars().next().unwrap();
        plain.push(c);
        i += c.len_utf8();
    }
    flush(&mut plain, &mut runs, flags);
    Ok(runs)
}
fn prose(runs: Vec<Value>) -> Value {
    let mut media = vec![];
    let mut text = vec![];
    for run in runs {
        if let Some(m) = run.get("media") {
            if !media.iter().any(|v: &Value| v["path"] == m["path"]) {
                media.push(m.clone());
            }
        } else {
            text.push(run);
        }
    }
    let mut out = json!({"runs":text});
    if !media.is_empty() {
        out["media"] = json!(media);
    }
    out
}
fn prose_block(kind: &str, text: &str, depth: usize) -> io::Result<Value> {
    let mut out = prose(inline(text, &json!({}), depth)?);
    out["kind"] = json!(kind);
    Ok(out)
}
fn fence(line: &str) -> Option<(usize, char, usize, String)> {
    let captures = regex(r"^( {0,3})(`{3,}|~{3,})\s*([^\s`]*)[^`]*$").captures(line)?;
    Some((
        captures[1].len(),
        captures[2].chars().next()?,
        captures[2].len(),
        captures[3].into(),
    ))
}
fn heading(line: &str) -> Option<(usize, String)> {
    let m = regex(r"^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$").captures(line)?;
    Some((
        m[1].len(),
        m.get(2)
            .map(|v| v.as_str().trim().into())
            .unwrap_or_default(),
    ))
}
fn rule(line: &str) -> bool {
    let line = line.trim();
    let mut chars = line.chars().filter(|c| !c.is_whitespace());
    let Some(first) = chars.next() else {
        return false;
    };
    if !"-*_".contains(first) {
        return false;
    }
    let tail = chars.collect::<Vec<_>>();
    tail.len() >= 2 && tail.iter().all(|c| *c == first)
}
fn quote(line: &str) -> Option<&str> {
    let clean = line.trim_start_matches(' ');
    if line.len() - clean.len() > 3 {
        return None;
    }
    clean
        .strip_prefix('>')
        .map(|v| v.strip_prefix(' ').unwrap_or(v))
}
struct Item {
    indent: usize,
    marker: String,
    gap: String,
    text: String,
}
fn item(line: &str) -> Option<Item> {
    let m = regex(r"^( {0,3})([-*+]|\d{1,9}[.)])(?:([ \t]+)(.*))?$").captures(line)?;
    Some(Item {
        indent: m[1].len(),
        marker: m[2].into(),
        gap: m.get(3).map(|v| v.as_str()).unwrap_or(" ").into(),
        text: m.get(4).map(|v| v.as_str()).unwrap_or_default().into(),
    })
}
fn interrupts(line: &str) -> bool {
    fence(line).is_some()
        || heading(line).is_some()
        || rule(line)
        || quote(line).is_some()
        || item(line).is_some_and(|v| !v.text.trim().is_empty())
}
fn indent(line: &str) -> usize {
    line.bytes().take_while(|c| *c == b' ').count()
}
fn cells(line: &str) -> Vec<String> {
    let line = line.trim().strip_prefix('|').unwrap_or(line.trim());
    let line = if line.ends_with('|') && !line.ends_with("\\|") {
        &line[..line.len() - 1]
    } else {
        line
    };
    let mut out = vec![];
    let mut start = 0;
    for (i, c) in line.char_indices() {
        if c == '|' && !line[..i].ends_with('\\') {
            out.push(line[start..i].trim().into());
            start = i + 1;
        }
    }
    out.push(line[start..].trim().into());
    out
}
fn separator(line: &str) -> bool {
    line.contains('-')
        && cells(line)
            .iter()
            .all(|v| regex(r"^:?-{3,}:?$").is_match(v))
}
fn paragraph(lines: &[String], depth: usize) -> io::Result<Value> {
    let mut text = String::new();
    for (i, line) in lines.iter().enumerate() {
        let line = line.trim_start();
        if i + 1 == lines.len() {
            text.push_str(line.trim_end());
        } else if line.ends_with("  ") {
            text.push_str(line.trim_end());
            text.push('\n');
        } else if line.ends_with('\\') && !line.ends_with("\\\\") {
            text.push_str(&line[..line.len() - 1]);
            text.push('\n');
        } else {
            text.push_str(line.trim_end());
            text.push(' ');
        }
    }
    prose_block("p", &text, depth)
}
fn flush_paragraph(
    para: &mut Vec<String>,
    blocks: &mut Vec<Value>,
    depth: usize,
) -> io::Result<()> {
    if !para.is_empty() {
        blocks.push(paragraph(para, depth)?);
        para.clear();
    }
    Ok(())
}
fn parse(lines: &[String], depth: usize) -> io::Result<Vec<Value>> {
    if depth > 64 {
        return Err(error("Markdown nesting exceeds 64 levels"));
    }
    let lines = lines
        .iter()
        .map(|line| {
            let n = line.bytes().take_while(|v| *v == b'\t').count();
            format!("{}{}", "    ".repeat(n), &line[n..])
        })
        .collect::<Vec<_>>();
    let mut blocks = vec![];
    let mut para = vec![];
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if line.trim().is_empty() {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            i += 1;
            continue;
        }
        if let Some((padding, marker, length, lang)) = fence(line) {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            let mut body = vec![];
            i += 1;
            while i < lines.len() {
                let clean = lines[i].trim_start_matches(' ');
                let count = clean.chars().take_while(|c| *c == marker).count();
                if lines[i].len() - clean.len() <= 3
                    && count >= length
                    && clean[count..].trim().is_empty()
                {
                    i += 1;
                    break;
                }
                let remove = indent(&lines[i]).min(padding);
                body.push(lines[i][remove..].to_owned());
                i += 1;
            }
            let mut block = json!({"kind":"code","text":body.join("\n")});
            if !lang.is_empty() {
                block["lang"] = json!(lang);
            }
            blocks.push(block);
            continue;
        }
        if let Some((level, text)) = heading(line) {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            let mut block = prose_block("h", &text, depth)?;
            block["level"] = json!(level);
            blocks.push(block);
            i += 1;
            continue;
        }
        if !para.is_empty() && regex(r"^ {0,3}(=+|-{3,})[ \t]*$").is_match(line) {
            let text = para
                .iter()
                .map(|v: &String| v.trim())
                .collect::<Vec<_>>()
                .join(" ");
            para.clear();
            let mut block = prose_block("h", &text, depth)?;
            block["level"] = json!(if line.trim().starts_with('=') { 1 } else { 2 });
            blocks.push(block);
            i += 1;
            continue;
        }
        if rule(line) {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            blocks.push(json!({"kind":"hr"}));
            i += 1;
            continue;
        }
        if line.contains('|')
            && i + 1 < lines.len()
            && separator(&lines[i + 1])
            && cells(line).len() == cells(&lines[i + 1]).len()
        {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            let header = cells(line)
                .iter()
                .map(|c| inline(c, &json!({}), depth))
                .collect::<io::Result<Vec<_>>>()?;
            let align = cells(&lines[i + 1])
                .iter()
                .map(|c| {
                    if c.starts_with(':') && c.ends_with(':') {
                        "center"
                    } else if c.ends_with(':') {
                        "right"
                    } else {
                        "left"
                    }
                })
                .collect::<Vec<_>>();
            let mut rows = vec![];
            i += 2;
            while i < lines.len() && !lines[i].trim().is_empty() && lines[i].contains('|') {
                let row = cells(&lines[i]);
                rows.push(
                    (0..header.len())
                        .map(|n| {
                            inline(
                                row.get(n).map(String::as_str).unwrap_or_default(),
                                &json!({}),
                                depth,
                            )
                        })
                        .collect::<io::Result<Vec<_>>>()?,
                );
                i += 1;
            }
            blocks.push(json!({"kind":"table","headers":header,"align":align,"rows":rows}));
            continue;
        }
        if quote(line).is_some() {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            let mut inner = vec![];
            while i < lines.len() {
                if let Some(next) = quote(&lines[i]) {
                    inner.push(next.into());
                } else if !lines[i].trim().is_empty()
                    && !inner.is_empty()
                    && !inner.last().unwrap_or(&String::new()).trim().is_empty()
                    && !interrupts(&lines[i])
                {
                    inner.push(lines[i].clone());
                } else {
                    break;
                }
                i += 1;
            }
            blocks.push(json!({"kind":"quote","blocks":parse(&inner,depth+1)?}));
            continue;
        }
        if item(line).is_some_and(|v| para.is_empty() || !v.text.trim().is_empty()) {
            flush_paragraph(&mut para, &mut blocks, depth)?;
            let (next, list) = parse_list(&lines, i, depth)?;
            blocks.push(list);
            i = next;
            continue;
        }
        para.push(line.clone());
        i += 1;
    }
    flush_paragraph(&mut para, &mut blocks, depth)?;
    Ok(blocks)
}
fn parse_list(lines: &[String], start: usize, depth: usize) -> io::Result<(usize, Value)> {
    let first = item(&lines[start]).unwrap();
    let ordered = first.marker.as_bytes()[0].is_ascii_digit();
    let delimiter = if ordered {
        first.marker.chars().next_back().unwrap()
    } else {
        first.marker.chars().next().unwrap()
    };
    let mut items = vec![];
    let mut i = start;
    while i < lines.len() {
        let Some(next) = item(&lines[i]) else {
            break;
        };
        let same = next.marker.as_bytes()[0].is_ascii_digit() == ordered
            && if ordered {
                next.marker.ends_with(delimiter)
            } else {
                next.marker.starts_with(delimiter)
            };
        if !same {
            break;
        }
        let width = next.indent
            + next.marker.len()
            + if next.gap.len() > 4 {
                1
            } else {
                next.gap.len()
            };
        let mut body = vec![if next.gap.len() > 4 {
            format!("{}{}", &next.gap[1..], next.text)
        } else {
            next.text
        }];
        i += 1;
        let mut in_fence = fence(&body[0]).is_some();
        while i < lines.len() {
            let next = &lines[i];
            if next.trim().is_empty() {
                let mut ahead = i + 1;
                while ahead < lines.len() && lines[ahead].trim().is_empty() {
                    ahead += 1;
                }
                if in_fence || ahead < lines.len() && indent(&lines[ahead]) >= width {
                    body.push(String::new());
                    i += 1;
                    continue;
                }
                break;
            }
            if indent(next) >= width {
                let line = next[width..].to_owned();
                if fence(&line).is_some() {
                    in_fence = !in_fence;
                }
                body.push(line);
                i += 1;
                continue;
            }
            if in_fence {
                let line = next.trim_start().to_owned();
                if fence(&line).is_some() {
                    in_fence = false;
                }
                body.push(line);
                i += 1;
                continue;
            }
            if !body.last().unwrap().trim().is_empty()
                && !interrupts(next)
                && item(next).is_none()
                && fence(body.last().unwrap()).is_none()
            {
                body.push(next.trim_start().into());
                i += 1;
                continue;
            }
            break;
        }
        let task = regex(r"^\[([ xX])\](?:[ \t]+|$)").captures(&body[0]);
        let checked = task.as_ref().map(|m| &m[1] != " ");
        let used = task.map(|m| m[0].len());
        if let Some(used) = used {
            body[0] = body[0][used..].into();
        }
        let mut entry = json!({"blocks":parse(&body,depth+1)?});
        if let Some(checked) = checked {
            entry["checked"] = json!(checked);
        }
        items.push(entry);
        let mut ahead = i;
        while ahead < lines.len() && lines[ahead].trim().is_empty() {
            ahead += 1;
        }
        if ahead < lines.len() && item(&lines[ahead]).is_some() && indent(&lines[ahead]) < width {
            i = ahead;
        }
    }
    Ok((
        i,
        json!({"kind":"list","ordered":ordered,"start":if ordered{first.marker[..first.marker.len()-1].parse::<u64>().unwrap()}else{1},"items":items}),
    ))
}
pub fn blocks(text: &str) -> io::Result<Vec<Value>> {
    if text.len() > 2_000_000 {
        return Err(error(
            "Markdown view exceeds 2 MB; full text remains archived",
        ));
    }
    parse(
        &text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .split('\n')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        0,
    )
}
pub fn plain(blocks: &[Value]) -> String {
    fn runs(v: &Value) -> String {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["text"].as_str())
            .collect()
    }
    blocks
        .iter()
        .map(|v| match v["kind"].as_str() {
            Some("code") => v["text"].as_str().unwrap_or_default().into(),
            Some("hr") => String::new(),
            Some("quote") => plain(v["blocks"].as_array().unwrap()),
            Some("list") => v["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| plain(i["blocks"].as_array().unwrap()))
                .collect::<Vec<_>>()
                .join("\n"),
            Some("table") => std::iter::once(&v["headers"])
                .chain(v["rows"].as_array().into_iter().flatten())
                .map(|row| {
                    row.as_array()
                        .unwrap()
                        .iter()
                        .map(runs)
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => runs(&v["runs"]),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn inline_runs(text: &str, flags: Option<&Value>) -> io::Result<Vec<Value>> {
    if text.len() > 2_000_000 {
        return Err(error(
            "Markdown view exceeds 2 MB; full text remains archived",
        ));
    }
    inline(text, flags.unwrap_or(&json!({})), 0)
}
pub fn words(runs: &[Value]) -> Vec<Value> {
    static SPACE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let mut words = vec![];
    for (ri, run) in runs.iter().enumerate() {
        if run.get("media").is_some() {
            continue;
        }
        if run["href"].as_str().is_some_and(|v| !v.is_empty()) || run["c"] == true {
            let mut word = run.clone();
            word["key"] = json!(ri.to_string());
            words.push(word);
            continue;
        }
        let text = run["text"].as_str().unwrap_or_default();
        let mut at = 0;
        let mut wi = 0;
        let push = |words: &mut Vec<Value>, part: &str, wi: usize| {
            if !part.is_empty() {
                let mut word = run.clone();
                word["text"] = json!(part);
                word["key"] = json!(format!("{ri}-{wi}"));
                words.push(word);
            }
        };
        for space in SPACE.find_iter(text) {
            push(&mut words, &text[at..space.start()], wi);
            wi += 1;
            push(&mut words, space.as_str(), wi);
            wi += 1;
            at = space.end();
        }
        push(&mut words, &text[at..], wi);
    }
    words
}
