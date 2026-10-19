//! The font service's wire format: one binary message per request and per
//! reply on the font port main holds to the interactive engine.
//!
//! The port carries nothing else, and both ends are Powermove code (main's
//! `apps/desktop/src/main/engine-fonts.ts` and this module), so the format
//! is deliberately small instead of a `pm-protocol` envelope: registry
//! answers stay `pm-text`'s JSON strings end to end (main forwards them to
//! the renderer and into `pm-wasm` untouched), and font bytes and subsets
//! travel as raw bytes, never re-encoded.
//!
//! ```text
//! request: id u32 LE | op u8     | json length u32 LE | JSON arguments (UTF-8) | binary tail
//! reply:   id u32 LE | status u8 | payload
//! ```
//!
//! A reply's status says what the payload is: JSON text, raw bytes, or an
//! error message. Ids are the requester's; replies carry them back in any
//! order (the service answers in order today).

/// Request operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    /// `{ generationBase }` → `{ generation }`. Main sends it first on every
    /// new port: answers from a restarted engine must not reuse an earlier
    /// launch's generations (the renderer drops its answers when the
    /// generation changes).
    Open = 1,
    /// `{ queries: Query[], known?: number }` → `{ generation, faces, answers }`.
    /// `known` is the generation the requester last saw; when it differs from
    /// the current one, queries naming faces this registry never handed out
    /// (an earlier launch's face ids) are skipped instead of refused.
    Answer = 2,
    /// `{ blob, index }` → subset bytes.
    Subset = 3,
    /// `{ base }` → `FallbackManifest`.
    Manifest = 4,
    /// `{ family }` → `FaceSource[]`.
    FamilyFaces = 5,
    /// `{ family, weight?, style?, stretch?, index? }` + font bytes → `{ generation }`.
    AddProjectFont = 6,
    /// `{ family }` + font bytes → `{ generation }`.
    SetFontSource = 7,
    /// `{}` → `{ generation }`.
    ClearProjectFonts = 8,
    /// `{}` → `{ generation }`.
    Generation = 9,
    /// `{}` → `string[]` (installed families, then project families).
    FamilyNames = 10,
    /// `{ seeds: FontCacheSeed[] }` → `{ generation }`. The editor
    /// renderer's font cache seeds (amendment 19; `docs/engine/text.md`):
    /// the engine's own text layout measures as that renderer does. Main
    /// sends them last when it replays to a new port; render threads' font
    /// requests wait for them.
    Seeds = 11,
}

impl Op {
    pub fn from_u8(v: u8) -> Option<Op> {
        Some(match v {
            1 => Op::Open,
            2 => Op::Answer,
            3 => Op::Subset,
            4 => Op::Manifest,
            5 => Op::FamilyFaces,
            6 => Op::AddProjectFont,
            7 => Op::SetFontSource,
            8 => Op::ClearProjectFonts,
            9 => Op::Generation,
            10 => Op::FamilyNames,
            11 => Op::Seeds,
            _ => return None,
        })
    }
}

/// Reply statuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    /// The payload is UTF-8 JSON.
    Json = 0,
    /// The payload is raw bytes (a subset).
    Bytes = 1,
    /// The payload is a UTF-8 error message.
    Error = 2,
}

/// A decoded request.
#[derive(Debug, PartialEq)]
pub struct Request<'a> {
    pub id: u32,
    pub op: Op,
    /// UTF-8 JSON arguments (`{}` when the operation takes none).
    pub json: &'a str,
    /// Font bytes for `AddProjectFont` and `SetFontSource`; empty otherwise.
    pub tail: &'a [u8],
}

/// Why a message is not a request. The id is known when the header was
/// readable, so the service can still answer with an error.
#[derive(Debug, PartialEq)]
pub struct Malformed {
    pub id: Option<u32>,
    pub reason: String,
}

const HEADER: usize = 9;

/// Decodes a request.
pub fn decode_request(bytes: &[u8]) -> Result<Request<'_>, Malformed> {
    if bytes.len() < HEADER {
        return Err(Malformed {
            id: None,
            reason: format!(
                "font request of {} bytes is shorter than its header",
                bytes.len()
            ),
        });
    }
    let id = u32::from_le_bytes(bytes[0..4].try_into().expect("4 bytes"));
    let bad = |reason: String| Malformed {
        id: Some(id),
        reason,
    };
    let op =
        Op::from_u8(bytes[4]).ok_or_else(|| bad(format!("unknown font operation {}", bytes[4])))?;
    let len = u32::from_le_bytes(bytes[5..9].try_into().expect("4 bytes")) as usize;
    let rest = &bytes[HEADER..];
    if len > rest.len() {
        return Err(bad(format!(
            "font request JSON of {len} bytes overruns the {}-byte message",
            bytes.len()
        )));
    }
    let json = std::str::from_utf8(&rest[..len])
        .map_err(|e| bad(format!("font request JSON is not UTF-8: {e}")))?;
    Ok(Request {
        id,
        op,
        json,
        tail: &rest[len..],
    })
}

