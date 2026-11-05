use pm_agents::{
    Event, RunResult,
    history::{self, Receipt},
    persistence::{Store, Threads},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};
#[derive(Default)]
struct Memory {
    data: BTreeMap<String, Value>,
    fail: bool,
}
impl Store for Memory {
    fn get(&mut self, key: &str) -> io::Result<Option<Value>> {
        Ok(self.data.get(key).cloned())
    }
    fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("injected durable publication failure"));
        }
        self.data.insert(key.into(), value.clone());
        Ok(())
    }
}
fn receipt(sequence: u64, event: Event) -> Receipt {
    Receipt {
        source: "run".into(),
        sequence,
        event,
    }
}
#[test]
fn compaction_question_decision_and_terminal_history_survive_reopen_exactly() {
    let mut store = Memory::default();
    let mut threads = Threads::load(&mut store, "p", "thread", 0).unwrap();
    let events = vec![
        receipt(
            0,
            Event::Session {
                run: "run".into(),
                id: "provider-session".into(),
            },
        ),
        receipt(
            2,
            Event::Trace {
                run: "run".into(),
                value: json!({"kind":"question","itemId":"q","questions":[{"id":"colour","question":"Original intent?"}],"answers":{"colour":["Blue"]},"future":{"keep":true}}),
            },
        ),
        receipt(
            3,
            Event::Trace {
                run: "run".into(),
                value: json!({"kind":"tool-start","itemId":"compact-1","label":"Compacting context…"}),
            },
        ),
        receipt(
            4,
            Event::Trace {
                run: "run".into(),
                value: json!({"kind":"tool-end","itemId":"compact-1","output":"Earlier context summarized"}),
            },
        ),
        receipt(
            5,
            Event::Completed {
                run: "run".into(),
                result: RunResult::failure("interrupted", true),
            },
        ),
    ];
    history::publish(&mut threads, &mut store, "thread", &events).unwrap();
    let expected = events
        .iter()
        .map(|v| serde_json::to_value(v).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(history::read(&threads, "thread").unwrap(), expected);
    let reopened = Threads::load(&mut store, "p", "ignored", 6).unwrap();
    assert_eq!(history::read(&reopened, "thread").unwrap(), expected);
    assert_eq!(threads.raw(), reopened.raw());
}
#[test]
fn failed_publication_acknowledges_nothing_and_retry_is_idempotent() {
    let mut store = Memory::default();
    let mut threads = Threads::load(&mut store, "p", "thread", 0).unwrap();
    let original = threads.raw().clone();
    let events = [receipt(
        7,
        Event::Progress {
            run: "run".into(),
            text: "authored progress".into(),
        },
    )];
    store.fail = true;
    assert!(history::publish(&mut threads, &mut store, "thread", &events).is_err());
    assert_eq!(threads.raw(), &original);
    assert!(store.data.is_empty());
    store.fail = false;
    history::publish(&mut threads, &mut store, "thread", &events).unwrap();
    history::publish(&mut threads, &mut store, "thread", &events).unwrap();
    assert_eq!(history::read(&threads, "thread").unwrap().len(), 1);
}
#[test]
fn conflicting_reordered_and_foreign_receipts_refuse_the_whole_batch() {
    let mut store = Memory::default();
    let mut threads = Threads::load(&mut store, "p", "thread", 0).unwrap();
    history::publish(
        &mut threads,
        &mut store,
        "thread",
        &[receipt(5, Event::Ready)],
    )
    .unwrap();
    let original = threads.raw().clone();
    for events in [
        vec![
            receipt(6, Event::Ready),
            receipt(5, Event::Stopped("changed".into())),
        ],
        vec![receipt(4, Event::Ready)],
        vec![receipt(
            6,
            Event::Progress {
                run: "other".into(),
                text: "foreign".into(),
            },
        )],
    ] {
        assert!(history::publish(&mut threads, &mut store, "thread", &events).is_err());
        assert_eq!(threads.raw(), &original);
        assert_eq!(store.data["agentThreads.p"], original);
    }
}
#[test]
fn unknown_history_is_preserved_and_archiving_keeps_complete_receipts() {
    let mut store = Memory::default();
    let mut threads = Threads::load(&mut store, "p", "thread", 0).unwrap();
    history::publish(
        &mut threads,
        &mut store,
        "thread",
        &[receipt(1, Event::Ready)],
    )
    .unwrap();
    threads.archive_thread("thread", "replacement", 2).unwrap();
    threads.save(&mut store).unwrap();
    let reopened = Threads::load(&mut store, "p", "ignored", 3).unwrap();
    assert_eq!(
        reopened.raw()["archivedThreads"][0]["nativeRunEvents"][0]["sequence"],
        1
    );
    threads.thread_mut("replacement").unwrap()["nativeRunEvents"] = json!({"future":"preserve"});
    let before = threads.raw().clone();
    assert!(
        history::publish(
            &mut threads,
            &mut store,
            "replacement",
            &[receipt(3, Event::Ready)]
        )
        .is_err()
    );
    assert_eq!(threads.raw(), &before);
}
