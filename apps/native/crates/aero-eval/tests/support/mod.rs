//! Test support: a toy harness that speaks an OpenAI-Responses-style
//! streaming protocol, and a scripted model upstream. Neither contacts a
//! provider; both run on loopback.
#![allow(dead_code)]
use pm_aero_eval::replay::http;
use pm_aero_eval::replay::sse::{self, Event};
use serde_json::{Value, json};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub const SYSTEM: &str = "You are a Powermove agent. Use the tools; finish with checks.";

/// A minimal agent loop: send the conversation, read the streamed reply,
/// run any tool call against a fake tool host, repeat until a final answer.
pub struct Toy {
    pub system: String,
    pub prompt: String,
    /// The run's working folder; tool results mention it (normalised away).
    pub folder: String,
    /// Ask once more after the final answer (a regression the replay catches).
    pub extra_request: bool,
    /// Stop after this many requests (a harness that gives up early).
    pub max_requests: usize,
}

#[derive(Debug, Default, PartialEq)]
pub struct ToyOutcome {
    pub text: String,
    pub tool_calls: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub error: Option<String>,
}

impl Toy {
    pub fn new(folder: &str) -> Self {
        Self {
            system: SYSTEM.into(),
            prompt: "Take 2 is the flubbed one. Lose it and close the gap.".into(),
            folder: folder.into(),
            extra_request: false,
            max_requests: 10,
        }
    }

    fn tool(&self, name: &str, args: &Value) -> String {
        match name {
            "project" => json!({"revision": 7, "layers": ["Take 1", "Take 2", "Take 3"],
                "file": format!("{}/work/interview.pmv", self.folder)})
            .to_string(),
            "edit" => json!({"revision": 8, "removed": args["layer"], "check": "ok"}).to_string(),
            _ => json!({"error": format!("unknown tool {name}")}).to_string(),
        }
    }

    pub fn run(&self, origin: &str) -> ToyOutcome {
        let mut out = ToyOutcome::default();
        let mut input = vec![json!({"role": "user", "content": self.prompt})];
        let cache_key = format!("run-{}", std::process::id() as u64 * 7919 + rand_ish());
        for n in 0..self.max_requests {
            let body = json!({
                "model": "toy-model",
                "instructions": self.system,
                "input": input,
                "tools": [{"type": "function", "name": "project"}, {"type": "function", "name": "edit"}],
                "stream": true,
                "prompt_cache_key": cache_key,
            });
            let resp = match http::send(
                origin,
                "POST",
                "/v1/responses",
                &[
                    ("content-type".into(), "application/json".into()),
                    ("authorization".into(), "Bearer not-a-real-key".into()),
                ],
                body.to_string().as_bytes(),
            ) {
                Ok(r) => r,
                Err(e) => {
                    out.error = Some(e);
                    return out;
                }
            };
            if resp.status != 200 {
                out.error = Some(String::from_utf8_lossy(&resp.body().unwrap_or_default()).into());
                return out;
            }
            let events = sse::parse(&resp.body().unwrap());
            let mut calls = Vec::new();
            for e in &events {
                let d = e.data.clone().unwrap_or(Value::Null);
                match e.event.as_deref() {
                    Some("response.output_text.delta") => {
                        out.text.push_str(d["delta"].as_str().unwrap_or(""))
                    }
                    Some("response.output_item.done") if d["item"]["type"] == "function_call" => {
                        calls.push(d["item"].clone())
                    }
                    Some("response.completed") => {
                        let u = &d["response"]["usage"];
                        out.input_tokens += u["input_tokens"].as_u64().unwrap_or(0);
                        out.output_tokens += u["output_tokens"].as_u64().unwrap_or(0);
                    }
                    _ => {}
                }
            }
            if calls.is_empty() {
                if self.extra_request && n + 1 < self.max_requests {
                    input.push(json!({"role": "user", "content": "anything else?"}));
                    let r = http::send(
                        origin,
                        "POST",
                        "/v1/responses",
                        &[],
                        json!({"model": "toy-model", "input": input})
                            .to_string()
                            .as_bytes(),
                    );
                    if let Ok(r) = r
                        && r.status != 200
                    {
                        out.error =
                            Some(String::from_utf8_lossy(&r.body().unwrap_or_default()).into());
                    }
                }
                return out;
            }
            for c in calls {
                let name = c["name"].as_str().unwrap_or("").to_owned();
                let args: Value = serde_json::from_str(c["arguments"].as_str().unwrap_or("{}"))
                    .unwrap_or(Value::Null);
                let result = self.tool(&name, &args);
                out.tool_calls.push(name);
                input.push(c.clone());
                input.push(json!({"type": "function_call_output", "call_id": c["call_id"], "output": result}));
            }
        }
        out
    }
}

