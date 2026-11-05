//! Supplemental native MCP namespace, bound to a real kernel actor/document.
//! Hosts retain the frozen tools unchanged and forward this handler's one new
//! tool. No second document writer, implicit profile discovery or JS runtime.
use crate::{
    Caller, KernelHandle, Notice, Reply,
    pro_editing::{OPERATIONS, ProAction, ProReply, ProRequest},
};
use pm_model::{Map, Value};

pub const TOOL: &str = "powermove_native_pro_edit";
const PROTOCOL: &str = "2025-06-18";
/// One connection's read fence. Reads refresh it; mutations use the last observed
/// revision. Close/reopen never silently rebinds this connection's document.
pub struct ProMcp {
    kernel: KernelHandle,
    project: String,
    caller: Caller,
    document: u64,
    revision: u64,
    state_revision: u64,
}
fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut m = Map::new();
    for (key, v) in entries {
        m.insert(key, v);
    }
    Value::Object(m)
}
fn s(v: impl Into<String>) -> Value {
    Value::from(v.into())
}
fn content(value: Value, is_error: bool) -> Value {
    object([
        (
            "content",
            Value::Array(vec![object([
                ("type", s("text")),
                ("text", s(pm_model::json::to_canonical_string(&value))),
            ])]),
        ),
        ("isError", Value::Bool(is_error)),
    ])
}
fn failure(message: impl Into<String>) -> Value {
    content(object([("error", s(message))]), true)
}
fn notice(n: Notice) -> Value {
    match n {
        Notice::NotDurable { message } => {
            object([("kind", s("notDurable")), ("message", s(message))])
        }
        Notice::CommitWithNotice { label, reason } => object([
            ("kind", s("commitWithNotice")),
            ("label", s(label)),
            ("reason", s(reason)),
        ]),
        Notice::ForeignSplit { label, writes } => object([
            ("kind", s("foreignSplit")),
            ("label", s(label)),
            ("writes", s(writes.to_string())),
        ]),
        Notice::Sealed { label, writes } => object([
            ("kind", s("sealed")),
            ("label", s(label)),
            ("writes", s(writes.to_string())),
        ]),
        Notice::Barrier { label } => object([("kind", s("barrier")), ("label", s(label))]),
    }
}
impl ProMcp {
    pub fn new(kernel: KernelHandle, project: &str, mut caller: Caller) -> Result<Self, String> {
        if caller.writer != Caller::native().writer {
            return Err("Native pro MCP requires a native caller".into());
        }
        caller.origin = crate::Origin::Mcp;
        let p = kernel.published(project).ok_or("Project is not open")?;
        let state_revision = kernel.state().revision;
        Ok(Self {
            kernel,
            project: project.into(),
            caller,
            document: p.snapshot.doc_id,
            revision: p.revision(),
            state_revision,
        })
    }
    pub fn tool() -> Value {
        let mut schema=pm_model::json::parse(r##"{
            "type":"object","required":["op"],"additionalProperties":false,
            "properties":{
                "op":{"type":"string"},
                "layerIds":{"type":"array","items":{"type":"string"}},
                "layerId":{"type":"string"},"leftId":{"type":"string"},"rightId":{"type":"string"},
                "previousId":{"type":"string"},"nextId":{"type":"string"},
                "videoId":{"type":"string"},"audioId":{"type":"string"},
                "linkId":{"type":["string","null"]},"tailLinkId":{"type":["string","null"]},
                "delta":{"type":"number"},"time":{"type":"number"},"edge":{"enum":["in","out"]},
                "linked":{"type":"boolean","description":"Required for timing operations. Expand the persistent A/V pair."},
                "bypass":{"type":"boolean"},"add":{"type":"boolean"},
                "tracks":{"type":"array","items":{"$ref":"#/$defs/track"}},
                "tails":{"type":"array","items":{"type":"object","required":["layerId","tailId"],"additionalProperties":false,"properties":{"layerId":{"type":"string"},"tailId":{"type":"string"}}}},
                "compId":{"type":["string","null"]},"assetId":{"type":"string"},
                "sourceIn":{"type":["number","null"]},"sourceOut":{"type":["number","null"]},
                "timelineIn":{"type":["number","null"]},"timelineOut":{"type":["number","null"]},
                "mode":{"enum":["insert","overwrite"]},
                "targets":{"type":"array","items":{"type":"object","required":["media","track","layerId"],"additionalProperties":false,"properties":{"media":{"enum":["video","audio"]},"track":{"$ref":"#/$defs/track"},"layerId":{"type":"string"}}}},
                "splits":{"type":"array","items":{"type":"object","required":["layerId","tailId"],"additionalProperties":false,"properties":{"layerId":{"type":"string"},"tailId":{"type":"string"},"tailLinkId":{"type":["string","null"]}}}},
                "key":{"enum":["j","k","l","reverse","around"]},"before":{"type":"number"},"after":{"type":"number"}
            },
            "$defs":{"track":{"type":"object","required":["track"],"additionalProperties":false,"properties":{"groupId":{"type":["string","null"]},"track":{"type":"integer","minimum":-2147483648,"maximum":2147483647}}}}
        }"##).expect("static pro tool schema");
        schema
            .as_object_mut()
            .unwrap()
            .get_mut("properties")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("op")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(
                "enum",
                Value::Array(OPERATIONS.iter().map(|op| s(*op)).collect()),
            );
        object([
            ("name", s(TOOL)),
            (
                "description",
                s(
                    "Native pro edits by stable id, in seconds. Inspect with layerIds first to refresh the document/state fence and sync badges. Timing commands require explicit linked; ripple commands require tracks. sourceMarks replaces the four marks (omitted/null clears a mark); sourceEdit uses those marks and requires targets and splits (possibly empty). select requires bypass and add. Each document operation is undoable through the kernel. Shuttle reports session intent; unavailable/paused keys decline. No legacy MCP tool is replaced.",
                ),
            ),
            ("inputSchema", schema),
        ])
    }
    async fn invoke(&mut self, args: &Value) -> Value {
        let action = match ProAction::decode(args) {
            Ok(a) => a,
            Err(e) => return failure(e.to_string()),
        };
        let r = ProRequest {
            caller: self.caller,
            document: self.document,
            revision: self.revision,
            state_revision: Some(self.state_revision),
            txid: None,
            action,
        };
        let reply = match self.kernel.pro_editing(&self.project, r).await {
            Some(Ok(v)) => v,
            Some(Err(e)) => return failure(e.to_string()),
            None => return failure("Kernel stopped; outcome is unknown. Inspect before retrying."),
        };
        let reply = match reply {
            ProReply::SourceEdit {
                ack,
                state_revision,
            } => {
                self.state_revision = state_revision;
                ProReply::Document(ack)
            }
            other => other,
        };
        match reply {
            ProReply::SourceEdit { .. } => unreachable!("normalized above"),
            ProReply::Inspection(v) => {
                let read = |key| {
                    v.get(key)
                        .and_then(Value::as_str)
                        .and_then(|s| s.parse::<u64>().ok())
                };
                let (Some(document), Some(revision), Some(state_revision)) =
                    (read("documentId"), read("revision"), read("stateRevision"))
                else {
                    return failure("Malformed actor inspection");
                };
                if document != self.document {
                    return failure("Document lifetime changed");
                }
                self.revision = revision;
                self.state_revision = state_revision;
                content(v, false)
            }
            ProReply::Declined => content(
                object([
                    ("handled", Value::Bool(false)),
                    ("declined", Value::Bool(true)),
                ]),
                false,
            ),
            ProReply::State(ack) => {
                self.state_revision = ack.revision;
                content(
                    object([
                        ("handled", Value::Bool(true)),
                        ("kind", s("sessionState")),
                        ("stateRevision", s(ack.revision.to_string())),
                    ]),
                    false,
                )
            }
            ProReply::Document(ack) => {
                let (result, is_error) = match ack.reply {
                    Reply::Applied { revision, changed } => {
                        self.revision = revision;
                        (
                            object([
                                ("kind", s("document")),
                                ("revision", s(revision.to_string())),
                                ("changed", Value::Bool(changed)),
                            ]),
                            false,
                        )
                    }
                    Reply::Bool(changed) => {
                        self.revision = ack.revision;
                        (
                            object([
                                ("kind", s("history")),
                                ("revision", s(ack.revision.to_string())),
                                ("changed", Value::Bool(changed)),
                            ]),
                            false,
                        )
                    }
                    Reply::Unit => {
                        self.revision = ack.revision;
                        (
                            object([
                                ("kind", s("selection")),
                                ("revision", s(ack.revision.to_string())),
                            ]),
                            false,
                        )
                    }
                    Reply::Throw(e) => (object([("error", s(e.to_string()))]), true),
                    _ => (
                        object([(
                            "error",
                            s("Unexpected kernel reply; inspect before retrying"),
                        )]),
                        true,
                    ),
                };
                content(
                    object([
                        ("result", result),
                        (
                            "folded",
                            ack.folded.map(|n| s(n.to_string())).unwrap_or(Value::Null),
                        ),
                        ("notice", ack.notice.map(notice).unwrap_or(Value::Null)),
                    ]),
                    is_error,
                )
            }
        }
    }
    /// Handle an MCP JSON-RPC message without blocking the caller thread. A
    /// notification returns None. The app owns transport/authentication, frame
    /// size limits and client lifecycle; the stdio runner supplies framing.
    pub async fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let valid_id = id
            .as_ref()
            .is_none_or(|v| matches!(v, Value::String(_) | Value::Number(_) | Value::Null));
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || method.is_none()
            || !valid_id
        {
            return Some(object([
                ("jsonrpc", s("2.0")),
                ("id", Value::Null),
                (
                    "error",
                    object([
                        ("code", Value::Number(-32600.)),
                        ("message", s("Invalid request")),
                    ]),
                ),
            ]));
        }
        let id = id?; // Notifications never cause an edit or a response.
        let mut code = -32602.;
        let result = match method.unwrap() {
            "initialize" => Ok(object([
                ("protocolVersion", s(PROTOCOL)),
                ("capabilities", object([("tools", object([]))])),
                (
                    "serverInfo",
                    object([
                        ("name", s("powermove-native-pro-edit")),
                        ("version", s(env!("CARGO_PKG_VERSION"))),
                    ]),
                ),
            ])),
            "ping" => Ok(object([])),
            "tools/list" => Ok(object([("tools", Value::Array(vec![Self::tool()]))])),
            "tools/call" => match message.get("params") {
                Some(params) if params.get("name").and_then(Value::as_str) == Some(TOOL) => {
                    match params.get("arguments") {
                        Some(args) => Ok(self.invoke(args).await),
                        None => Err("Missing tool arguments"),
                    }
                }
                _ => Err("Unknown tool"),
            },
            _ => {
                code = -32601.;
                Err("Method not found")
            }
        };
        Some(match result {
            Ok(result) => object([("jsonrpc", s("2.0")), ("id", id), ("result", result)]),
            Err(message) => object([
                ("jsonrpc", s("2.0")),
                ("id", id),
                (
                    "error",
                    object([("code", Value::Number(code)), ("message", s(message))]),
                ),
            ]),
        })
    }
}
