//! Synchronous native access to the same evaluator used by the WASM ABI.
//!
//! Replies are JSON fragments with `Reply` number markers. Request values use
//! pm-model's JSON parser so JSON.stringify's unpaired surrogates survive.
use pm_eval::{PropId, ValueKind};
use pm_model::{Document, Node, NodeKind, Value, consistency::ConsistencyHash, parallel::join};

use crate::{ApplyError, Engine, reply::Reply};
use std::collections::HashSet;

// The ordinary file-open path uses the worker. Existing synchronous project
// assignments and undo must still provide immediate read-your-writes behavior.
pub const MAX_SYNC_RESYNC: usize = 512 * 1024 * 1024;

/// Parsing and both cold digests belong on the ingestion worker. Moving this
/// value into an evaluator does not walk or reserialize the document.
pub struct PreparedDocument {
    pub document: Document,
    pub hash: String,
    pub integrity: String,
    pub source_bytes: usize,
    pub tree_bytes_estimate: usize,
    consistency: ConsistencyHash,
}

impl PreparedDocument {
    pub fn parse(bytes: &[u8], revision: u64) -> Result<Self, String> {
        Ok(Self::from_document(
            Self::parse_tree(bytes, revision)?,
            bytes.len(),
        ))
    }

    /// The reader drops its owned file bytes after this step, before hashing
    /// allocates integrity-cache entries or serializer scratch space.
    pub fn parse_tree(bytes: &[u8], revision: u64) -> Result<Document, String> {
        let root = if bytes.is_empty() {
            None
        } else {
            Some(Node::parse_bytes(bytes, revision).map_err(|e| e.to_string())?)
        };
        Ok(Document::from_root(root, revision))
    }

    pub fn from_document(document: Document, source_bytes: usize) -> Self {
        // Three independent full walks of one immutable tree: natively they
        // run side by side, and both digests on several threads of their own.
        let (hash, ((integrity, consistency), tree_bytes_estimate)) = join(
            || document.canonical_hash().unwrap_or_default(),
            || {
                join(
                    || {
                        let mut consistency = ConsistencyHash::default();
                        (consistency.digest(document.root()), consistency)
                    },
                    || {
                        let mut seen = StorageSeen::default();
                        std::mem::size_of::<Document>()
                            + document
                                .root()
                                .map_or(0, |root| estimate_node_payload(root, &mut seen))
                    },
                )
            },
        );
        Self {
            document,
            hash,
            integrity,
            source_bytes,
            tree_bytes_estimate,
            consistency,
        }
    }
}

// A retained-tree estimate, explicitly not allocator accounting. Includes
// nodes, string payloads and container bookkeeping, but not allocator rounding,
// evaluator plans or fonts. Bound diagnostic scratch space too: identities
// beyond this budget may be conservatively counted more than once.
#[derive(Default)]
struct StorageSeen {
    containers: HashSet<usize>,
    schemas: HashSet<usize>,
}
fn first_allocation(seen: &mut HashSet<usize>, identity: usize, limit: usize) -> bool {
    if seen.contains(&identity) {
        return false;
    }
    if seen.len() < limit {
        seen.insert(identity);
    }
    true
}
fn estimate_node_bytes(node: &Node, seen: &mut StorageSeen) -> usize {
    std::mem::size_of::<Node>() + estimate_node_payload(node, seen)
}
fn estimate_node_payload(node: &Node, seen: &mut StorageSeen) -> usize {
    match node.kind() {
        NodeKind::String(s) => s.heap_bytes(),
        NodeKind::Array(items) => {
            if !first_allocation(
                &mut seen.containers,
                std::sync::Arc::as_ptr(items) as usize,
                16384,
            ) {
                return 0;
            }
            2 * std::mem::size_of::<usize>()
                + std::mem::size_of::<Vec<Node>>()
                + items
                    .iter()
                    .map(|node| estimate_node_bytes(node, seen))
                    .sum::<usize>()
                + (items.capacity() - items.len()) * std::mem::size_of::<Node>()
        }
        NodeKind::Object(map) => {
            if !first_allocation(
                &mut seen.containers,
                std::sync::Arc::as_ptr(map) as usize,
                16384,
            ) {
                return 0;
            }
            let (schema, schema_bytes, capacity) = map.storage_estimate();
            2 * std::mem::size_of::<usize>()
                + std::mem::size_of::<pm_model::json::ObjectMap<Node>>()
                + if first_allocation(&mut seen.schemas, schema, 4096) {
                    schema_bytes
                } else {
                    0
                }
                + map
                    .values()
                    .map(|node| estimate_node_bytes(node, seen))
                    .sum::<usize>()
                + (capacity - map.len()) * std::mem::size_of::<Node>()
        }
        _ => 0,
    }
}

