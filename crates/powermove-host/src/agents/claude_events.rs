//! Claude Code `--output-format stream-json` → progress and trace rows
//! (port of `claude/events.ts`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::trace::{self, PROGRESS_CHARS, TRACE_CHARS, fragment_text, human_label, output_excerpt, tool_detail, truncate};

const MAX_TOOL_BLOCKS: usize = 64;
const MAX_TOOL_INPUT_BYTES: usize = 64 * 1024;
/// AskUserQuestion is shown as a question card (via the permission prompt), never as a tool row.
const QUESTION_TOOL: &str = "AskUserQuestion";

#[derive(Debug, PartialEq)]
pub enum Event {
    Progress(String),
    Trace(Value),
    Session(String),
    /// A `control_request` or `control_cancel_request` (stdio permission prompts).
    Control(Value),
    /// The turn's `result` arrived; the CLI waits on stdin until it closes.
    Result,
}

struct ToolBlock {
    id: String,
    name: String,
    partial_json: String,
    detail_unavailable: bool,
}

#[derive(Default)]
pub struct Parser {
    pub project_cwd: Option<PathBuf>,
    session: Option<String>,
    pub output: Option<Value>,
    pub text: String,
    pub error: Option<String>,
    streamed: Option<HashMap<u64, ToolBlock>>,
    /// Once the CLI streams one message it streams them all; later `assistant`
    /// events repeat blocks already delivered as deltas.
    streaming_mode: bool,
    compaction_id: Option<String>,
    compactions: u32,
    hidden: HashSet<String>,
}

/// One line, control characters flattened (`normalizedText`).
fn normalized_text(value: &Value, limit: usize) -> String {
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let Some(text) = value.as_str() else { return String::new() };
    let flattened: String = text.chars().map(|character| if character <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&character) { ' ' } else { character }).collect();
    truncate(SPACES.replace_all(&flattened, " ").trim(), limit)
}

/// Prose keeps its line structure (`normalizedProse`).
fn normalized_prose(value: &Value, limit: usize) -> String {
    static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" *\n").unwrap());
    static BLANK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
    let Some(text) = value.as_str() else { return String::new() };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text: String = text.chars().map(|character| if (character <= '\u{1f}' && character != '\n') || ('\u{7f}'..='\u{9f}').contains(&character) { ' ' } else { character }).collect();
    let text = TRAILING.replace_all(&text, "\n");
    truncate(BLANK.replace_all(&text, "\n\n").trim(), limit)
}

fn short_id(value: &Value) -> Option<String> {
    value.as_str().filter(|id| !id.is_empty() && id.chars().count() <= 120).map(str::to_owned)
}

impl Parser {
    pub fn new(project_cwd: Option<PathBuf>) -> Self {
        Self { project_cwd, ..Self::default() }
    }

