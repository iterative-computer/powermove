//! Nesting safety for SVG documents (docs/engine/vector.md, "Parse depth").
//!
//! roxmltree, usvg's converter, resvg's renderer, and the tree's own drop
//! each recurse once per element level, and engine worker threads have
//! 2 MiB stacks. A document is pre-scanned for its nesting without
//! recursion ([`element_depth`]); a document deeper than Blink's XML tree
//! limit ([`MAX_ELEMENT_DEPTH`]) is refused before anything recursive runs,
//! as Chromium shows it as a broken image; a deep one runs its recursive
//! work on a scoped thread whose stack is sized to its nesting
//! ([`with_stack`]).

/// The deepest element nesting Blink's XML parser accepts
/// (`kMaxXMLTreeDepth`); a deeper document is a broken image in Chromium.
pub const MAX_ELEMENT_DEPTH: usize = 5000;

/// Documents at most this deep run their recursive work on the caller's
/// thread. Measured in a debug build on a 2 MiB thread: 1,000 levels run
/// inline overflow it; this many parse, draw, and drop there
/// (`svg::tests::deep_documents_are_safe_on_a_small_stack`).
pub const INLINE_DEPTH: usize = 128;

/// The deepest element nesting in `data`, counted without recursion over
/// comments, CDATA, processing instructions, declarations, and quoted
/// attribute values (entity expansions are not counted).
pub fn element_depth(data: &[u8]) -> usize {
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    let skip_past = |i: &mut usize, end: &[u8]| match data[*i..].windows(end.len()).position(|w| w == end) {
        Some(at) => *i += at + end.len(),
        None => *i = data.len(),
    };
    while i < data.len() {
        let Some(lt) = data[i..].iter().position(|&b| b == b'<') else {
            break;
        };
        i += lt;
        let rest = &data[i..];
        if rest.starts_with(b"<!--") {
            i += 4;
            skip_past(&mut i, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            skip_past(&mut i, b"]]>");
        } else if rest.starts_with(b"<?") {
            skip_past(&mut i, b"?>");
        } else if rest.starts_with(b"</") {
            i += 2;
            skip_past(&mut i, b">");
            depth = depth.saturating_sub(1);
        } else if rest.starts_with(b"<!") {
            // DOCTYPE: skip to its end over an internal subset.
            i += 2;
            let mut bracket = 0usize;
            while i < data.len() {
                match data[i] {
                    b'[' => bracket += 1,
                    b']' => bracket = bracket.saturating_sub(1),
                    b'"' | b'\'' => {
                        let q = data[i];
                        i += 1;
                        skip_past(&mut i, &[q]);
                        continue;
                    }
                    b'>' if bracket == 0 => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
        } else {
            i += 1;
            let mut empty = false;
            while i < data.len() {
                match data[i] {
                    b'>' => {
                        i += 1;
                        break;
                    }
                    b'/' if data.get(i + 1) == Some(&b'>') => {
                        empty = true;
                        i += 2;
                        break;
                    }
                    b'"' | b'\'' => {
                        let q = data[i];
                        i += 1;
                        skip_past(&mut i, &[q]);
                    }
                    _ => i += 1,
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

/// The stack a document nested `depth` levels gets: 16 MiB plus 64 KiB per
/// level (at most about 330 MiB of reserved, lazily committed address
/// space at the limit).
pub fn stack_bytes(depth: usize) -> usize {
    (16 << 20) + depth.min(MAX_ELEMENT_DEPTH) * (64 << 10)
}

/// Runs `f` where a document nested `depth` levels can recurse: on this
/// thread when it is shallow ([`INLINE_DEPTH`]), else on a scoped thread
/// with [`stack_bytes`]. Panics in `f` propagate to the caller.
pub fn with_stack<T: Send>(depth: usize, f: impl FnOnce() -> T + Send) -> T {
    if depth <= INLINE_DEPTH {
        return f();
    }
    std::thread::scope(|s| {
        match std::thread::Builder::new()
            .name("svg-deep".into())
            .stack_size(stack_bytes(depth))
            .spawn_scoped(s, f)
        {
            Ok(handle) => match handle.join() {
                Ok(v) => v,
                Err(panic) => std::panic::resume_unwind(panic),
            },
            // No thread (address space exhausted): there is no safe place
            // to recurse, and the caller's stack is too small.
            Err(e) => panic!("could not start a thread for a {depth}-level SVG: {e}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_counts_elements_not_markup() {
        assert_eq!(element_depth(b""), 0);
        assert_eq!(element_depth(b"<svg/>"), 1);
        assert_eq!(element_depth(b"<svg><g><rect/></g></svg>"), 3);
        // Comments, CDATA, PIs, doctypes, and quoted `>`/`<` are not elements.
        assert_eq!(
            element_depth(
                b"<?xml version=\"1.0\"?><!DOCTYPE svg [<!ENTITY a \"<g>\">]><svg a='<g>' b=\"/>\"><!-- <g><g> --><![CDATA[<g>]]><g/></svg>"
            ),
            2
        );
        let deep = format!("<svg>{}{}</svg>", "<g>".repeat(41), "</g>".repeat(41));
        assert_eq!(element_depth(deep.as_bytes()), 42);
        // Unclosed input still counts what opened.
        assert_eq!(element_depth(b"<svg><g><g"), 3);
    }

    #[test]
    fn stacks_grow_with_depth_and_stop_at_the_limit() {
        assert_eq!(stack_bytes(0), 16 << 20);
        assert_eq!(stack_bytes(MAX_ELEMENT_DEPTH), stack_bytes(MAX_ELEMENT_DEPTH * 4));
        assert_eq!(with_stack(1, || 7), 7);
        assert_eq!(
            with_stack(4000, || std::thread::current().name().map(str::to_owned)),
            Some("svg-deep".into())
        );
    }
}
