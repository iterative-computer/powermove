//! Whether an SVG file is a broken `<img>` in Chromium (compat level 1).
//!
//! Today's viewer draws an SVG asset through an `<img>`: Blink parses it
//! with libxml2 and shows nothing when the document is not an SVG image.
//! The engine paints that empty image only where Blink's outcome is
//! established; every other document stays an explicit unsupported SVG
//! image (plan amendment 22), because roxmltree is stricter than libxml2 in
//! some places (DTDs) and looser or narrower in others (resource limits).
//!
//! The rules below were measured against Electron 44 (Blob `<img>` plus
//! Canvas2D `drawImage`); `pm-media/examples/svg_broken_compare.rs` replays
//! a probe directory against them:
//!
//! - Any libxml2 error, fatal or not (well-formedness and namespace errors),
//!   makes Blink move the root out of the document, so the image is broken.
//!   Warnings do not.
//! - An undeclared entity is a fatal error only when the document is
//!   `standalone="yes"` or has neither an external subset nor a parameter
//!   entity reference (XML 1.0 "WFC: Entity Declared"); otherwise libxml2
//!   only warns and the image paints. A declared external entity is never
//!   loaded and paints.
//! - An element nested deeper than [`MAX_ELEMENT_DEPTH`] is fatal
//!   (Blink's `kMaxXMLTreeDepth`): depth 5,000 paints and 5,001 does not.
//! - A well-formed document whose root is not `svg` in the SVG namespace is
//!   broken, unless the internal subset declares attribute defaults
//!   (`<!ATTLIST svg xmlns ... "http://www.w3.org/2000/svg">` paints).
//!
//! Undecided (never classified broken): errors inside the DOCTYPE, entity
//! depth or reference limits, roxmltree resource limits, entities whose
//! values contain markup, SVGZ, and anything this scanner cannot read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use resvg::usvg::roxmltree;

/// The deepest element nesting Blink's XML parser accepts.
pub const MAX_ELEMENT_DEPTH: usize = 5000;

/// What the prolog declares that decides libxml2's entity rules.
#[derive(Default, Debug)]
struct Prolog {
    standalone: bool,
    doctype: bool,
    /// The DOCTYPE has an internal subset (`[...]`).
    internal_subset: bool,
    /// The DOCTYPE names a SYSTEM or PUBLIC external subset.
    external_subset: bool,
    /// The internal subset references a parameter entity (`%name;`).
    pe_refs: bool,
    /// The internal subset declares attribute lists (defaults may bind
    /// namespaces).
    attlist: bool,
    /// A general entity's replacement text contains markup.
    markup_entities: bool,
    /// General entities declared in the internal subset.
    entities: Vec<String>,
    /// Byte offset just past the DOCTYPE (0 when there is none).
    doctype_end: usize,
    /// Byte offset of the first byte after the prolog.
    body: usize,
}

/// Whether Chromium shows `data` as a broken image (see the module docs).
/// `false` means "not established", not "valid".
pub fn broken_image(data: &[u8]) -> bool {
    if data.starts_with(&[0x1f, 0x8b]) {
        return false;
    }
    let decoded = super::svg::utf8_source(data);
    let Ok(text) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some(prolog) = prolog(text) else {
        return false;
    };
    if prolog.markup_entities {
        return false;
    }
    let depth = max_depth(&text.as_bytes()[prolog.body..]);
    if depth > MAX_ELEMENT_DEPTH {
        return true;
    }
    // roxmltree recurses once per element level; give it a stack sized to
    // the document instead of the caller's (engine worker threads have
    // 2 MiB). The nesting is at most MAX_ELEMENT_DEPTH here.
    let stack = (16 << 20) + depth * (8 << 10);
    let parsed = std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("svg-broken-image".into())
            .stack_size(stack)
            .spawn_scoped(s, move || {
                let options = roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                };
                match roxmltree::Document::parse_with_options(text, options) {
                    Ok(doc) => {
                        let root = doc.root_element();
                        Ok(root.tag_name().name() == "svg"
                            && root.tag_name().namespace() == Some("http://www.w3.org/2000/svg"))
                    }
                    Err(e) => Err(e),
                }
            })
            .map(|h| h.join())
    });
    let Ok(Ok(parsed)) = parsed else {
        // The thread could not start or panicked: not established.
        return false;
    };
    match parsed {
        Ok(svg_root) => !svg_root && !prolog.attlist,
        Err(e) => error_is_broken(text, &prolog, &e),
    }
}