fn rand_ish() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
}

/// The scripted model: reply `n` streams `script[n]`.
pub fn openai_script() -> Vec<Vec<Event>> {
    let call = |id: &str, name: &str, args: Value| {
        vec![
            Event::json(
                Some("response.created"),
                json!({"type": "response.created", "response": {"id": format!("resp_{id}")}}),
            ),
            Event::json(
                Some("response.output_item.done"),
                json!({"type": "response.output_item.done",
                "item": {"type": "function_call", "call_id": format!("call_{id}"), "name": name, "arguments": args.to_string()}}),
            ),
            Event::json(
                Some("response.completed"),
                json!({"type": "response.completed",
                "response": {"id": format!("resp_{id}"), "usage": {"input_tokens": 900, "input_tokens_details": {"cached_tokens": 600}, "output_tokens": 40}}}),
            ),
        ]
    };
    vec![
        call("1", "project", json!({"view": "timeline"})),
        call("2", "edit", json!({"layer": "Take 2", "ripple": true})),
        vec![
            Event::json(
                Some("response.created"),
                json!({"type": "response.created", "response": {"id": "resp_3"}}),
            ),
            Event::json(
                Some("response.output_text.delta"),
                json!({"type": "response.output_text.delta", "delta": "Removed Take 2 "}),
            ),
            Event::json(
                Some("response.output_text.delta"),
                json!({"type": "response.output_text.delta", "delta": "and closed the gap."}),
            ),
            Event::json(
                Some("response.completed"),
                json!({"type": "response.completed",
                "response": {"id": "resp_3", "usage": {"input_tokens": 1200, "input_tokens_details": {"cached_tokens": 900}, "output_tokens": 12}}}),
            ),
            Event {
                event: None,
                data: None,
                text: Some("[DONE]".into()),
            },
        ],
    ]
}

/// A plain-HTTP upstream that streams the script, one reply per request.
pub struct Upstream {
    pub origin: String,
    pub requests: Arc<Mutex<Vec<Value>>>,
}

pub fn upstream(script: Vec<Vec<Event>>) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    std::thread::spawn(move || {
        let mut n = 0;
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let Ok(req) = http::read_request(&s) else {
                continue;
            };
            seen.lock()
                .unwrap()
                .push(serde_json::from_slice(&req.body).unwrap_or(Value::Null));
            let Some(reply) = script.get(n) else {
                let _ = http::write_json(&mut s, 500, b"{\"error\":\"script exhausted\"}");
                continue;
            };
            n += 1;
            http::start_sse(&mut s, 200).unwrap();
            use std::io::Write;
            for e in reply {
                let mut b = Vec::new();
                e.write(&mut b);
                s.write_all(&b).unwrap();
            }
        }
    });
    Upstream { origin, requests }
}

/// The cut the fake CLIs and the fake Aero apply when the stream says so.
pub fn correct_cut() -> Value {
    json!([{"op": "delete", "name": "Take 2"},
           {"op": "set", "name": "B-roll", "path": "/from", "value": 2},
           {"op": "set", "name": "Take 3", "path": "/from", "value": 4},
           {"op": "set", "name": "Take 4", "path": "/from", "value": 6}])
}

/// A synthetic recording for the fake harnesses: turn 0 streams the cut,
/// turn 1 streams the final answer with usage. Built for one slot.
pub fn fake_recording(
    harness: &str,
    task: &str,
    model: &str,
    effort: &str,
    prompt: &str,
) -> pm_aero_eval::replay::Recording {
    use pm_aero_eval::replay::{Exchange, Normalise, Recording, Request, Response};
    let (provider, path) = match harness {
        "claude-code" => ("anthropic-messages", "/v1/messages"),
        _ => ("openai-responses", "/v1/responses"),
    };
    let turn = |n: u32, events: Vec<Event>| Exchange {
        request: Request {
            method: "POST".into(),
            path: path.into(),
            body: json!({"model": model, "prompt": prompt, "turn": n}),
        },
        response: Response {
            status: 200,
            events,
            body: None,
        },
    };
    Recording {
        format: pm_aero_eval::replay::FORMAT.into(),
        version: 1,
        synthetic: true,
        provider: provider.into(),
        harness: harness.into(),
        model: model.into(),
        effort: effort.into(),
        task: task.into(),
        normalise: Normalise::default(),
        exchanges: vec![
            turn(
                0,
                vec![Event::json(
                    Some("fake.commands"),
                    json!({"commands": correct_cut()}),
                )],
            ),
            turn(
                1,
                vec![Event::json(
                    Some("fake.done"),
                    json!({"text": "Cut done.", "usage": {"input": 1000, "cached": 400, "output": 50}}),
                )],
            ),
        ],
        review: None,
    }
}