/// Encodes a request (main's side; used by tests and the Rust host).
pub fn encode_request(id: u32, op: Op, json: &str, tail: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + json.len() + tail.len());
    out.extend_from_slice(&id.to_le_bytes());
    out.push(op as u8);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    out.extend_from_slice(tail);
    out
}

/// Encodes a reply.
pub fn encode_reply(id: u32, status: Status, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.extend_from_slice(&id.to_le_bytes());
    out.push(status as u8);
    out.extend_from_slice(payload);
    out
}

/// A decoded reply.
#[derive(Debug, PartialEq)]
pub struct Reply<'a> {
    pub id: u32,
    pub status: Status,
    pub payload: &'a [u8],
}

/// Decodes a reply (main's side; used by tests and the Rust host).
pub fn decode_reply(bytes: &[u8]) -> Option<Reply<'_>> {
    if bytes.len() < 5 {
        return None;
    }
    let id = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let status = match bytes[4] {
        0 => Status::Json,
        1 => Status::Bytes,
        2 => Status::Error,
        _ => return None,
    };
    Some(Reply {
        id,
        status,
        payload: &bytes[5..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let bytes = encode_request(
            7,
            Op::AddProjectFont,
            r#"{"family":"Brand"}"#,
            &[0, 1, 0, 0],
        );
        let r = decode_request(&bytes).unwrap();
        assert_eq!(
            r,
            Request {
                id: 7,
                op: Op::AddProjectFont,
                json: r#"{"family":"Brand"}"#,
                tail: &[0, 1, 0, 0]
            }
        );
        let bytes = encode_request(u32::MAX, Op::Generation, "{}", &[]);
        let r = decode_request(&bytes).unwrap();
        assert_eq!(
            (r.id, r.op, r.json, r.tail.len()),
            (u32::MAX, Op::Generation, "{}", 0)
        );
    }

    #[test]
    fn malformed_requests_say_why_and_keep_the_id_when_they_can() {
        assert_eq!(decode_request(&[1, 2, 3]).unwrap_err().id, None);
        let mut bytes = encode_request(9, Op::Answer, "[]", &[]);
        bytes[4] = 99;
        let e = decode_request(&bytes).unwrap_err();
        assert_eq!(e.id, Some(9));
        assert!(e.reason.contains("unknown font operation 99"));
        let mut bytes = encode_request(3, Op::Answer, "{}", &[]);
        bytes[5] = 200;
        assert!(
            decode_request(&bytes)
                .unwrap_err()
                .reason
                .contains("overruns")
        );
        let mut bytes = encode_request(4, Op::Answer, "ab", &[]);
        bytes[9] = 0xff;
        assert!(decode_request(&bytes).unwrap_err().reason.contains("UTF-8"));
    }

    #[test]
    fn replies_round_trip() {
        let bytes = encode_reply(5, Status::Bytes, &[1, 2, 3]);
        assert_eq!(bytes, [5, 0, 0, 0, 1, 1, 2, 3]);
        assert_eq!(
            decode_reply(&bytes),
            Some(Reply {
                id: 5,
                status: Status::Bytes,
                payload: &[1, 2, 3]
            })
        );
        assert_eq!(decode_reply(&[0, 0, 0, 0, 7]), None);
    }

    /// The bytes `engine-fonts.test.ts` checks main's codec against.
    #[test]
    fn golden_bytes_match_mains_codec() {
        assert_eq!(
            encode_request(0x0102_0304, Op::Subset, r#"{"blob":"ab","index":1}"#, &[]),
            [
                [4, 3, 2, 1, 3, 23, 0, 0, 0].as_slice(),
                br#"{"blob":"ab","index":1}"#.as_slice()
            ]
            .concat()
        );
        assert_eq!(
            encode_reply(258, Status::Error, b"no"),
            [2, 1, 0, 0, 2, b'n', b'o']
        );
    }
}
