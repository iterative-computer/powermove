//! Main-reference A04 conversation grouping. Plans keep source indices and step
//! ranges, so 60k archived steps are not copied into a presentation snapshot.
//! Reference: c7f46a05641b23944d3692c9d80f9677c7e1ef1d, panels/agent/work-groups.ts.
use serde_json::{Value, json};
use std::ops::Range;
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub index: usize,
    pub key: String,
    pub steps: Option<Range<usize>>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Message(Item),
    Work {
        key: String,
        duration_ms: Option<serde_json::Number>,
        items: Vec<Item>,
    },
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
fn prompt(v: &Value) -> bool {
    v["role"] == "user" && !truthy(&v["steering"])
}
fn item(messages: &[Value], index: usize, suffix: &str, steps: Option<Range<usize>>) -> Item {
    Item {
        index,
        key: format!(
            "{}-{index}{suffix}",
            messages[index]["role"].as_str().unwrap_or("undefined")
        ),
        steps,
    }
}
fn split(message: &Value) -> (usize, usize) {
    let Some(steps) = message["steps"].as_array() else {
        return (0, 0);
    };
    let mut cut = steps.len();
    while cut > 0 && steps[cut - 1]["kind"] == "text" {
        cut -= 1;
    }
    (cut, steps.len())
}
/// Source messages are immutable. A live turn stays unfolded; a finished turn
/// folds its work only when a reply exists, retaining steering and interim notes.
pub fn group(messages: &[Value], live_from: Option<usize>) -> Vec<Entry> {
    let mut out = vec![];
    let mut index = 0;
    while index < messages.len() && !prompt(&messages[index]) {
        out.push(Entry::Message(item(messages, index, "", None)));
        index += 1;
    }
    while index < messages.len() {
        let start = index;
        let mut end = start + 1;
        while end < messages.len() && !prompt(&messages[end]) {
            end += 1;
        }
        index = end;
        out.push(Entry::Message(item(messages, start, "", None)));
        let body = start + 1..end;
        let live = live_from.is_some_and(|i| i >= start && i < end);
        if live || body.is_empty() {
            out.extend(body.map(|i| Entry::Message(item(messages, i, "", None))));
            continue;
        }
        let last = end - 1;
        let mut work_end;
        let mut work = vec![];
        let mut reply = vec![];
        if messages[last]["role"] == "assistant" {
            reply.push(item(messages, last, "", None));
            work_end = last;
            if work_end > start + 1 {
                let before = work_end - 1;
                if messages[before]["role"] == "trace" && !truthy(&messages[before]["steering"]) {
                    let (cut, len) = split(&messages[before]);
                    if cut < len {
                        work_end -= 1;
                        if cut > 0 {
                            work.push(item(messages, before, ":work", Some(0..cut)));
                        }
                        reply.insert(0, item(messages, before, ":reply", Some(cut..len)));
                    }
                }
            }
        } else if messages[last]["role"] == "trace" {
            let (cut, len) = split(&messages[last]);
            if cut == len {
                out.extend(body.map(|i| Entry::Message(item(messages, i, "", None))));
                continue;
            }
            work_end = last;
            if cut > 0 {
                work.push(item(messages, last, ":work", Some(0..cut)));
            }
            reply.push(item(messages, last, ":reply", Some(cut..len)));
        } else {
            out.extend(body.map(|i| Entry::Message(item(messages, i, "", None))));
            continue;
        }
        let mut earlier = (start + 1..work_end)
            .map(|i| item(messages, i, "", None))
            .collect::<Vec<_>>();
        earlier.extend(work);
        work = earlier;
        if work.is_empty() {
            out.extend(body.map(|i| Entry::Message(item(messages, i, "", None))));
            continue;
        }
        let duration_ms = body
            .filter(|i| messages[*i]["role"] == "trace")
            .filter_map(|i| messages[i]["durationMs"].as_number().cloned())
            .max_by(|a, b| a.as_f64().unwrap().total_cmp(&b.as_f64().unwrap()));
        out.push(Entry::Work {
            key: format!("work-{start}"),
            duration_ms,
            items: work,
        });
        out.extend(reply.into_iter().map(Entry::Message));
    }
    out
}
/// Resolve only requested items/pages. This diagnostic helper is also the
/// hermetic comparison boundary for the pinned main-generated oracle fixture.
pub fn materialize(messages: &[Value], entries: &[Entry]) -> Value {
    fn resolve(messages: &[Value], i: &Item) -> Value {
        let mut message = messages[i.index].clone();
        if let Some(range) = &i.steps {
            message["steps"] = json!(
                &messages[i.index]["steps"]
                    .as_array()
                    .expect("validated source steps")[range.clone()]
            );
        }
        json!({"index":i.index,"message":message,"key":i.key})
    }
    json!(entries.iter().map(|e|match e{
        Entry::Message(i)=>{let mut v=resolve(messages,i);v["kind"]=json!("message");v},
        Entry::Work{key,duration_ms,items}=>{let mut v=json!({"kind":"work","key":key,"items":items.iter().map(|i|resolve(messages,i)).collect::<Vec<_>>()});if let Some(ms)=duration_ms{v["durationMs"]=json!(ms);}v}
    }).collect::<Vec<_>>())
}
pub fn work_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!(
            "{}h {}m {}s",
            seconds / 3600,
            seconds % 3600 / 60,
            seconds % 60
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_pinned_main_oracle_without_changing_the_source_messages() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/main-work-groups.json")).unwrap();
        assert_eq!(
            fixture["reference"],
            "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
        );
        for case in fixture["cases"].as_array().unwrap() {
            let messages = case["conversation"].as_array().unwrap();
            let before = messages.clone();
            let live = case["liveFrom"].as_u64().map(|i| i as usize);
            assert_eq!(
                materialize(messages, &group(messages, live)),
                case["expected"],
                "{}",
                case["name"]
            );
            assert_eq!(messages, &before);
        }
        for case in fixture["durations"].as_array().unwrap() {
            assert_eq!(
                work_duration(case["ms"].as_u64().unwrap()),
                case["expected"].as_str().unwrap()
            );
        }
    }
    #[test]
    fn large_trace_group_plan_retains_ranges_instead_of_copying_step_payloads() {
        let messages = vec![
            json!({"role":"user","text":"Keep"}),
            json!({"role":"trace","steps":(0..60_000).map(|i|json!({"id":i,"kind":"tool"})).chain([json!({"kind":"text","text":"Reply"})]).collect::<Vec<_>>()}),
        ];
        let entries = group(&messages, None);
        assert_eq!(entries.len(), 3);
        let Entry::Work { items, .. } = &entries[1] else {
            panic!("work group")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].steps, Some(0..60_000));
    }
}
