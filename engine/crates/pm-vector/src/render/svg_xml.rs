//! Reading an SVG image's XML as Chromium's parser (libxml2) reads it where
//! roxmltree is stricter (docs/engine/vector.md, "SVG images"). Documents
//! Chromium shows as a broken image never get here
//! (`pm_media::image::svg::broken_image_file` decides that first); these
//! are documents it paints:
//!
//! - an entity reference libxml2 only warns about (no declaration, but an
//!   external DTD subset or a parameter entity reference that could have
//!   declared it) expands to nothing;
//! - external entities are never loaded: they expand to nothing;
//! - parameter entities declared in the internal subset are expanded there
//!   (roxmltree does not read them);
//! - usvg converts at most 1,024 element levels: past that, `<g>` elements
//!   without attributes (which only group) are removed, as they change
//!   nothing drawn.
//!
//! One semantic correction (V-A2): an outermost `<svg>` without a valid
//! `viewBox` whose `width` or `height` is missing, `auto`, or a percentage
//! has the `<img>` default size there (CSS 2.1 §10.3.2/§10.6.2, CSS Images 3
//! §5.3: 300 × 150); usvg would size it to its content's bounds and stretch
//! the content over the image.

use std::borrow::Cow;

use super::svg_depth::{Tag, element_depth, for_each_tag};

/// usvg's deepest element nesting.
pub(crate) const USVG_DEPTH: usize = 1024;

/// Most unknown entity references read as nothing in one document.
const MAX_UNKNOWN_ENTITIES: usize = 256;

/// External entities are never resolved (roxmltree discards them), so a
/// reference to one is an unknown reference: read as nothing.
fn options<'a>() -> usvg::roxmltree::ParsingOptions<'a> {
    usvg::roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    }
}

/// The text libxml2 reads `text` as (see the module docs).
pub(crate) fn libxml2_text(text: &str) -> Result<Cow<'_, str>, usvg::roxmltree::Error> {
    let mut text = expand_parameter_entities(text);
    if let Some(sized) = default_size(&text) {
        text = Cow::Owned(sized);
    }
    if element_depth(text.as_bytes()) > USVG_DEPTH {
        text = Cow::Owned(without_plain_groups(&text));
    }
    for _ in 0..MAX_UNKNOWN_ENTITIES {
        match usvg::roxmltree::Document::parse_with_options(&text, options()) {
            Err(usvg::roxmltree::Error::UnknownEntityReference(name, pos)) => {
                let at = offset(&text, pos)
                    .filter(|&at| text[at..].starts_with(&format!("&{name};")))
                    .ok_or(usvg::roxmltree::Error::UnknownEntityReference(name.clone(), pos))?;
                let mut owned = text.into_owned();
                owned.replace_range(at..at + name.len() + 2, "");
                text = Cow::Owned(owned);
            }
            Err(e) => return Err(e),
            Ok(_) => return Ok(text),
        }
    }
    match usvg::roxmltree::Document::parse_with_options(&text, options()) {
        Ok(_) => {}
        Err(e) => return Err(e),
    }
    Ok(text)
}

/// Parses text [`libxml2_text`] returned.
pub(crate) fn parse(text: &str) -> Result<usvg::roxmltree::Document<'_>, usvg::roxmltree::Error> {
    usvg::roxmltree::Document::parse_with_options(text, options())
}

/// The byte offset of a roxmltree position (1-based row and char column).
fn offset(text: &str, pos: usvg::roxmltree::TextPos) -> Option<usize> {
    let mut line = 0usize;
    for _ in 1..pos.row {
        line += text[line..].find('\n')? + 1;
    }
    let mut chars = text[line..].char_indices();
    for _ in 1..pos.col {
        chars.next()?;
    }
    Some(line + chars.next().map_or(text.len() - line, |(i, _)| i))
}