    pub fn line(&mut self, line: &str) -> Vec<Event> {
        let mut out = Vec::new();
        let Ok(event) = serde_json::from_str::<Value>(line) else { return out };
        if !event.is_object() {
            return out;
        }
        if self.session.is_none() {
            if let Some(id) = event["session_id"].as_str().map(str::trim).filter(|id| !id.is_empty() && id.len() <= 200) {
                self.session = Some(id.to_owned());
                out.push(Event::Session(id.to_owned()));
            }
        }
        match event["type"].as_str().unwrap_or_default() {
            "stream_event" if event["event"].is_object() => self.stream_event(&event["event"], &mut out),
            "assistant" => {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if self.streaming_mode {
                        if block["type"] == "text" {
                            let text = normalized_text(&block["text"], TRACE_CHARS);
                            if !text.is_empty() {
                                out.push(Event::Progress(truncate(&text, PROGRESS_CHARS)));
                            }
                        }
                    } else {
                        self.assistant_block(block, &mut out);
                    }
                }
            }
            "user" => {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    self.tool_result(block, &mut out);
                }
            }
            "system" => self.system_event(&event, &mut out),
            "control_request" | "control_cancel_request" => out.push(Event::Control(event)),
            "result" => {
                self.output = event.get("structured_output").filter(|value| !value.is_null()).cloned();
                self.text = event["result"].as_str().unwrap_or_default().to_owned();
                if event["is_error"] == true || event["subtype"] == "error" {
                    let text = normalized_text(&event["result"], 4_000);
                    self.error = Some(if text.is_empty() { "Claude generation failed.".into() } else { text });
                }
                out.push(Event::Result);
            }
            _ => {}
        }
        out
    }

    fn system_event(&mut self, event: &Value, out: &mut Vec<Event>) {
        match event["subtype"].as_str().unwrap_or_default() {
            "status" if event["status"] == "compacting" && self.compaction_id.is_none() => {
                self.compactions += 1;
                let id = format!("claude-compact-{}", self.compactions);
                out.push(Event::Trace(trace::tool_start(&id, "compact", "Compacting context", "")));
                self.compaction_id = Some(id);
            }
            "compact_boundary" => {
                let id = match self.compaction_id.take() {
                    Some(id) => id,
                    None => {
                        self.compactions += 1;
                        let id = format!("claude-compact-{}", self.compactions);
                        out.push(Event::Trace(trace::tool_start(&id, "compact", "Compacting context", "")));
                        id
                    }
                };
                out.push(Event::Trace(trace::tool_end(&id, false, "Earlier context summarized")));
            }
            "api_retry" => {
                let attempt = event["attempt"].as_u64().unwrap_or(0);
                let max = event["max_retries"].as_u64().unwrap_or(0);
                out.push(Event::Progress(if attempt > 0 && max > 0 { format!("Retrying the request ({attempt} of {max})…") } else { "Retrying the request…".into() }));
            }
            _ => {}
        }
    }

    fn tool_row(&self, id: &str, name: &str, input: Option<&Value>) -> Value {
        let detail = input.map(|input| tool_detail(name, input, self.project_cwd.as_deref())).unwrap_or_default();
        trace::tool_start(id, &name.to_lowercase(), &human_label(name), &detail)
    }

    fn stream_event(&mut self, event: &Value, out: &mut Vec<Event>) {
        if event["type"] == "message_start" {
            self.streaming_mode = true;
            self.streamed = Some(HashMap::new());
            return;
        }
        let Some(tools) = self.streamed.as_mut() else { return };
        let index = event["index"].as_u64();
        match event["type"].as_str().unwrap_or_default() {
            "content_block_start" => {
                let block = &event["content_block"];
                let (Some(index), true) = (index, block["type"] == "tool_use") else { return };
                let (Some(id), Some(name)) = (short_id(&block["id"]), block["name"].as_str().filter(|name| name.chars().count() <= 80)) else { return };
                if !tools.contains_key(&index) && tools.len() >= MAX_TOOL_BLOCKS {
                    return;
                }
                if name == QUESTION_TOOL {
                    self.hidden.insert(id);
                    return;
                }
                let name = Some(normalized_text(&json!(name), 80)).filter(|name| !name.is_empty()).unwrap_or_else(|| "tool".into());
                tools.insert(index, ToolBlock { id: id.clone(), name: name.clone(), partial_json: String::new(), detail_unavailable: false });
                out.push(Event::Trace(trace::tool_start(&id, &name.to_lowercase(), &human_label(&name), "")));
            }
            "content_block_delta" => {
                let delta = &event["delta"];
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => push_fragment(out, "answer", &delta["text"]),
                    "thinking_delta" => push_fragment(out, "thought", &delta["thinking"]),
                    "input_json_delta" => {
                        let (Some(tool), Some(partial)) = (index.and_then(|index| tools.get_mut(&index)), delta["partial_json"].as_str()) else { return };
                        if tool.detail_unavailable {
                            return;
                        }
                        if tool.partial_json.len() + partial.len() > MAX_TOOL_INPUT_BYTES {
                            tool.detail_unavailable = true;
                            tool.partial_json.clear();
                        } else {
                            tool.partial_json.push_str(partial);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let Some(tool) = index.and_then(|index| tools.remove(&index)) else { return };
                let input = if tool.detail_unavailable { None } else { serde_json::from_str::<Value>(&tool.partial_json).ok() };
                out.push(Event::Trace(self.tool_row(&tool.id, &tool.name, input.as_ref())));
                out.push(Event::Progress(format!("Using {}…", human_label(&tool.name))));
            }
            "message_stop" => self.streamed = None,
            _ => {}
        }
    }

    fn assistant_block(&mut self, block: &Value, out: &mut Vec<Event>) {
        match block["type"].as_str().unwrap_or_default() {
            "text" => {
                let text = normalized_prose(&block["text"], TRACE_CHARS);
                if text.is_empty() {
                    return;
                }
                out.push(Event::Trace(trace::text_step("answer", &text)));
                let progress = normalized_text(&block["text"], PROGRESS_CHARS);
                if !progress.is_empty() {
                    out.push(Event::Progress(progress));
                }
            }
            "thinking" => {
                let text = normalized_prose(&block["thinking"], TRACE_CHARS);
                if !text.is_empty() {
                    out.push(Event::Trace(trace::text_step("thought", &text)));
                }
            }
            "tool_use" => {
                let (Some(id), Some(name)) = (short_id(&block["id"]), block["name"].as_str().filter(|name| name.chars().count() <= 80)) else { return };
                if name == QUESTION_TOOL {
                    self.hidden.insert(id);
                    return;
                }
                let name = Some(normalized_text(&json!(name), 80)).filter(|name| !name.is_empty()).unwrap_or_else(|| "tool".into());
                out.push(Event::Trace(self.tool_row(&id, &name, Some(&block["input"]))));
                out.push(Event::Progress(format!("Using {}…", human_label(&name))));
            }
            _ => {}
        }
    }

    fn tool_result(&self, block: &Value, out: &mut Vec<Event>) {
        let (true, Some(id)) = (block["type"] == "tool_result", short_id(&block["tool_use_id"])) else { return };
        if self.hidden.contains(&id) {
            return;
        }
        let content = match &block["content"] {
            Value::String(text) => text.clone(),
            Value::Array(items) => items.iter().filter(|item| item["type"] == "text").filter_map(|item| item["text"].as_str()).collect::<Vec<_>>().join("\n"),
            _ => String::new(),
        };
        let output = if content.is_empty() { output_excerpt(block) } else { output_excerpt(&Value::String(content)) };
        out.push(Event::Trace(trace::tool_end(&id, block["is_error"] == true, &output)));
    }
}

fn push_fragment(out: &mut Vec<Event>, kind: &str, value: &Value) {
    let text = value.as_str().map(fragment_text).unwrap_or_default();
    if !text.is_empty() {
        out.push(Event::Trace(trace::text_step(kind, &text)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_text_tools_and_the_result() {
        let mut parser = Parser::new(None);
        assert_eq!(parser.line(r#"{"type":"system","subtype":"init","session_id":"s-1"}"#), vec![Event::Session("s-1".into())]);
        parser.line(r#"{"type":"stream_event","event":{"type":"message_start"}}"#);
        assert_eq!(parser.line(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi "}}}"#), vec![Event::Trace(json!({ "kind": "answer", "text": "Hi " }))]);
        parser.line(r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t1","name":"mcp__powermove__get_project_state"}}}"#);
        parser.line(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"page\":1}"}}}"#);
        let stop = parser.line(r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#);
        assert!(matches!(&stop[0], Event::Trace(step) if step["label"] == "Get project state"));
        // The repeated assistant message only refreshes progress.
        assert_eq!(parser.line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there"}]}}"#), vec![Event::Progress("Hi there".into())]);
        let ended = parser.line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"ok"}]}]}}"#);
        assert_eq!(ended, vec![Event::Trace(json!({ "kind": "tool-end", "itemId": "t1", "isError": false, "output": "ok" }))]);
        assert_eq!(parser.line(r#"{"type":"result","subtype":"success","result":"{}","structured_output":{"summary":"done"}}"#), vec![Event::Result]);
        assert_eq!(parser.output, Some(json!({ "summary": "done" })));
        assert!(parser.error.is_none());
    }

    #[test]
    fn hides_questions_and_reports_errors() {
        let mut parser = Parser::new(None);
        assert!(parser.line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"q1","name":"AskUserQuestion","input":{}}]}}"#).is_empty());
        assert!(parser.line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"q1","content":"x"}]}}"#).is_empty());
        parser.line(r#"{"type":"result","subtype":"error","is_error":true,"result":"Not logged in"}"#);
        assert_eq!(parser.error.as_deref(), Some("Not logged in"));
    }
}