pub fn member<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    args.as_object()?.get(key)
}

pub fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    member(args, key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))
}

pub fn integer(args: &Value, key: &str) -> Result<u64, String> {
    let n = member(args, key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{key} must be an integer"))?;
    if n.is_finite() && (0.0..=9_007_199_254_740_991.0).contains(&n) && n.fract() == 0.0 {
        Ok(n as u64)
    } else {
        Err(format!("{key} must be a safe nonnegative integer"))
    }
}

fn id(args: &Value) -> Result<u32, String> {
    u32::try_from(integer(args, "id")?).map_err(|_| "id is outside u32".into())
}

fn number(args: &Value, key: &str) -> Result<f64, String> {
    if let Some(Value::Number(n)) = member(args, key) {
        return Ok(*n);
    }
    if let Some(Value::Object(marker)) = member(args, key)
        && marker.len() == 1
    {
        match marker.get("\0").and_then(Value::as_str) {
            Some("NaN") => return Ok(f64::NAN),
            Some("Infinity") => return Ok(f64::INFINITY),
            Some("-Infinity") => return Ok(f64::NEG_INFINITY),
            Some("-0") => return Ok(-0.0),
            _ => {}
        }
    }
    Err(format!(
        "{key} must be a number or an exact special-number marker"
    ))
}

pub fn bytes(args: &Value, key: &str) -> Result<Vec<u8>, String> {
    let items = member(args, key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be a byte array"))?;
    items
        .iter()
        .map(|item| {
            item.as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 255.0 && n.fract() == 0.0)
                .map(|n| n as u8)
                .ok_or_else(|| format!("{key} contains an invalid byte"))
        })
        .collect()
}

/// A native evaluator with the WASM wrapper's synchronous semantics.
pub struct Kernel {
    pub engine: Engine,
    pub reply_special: bool,
    pub source_bytes: usize,
    pub tree_bytes_estimate: usize,
    peak_estimate: usize,
    hash: Option<String>,
    integrity: Option<String>,
    status: u32,
}

impl Default for Kernel {
    fn default() -> Self {
        Self {
            engine: Engine::new(),
            reply_special: false,
            source_bytes: 0,
            tree_bytes_estimate: 0,
            peak_estimate: 0,
            hash: Some(String::new()),
            integrity: Some(String::new()),
            status: 0,
        }
    }
}

impl Kernel {
    pub fn adopt(&mut self, prepared: PreparedDocument) {
        self.source_bytes = prepared.source_bytes;
        self.tree_bytes_estimate = prepared.tree_bytes_estimate;
        self.peak_estimate = self
            .peak_estimate
            .max(self.live_estimate().saturating_add(self.source_bytes));
        self.hash = Some(prepared.hash);
        self.integrity = Some(prepared.integrity);
        self.status = 0;
        self.engine
            .adopt_document(prepared.document, prepared.consistency);
    }

    pub fn cached_hash(&self) -> Option<&str> {
        self.hash.as_deref()
    }
    pub fn cached_integrity(&self) -> Option<&str> {
        self.integrity.as_deref()
    }

    /// Install only worker-warmed caches for the exact current shared tree.
    /// The host checks its owner, generation, revision and root identity first.
    /// Interned properties, evaluator plans, fonts and expression status stay.
    pub fn cache_refresh(&mut self, prepared: &mut PreparedDocument) {
        self.hash = Some(prepared.hash.clone());
        self.integrity = Some(prepared.integrity.clone());
        self.tree_bytes_estimate = prepared.tree_bytes_estimate;
        self.engine.refresh_document(
            prepared.document.clone(),
            std::mem::take(&mut prepared.consistency),
        );
    }

    fn live_estimate(&self) -> usize {
        std::mem::size_of::<Self>() + self.tree_bytes_estimate.max(self.source_bytes)
    }

    /// No cold hash is computed here: registration may inspect this after a
    /// mutation to invalidate verification without serializing the whole tree.
    pub fn document(&self) -> &Document {
        self.engine.document()
    }

    pub fn dispatch(&mut self, op: &str, args: &Value) -> Result<String, String> {
        self.dispatch_bytes(op, args, None)
    }

    /// Binary payloads cross NAPI as buffers rather than JSON number arrays.
    /// Apply borrows its body for decoding; fonts take one persistent copy.
    pub fn dispatch_bytes(
        &mut self,
        op: &str,
        args: &Value,
        data: Option<&[u8]>,
    ) -> Result<String, String> {
        self.reply_special = false;
        let mut out = Reply::new();
        match op {
            "resync" => {
                let json = text(args, "json")?;
                if json.len() > MAX_SYNC_RESYNC { return Err("resync exceeds 512 MiB; use load".into()); }
                let prepared = PreparedDocument::parse(json.as_bytes(), integer(args, "revision")?)?;
                self.adopt(prepared);
                out.raw("null");
            }
            "apply" => {
                let (Ok(base), Ok(revision)) = (integer(args, "base"), integer(args, "revision")) else {
                    return Ok("{\"code\":2,\"message\":\"invalid revision\"}".into());
                };
                let owned_body;
                let body = match data { Some(body) => body, None => { owned_body = bytes(args, "body")?; &owned_body } };
                let (code, message) = match self.engine.apply(body, base, revision) {
                    Ok(_) => (0, String::new()),
                    Err(e) => {
                        let code = match &e { ApplyError::Gap { .. } => 1, ApplyError::Revision => 2, ApplyError::Decode(_) => 3, ApplyError::Mismatch(_) => 4 };
                        (code, e.to_string())
                    }
                };
                if code == 0 || code == 4 {
                    self.hash = None;
                    self.integrity = None;
                    // Bounded incremental accounting; never walk a large tree
                    // on each edit merely to refresh diagnostic estimates.
                    self.tree_bytes_estimate = self.tree_bytes_estimate.saturating_add(body.len());
                    self.peak_estimate = self.peak_estimate.max(self.live_estimate());
                }
                out.raw(&format!("{{\"code\":{code},\"message\":")); out.str(&message); out.raw("}");
            }
            "revision" => out.num(self.engine.revision() as f64),
            "hash" => {
                let hash = self.hash.get_or_insert_with(|| self.engine.hash().unwrap_or_default());
                out.str(hash);
            }
            "consistencyHash" => {
                let integrity = self.engine.consistency_hash();
                out.str(&integrity);
                self.integrity = Some(integrity);
            }
            "toJson" => out.str(&self.engine.to_json()),
            "intern" => out.num(self.engine.intern(args).map_or(-1.0, |id| id.index() as f64)),
            "release" => { self.engine.release(PropId::from_index(id(args)?)); out.raw("null"); }
            "ev" => {
                let id = PropId::from_index(id(args)?);
                let (kind, n) = self.engine.ev_id(id, number(args, "t")?);
                self.status = kind as u32 | if self.engine.expression_error(id).is_some() { 0x80 } else { 0 };
                out.raw("{\"number\":"); out.num(n);
                out.raw(&format!(",\"kind\":{},\"last\":", kind as u8));
                if kind == ValueKind::Number { out.raw("null"); } else { out.raw(&self.engine.last_json()); }
                let last_special = kind != ValueKind::Number && self.engine.last_special();
                out.special |= last_special;
                out.raw(&format!(",\"status\":{},\"lastSpecial\":{last_special}}}", self.status));
            }
            "evBatch" => {
                let items = member(args, "ids").and_then(Value::as_array).ok_or("ids must be an array")?;
                let ids: Vec<u32> = items.iter().map(|v| v.as_f64().filter(|n| n.is_finite() && *n >= 0.0 && *n <= u32::MAX as f64 && n.fract() == 0.0).map(|n| n as u32).ok_or_else(|| "ids contains an invalid id".to_owned())).collect::<Result<_, _>>()?;
                let mut values = vec![0.0; ids.len()]; let mut kinds = vec![0; ids.len()];
                self.engine.ev_batch(&ids, number(args, "t")?, &mut values, &mut kinds);
                out.raw("{\"values\":"); out.nums(&values); out.raw(",\"kinds\":[");
                for (i, kind) in kinds.iter().enumerate() { if i > 0 { out.raw(","); } out.num(*kind as f64); }
                out.raw(&format!("],\"status\":{}}}", self.status));
            }
            "expressionError" => match self.engine.expression_error(PropId::from_index(id(args)?)) { Some(e) => out.str(e), None => out.raw("null") },
            "query" => {
                let reply = self.engine.query(pm_model::json::to_canonical_string(args).as_bytes());
                self.reply_special = reply.special;
                return Ok(reply.text);
            }
            "define" => { self.engine.set_definition(args); out.raw("null"); }
            "addFontAnswers" => { self.engine.add_font_answers(text(args, "json")?.as_bytes())?; out.raw("null"); }
            "addFont" => {
                let blob = text(args, "blob")?;
                let index = u32::try_from(integer(args, "index")?).map_err(|_| "index is outside u32")?;
                self.engine.add_font(blob, index, match data { Some(data) => data.to_vec(), None => bytes(args, "data")? }); out.raw("null");
            }
            "setFontSeeds" => out.bool(self.engine.set_font_seeds(text(args, "json")?.as_bytes())?),
            "clearFonts" => { self.engine.clear_fonts(); out.raw("null"); }
            "fontOutlines" => {
                let enabled = member(args, "enabled").and_then(Value::as_bool).ok_or("enabled must be a boolean")?;
                self.engine.set_font_outlines(enabled); out.raw("null");
            }
            "fontReport" => out.raw(&self.engine.font_report()),
            "stats" => {
                let s = self.engine.stats; let c = self.engine.evaluation_cache_stats();
                out.raw(&serde_json::json!({ "batches":s.batches,"ops":s.ops,"resyncs":s.resyncs,"queries":s.queries,"unanswered":s.unanswered,"detachedHits":s.detached_hits,"detachedMisses":s.detached_misses,"revision":self.engine.revision(),"evalPlans":c.plans,"evalReused":c.reused,"evalMemoHits":c.memo_hits }).to_string());
            }
            "heap" => out.raw(&serde_json::json!({"live":self.live_estimate(),"peak":self.peak_estimate.max(self.live_estimate()),"memory":0,"input":0,"output":0,"estimated":true,"shared":true,"sourceBytes":self.source_bytes,"treeBytesEstimate":self.tree_bytes_estimate}).to_string()),
            _ => return Err(format!("unknown document kernel operation {op:?}")),
        }
        self.reply_special = out.special;
        Ok(out.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(json: &str) -> Value {
        pm_model::json::parse(json).unwrap()
    }

    #[test]
    fn tree_storage_estimate_counts_inline_strings_and_shared_key_schemas_once() {
        let base = std::mem::size_of::<Document>();
        assert_eq!(
            PreparedDocument::parse(br#""short""#, 1)
                .unwrap()
                .tree_bytes_estimate,
            base
        );
        assert_eq!(
            PreparedDocument::parse(br#""0123456789abcdef0123456789abcdef""#, 1)
                .unwrap()
                .tree_bytes_estimate,
            // Heap text is the bytes plus the box behind JsString's thin pointer.
            base + std::mem::size_of::<Box<str>>() + 32
        );
        assert_eq!(
            PreparedDocument::parse(br#""\ud800""#, 1)
                .unwrap()
                .tree_bytes_estimate,
            base + std::mem::size_of::<Box<[u8]>>() + 3
        );
        let prepared =
            PreparedDocument::parse(br#"[{"a":"b","c":"d"},{"a":"e","c":"f"}]"#, 1).unwrap();
        let NodeKind::Array(items) = prepared.document.root().unwrap().kind() else {
            panic!("array");
        };
        let (NodeKind::Object(first), NodeKind::Object(second)) =
            (items[0].kind(), items[1].kind())
        else {
            panic!("objects");
        };
        let (schema, schema_bytes, capacity) = first.storage_estimate();
        assert_eq!(schema, second.storage_estimate().0);
        let arc = 2 * std::mem::size_of::<usize>();
        let expected = base
            + arc
            + std::mem::size_of::<Vec<Node>>()
            + items.capacity() * std::mem::size_of::<Node>()
            + 2 * (arc
                + std::mem::size_of::<pm_model::json::ObjectMap<Node>>()
                + capacity * std::mem::size_of::<Node>())
            + schema_bytes;
        assert_eq!(prepared.tree_bytes_estimate, expected);
    }

    #[test]
    fn tree_storage_estimate_keeps_schema_accounting_after_container_budget_fills() {
        // Introduce the second common schema only after the container budget
        // fills. One shared identity set would charge it for every later map.
        let json = format!(
            "[{}]",
            std::iter::repeat_n(r#"{"a":1}"#, 17000)
                .chain(std::iter::repeat_n(r#"{"b":2}"#, 1000))
                .collect::<Vec<_>>()
                .join(",")
        );
        let prepared = PreparedDocument::parse(json.as_bytes(), 1).unwrap();
        let NodeKind::Array(items) = prepared.document.root().unwrap().kind() else {
            panic!("array");
        };
        let (NodeKind::Object(first), NodeKind::Object(last)) =
            (items[0].kind(), items[17000].kind())
        else {
            panic!("objects");
        };
        let (first_schema, first_bytes, first_capacity) = first.storage_estimate();
        let (last_schema, last_bytes, last_capacity) = last.storage_estimate();
        assert_ne!(first_schema, last_schema);
        assert_eq!(first_capacity, 1);
        assert_eq!(last_capacity, 1);
        let arc = 2 * std::mem::size_of::<usize>();
        let expected = std::mem::size_of::<Document>()
            + arc
            + std::mem::size_of::<Vec<Node>>()
            + items.capacity() * std::mem::size_of::<Node>()
            + items.len()
                * (arc
                    + std::mem::size_of::<pm_model::json::ObjectMap<Node>>()
                    + std::mem::size_of::<Node>())
            + first_bytes
            + last_bytes;
        assert_eq!(prepared.tree_bytes_estimate, expected);
    }

    #[test]
    fn binary_kernel_payloads_bypass_json_byte_arrays_and_preserve_apply_fences() {
        let mut kernel = Kernel::default();
        kernel
            .dispatch("resync", &args(r#"{"json":"{}","revision":1}"#))
            .unwrap();
        let op = pm_model::ops::Op::from_value(&args(
            r#"{"path":["value"],"exists":true,"value":"binary"}"#,
        ))
        .unwrap();
        let body = pm_model::binary::encode(&[op]).unwrap();
        let applied = kernel
            .dispatch_bytes(
                "apply",
                &args(r#"{"body":"ignored","base":1,"revision":2}"#),
                Some(&body),
            )
            .unwrap();
        assert!(applied.contains(r#""code":0"#), "{applied}");
        assert_eq!(kernel.engine.to_json(), r#"{"value":"binary"}"#);
        let gap = kernel
            .dispatch_bytes("apply", &args(r#"{"base":0,"revision":3}"#), Some(&body))
            .unwrap();
        assert!(gap.contains(r#""code":1"#), "{gap}");
        assert_eq!(kernel.engine.revision(), 2);
        assert_eq!(
            kernel
                .dispatch_bytes(
                    "addFont",
                    &args(r#"{"blob":"synthetic","index":0,"data":"ignored"}"#),
                    Some(&[0, 255, 1])
                )
                .unwrap(),
            "null"
        );
    }

    #[test]
    fn native_kernel_preserves_reply_markers_and_synchronous_evaluation() {
        let mut kernel = Kernel::default();
        kernel.dispatch("resync", &args(r#"{"json":"{\"id\":\"p\",\"compId\":\"c\",\"assets\":{},\"comps\":{\"c\":{\"id\":\"c\",\"fps\":30,\"layers\":[{\"id\":\"a\",\"from\":0,\"p\":{\"x\":{\"v\":-0,\"kf\":[]}}}]}}}","revision":1}"#)).unwrap();
        let id = kernel
            .dispatch(
                "intern",
                &args(r#"{"comp":"c","layer":"a","key":"x","via":"channel"}"#),
            )
            .unwrap();
        let answer = kernel
            .dispatch("ev", &args(&format!(r#"{{"id":{id},"t":0}}"#)))
            .unwrap();
        assert!(answer.contains(r#""number":{"\u0000":"-0"}"#), "{answer}");
        assert!(answer.contains(r#""kind":0"#));
        let batch = kernel
            .dispatch("evBatch", &args(&format!(r#"{{"ids":[{id}],"t":0}}"#)))
            .unwrap();
        assert!(batch.contains(r#""values":[{"\u0000":"-0"}]"#), "{batch}");
        assert_eq!(
            kernel.dispatch("consistencyHash", &Value::Null).unwrap(),
            format!("\"{}\"", kernel.engine.consistency_hash())
        );
        let expected = kernel
            .engine
            .query(br#"{"op":"mul","a":[1,0,0,1,0,0],"b":[1,0,0,1,2,3]}"#)
            .text;
        assert_eq!(
            kernel
                .dispatch(
                    "query",
                    &args(r#"{"op":"mul","a":[1,0,0,1,0,0],"b":[1,0,0,1,2,3]}"#)
                )
                .unwrap(),
            expected
        );
        let nonfinite = args(r#"{"op":"projectPoint","m":[0,0,0,0,0,0,0,0,0],"x":0,"y":0}"#);
        assert!(
            kernel
                .dispatch("query", &nonfinite)
                .unwrap()
                .contains(r#"{"\u0000":"NaN"}"#)
        );
    }

    #[test]
    fn malformed_inputs_are_rejected_and_sync_replacements_preserve_compatibility() {
        let mut kernel = Kernel::default();
        assert!(
            kernel
                .dispatch("apply", &args(r#"{"body":[256],"base":0,"revision":1}"#))
                .is_err()
        );
        assert_eq!(kernel.engine.revision(), 0);
        assert_eq!(
            kernel
                .dispatch("apply", &args(r#"{"body":[],"base":-1,"revision":1}"#))
                .unwrap(),
            r#"{"code":2,"message":"invalid revision"}"#
        );
        let json = serde_json::json!({"json":format!("{{\"data\":\"{}\"}}", "x".repeat(128 * 1024)),"revision":1}).to_string();
        kernel.dispatch("resync", &args(&json)).unwrap();
        assert_eq!(kernel.engine.revision(), 1);
        assert!(
            kernel
                .dispatch("resync", &args(r#"{"json":"invalid","revision":2}"#))
                .is_err()
        );
        assert_eq!(kernel.engine.revision(), 1);
        assert!(kernel.dispatch("ev", &args(r#"{"id":-1,"t":0}"#)).is_err());
    }

    #[test]
    fn document_marker_shaped_objects_keep_the_original_reply_special_flags() {
        let mut kernel = Kernel::default();
        kernel.adopt(
            PreparedDocument::parse(
                br#"{"comps":{"c":{"layers":[{"id":"a","p":{"x":{"v":{"\u0000":"NaN"}}}}]}}}"#,
                1,
            )
            .unwrap(),
        );
        let id = kernel
            .dispatch(
                "intern",
                &args(r#"{"comp":"c","layer":"a","key":"x","via":"channel"}"#),
            )
            .unwrap();
        let scalar = kernel
            .dispatch("ev", &args(&format!(r#"{{"id":{id},"t":0}}"#)))
            .unwrap();
        assert!(kernel.reply_special, "the unused numeric sentinel is NaN");
        assert!(scalar.contains(r#""lastSpecial":false"#), "{scalar}");
        assert!(scalar.contains(r#""last":{"\u0000":"NaN"}"#), "{scalar}");
        let query = kernel
            .dispatch(
                "query",
                &args(r#"{"op":"ev","comp":"c","layer":0,"key":"x","t":0}"#),
            )
            .unwrap();
        assert!(!kernel.reply_special, "{query}");
        assert!(query.contains(r#"{"\u0000":"NaN"}"#), "{query}");
        let unicode = kernel
            .dispatch(
                "query",
                &args(r#"{"op":"countTextUnits","lines":["\ud800"],"unit":"characters"}"#),
            )
            .unwrap();
        assert_eq!(unicode, r#"{"ok":1}"#);
    }

    #[test]
    fn scalar_and_batch_time_markers_match_the_original_f64_evaluator_calls() {
        let project = br#"{"id":"p","compId":"c","comps":{"c":{"id":"c","fps":30,"layers":[{"id":"a","from":0,"p":{"x":{"v":5,"kf":[],"expr":"time"}}}]}}}"#;
        let address = args(r#"{"comp":"c","layer":"a","key":"x","via":"channel"}"#);
        let mut original = Engine::new();
        original.resync(project, 1).unwrap();
        let original_id = original.intern(&address).unwrap();
        let mut kernel = Kernel::default();
        kernel.adopt(PreparedDocument::parse(project, 1).unwrap());
        let id = kernel.dispatch("intern", &address).unwrap();
        for (marker, time) in [
            ("NaN", f64::NAN),
            ("Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
            ("-0", -0.0),
        ] {
            let request = args(&format!(r#"{{"id":{id},"t":{{"\u0000":"{marker}"}}}}"#));
            let decoded = number(&request, "t").unwrap();
            assert!(if time.is_nan() {
                decoded.is_nan()
            } else {
                decoded.to_bits() == time.to_bits()
            });
            let (kind, n) = original.ev_id(original_id, time);
            let mut expected = Reply::new();
            expected.num(n);
            let answer = kernel.dispatch("ev", &request).unwrap();
            assert!(
                answer.contains(&format!("\"number\":{}", expected.text)),
                "{marker}: {answer}"
            );
            assert!(
                answer.contains(&format!("\"kind\":{}", kind as u8)),
                "{marker}: {answer}"
            );
            let mut values = [0.0];
            let mut kinds = [0];
            original.ev_batch(&[original_id.index()], time, &mut values, &mut kinds);
            let mut expected = Reply::new();
            expected.nums(&values);
            let batch = kernel
                .dispatch(
                    "evBatch",
                    &args(&format!(r#"{{"ids":[{id}],"t":{{"\u0000":"{marker}"}}}}"#)),
                )
                .unwrap();
            assert!(
                batch.contains(&format!("\"values\":{}", expected.text)),
                "{marker}: {batch}"
            );
            assert!(
                batch.contains(&format!("\"kinds\":[{}]", kinds[0])),
                "{marker}: {batch}"
            );
        }
        assert!(number(&args(r#"{"t":{"\u0000":"NaN","extra":1}}"#), "t").is_err());
        assert!(number(&args(r#"{"t":{"\u0000":"undefined"}}"#), "t").is_err());
    }
}