/// The internal subset with its parameter entities expanded (declared
/// before use, as XML requires) and their declarations removed.
fn expand_parameter_entities(text: &str) -> Cow<'_, str> {
    let Some(doctype) = text.find("<!DOCTYPE") else {
        return Cow::Borrowed(text);
    };
    let Some(open) = text[doctype..].find('[').map(|i| doctype + i) else {
        return Cow::Borrowed(text);
    };
    // The subset ends at the first `]` outside quotes.
    let bytes = text.as_bytes();
    let (mut i, mut quote) = (open + 1, None);
    while i < bytes.len() {
        match (quote, bytes[i]) {
            (None, b'"' | b'\'') => quote = Some(bytes[i]),
            (Some(q), b) if b == q => quote = None,
            (None, b']') => break,
            _ => {}
        }
        i += 1;
    }
    let subset = &text[open + 1..i.min(text.len())];
    if !subset.contains('%') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(subset.len());
    let mut values: Vec<(String, String)> = Vec::new();
    let mut rest = subset;
    while !rest.is_empty() {
        if let Some(decl) = rest.strip_prefix("<!ENTITY")
            && let Some(after) = decl.trim_start().strip_prefix('%')
            && let Some((name, value, len)) = parameter_entity(after)
        {
            values.push((name.to_owned(), expand(&value, &values)));
            rest = &after[len..];
            continue;
        }
        if let Some(r) = rest.strip_prefix('%')
            && let Some(end) = r.find(';')
            && let Some((_, v)) = values.iter().find(|(n, _)| n == &r[..end])
        {
            out.push_str(v);
            rest = &r[end + 1..];
            continue;
        }
        let c = rest.chars().next().expect("not empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    Cow::Owned(format!("{}{out}{}", &text[..open + 1], &text[open + 1 + subset.len()..]))
}

/// `name "value">` after `<!ENTITY %`: the name, the literal value, and the
/// declaration's length.
fn parameter_entity(s: &str) -> Option<(&str, String, usize)> {
    let lead = s.len() - s.trim_start().len();
    let s2 = &s[lead..];
    let name_len = s2.find(|c: char| c.is_whitespace())?;
    let name = &s2[..name_len];
    let after = &s2[name_len..];
    let ws = after.len() - after.trim_start().len();
    let v = &after[ws..];
    let q = v.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let close = v[1..].find(q)? + 1;
    let value = v[1..close].to_owned();
    let tail = &v[close + 1..];
    let gt = tail.find('>')?;
    Some((name, value, lead + name_len + ws + close + 1 + gt + 1))
}

fn expand(value: &str, values: &[(String, String)]) -> String {
    let mut out = value.to_owned();
    for (n, v) in values {
        out = out.replace(&format!("%{n};"), v);
    }
    out
}

/// The root tag's attributes: (name, value, byte range of the attribute).
fn attributes(tag: &str) -> Vec<(&str, &str, core::ops::Range<usize>)> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    // Past `<name`.
    let mut i = tag.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(tag.len());
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !matches!(bytes[i], b'=' | b'>' | b'/') && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == start {
            return out;
        }
        let name = &tag[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            return out;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let Some(&q) = bytes.get(i).filter(|b| matches!(b, b'"' | b'\'')) else {
            return out;
        };
        let Some(len) = tag[i + 1..].find(q as char) else {
            return out;
        };
        out.push((name, &tag[i + 1..i + 1 + len], start..i + 2 + len));
        i += len + 2;
    }
}

