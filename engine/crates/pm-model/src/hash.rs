//! The canonical document hash: SHA-256 of the canonical serialization
//! ([`crate::json::write`]), as 64 lowercase hex digits. The production
//! consistency check (section 6, item 7) compares this with the renderer's
//! `canonicalHash(canonicalProject(PM.proj))` from
//! `apps/desktop/src/shared/canonical-json.ts`.

use sha2::{Digest, Sha256};

use crate::json::write::{
    JsonTree, Sink, View, to_canonical_string, write_canonical, write_canonical_split,
};

struct HashSink(Sha256);

impl Sink for HashSink {
    fn push_str(&mut self, s: &str) {
        self.0.update(s.as_bytes());
    }
}

/// SHA-256 of the canonical serialization, without building the whole string.
pub fn canonical_digest<T: JsonTree>(value: &T) -> [u8; 32] {
    let mut sink = HashSink(Sha256::new());
    write_canonical(value, &mut sink);
    sink.0.finalize().into()
}

/// Members of at least this many nodes, and at most `UNIT_MAX_NODES`, are
/// serialized by workers in [`canonical_digest_parallel`].
const UNIT_MIN_NODES: usize = 1 << 10;
const UNIT_MAX_NODES: usize = 1 << 16;

/// [`canonical_digest`] with the serialization spread over
/// [`crate::parallel::workers`] threads: members of a few thousand nodes are
/// serialized by workers, a wave at a time, while this thread hashes the
/// previous wave. The digest is the same (the same bytes, in order).
pub fn canonical_digest_parallel<T: JsonTree + Sync>(value: &T) -> [u8; 32] {
    let workers = crate::parallel::workers();
    if workers < 2 {
        return canonical_digest(value);
    }
    let (texts, members) = write_canonical_split(value, |member| {
        (UNIT_MIN_NODES..=UNIT_MAX_NODES).contains(&nodes_up_to(member, UNIT_MAX_NODES + 1))
    });
    let mut sha = Sha256::new();
    let mut texts = texts.into_iter();
    sha.update(texts.next().expect("one piece more than members"));
    let mut hash = |written: Vec<String>, sha: &mut Sha256| {
        for text in written {
            sha.update(text);
            sha.update(texts.next().expect("one piece more than members"));
        }
    };
    let mut previous = Vec::new();
    for wave in members.chunks(workers * 4) {
        let (written, ()) = crate::parallel::join(
            || crate::parallel::map(wave.len(), |i| to_canonical_string(wave[i])),
            || hash(core::mem::take(&mut previous), &mut sha),
        );
        previous = written;
    }
    hash(previous, &mut sha);
    sha.finalize().into()
}

/// The nodes in `value`, counted up to `limit`.
fn nodes_up_to<T: JsonTree>(value: &T, limit: usize) -> usize {
    fn count<T: JsonTree>(value: &T, left: &mut usize) {
        if *left == 0 {
            return;
        }
        *left -= 1;
        match value.view() {
            View::Array(items) => items.iter().for_each(|item| count(item, left)),
            View::Object(map) => map.iter().for_each(|(_, child)| count(child, left)),
            _ => {}
        }
    }
    let mut left = limit;
    count(value, &mut left);
    limit - left
}

/// [`canonical_digest`] as lowercase hex.
pub fn canonical_hash<T: JsonTree>(value: &T) -> String {
    to_hex(&canonical_digest(value))
}

/// SHA-256 of raw bytes as lowercase hex (PMV4 checksums).
pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes).into())
}

/// 32 bytes as 64 lowercase hex digits.
pub fn hex(digest: &[u8; 32]) -> String {
    to_hex(digest)
}

fn to_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{Value, parse};

    #[test]
    fn hashes_the_canonical_bytes() {
        // sha256("{}") and sha256("") are well-known.
        assert_eq!(
            canonical_hash(&Value::Object(Default::default())),
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let a = parse(r#"{"b":[1,2],"a":"x"}"#).unwrap();
        let b = parse(r#"{"a":"x","b":[1.0,2e0]}"#).unwrap();
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        assert_eq!(canonical_hash(&a), sha256_hex(br#"{"a":"x","b":[1,2]}"#));
    }

    #[test]
    fn the_parallel_digest_is_the_canonical_digest() {
        // Layers of keyframes: members on both sides of each unit bound, keys
        // out of order, nested and top-level splits.
        let layer = |i: usize, keys: usize| {
            let frames: Vec<String> = (0..keys)
                .map(|k| format!(r#"{{"v":[{k},{i}.5,"é\u0000"],"t":{k}}}"#))
                .collect();
            format!(
                r#"{{"z":{i},"id":"L{i}","keys":[{}],"a":{{"b":[{}]}}}}"#,
                frames.join(","),
                (0..i % 7)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let layers: Vec<String> = (0..64)
            .map(|i| layer(i, [1, 120, 400, 4000, 30000][i % 5]))
            .collect();
        let text = format!(
            r#"{{"name":"p","comps":{{"c":{{"layers":[{}]}}}},"big":[{}],"aa":null}}"#,
            layers.join(","),
            (0..70000)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let value = parse(&text).unwrap();
        assert_eq!(canonical_digest_parallel(&value), canonical_digest(&value));
        for small in ["null", "[]", "{}", r#"{"a":[1,{"b":2}]}"#] {
            let value = parse(small).unwrap();
            assert_eq!(canonical_digest_parallel(&value), canonical_digest(&value));
        }
    }
}
