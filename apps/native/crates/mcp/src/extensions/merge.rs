//! Line-based three-way merge, as main's `mergeText` (diff3 over `\n` lines).
//! `None` means a conflict or a file that is not UTF-8 text.

/// Longest common subsequence of `a` and `b` as increasing index pairs. Common
/// ends are matched first; the middle is bounded so a huge rewrite reports a
/// conflict instead of exhausting memory.
fn matches(a: &[&str], b: &[&str]) -> Option<Vec<(usize, usize)>> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    if am.len().saturating_mul(bm.len()) > 4_000_000 {
        return None;
    }
    let width = bm.len() + 1;
    let mut table = vec![0u32; (am.len() + 1) * width];
    for i in (0..am.len()).rev() {
        for j in (0..bm.len()).rev() {
            table[i * width + j] = if am[i] == bm[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut pairs: Vec<_> = (0..prefix).map(|i| (i, i)).collect();
    let (mut i, mut j) = (0, 0);
    while i < am.len() && j < bm.len() {
        if am[i] == bm[j] {
            pairs.push((prefix + i, prefix + j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs.extend((0..suffix).map(|k| (a.len() - suffix + k, b.len() - suffix + k)));
    Some(pairs)
}

pub fn merge_text(base: Option<&[u8]>, ours: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    let base = std::str::from_utf8(base.unwrap_or_default()).ok()?;
    let (ours, theirs) = (
        std::str::from_utf8(ours).ok()?,
        std::str::from_utf8(theirs).ok()?,
    );
    let (b, o, t): (Vec<_>, Vec<_>, Vec<_>) = (
        base.split('\n').collect(),
        ours.split('\n').collect(),
        theirs.split('\n').collect(),
    );
    let mut in_ours = vec![None; b.len()];
    for (i, j) in matches(&b, &o)? {
        in_ours[i] = Some(j);
    }
    let mut in_theirs = vec![None; b.len()];
    for (i, j) in matches(&b, &t)? {
        in_theirs[i] = Some(j);
    }
    let mut out: Vec<&str> = vec![];
    let (mut bi, mut oi, mut ti) = (0, 0, 0);
    loop {
        if bi < b.len() && in_ours[bi] == Some(oi) && in_theirs[bi] == Some(ti) {
            out.push(b[bi]);
            (bi, oi, ti) = (bi + 1, oi + 1, ti + 1);
            continue;
        }
        // The next line all three keep closes this unstable chunk.
        let next = (bi..b.len()).find(|&k| in_ours[k].is_some() && in_theirs[k].is_some());
        let (bj, oj, tj) = match next {
            Some(k) => (k, in_ours[k].unwrap(), in_theirs[k].unwrap()),
            None => (b.len(), o.len(), t.len()),
        };
        let (base_chunk, ours_chunk, theirs_chunk) = (&b[bi..bj], &o[oi..oj], &t[ti..tj]);
        if ours_chunk == base_chunk {
            out.extend(theirs_chunk);
        } else if theirs_chunk == base_chunk || ours_chunk == theirs_chunk {
            out.extend(ours_chunk);
        } else {
            return None;
        }
        if next.is_none() {
            break;
        }
        (bi, oi, ti) = (bj, oj, tj);
    }
    Some(out.join("\n").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::merge_text;
    fn merge(base: &str, ours: &str, theirs: &str) -> Option<String> {
        merge_text(Some(base.as_bytes()), ours.as_bytes(), theirs.as_bytes())
            .map(|b| String::from_utf8(b).unwrap())
    }
    #[test]
    fn separate_edits_merge_and_overlapping_edits_conflict() {
        let base = "a\nb\nc\nd\ne\n";
        assert_eq!(
            merge(base, "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n").as_deref(),
            Some("a\nB\nc\nD\ne\n")
        );
        assert_eq!(
            merge(base, "a\nb\nc\nd\ne\nours\n", "zero\na\nb\nc\nd\ne\n").as_deref(),
            Some("zero\na\nb\nc\nd\ne\nours\n")
        );
        assert_eq!(merge(base, "a\nX\nc\nd\ne\n", "a\nY\nc\nd\ne\n"), None);
        assert_eq!(
            merge(base, "a\nX\nc\nd\ne\n", "a\nX\nc\nd\ne\n").as_deref(),
            Some("a\nX\nc\nd\ne\n")
        );
        assert_eq!(
            merge(base, "a\nc\nd\ne\n", "a\nb\nc\nd\ne\n").as_deref(),
            Some("a\nc\nd\ne\n")
        );
        assert_eq!(merge_text(None, b"x", b"y"), None);
        assert_eq!(merge_text(Some(b"a"), &[0xff], b"a"), None);
    }
}
