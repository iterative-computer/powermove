//! Source-derived conversation grouping only. Native views use Opus tokens and
//! specs; this module chooses no layout, colors, materials, typography or copy.
use serde_json::{Value, json};
fn item(index: usize, message: &Value, part: &str) -> Value {
    json!({"index":index,"message":message,"key":format!("{}-{index}{part}",message["role"].as_str().unwrap_or_default())})
}
fn plain(index: usize, message: &Value) -> Value {
    let mut item = item(index, message, "");
    item["kind"] = json!("message");
    item
}
fn split(message: &Value) -> (Vec<Value>, Vec<Value>) {
    let steps = message["steps"].as_array().cloned().unwrap_or_default();
    let cut = steps
        .iter()
        .rposition(|v| v["kind"] != "text")
        .map_or(0, |i| i + 1);
    (steps[..cut].to_vec(), steps[cut..].to_vec())
}
fn part(index: usize, message: &Value, steps: Vec<Value>, suffix: &str) -> Value {
    let mut message = message.clone();
    message["steps"] = json!(steps);
    item(index, &message, suffix)
}
pub fn group(conversation: &[Value], live_from: Option<usize>) -> Vec<Value> {
    let mut entries = vec![];
    let mut index = 0;
    let prompt = |v: &Value| v["role"] == "user" && v["steering"] != true;
    while index < conversation.len() && !prompt(&conversation[index]) {
        entries.push(plain(index, &conversation[index]));
        index += 1;
    }
    while index < conversation.len() {
        let start = index;
        let end = (start + 1..conversation.len())
            .find(|i| prompt(&conversation[*i]))
            .unwrap_or(conversation.len());
        index = end;
        entries.push(plain(start, &conversation[start]));
        let body = (start + 1..end).collect::<Vec<_>>();
        let live = live_from.is_some_and(|i| i >= start && i < end);
        let mut work = vec![];
        let mut reply = vec![];
        let mut work_end = body.len();
        let mut fold = !live && !body.is_empty();
        if fold {
            let last = *body.last().unwrap();
            let message = &conversation[last];
            if message["role"] == "assistant" {
                reply.push(item(last, message, ""));
                work_end -= 1;
                if work_end > 0 {
                    let before = body[work_end - 1];
                    let previous = &conversation[before];
                    if previous["role"] == "trace" && previous["steering"] != true {
                        let (steps, text) = split(previous);
                        if !text.is_empty() {
                            work_end -= 1;
                            if !steps.is_empty() {
                                work.push(part(before, previous, steps, ":work"));
                            }
                            reply.insert(0, part(before, previous, text, ":reply"));
                        }
                    }
                }
            } else if message["role"] == "trace" {
                let (steps, text) = split(message);
                if text.is_empty() {
                    fold = false;
                } else {
                    work_end -= 1;
                    if !steps.is_empty() {
                        work.push(part(last, message, steps, ":work"));
                    }
                    reply.push(part(last, message, text, ":reply"));
                }
            } else {
                fold = false;
            }
        }
        if fold {
            let mut earlier = body[..work_end]
                .iter()
                .map(|i| item(*i, &conversation[*i], ""))
                .collect::<Vec<_>>();
            earlier.extend(work);
            work = earlier;
            if work.is_empty() {
                fold = false;
            }
        }
        if !fold {
            for i in body {
                entries.push(plain(i, &conversation[i]));
            }
            continue;
        }
        let mut entry = json!({"kind":"work","key":format!("work-{start}"),"items":work});
        if let Some(duration) = body
            .iter()
            .filter(|i| conversation[**i]["role"] == "trace")
            .filter_map(|i| conversation[*i].get("durationMs").filter(|v| v.is_number()))
            .max_by(|a, b| a.as_f64().unwrap().total_cmp(&b.as_f64().unwrap()))
        {
            entry["durationMs"] = duration.clone();
        }
        entries.push(entry);
        for mut message in reply {
            message["kind"] = json!("message");
            entries.push(message);
        }
    }
    entries
}
/// A monotonic run clock survives steering; reopening restores accumulated
/// duration without counting time while the application was not executing.
#[derive(Default)]
pub struct WorkingClock {
    elapsed: u64,
    start: Option<u64>,
}
impl WorkingClock {
    pub fn restore(elapsed: u64) -> Self {
        Self {
            elapsed,
            start: None,
        }
    }
    pub fn start(&mut self, now: u64) {
        self.start.get_or_insert(now);
    }
    pub fn duration(&self, now: u64) -> u64 {
        self.elapsed
            .saturating_add(self.start.map_or(0, |start| now.saturating_sub(start)))
    }
    pub fn stop(&mut self, now: u64) {
        self.elapsed = self.duration(now);
        self.start = None;
    }
}