fn error_is_broken(text: &str, prolog: &Prolog, e: &roxmltree::Error) -> bool {
    use roxmltree::Error as E;
    // Positions inside the DOCTYPE: roxmltree reads only part of DTD syntax.
    let in_doctype = |pos: roxmltree::TextPos| {
        prolog.doctype && offset(text, pos).is_none_or(|at| at < prolog.doctype_end)
    };
    match e {
        // Resource and expansion limits that libxml2 does not share.
        E::NodesLimitReached
        | E::AttributesLimitReached
        | E::NamespacesLimitReached
        | E::EntityReferenceLoop(_)
        | E::EntityResolver(..)
        | E::DtdDetected => false,
        E::UnknownEntityReference(name, pos) => {
            if in_doctype(*pos) || prolog.entities.iter().any(|n| n == name) {
                return false;
            }
            prolog.standalone || (!prolog.external_subset && !prolog.pe_refs)
        }
        // Attribute defaults in the internal subset may declare namespaces.
        E::InvalidXmlPrefixUri(pos)
        | E::UnexpectedXmlUri(pos)
        | E::UnexpectedXmlnsUri(pos)
        | E::InvalidElementNamePrefix(pos)
        | E::DuplicatedNamespace(_, pos)
        | E::UnknownNamespace(_, pos) => !prolog.attlist && !in_doctype(*pos),
        // roxmltree may misread a valid internal subset to the end.
        E::UnexpectedEndOfStream => !prolog.internal_subset,
        E::NoRootNode | E::UnclosedRootNode => true,
        E::UnexpectedCloseTag(_, _, pos)
        | E::UnexpectedEntityCloseTag(pos)
        | E::MalformedEntityReference(pos)
        | E::InvalidAttributeValue(pos)
        | E::DuplicatedAttribute(_, pos)
        | E::UnexpectedDeclaration(pos)
        | E::InvalidName(pos)
        | E::NonXmlChar(_, pos)
        | E::InvalidChar(_, _, pos)
        | E::InvalidChar2(_, _, pos)
        | E::InvalidString(_, pos)
        | E::InvalidExternalID(pos)
        | E::InvalidComment(pos)
        | E::InvalidCharacterData(pos)
        | E::UnknownToken(pos) => !in_doctype(*pos),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

/// The byte offset of a roxmltree position (1-based row and char column).
fn offset(text: &str, pos: roxmltree::TextPos) -> Option<usize> {
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

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_name_byte(b: u8) -> bool {
    !is_space(b)
        && !matches!(
            b,
            b'>' | b'/' | b'=' | b'"' | b'\'' | b'[' | b']' | b'%' | b';' | b'<'
        )
}

struct Scan<'a> {
    s: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn rest(&self) -> &'a [u8] {
        &self.s[self.at.min(self.s.len())..]
    }
    fn starts(&self, p: &[u8]) -> bool {
        self.rest().starts_with(p)
    }
    fn spaces(&mut self) -> bool {
        let start = self.at;
        while self.rest().first().is_some_and(|&b| is_space(b)) {
            self.at += 1;
        }
        self.at > start
    }
    fn name(&mut self) -> Option<&'a str> {
        let start = self.at;
        while self.rest().first().is_some_and(|&b| is_name_byte(b)) {
            self.at += 1;
        }
        (self.at > start)
            .then(|| std::str::from_utf8(&self.s[start..self.at]).ok())
            .flatten()
    }
    /// Skips past the next `end`, returning the bytes before it.
    fn until(&mut self, end: &[u8]) -> Option<&'a [u8]> {
        let rest = self.rest();
        let at = rest.windows(end.len()).position(|w| w == end)?;
        self.at += at + end.len();
        Some(&rest[..at])
    }
    fn literal(&mut self) -> Option<&'a [u8]> {
        let q = *self.rest().first().filter(|q| matches!(q, b'"' | b'\''))?;
        self.at += 1;
        self.until(&[q])
    }
    /// `SYSTEM lit` or `PUBLIC lit lit` (the system literal is optional
    /// after PUBLIC in a NOTATION; elsewhere roxmltree reports it).
    fn external_id(&mut self) -> Option<bool> {
        if self.starts(b"SYSTEM") {
            self.at += 6;
            self.spaces();
            self.literal()?;
            Some(true)
        } else if self.starts(b"PUBLIC") {
            self.at += 6;
            self.spaces();
            self.literal()?;
            self.spaces();
            if matches!(self.rest().first(), Some(b'"' | b'\'')) {
                self.literal()?;
            }
            Some(true)
        } else {
            Some(false)
        }
    }
}

