//! The native side of the native/wasm32 identity check
//! (`scripts/identity.mjs`): replays a request file through [`Engine`] and
//! prints one reply per line.
//!
//! Input (JSON): `{ "cases": [ { "doc": "<canonical project JSON>",
//! "requests": [ <query request>, ... ], "fast": [ { "address": {...},
//! "times": [...] } ] } ] }`. Output: for each query its reply text; for each
//! fast-path address and time `fast <kind> <f64 bits as hex>`.
use std::io::Write;

use pm_model::json::{parse_bytes, write::to_canonical_string};
use pm_wasm::Engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: replay <requests.json>");
    let bytes = std::fs::read(&path).expect("read the request file");
    let input = parse_bytes(&bytes).expect("request JSON");
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let cases = input
        .get("cases")
        .and_then(|c| c.as_array())
        .expect("cases");
    for (i, case) in cases.iter().enumerate() {
        let mut engine = Engine::new();
        let doc = case.get("doc").and_then(|d| d.as_str()).expect("doc");
        engine.resync(doc.as_bytes(), 1).expect("resync");
        for request in case
            .get("requests")
            .and_then(|r| r.as_array())
            .into_iter()
            .flatten()
        {
            let reply = engine.query(to_canonical_string(request).as_bytes());
            writeln!(out, "{}", reply.text).unwrap();
        }
        for fast in case
            .get("fast")
            .and_then(|r| r.as_array())
            .into_iter()
            .flatten()
        {
            let address = fast.get("address").expect("address");
            let Some(id) = engine.intern(address) else {
                writeln!(out, "fast unaddressable").unwrap();
                continue;
            };
            for t in fast
                .get("times")
                .and_then(|t| t.as_array())
                .into_iter()
                .flatten()
            {
                let (kind, n) = engine.ev_id(id, t.as_f64().unwrap_or(f64::NAN));
                let bits = if n.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    n.to_bits()
                };
                writeln!(out, "fast {} {:016x}", kind as u8, bits).unwrap();
            }
        }
        writeln!(out, "case {i} hash {}", engine.hash().unwrap_or_default()).unwrap();
    }
}
