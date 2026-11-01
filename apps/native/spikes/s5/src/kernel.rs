//! Minimal synthetic lease actor, not the production kernel or an Electron oracle.
//! Calls here are from the CLI/test harness; AppKit never calls this blocking shim.
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::mpsc,
    thread,
};
type Row = BTreeMap<String, i64>;
type Doc = BTreeMap<String, Row>;
#[derive(Clone, Debug)]
enum Change {
    Field {
        id: String,
        field: String,
        before: i64,
        after: i64,
    },
    Row {
        id: String,
        before: Option<Row>,
        after: Option<Row>,
    },
}
impl Change {
    fn inverse(&self, doc: &mut Doc) -> bool {
        match self {
            Self::Field {
                id,
                field,
                before,
                after,
            } => {
                if doc.get(id).and_then(|r| r.get(field)) != Some(after) {
                    return false;
                }
                doc.get_mut(id).unwrap().insert(field.clone(), *before);
            }
            Self::Row { id, before, after } => {
                if doc.get(id) != after.as_ref() {
                    return false;
                }
                if let Some(row) = before {
                    doc.insert(id.clone(), row.clone());
                } else {
                    doc.remove(id);
                }
            }
        }
        true
    }
}
struct Gesture {
    id: u64,
    owner: String,
    depth: usize,
    live_source: bool,
    ops: Vec<(String, Change)>,
}
struct Kernel {
    doc: Doc,
    revision: u64,
    next: u64,
    pending: Option<Gesture>,
    history: Vec<Vec<Change>>,
    acks: HashMap<(String, u64, u64), Value>,
}
impl Kernel {
    fn new() -> Self {
        Self {
            doc: BTreeMap::from([(
                "layer0".into(),
                BTreeMap::from([("x".into(), 0), ("y".into(), 0)]),
            )]),
            revision: 0,
            next: 0,
            pending: None,
            history: vec![],
            acks: HashMap::new(),
        }
    }
    fn change(&mut self, q: &Value) -> Result<Change, &'static str> {
        let id = q["id"].as_str().ok_or("missing id")?.to_string();
        match q["kind"].as_str().unwrap_or("") {
            "set" => {
                let field = q["field"].as_str().ok_or("missing field")?.to_string();
                let value = q["value"].as_i64().ok_or("bad value")?;
                let row = self.doc.get_mut(&id).ok_or("target deleted")?;
                let before = *row.get(&field).ok_or("unknown path")?;
                // Validate a staged mutation before publication. Invalid values never reach authority.
                let mut staged = row.clone();
                staged.insert(field.clone(), value);
                if value < 0 {
                    return Err("invalid negative value");
                }
                *row = staged;
                Ok(Change::Field {
                    id,
                    field,
                    before,
                    after: value,
                })
            }
            "create" => {
                if self.doc.contains_key(&id) {
                    return Err("id exists");
                }
                let after = Row::from([("x".into(), 0), ("y".into(), 0)]);
                self.doc.insert(id.clone(), after.clone());
                Ok(Change::Row {
                    id,
                    before: None,
                    after: Some(after),
                })
            }
            "delete" => {
                let before = self.doc.remove(&id).ok_or("target deleted")?;
                Ok(Change::Row {
                    id,
                    before: Some(before),
                    after: None,
                })
            }
            _ => Err("unknown operation"),
        }
    }
    fn handle(&mut self, q: Value) -> Value {
        let writer = q["writer"].as_str().unwrap_or("page").to_string();
        match q["op"].as_str().unwrap_or("") {
            "begin" => {
                if let Some(p) = &mut self.pending {
                    p.depth += 1;
                    return json!({"gesture":p.id,"depth":p.depth});
                }
                self.next += 1;
                self.pending = Some(Gesture {
                    id: self.next,
                    owner: writer,
                    depth: 1,
                    live_source: q["source"].as_bool().unwrap_or(false),
                    ops: vec![],
                });
                json!({"gesture":self.next,"depth":1})
            }
            "write" => {
                let key = (
                    writer.clone(),
                    q["generation"].as_u64().unwrap_or(1),
                    q["txid"].as_u64().unwrap_or(0),
                );
                if let Some(ack) = self.acks.get(&key) {
                    return ack.clone();
                }
                let result = if q["api"] == "edit.mutate"
                    && self.pending.as_ref().is_some_and(|p| p.live_source)
                {
                    Err("live source edit")
                } else {
                    self.change(&q)
                };
                let ack = match result {
                    Ok(change) => {
                        self.revision += 1;
                        let folded = self.pending.as_ref().map(|p| p.id);
                        if let Some(p) = &mut self.pending {
                            p.ops.push((writer, change));
                        } else {
                            self.history.push(vec![change]);
                        }
                        json!({"ok":true,"revision":self.revision,"folded":folded})
                    }
                    Err(message) => {
                        json!({"ok":false,"message":message,"resync":self.doc.get(q["id"].as_str().unwrap_or("")),"revision":self.revision})
                    }
                };
                self.acks.insert(key, ack.clone());
                ack
            }
            "query" => self
                .acks
                .get(&(
                    writer,
                    q["generation"].as_u64().unwrap_or(1),
                    q["txid"].as_u64().unwrap_or(0),
                ))
                .cloned()
                .unwrap_or(Value::Null),
            "commit" => {
                let Some(mut p) = self.pending.take() else {
                    return json!(false);
                };
                p.depth -= 1;
                if p.depth > 0 {
                    self.pending = Some(p);
                    return json!(false);
                }
                let ops: Vec<_> = p.ops.into_iter().map(|(_, c)| c).collect();
                let changed = !ops.is_empty();
                if changed {
                    self.history.push(ops);
                }
                json!(changed)
            }
            "history.cancel" => {
                self.pending = None;
                Value::Null
            }
            "edit.cancel" => {
                let Some(p) = self.pending.take() else {
                    return json!(false);
                };
                let mut staged = self.doc.clone();
                let valid = p
                    .ops
                    .iter()
                    .rev()
                    .filter(|(w, _)| w == &p.owner)
                    .all(|(_, c)| c.inverse(&mut staged));
                if !valid {
                    self.history
                        .push(p.ops.into_iter().map(|(_, c)| c).collect());
                    return json!({"cancel":"commit_with_notice"});
                }
                self.doc = staged;
                self.revision += 1;
                let foreign: Vec<_> = p
                    .ops
                    .into_iter()
                    .filter(|(w, _)| w != &p.owner)
                    .map(|(_, c)| c)
                    .collect();
                if !foreign.is_empty() {
                    self.history.push(foreign);
                }
                json!({"cancel":"owner_reverted_foreign_split"})
            }
            "owner_failed" => {
                if let Some(p) = self.pending.take() {
                    self.history
                        .push(p.ops.into_iter().map(|(_, c)| c).collect());
                }
                json!({"sealed":true})
            }
            "undo" => {
                let Some(ops) = self.history.last() else {
                    return json!(false);
                };
                let mut staged = self.doc.clone();
                if !ops.iter().rev().all(|c| c.inverse(&mut staged)) {
                    return json!(false);
                }
                self.history.pop();
                self.doc = staged;
                self.revision += 1;
                json!(true)
            }
            "state" => {
                json!({"document":self.doc,"revision":self.revision,"entries":self.history.len(),"gesture":self.pending.as_ref().map(|p|p.id)})
            }
            _ => json!({"ok":false,"message":"unknown request"}),
        }
    }
}
struct Call {
    q: Value,
    reply: mpsc::Sender<Value>,
}
pub struct Actor {
    send: Option<mpsc::Sender<Call>>,
    join: Option<thread::JoinHandle<()>>,
}
impl Actor {
    fn new() -> Self {
        let (send, recv) = mpsc::channel::<Call>();
        let join = thread::Builder::new()
            .name("s5-lease-stub".into())
            .spawn(move || {
                let mut k = Kernel::new();
                for call in recv {
                    let answer = k.handle(call.q);
                    let _ = call.reply.send(answer);
                }
            })
            .unwrap();
        Self {
            send: Some(send),
            join: Some(join),
        }
    }
    fn call(&self, q: Value) -> Value {
        let (reply, recv) = mpsc::channel();
        self.send.as_ref().unwrap().send(Call { q, reply }).unwrap();
        recv.recv().unwrap()
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        self.send.take();
        self.join.take().unwrap().join().unwrap();
    }
}
fn write(a: &Actor, writer: &str, txid: u64, field: &str, value: i64) -> Value {
    a.call(json!({"op":"write","writer":writer,"generation":1,"txid":txid,"kind":"set","id":"layer0","field":field,"value":value}))
}
fn state(a: &Actor) -> Value {
    a.call(json!({"op":"state"}))
}
/// Assertions are the executable trace oracle for these synthetic invariants only.
pub fn traces() -> Vec<Value> {
    let mut out = vec![];
    {
        let a = Actor::new();
        let ack = write(&a, "page", 1, "x", 3);
        let retry = write(&a, "page", 1, "x", 3);
        let queried = a.call(json!({"op":"query","writer":"page","txid":1}));
        assert_eq!(ack, retry);
        assert_eq!(ack, queried);
        assert_eq!(state(&a)["revision"], 1);
        out.push(json!({"test":"lost_ack_dedup_query","status":"pass","ack":ack}));
    }
    {
        let a = Actor::new();
        let rejected = write(&a, "page", 1, "x", -1);
        assert_eq!(rejected["ok"], false);
        assert_eq!(state(&a)["document"]["layer0"]["x"], 0);
        a.call(json!({"op":"write","writer":"native","txid":1,"kind":"delete","id":"layer0"}));
        let flush = write(&a, "page", 2, "x", 9);
        assert_eq!(flush["ok"], false);
        assert!(flush["resync"].is_null());
        out.push(json!({"test":"rejection_after_optimistic_mutation_deleted_path_resync","status":"pass","reply":flush}));
    }
    {
        let a = Actor::new();
        a.call(json!({"op":"begin","writer":"native"}));
        write(&a, "native", 1, "x", 1);
        let fold = write(&a, "page", 1, "y", 2);
        assert!(!fold["folded"].is_null());
        let cancel = a.call(json!({"op":"edit.cancel"}));
        assert_eq!(state(&a)["document"]["layer0"], json!({"x":0,"y":2}));
        assert_eq!(state(&a)["entries"], 1);
        assert_eq!(a.call(json!({"op":"undo"})), true);
        assert_eq!(state(&a)["document"]["layer0"]["y"], 0);
        out.push(json!({"test":"native_drag_page_fold_selective_cancel_undo","status":"pass","fold":fold,"cancel":cancel}));
    }
    {
        let a = Actor::new();
        a.call(json!({"op":"begin","writer":"page"}));
        write(&a, "page", 1, "x", 1);
        write(&a, "mcp", 1, "x", 2);
        let cancel = a.call(json!({"op":"edit.cancel"}));
        assert_eq!(cancel["cancel"], "commit_with_notice");
        assert_eq!(state(&a)["document"]["layer0"]["x"], 2);
        out.push(json!({"test":"overlapping_properties_commit_with_notice","status":"pass","cancel":cancel}));
    }
    {
        let a = Actor::new();
        a.call(json!({"op":"begin","writer":"page"}));
        a.call(json!({"op":"write","writer":"page","txid":1,"kind":"create","id":"new"}));
        a.call(json!({"op":"write","writer":"agent","txid":1,"kind":"set","id":"new","field":"x","value":7}));
        let cancel = a.call(json!({"op":"edit.cancel"}));
        assert_eq!(cancel["cancel"], "commit_with_notice");
        assert_eq!(state(&a)["document"]["new"]["x"], 7);
        out.push(json!({"test":"create_edit_dependency_cancel_keeps_ack","status":"pass"}));
    }
    {
        let a = Actor::new();
        a.call(json!({"op":"begin","writer":"page"}));
        write(&a, "page", 1, "x", 1);
        a.call(json!({"op":"history.cancel"}));
        assert_eq!(state(&a)["document"]["layer0"]["x"], 1);
        assert_eq!(state(&a)["entries"], 0);
        out.push(json!({"test":"history_cancel_keeps_edits_drops_entry","status":"pass"}));
    }
    {
        let a = Actor::new();
        a.call(json!({"op":"begin","writer":"page","source":true}));
        let rejected=a.call(json!({"op":"write","writer":"other","txid":1,"api":"edit.mutate","kind":"create","id":"blocked"}));
        assert_eq!(rejected["ok"], false);
        write(&a, "page", 1, "x", 1);
        let folded = write(&a, "mcp", 1, "y", 2);
        write(&a, "native", 1, "x", 3);
        write(&a, "agent", 1, "y", 4);
        assert!(!folded["folded"].is_null());
        assert_eq!(state(&a)["revision"], 4);
        a.call(json!({"op":"owner_failed"}));
        assert_eq!(state(&a)["entries"], 1);
        assert!(state(&a)["gesture"].is_null());
        assert_eq!(state(&a)["document"]["layer0"], json!({"x":3,"y":4}));
        assert_eq!(a.call(json!({"op":"undo"})), true);
        out.push(json!({"test":"open_begin_never_blocks_writers_live_mutate_refuses_owner_failure_seals","status":"pass","folded":folded}));
    }
    {
        let a = Actor::new();
        a.call(json!({"op":"begin","writer":"page"}));
        a.call(json!({"op":"write","writer":"page","txid":1,"kind":"delete","id":"layer0"}));
        a.call(json!({"op":"write","writer":"agent","txid":1,"kind":"create","id":"layer0"}));
        write(&a, "agent", 2, "x", 9);
        let cancel = a.call(json!({"op":"edit.cancel"}));
        assert_eq!(cancel["cancel"], "commit_with_notice");
        assert_eq!(state(&a)["document"]["layer0"]["x"], 9);
        out.push(json!({"test":"delete_recreate_dependency_cancel_keeps_ack","status":"pass"}));
    }
    out
}
#[cfg(test)]
mod tests {
    #[test]
    fn lease_invariants() {
        assert_eq!(super::traces().len(), 8);
    }
}