/// Reads the XML declaration and DOCTYPE. `None` when the prolog uses
/// syntax this scanner does not follow.
fn prolog(text: &str) -> Option<Prolog> {
    let mut p = Prolog::default();
    let mut s = Scan {
        s: text.as_bytes(),
        at: 0,
    };
    if s.starts(b"<?xml") && s.rest().get(5).is_some_and(|&b| is_space(b)) {
        let decl = s.until(b"?>")?;
        p.standalone = standalone_yes(decl);
    }
    loop {
        s.spaces();
        if s.starts(b"<!--") {
            s.at += 4;
            s.until(b"-->")?;
        } else if s.starts(b"<?") {
            s.until(b"?>")?;
        } else if s.starts(b"<!DOCTYPE") && !p.doctype {
            p.doctype = true;
            s.at += 9;
            s.spaces();
            s.name()?;
            s.spaces();
            p.external_subset = s.external_id()?;
            s.spaces();
            if s.starts(b"[") {
                s.at += 1;
                p.internal_subset = true;
                internal_subset(&mut s, &mut p)?;
                s.spaces();
            }
            if !s.starts(b">") {
                return None;
            }
            s.at += 1;
            p.doctype_end = s.at;
        } else {
            p.body = s.at;
            return Some(p);
        }
    }
}

fn standalone_yes(decl: &[u8]) -> bool {
    let Some(at) = decl.windows(10).position(|w| w == b"standalone") else {
        return false;
    };
    let mut s = Scan {
        s: &decl[at + 10..],
        at: 0,
    };
    s.spaces();
    if !s.starts(b"=") {
        return false;
    }
    s.at += 1;
    s.spaces();
    s.literal() == Some(b"yes")
}

fn internal_subset(s: &mut Scan, p: &mut Prolog) -> Option<()> {
    loop {
        s.spaces();
        if s.starts(b"]") {
            s.at += 1;
            return Some(());
        } else if s.starts(b"<!--") {
            s.at += 4;
            s.until(b"-->")?;
        } else if s.starts(b"<?") {
            s.until(b"?>")?;
        } else if s.starts(b"%") {
            s.at += 1;
            s.name()?;
            if !s.starts(b";") {
                return None;
            }
            s.at += 1;
            p.pe_refs = true;
        } else if s.starts(b"<!ENTITY") {
            s.at += 8;
            s.spaces();
            let parameter = s.starts(b"%");
            if parameter {
                s.at += 1;
                s.spaces();
            }
            let name = s.name()?.to_owned();
            s.spaces();
            if matches!(s.rest().first(), Some(b'"' | b'\'')) {
                let value = s.literal()?;
                if !parameter && value.contains(&b'<') {
                    p.markup_entities = true;
                }
                if value.contains(&b'%') {
                    // A parameter entity reference inside a literal.
                    p.pe_refs = true;
                }
            } else if !s.external_id()? {
                return None;
            }
            // `NDATA name`, then the end of the declaration.
            declaration_end(s, p)?;
            if !parameter {
                p.entities.push(name);
            }
        } else if s.starts(b"<!ATTLIST") || s.starts(b"<!ELEMENT") || s.starts(b"<!NOTATION") {
            if s.starts(b"<!ATTLIST") {
                p.attlist = true;
            }
            s.at += 2;
            declaration_end(s, p)?;
        } else {
            return None;
        }
    }
}