/// The root tag with the `<img>` default size where it has none (see the
/// module docs), or `None` when it needs no change.
fn default_size(text: &str) -> Option<String> {
    let mut root = None;
    for_each_tag(text.as_bytes(), |tag| {
        if let (None, Tag::Open { start, end, .. }) = (&root, tag) {
            root = Some(start..end);
        }
    });
    let root = root?;
    let tag = &text[root.clone()];
    let attrs = attributes(tag);
    let valid_view_box = attrs.iter().any(|(n, v, _)| {
        *n == "viewBox" && {
            let n: Vec<f64> = v
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            n.len() == 4 && n[2] > 0.0 && n[3] > 0.0
        }
    });
    if valid_view_box {
        return None;
    }
    let mut edits: Vec<(core::ops::Range<usize>, String)> = Vec::new();
    for (name, default) in [("width", "300"), ("height", "150")] {
        let found = attrs.iter().find(|(n, _, _)| *n == name);
        let sized = found.is_some_and(|(_, v, _)| {
            let v = v.trim();
            !v.is_empty() && v != "auto" && !v.ends_with('%')
        });
        if sized {
            continue;
        }
        let at = match found {
            Some((_, _, range)) => range.clone(),
            None => {
                let end = tag.trim_end_matches('>').trim_end_matches('/').len();
                end..end
            }
        };
        edits.push((at, format!(" {name}=\"{default}\" ")));
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut tag = tag.to_owned();
    for (range, value) in edits {
        tag.replace_range(range, &value);
    }
    Some(format!("{}{tag}{}", &text[..root.start], &text[root.end..]))
}

/// `text` without `<g>` start tags that have no attributes, and their end
/// tags.
fn without_plain_groups(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut drop: Vec<(usize, usize)> = Vec::new();
    // Whether each open element is a plain `<g>`.
    let mut open: Vec<bool> = Vec::new();
    for_each_tag(bytes, |tag| match tag {
        Tag::Open { start, end, empty } => {
            let plain = !empty && text[start..end].trim_end() == "<g>";
            if plain {
                drop.push((start, end));
            }
            if !empty {
                open.push(plain);
            }
        }
        Tag::Close { start, end } => {
            if open.pop() == Some(true) {
                drop.push((start, end));
            }
        }
    });
    drop.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (start, end) in drop {
        out.push_str(&text[at..start]);
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> String {
        libxml2_text(text).unwrap().into_owned()
    }

    #[test]
    fn what_libxml2_reads_that_roxmltree_refuses() {
        let r = r#"<rect width="8" height="8"/>"#;
        let ns = r#"xmlns="http://www.w3.org/2000/svg" width="8" height="8""#;
        // Undeclared entities with an external subset that could declare
        // them: nothing.
        let ext = format!(
            r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg {ns}>{r}<text>a&nbsp;b&copy;</text></svg>"#
        );
        assert!(usvg::roxmltree::Document::parse_with_options(&ext, options()).is_err());
        let doc_text = read(&ext);
        let doc = parse(&doc_text).unwrap();
        assert_eq!(
            doc.descendants().find(|n| n.has_tag_name("text")).unwrap().text(),
            Some("ab")
        );
        // An external entity: never loaded.
        let entity = format!(r#"<!DOCTYPE svg [<!ENTITY e SYSTEM "e.xml">]><svg {ns}>{r}&e;</svg>"#);
        let t = read(&entity);
        assert!(parse(&t).is_ok());
        // A parameter entity declaring a general entity.
        let pe = format!(
            r#"<!DOCTYPE svg [<!ENTITY % a "<!ENTITY b 'x'>"> %a;]><svg {ns}>{r}<text>&b;</text></svg>"#
        );
        let t = read(&pe);
        let doc = parse(&t).unwrap();
        assert_eq!(
            doc.descendants().find(|n| n.has_tag_name("text")).unwrap().text(),
            Some("x")
        );
        // Ordinary documents are not copied.
        let plain = format!("<svg {ns}>{r}</svg>");
        assert!(matches!(libxml2_text(&plain).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn an_image_without_a_size_or_view_box_is_300_by_150() {
        let size = |svg: &str| {
            let t = read(svg);
            let doc = parse(&t).unwrap();
            let tree = usvg::Tree::from_xmltree(&doc, &usvg::Options::default()).unwrap();
            (tree.size().width(), tree.size().height())
        };
        let rect = r##"<rect width="100" height="60" fill="#1B2A4A"/>"##;
        let ns = r#"xmlns="http://www.w3.org/2000/svg""#;
        assert_eq!(size(&format!("<svg {ns}>{rect}</svg>")), (300.0, 150.0));
        assert_eq!(size(&format!(r#"<svg {ns} width="100">{rect}</svg>"#)), (100.0, 150.0));
        assert_eq!(size(&format!(r#"<svg width="50%" height='40' {ns}>{rect}</svg>"#)), (300.0, 40.0));
        assert_eq!(size(&format!(r#"<svg {ns} width="auto" height="auto"/>"#)), (300.0, 150.0));
        // A viewBox or both sizes: as written.
        let sized = format!(r#"<svg {ns} width="64" height="32">{rect}</svg>"#);
        assert!(matches!(libxml2_text(&sized).unwrap(), Cow::Borrowed(_)));
        let boxed = format!(r#"<svg {ns} viewBox="0 0 100 60">{rect}</svg>"#);
        assert!(matches!(libxml2_text(&boxed).unwrap(), Cow::Borrowed(_)));
        assert_eq!(size(&boxed), (100.0, 60.0));
    }

    #[test]
    fn plain_groups_go_only_past_usvgs_depth() {
        let ns = r#"xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8""#;
        let nest = |n: usize| {
            format!(
                r#"<svg {ns}><g opacity=".5">{}<rect width="8" height="8"/>{}</g></svg>"#,
                "<g>".repeat(n),
                "</g>".repeat(n)
            )
        };
        let shallow = nest(10);
        assert!(matches!(libxml2_text(&shallow).unwrap(), Cow::Borrowed(_)));
        let deep = read(&nest(4000));
        assert_eq!(
            deep,
            format!(r#"<svg {ns}><g opacity=".5"><rect width="8" height="8"/></g></svg>"#)
        );
    }
}
