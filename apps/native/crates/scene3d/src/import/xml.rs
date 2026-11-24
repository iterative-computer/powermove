use crate::{Error, Result};
use roxmltree::{Document, Node, ParsingOptions};

pub fn parse(bytes: &[u8]) -> Result<Document<'_>> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error("Model XML must be UTF-8".into()))?;
    Document::parse_with_options(
        text,
        ParsingOptions {
            nodes_limit: 2_000_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|e| Error(format!("Invalid model XML: {e}")))
}

pub fn children<'a, 'input>(
    node: Node<'a, 'input>,
    tag: &str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children().filter(move |child| child.has_tag_name(tag))
}

pub fn child<'a, 'input>(node: Node<'a, 'input>, tag: &str) -> Result<Node<'a, 'input>> {
    children(node, tag)
        .next()
        .ok_or_else(|| Error(format!("Model XML has no {tag} element")))
}

pub fn reference<'a>(uri: Option<&'a str>) -> Result<&'a str> {
    uri.and_then(|uri| uri.strip_prefix('#'))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error("Model XML reference must refer to a local ID".into()))
}