/// Skips to the `>` that ends a markup declaration, over quoted literals.
/// A `%` outside literals is a parameter entity reference.
fn declaration_end(s: &mut Scan, p: &mut Prolog) -> Option<()> {
    loop {
        match *s.rest().first()? {
            b'>' => {
                s.at += 1;
                return Some(());
            }
            b'"' | b'\'' => {
                s.literal()?;
            }
            b'<' => return None,
            b'%' => {
                p.pe_refs = true;
                s.at += 1;
            }
            _ => s.at += 1,
        }
    }
}

/// The deepest element nesting in the document body, over comments,
/// CDATA, processing instructions and quoted attribute values. Entity
/// expansions only add depth, so this is a lower bound.
fn max_depth(body: &[u8]) -> usize {
    let mut s = Scan { s: body, at: 0 };
    let (mut depth, mut max) = (0usize, 0usize);
    while let Some(lt) = s.rest().iter().position(|&b| b == b'<') {
        s.at += lt;
        if s.starts(b"<!--") {
            s.at += 4;
            if s.until(b"-->").is_none() {
                break;
            }
        } else if s.starts(b"<![CDATA[") {
            if s.until(b"]]>").is_none() {
                break;
            }
        } else if s.starts(b"<?") {
            if s.until(b"?>").is_none() {
                break;
            }
        } else if s.starts(b"</") {
            s.at += 2;
            if s.until(b">").is_none() {
                break;
            }
            depth = depth.saturating_sub(1);
        } else if s.starts(b"<!") {
            s.at += 2;
        } else {
            s.at += 1;
            // A start tag: attributes may quote `>`.
            let mut empty = false;
            loop {
                match s.rest().first() {
                    None => break,
                    Some(b'>') => {
                        s.at += 1;
                        break;
                    }
                    Some(b'/') if s.rest().get(1) == Some(&b'>') => {
                        empty = true;
                        s.at += 2;
                        break;
                    }
                    Some(b'"' | b'\'') => {
                        if s.literal().is_none() {
                            s.at = s.s.len();
                        }
                    }
                    Some(_) => s.at += 1,
                }
            }
            max = max.max(depth + 1);
            if !empty {
                depth += 1;
            }
        }
    }
    max
}

