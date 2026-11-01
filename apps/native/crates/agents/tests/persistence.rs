use pm_agents::persistence::{Store, Threads, attachment_ids};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};
#[derive(Default)]
struct Memory {
    data: BTreeMap<String, Value>,
    writes: Vec<String>,
    fail: Option<String>,
}
impl Store for Memory {
    fn get(&mut self, k: &str) -> io::Result<Option<Value>> {
        Ok(self.data.get(k).cloned())
    }
    fn set(&mut self, k: &str, v: &Value) -> io::Result<()> {
        if self.fail.as_deref() == Some(k) {
            return Err(io::Error::other("simulated disk failure"));
        }
        self.writes.push(k.into());
        self.data.insert(k.into(), v.clone());
        Ok(())
    }
}
#[test]
fn archive_roundtrip_preserves_unknown_and_interrupted_fields() {
    let mut s = Memory::default();
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    let r = t.thread_mut("t").unwrap();
    r["lastRunStatus"] = json!("running");
    r["futureField"] = json!({"opaque":[1,2,3]});
    r["orchestration"] =
        json!({"callerThreadId":"parent","clientRequestId":"key","fingerprint":"synthetic"});
    r["composerDraft"] = json!("café 🎞");
    t.save(&mut s).unwrap();
    let reloaded = Threads::load(&mut s, "p", "unused", 2).unwrap();
    assert_eq!(t.raw(), reloaded.raw());
}
#[test]
fn attachment_writes_precede_archive_and_cover_message_payloads() {
    let mut s = Memory::default();
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    let r = t.thread_mut("t").unwrap();
    r["attachments"] =
        json!([{"id":"a","dataUrl":"data:image/png;base64,c3ludGhldGlj","future":true}]);
    r["conversation"] = json!([{"role":"user","text":"brief","attachments":[{"id":"b","content":"synthetic attachment"}]}]);
    t.save(&mut s).unwrap();
    assert_eq!(
        s.writes,
        vec!["agentAttachment.a", "agentAttachment.b", "agentThreads.p"]
    );
    assert_eq!(attachment_ids(&s.data["agentThreads.p"]), vec!["a", "b"]);
    let view = t.hydrated(&mut s).unwrap();
    assert_eq!(
        view["threads"][0]["attachments"][0]["dataUrl"],
        "data:image/png;base64,c3ludGhldGlj"
    );
    assert_eq!(view["threads"][0]["attachments"][0]["future"], true);
}
#[test]
fn existing_attachment_id_conflict_never_overwrites_bytes_or_archive() {
    let mut s = Memory::default();
    s.data
        .insert("agentAttachment.a".into(), json!({"content":"old"}));
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    t.thread_mut("t").unwrap()["attachments"] = json!([{"id":"a","content":"changed"}]);
    assert!(t.save(&mut s).is_err());
    assert_eq!(s.data["agentAttachment.a"], json!({"content":"old"}));
    assert!(s.writes.is_empty());
}
#[test]
fn failed_payload_write_cannot_publish_dangling_reference() {
    let mut s = Memory {
        fail: Some("agentAttachment.a".into()),
        ..Memory::default()
    };
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    t.thread_mut("t").unwrap()["attachments"] = json!([{"id":"a","content":"synthetic"}]);
    assert!(t.save(&mut s).is_err());
    assert!(!s.data.contains_key("agentThreads.p"));
}
#[test]
fn failed_archive_write_retains_payload_for_retry() {
    let mut s = Memory {
        fail: Some("agentThreads.p".into()),
        ..Memory::default()
    };
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    t.thread_mut("t").unwrap()["attachments"] = json!([{"id":"a","content":"synthetic"}]);
    assert!(t.save(&mut s).is_err());
    assert!(s.data.contains_key("agentAttachment.a"));
    s.fail = None;
    t.save(&mut s).unwrap();
    assert_eq!(
        s.writes
            .iter()
            .filter(|s| *s == "agentAttachment.a")
            .count(),
        1
    );
}
#[test]
fn missing_payload_remains_recoverable() {
    let mut s = Memory::default();
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    t.thread_mut("t").unwrap()["attachments"] =
        json!([{"id":"missing","payloadStored":true,"name":"retained"}]);
    let view = t.hydrated(&mut s).unwrap();
    assert_eq!(view["threads"][0]["attachments"][0]["payloadStored"], true);
    t.save(&mut s).unwrap();
    assert_eq!(attachment_ids(&s.data["agentThreads.p"]), vec!["missing"]);
}
#[test]
fn removing_thread_archives_every_field_and_payload_reference() {
    let mut s = Memory::default();
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    t.thread_mut("t").unwrap()["attachments"] = json!([{"id":"a","content":"retained"}]);
    t.thread_mut("t").unwrap()["composerDraft"] = json!("draft");
    t.archive_thread("t", "replacement", 2).unwrap();
    t.save(&mut s).unwrap();
    let a = &s.data["agentThreads.p"];
    assert_eq!(a["archivedThreads"][0]["composerDraft"], "draft");
    assert_eq!(attachment_ids(a), vec!["a"]);
    assert_eq!(a["activeId"], "replacement");
    assert!(s.data.contains_key("agentAttachment.a"));
}
#[test]
fn unknown_version_and_duplicate_ids_refuse_without_writing() {
    for source in [
        json!({"version":2,"activeId":"t","threads":[]}),
        json!({"version":1,"activeId":"t","threads":[{"id":"t","conversation":[],"composerDraft":""},{"id":"t","conversation":[],"composerDraft":""}]}),
    ] {
        let mut s = Memory::default();
        s.data.insert("agentThreads.p".into(), source.clone());
        assert!(Threads::load(&mut s, "p", "new", 1).is_err());
        assert_eq!(s.data["agentThreads.p"], source);
        assert!(s.writes.is_empty());
    }
}
#[test]
fn new_thread_reuses_blank_but_retains_drafted_thread() {
    let mut s = Memory::default();
    let mut t = Threads::load(&mut s, "p", "a", 1).unwrap();
    assert_eq!(t.create("b", 2).unwrap()["id"], "a");
    t.thread_mut("a").unwrap()["composerDraft"] = json!("authored");
    assert_eq!(t.create("b", 3).unwrap()["id"], "b");
    assert!(t.select("a"));
    assert!(!t.select("missing"));
}
#[test]
fn oversized_attachment_refuses_before_any_store_write() {
    let mut s = Memory::default();
    let mut t = Threads::load(&mut s, "p", "t", 1).unwrap();
    t.thread_mut("t").unwrap()["attachments"] =
        json!([{"id":"a","content":"x".repeat(32*1024*1024)}]);
    assert!(t.save(&mut s).is_err());
    assert!(s.writes.is_empty());
}