/// [`broken_image`] for a file, cached by path, size and modification time
/// so plan builds do not re-read unchanged SVG assets.
pub fn broken_image_file(path: &Path) -> std::io::Result<bool> {
    type Key = (u64, Option<SystemTime>);
    static CACHE: Mutex<Option<HashMap<PathBuf, (Key, bool)>>> = Mutex::new(None);
    let meta = std::fs::metadata(path)?;
    let key = (meta.len(), meta.modified().ok());
    if let Some((k, broken)) = CACHE
        .lock()
        .ok()
        .and_then(|c| c.as_ref().and_then(|c| c.get(path).copied()))
        && k == key
        && key.1.is_some()
    {
        return Ok(broken);
    }
    let broken = broken_image(&std::fs::read(path)?);
    if let Ok(mut cache) = CACHE.lock() {
        let cache = cache.get_or_insert_with(HashMap::new);
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(path.to_owned(), (key, broken));
    }
    Ok(broken)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: &str = r#"<rect width="8" height="8" fill="red"/>"#;
    const NS: &str = r#"xmlns="http://www.w3.org/2000/svg" width="8" height="8""#;
    const SVG11: &str = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#;

    fn nest(levels: usize) -> String {
        // The svg root and `levels - 1` groups: the rect is `levels + 1` deep.
        format!(
            "<svg {NS}>{}{R}{}</svg>",
            "<g>".repeat(levels - 1),
            "</g>".repeat(levels - 1)
        )
    }

    /// Each case's Chromium outcome was measured in Electron 44 (Blob
    /// `<img>` and Canvas2D `drawImage`); `true` is a broken image.
    fn chromium_cases() -> Vec<(&'static str, String, bool)> {
        let x = |s: &str| {
            s.replace("{R}", R)
                .replace("{NS}", NS)
                .replace("{SVG11}", SVG11)
        };
        let mut v = vec![
            ("ok", x("<svg {NS}>{R}</svg>"), false),
            (
                "svg11Nbsp",
                x("{SVG11}<svg {NS}>{R}<text>&nbsp;</text></svg>"),
                false,
            ),
            (
                "svg11Copy",
                x("{SVG11}<svg {NS}>{R}<text>&copy;</text></svg>"),
                false,
            ),
            (
                "svg11Attr",
                x("{SVG11}<svg {NS}>{R}<text x=\"&foo;\">a</text></svg>"),
                false,
            ),
            (
                "svg11Standalone",
                x(
                    "<?xml version=\"1.0\" standalone=\"yes\"?>{SVG11}<svg {NS}>{R}<text>&nbsp;</text></svg>",
                ),
                true,
            ),
            (
                "svg11StandaloneNo",
                x(
                    "<?xml version=\"1.0\" standalone=\"no\"?>{SVG11}<svg {NS}>{R}<text>&nbsp;</text></svg>",
                ),
                false,
            ),
            (
                "svg11NoXmlns",
                x("{SVG11}<svg width=\"8\" height=\"8\">{R}</svg>"),
                true,
            ),
            (
                "sysid",
                x("<!DOCTYPE svg SYSTEM \"svg.dtd\"><svg {NS}>{R}<text>&foo;</text></svg>"),
                false,
            ),
            (
                "sysidInternal",
                x(
                    "<!DOCTYPE svg SYSTEM \"svg.dtd\" [<!ENTITY a \"b\">]><svg {NS}>{R}<text>&foo;</text></svg>",
                ),
                false,
            ),
            (
                "extEntity",
                x("<!DOCTYPE svg [<!ENTITY e SYSTEM \"e.xml\">]><svg {NS}>{R}&e;</svg>"),
                false,
            ),
            (
                "extEntityPublic",
                x(
                    "<!DOCTYPE svg [<!ENTITY e PUBLIC \"-//x//y\" \"e.xml\">]><svg {NS}>{R}&e;</svg>",
                ),
                false,
            ),
            (
                "peDef",
                x(
                    "<!DOCTYPE svg [<!ENTITY % a \"<!ENTITY b 'x'>\"> %a;]><svg {NS}>{R}<text>&b;</text></svg>",
                ),
                false,
            ),
            (
                "peUndeclared",
                x(
                    "<!DOCTYPE svg [<!ENTITY % a \"<!ENTITY b 'x'>\"> %a;]><svg {NS}>{R}<text>&zz;</text></svg>",
                ),
                false,
            ),
            (
                "peExternal",
                x(
                    "<!DOCTYPE svg [<!ENTITY % ext SYSTEM \"x.dtd\"> %ext;]><svg {NS}>{R}<text>&zz;</text></svg>",
                ),
                false,
            ),
            (
                "peNoRef",
                x("<!DOCTYPE svg [<!ENTITY % a \"x\">]><svg {NS}>{R}<text>&zz;</text></svg>"),
                true,
            ),
            (
                "internalOnlyUndeclared",
                x("<!DOCTYPE svg [<!ENTITY a \"b\">]><svg {NS}>{R}<text>&zz;</text></svg>"),
                true,
            ),
            (
                "noDtdUndeclared",
                x("<svg {NS}>{R}<text>&zz;</text></svg>"),
                true,
            ),
            (
                "noDtdUndeclaredAttr",
                x("<svg {NS}>{R}<text x=\"&zz;\">a</text></svg>"),
                true,
            ),
            (
                "emptyDoctypeUndeclared",
                x("<!DOCTYPE svg><svg {NS}>{R}<text>&zz;</text></svg>"),
                true,
            ),
            (
                "xhtmlNbsp",
                x(
                    "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\"><svg {NS}>{R}<text>&nbsp;</text></svg>",
                ),
                false,
            ),
            (
                "attlistXmlns",
                x(
                    "<!DOCTYPE svg [<!ATTLIST svg xmlns CDATA #FIXED \"http://www.w3.org/2000/svg\">]><svg width=\"8\" height=\"8\">{R}</svg>",
                ),
                false,
            ),
            ("mismatchAfter", x("<svg {NS}>{R}</g></svg>"), true),
            ("unclosedAfter", x("<svg {NS}>{R}<g>"), true),
            ("unboundPrefix", x("<svg {NS}>{R}<x:rect/></svg>"), true),
            (
                "unboundAttrPrefix",
                x("<svg {NS}>{R}<rect x:a=\"1\"/></svg>"),
                true,
            ),
            (
                "dupAttr",
                x("<svg {NS}>{R}<rect a=\"1\" a=\"2\"/></svg>"),
                true,
            ),
            ("ltInAttr", x("<svg {NS}>{R}<rect a=\"<\"/></svg>"), true),
            ("ampNoName", x("<svg {NS}>{R}<text>& b</text></svg>"), true),
            (
                "ampNoSemi",
                x("<svg {NS}>{R}<text>&amp b</text></svg>"),
                true,
            ),
            ("charRef0", x("<svg {NS}>{R}<text>&#0;</text></svg>"), true),
            ("ctrlChar", x("<svg {NS}>{R}<text>\u{1}</text></svg>"), true),
            ("fffe", x("<svg {NS}>{R}<text>\u{FFFE}</text></svg>"), true),
            (
                "doubleHyphen",
                x("<svg {NS}>{R}<!-- a -- b --></svg>"),
                true,
            ),
            (
                "leadingWsDecl",
                x("\n<?xml version=\"1.0\"?><svg {NS}>{R}</svg>"),
                true,
            ),
            ("leadingWs", x("\n  <svg {NS}>{R}</svg>"), false),
            ("textBefore", x("junk<svg {NS}>{R}</svg>"), true),
            ("textAfter", x("<svg {NS}>{R}</svg>junk"), true),
            ("twoRoots", x("<svg {NS}>{R}</svg><svg {NS}/>"), true),
            ("cdataEnd", x("<svg {NS}>{R}<text>a]]>b</text></svg>"), true),
            (
                "xmlPrefixWrong",
                x("<svg {NS} xmlns:xml=\"http://bad\">{R}</svg>"),
                true,
            ),
            (
                "dupNsDecl",
                x("<svg {NS} xmlns:a=\"u\" xmlns:a=\"u\">{R}</svg>"),
                true,
            ),
            ("badName", x("<svg {NS}>{R}<1rect/></svg>"), true),
            (
                "piXml",
                x("<svg {NS}>{R}<?xml version=\"1.0\"?></svg>"),
                true,
            ),
            ("doctypeAfter", x("<svg {NS}>{R}</svg><!DOCTYPE svg>"), true),
            (
                "colonName",
                x("<svg {NS}>{R}<a:b:c xmlns:a=\"u\"/></svg>"),
                true,
            ),
            (
                "xml11",
                x("<?xml version=\"1.1\"?><svg {NS}>{R}</svg>"),
                false,
            ),
            (
                "declSpaces",
                x(
                    "<?xml  version = \"1.0\"  encoding = \"UTF-8\"  standalone = \"no\" ?><svg {NS}>{R}</svg>",
                ),
                false,
            ),
            (
                "ampInAttr",
                x(
                    "<svg {NS} xmlns:xlink=\"http://www.w3.org/1999/xlink\">{R}<a xlink:href=\"a?b=1&c=2\"/></svg>",
                ),
                true,
            ),
            ("trailingNul", x("<svg {NS}>{R}</svg>\u{0}\u{0}"), true),
            (
                "unquotedAttr",
                x("<svg {NS}>{R}<rect width=8/></svg>"),
                true,
            ),
            (
                "attrNoSpace",
                x("<svg {NS}>{R}<rect a=\"1\"b=\"2\"/></svg>"),
                true,
            ),
            (
                "stylesheetPi",
                x("<?xml version=\"1.0\"?><?xml-stylesheet href=\"a.css\"?><svg {NS}>{R}</svg>"),
                false,
            ),
            (
                "cdataOk",
                x("<svg {NS}>{R}<style><![CDATA[ a > b {} ]]></style></svg>"),
                false,
            ),
            ("gtInAttr", x("<svg {NS}>{R}<rect a=\">\"/></svg>"), false),
            (
                "depthOpen5000",
                format!(
                    "<svg {NS}>{}<rect width=\"8\" height=\"8\" fill=\"red\"></rect>{}</svg>",
                    "<g>".repeat(4998),
                    "</g>".repeat(4998)
                ),
                false,
            ),
            (
                "depthOpen5001",
                format!(
                    "<svg {NS}>{}<rect width=\"8\" height=\"8\" fill=\"red\"></rect>{}</svg>",
                    "<g>".repeat(4999),
                    "</g>".repeat(4999)
                ),
                true,
            ),
            (
                "depthText5000",
                format!(
                    "<svg {NS}>{R}{}t{}</svg>",
                    "<g>".repeat(4999),
                    "</g>".repeat(4999)
                ),
                false,
            ),
            (
                "depthTwice",
                format!(
                    "<svg {NS}>{}{}{R}</svg>",
                    "<g>".repeat(4999) + &"</g>".repeat(4999),
                    "<g>".repeat(4999) + &"</g>".repeat(4999)
                ),
                false,
            ),
            (
                "depthCommentInside",
                format!(
                    "<svg {NS}>{R}{}<!-- <g><g> -->{}</svg>",
                    "<g>".repeat(4999),
                    "</g>".repeat(4999)
                ),
                false,
            ),
            (
                "depthCdataInside",
                format!(
                    "<svg {NS}>{R}{}<![CDATA[<g>]]>{}</svg>",
                    "<g>".repeat(4999),
                    "</g>".repeat(4999)
                ),
                false,
            ),
        ];
        for (n, broken) in [(2000, false), (4999, false), (5000, true), (6000, true)] {
            v.push(("nest", nest(n), broken));
        }
        v
    }

    /// Chromium-painted documents are never classified broken; broken ones
    /// are, except where the outcome is deliberately left undecided.
    #[test]
    fn matches_chromium_broken_images() {
        for (name, source, chromium) in chromium_cases() {
            assert_eq!(broken_image(source.as_bytes()), chromium, "{name}");
        }
        let x = |s: &str| s.replace("{R}", R).replace("{NS}", NS);
        for (name, source) in [
            // Broken in Chromium but left undecided: still an explicit
            // unsupported SVG image, never a wrong paint.
            (
                "publicNoSystem",
                x("<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"><svg {NS}>{R}</svg>"),
            ),
            (
                "doctypeGarbage",
                x("<!DOCTYPE svg [ junk ]><svg {NS}>{R}</svg>"),
            ),
            ("xml20", x("<?xml version=\"2.0\"?><svg {NS}>{R}</svg>")),
            ("emptyPrefixNs", x("<svg {NS} xmlns:a=\"\">{R}</svg>")),
            (
                "xmlnsPrefixDecl",
                x("<svg {NS} xmlns:xmlns=\"u\">{R}</svg>"),
            ),
            (
                "attlistImplied",
                x(
                    "<!DOCTYPE svg [<!ATTLIST svg xmlns CDATA #IMPLIED>]><svg width=\"8\" height=\"8\">{R}</svg>",
                ),
            ),
            (
                "loop",
                x(
                    "<!DOCTYPE svg [<!ENTITY a \"&b;\"><!ENTITY b \"&a;\">]><svg {NS}>{R}<text>&a;</text></svg>",
                ),
            ),
            (
                "depthEntity",
                format!(
                    "<!DOCTYPE svg [<!ENTITY d \"<g><g></g></g>\">]><svg {NS}>{R}{}&d;{}</svg>",
                    "<g>".repeat(4998),
                    "</g>".repeat(4998)
                ),
            ),
            // Painted by Chromium; roxmltree stops at its entity depth
            // limit or would need markup entities expanded.
            ("deepEntity", {
                let decls: String = (0..15)
                    .map(|i| {
                        let v = if i == 14 {
                            "x".to_owned()
                        } else {
                            format!("&e{};", i + 1)
                        };
                        format!("<!ENTITY e{i} \"{v}\">")
                    })
                    .collect();
                x(&format!(
                    "<!DOCTYPE svg [{decls}]><svg {{NS}}>{{R}}<text>&e0;</text></svg>"
                ))
            }),
            (
                "markupEntityOk",
                x(
                    "<!DOCTYPE svg [<!ENTITY r '<rect width=\"8\" height=\"8\" fill=\"red\"/>'>]><svg {NS}>&r;</svg>",
                ),
            ),
        ] {
            assert!(!broken_image(source.as_bytes()), "{name}");
        }
    }

    /// The cases `e2e/native-svg-broken.spec.ts` checks through the app,
    /// and valid documents usvg cannot paint fully (still gated, not
    /// broken).
    #[test]
    fn broken_images_are_distinct_from_unimplemented_svg_paints() {
        for data in [
            "",
            "<svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect",
            "<svg/>",
            "<svg width=\"64\" height=\"64\"><rect width=\"64\" height=\"64\"/></svg>",
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"/>",
            "<svg xmlns=\"http://www.w3.org/2000/svg\">&missing;</svg>",
        ] {
            assert!(broken_image(data.as_bytes()), "{data}");
        }
        for data in [
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>text</text></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject/></svg>"#,
            r#"<!DOCTYPE svg [<!ENTITY text "hello">]><svg xmlns="http://www.w3.org/2000/svg">&text;</svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="bad path"/></svg>"#,
        ] {
            assert!(!broken_image(data.as_bytes()), "{data}");
        }
        let source =
            r#"<?xml version="1.0" encoding="UTF-16"?><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        let utf16: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(!broken_image(&utf16));
        assert!(!broken_image(&[0x1f, 0x8b]));
    }

    /// The parse runs on its own stack: maximum-depth documents neither
    /// overflow a small caller stack nor get misclassified.
    #[test]
    fn deep_documents_parse_on_a_small_caller_stack() {
        let ok = nest(MAX_ELEMENT_DEPTH - 1);
        let deep = nest(50_000);
        std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || {
                assert!(!broken_image(ok.as_bytes()));
                assert!(broken_image(deep.as_bytes()));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn scans_prologs() {
        let p = prolog(r#"<?xml version="1.0" standalone='yes'?><!-- c --><!DOCTYPE svg SYSTEM "a" [<!ENTITY % p "x"> %p; <!ENTITY e SYSTEM "e"> <!ATTLIST svg a CDATA "v>">]><svg/>"#).unwrap();
        assert!(p.standalone && p.doctype && p.external_subset && p.pe_refs && p.attlist);
        assert_eq!(p.entities, ["e"]);
        assert!(p.internal_subset && !p.markup_entities);
        assert!(p.doctype_end > 0 && p.body == p.doctype_end);
        assert_eq!(max_depth(b"<a><b x='>' /><c><!-- <d> --></c></a>"), 2);
        let text = "a\nbc\u{e9}d";
        assert_eq!(offset(text, roxmltree::TextPos::new(2, 4)), Some(6));
    }
}
