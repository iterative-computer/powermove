mod support;
use pm_agents::{compatible, media, visual};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io, path::Path, process::Command};
fn fixture() -> Value {
    serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/main/contracts.json"))
            .unwrap(),
    )
    .unwrap()
}
fn expected(result: io::Result<Value>, case: &Value) {
    match result {
        Ok(v) => assert_eq!(v, case["output"], "input: {}", case["input"]),
        Err(e) => assert_eq!(
            e.to_string(),
            case["error"].as_str().unwrap(),
            "input: {}",
            case["input"]
        ),
    }
}
#[test]
fn main_reference_sources_and_recorder_are_pinned() {
    use sha2::{Digest, Sha256};
    let fixture = fixture();
    assert_eq!(
        fixture["reference"],
        "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
    );
    assert_eq!(fixture["synthetic"], true);
    for source in fixture["sources"].as_array().unwrap() {
        let output = Command::new("git")
            .args([
                "show",
                &format!(
                    "{}:{}",
                    fixture["reference"].as_str().unwrap(),
                    source["file"].as_str().unwrap()
                ),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            format!("{:x}", Sha256::digest(output.stdout)),
            source["sha256"]
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(
                fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("scripts/capture-main-contracts.cjs")
                )
                .unwrap()
            )
        ),
        fixture["captureSha256"]
    );
}
#[test]
fn main_provider_url_rules_and_preset_metadata_are_preserved() {
    for case in fixture()["urls"].as_array().unwrap() {
        expected(
            compatible::provider_url(case["input"].as_str().unwrap()).map(|v| json!(v)),
            case,
        );
    }
    assert_eq!(fixture()["presets"].as_array().unwrap().len(), 13);
}
#[test]
fn main_visual_inputs_events_and_inert_media_url_contracts_are_exact() {
    for case in fixture()["visual"].as_array().unwrap() {
        let input = &case["input"];
        if input["kind"] == "input" {
            expected(visual::input(&input["value"]), case);
        } else {
            assert_eq!(
                json!(visual::event(1, Some(1), "null", "fixture", &input["data"])),
                case["output"]
            );
        }
    }
    for case in fixture()["media"].as_array().unwrap() {
        let v = case["input"].as_str().unwrap();
        assert_eq!(
            json!({"extension":media::extension(v),"kind":media::kind(v),"local":media::local_path(v),"url":media::file_url(v)}),
            case["output"]
        );
    }
}
#[test]
fn media_grants_refuse_symlinks_and_read_bounded_inert_ranges() {
    let temp = support::Temp::new();
    fs::write(temp.0.join("clip.mp4"), b"0123456789").unwrap();
    let mut grant = media::Grant::open(&temp.0, Path::new("clip.mp4")).unwrap();
    let range = grant.response("GET", Some("bytes=2-5"));
    assert_eq!((range.status, range.start, range.length), (206, 2, 4));
    assert_eq!(
        grant.read(range.start, range.length as usize).unwrap(),
        b"2345"
    );
    assert_eq!(
        range.headers["Content-Security-Policy"],
        "default-src 'none'; sandbox"
    );
    assert!(!grant.response("HEAD", Some("bytes=2-5")).body);
    assert_eq!(grant.response("GET", Some("bytes=-4")).status, 416);
    assert!(grant.read(0, 4 * 1024 * 1024 + 1).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(temp.0.join("clip.mp4"), temp.0.join("link.mp4")).unwrap();
        assert!(media::Grant::open(&temp.0, Path::new("link.mp4")).is_err());
        std::os::unix::fs::symlink(&temp.0, temp.0.join("alias")).unwrap();
        assert!(media::Grant::open(&temp.0, Path::new("alias/clip.mp4")).is_err());
    }
}
#[test]
fn visual_replies_keep_exact_html_without_creating_a_privileged_bridge() {
    let mut thread = json!({"future":true});
    let html = "<script>document.body.textContent='authored'</script>";
    let reply = visual::publish(
        &mut thread,
        "run",
        "visual-one",
        &json!({"title":"Reply","html":html}),
    )
    .unwrap();
    assert_eq!(reply["html"], html);
    assert_eq!(visual::read(&reply), Some(reply.clone()));
    assert_eq!(
        visual::publish(
            &mut thread,
            "run",
            "visual-one",
            &json!({"title":"Reply","html":html})
        )
        .unwrap(),
        reply
    );
    assert!(
        visual::publish(
            &mut thread,
            "run",
            "visual-one",
            &json!({"title":"Changed","html":html})
        )
        .is_err()
    );
    assert_eq!(thread["visualReplies"].as_array().unwrap().len(), 1);
    assert_eq!(thread["future"], true);
    let data = json!({"channel":"powermove-visual","token":"t","type":"ready"});
    assert!(visual::event(2, Some(1), "null", "t", &data).is_none());
    assert!(visual::event(1, Some(1), "app://powermove", "t", &data).is_none());
    assert!(visual::CSP.contains("connect-src 'none'"));
}

struct ProviderHost {
    config: Value,
    keys: BTreeMap<String, String>,
    input: Value,
    requests: Vec<Value>,
    publications: usize,
}
impl compatible::Host for ProviderHost {
    fn config(&mut self) -> io::Result<Value> {
        Ok(self.config.clone())
    }
    fn key(&mut self, address: &str) -> io::Result<Option<compatible::Secret>> {
        Ok(self.keys.get(address).cloned().map(compatible::Secret::new))
    }
    fn publish(
        &mut self,
        expected: &Value,
        public: &Value,
        address: &str,
        key: Option<&compatible::Secret>,
    ) -> io::Result<()> {
        assert_eq!(expected, &self.config);
        self.config = public.clone();
        if let Some(key) = key {
            self.keys.insert(address.into(), key.expose().into());
        }
        self.publications += 1;
        Ok(())
    }
    fn request(
        &mut self,
        request: compatible::Request<'_>,
        _: usize,
    ) -> io::Result<compatible::Response> {
        self.requests.push(json!({"method":request.method,"url":request.url,"body":request.body,"key":request.key.map(compatible::Secret::expose),"timeoutMs":request.timeout_ms}));
        let url = &request.url;
        let input = &self.input;
        let (status, body) = if url.ends_with("/api/version") {
            if input["ollama"] == true {
                (200, json!({"version":"synthetic"}))
            } else {
                (404, json!({}))
            }
        } else if url.ends_with("/api/show") {
            let id = request.body.as_ref().unwrap()["model"].as_str().unwrap();
            (
                200,
                json!({"capabilities":input["caps"].get(id).cloned().unwrap_or_else(||json!([]))}),
            )
        } else if url.ends_with("/api/ps") {
            (
                200,
                json!({"models":[{"name":format!("{}:latest",input["model"].as_str().unwrap()),"context_length":input["context"]}]}),
            )
        } else {
            (
                input["status"].as_u64().unwrap_or(200) as u16,
                input.get("error").cloned().unwrap_or_else(|| {
                    if url.ends_with("/chat/completions") {
                        json!({"choices":[{"message":{"content":"OK"}}]})
                    } else {
                        input
                            .get("payload")
                            .cloned()
                            .unwrap_or_else(|| json!({"data":[]}))
                    }
                }),
            )
        };
        Ok(compatible::Response {
            status,
            body: serde_json::to_vec(&body).unwrap(),
        })
    }
}
#[test]
fn main_provider_discovery_setup_errors_and_exact_http_requests_match_source() {
    let fixture: Value = serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/main/provider.json"))
            .unwrap(),
    )
    .unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        fixture["reference"],
        "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
    );
    for source in fixture["sources"].as_array().unwrap() {
        let output = Command::new("git")
            .args([
                "show",
                &format!(
                    "{}:{}",
                    fixture["reference"].as_str().unwrap(),
                    source["file"].as_str().unwrap()
                ),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            format!("{:x}", Sha256::digest(output.stdout)),
            source["sha256"]
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(
                fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/capture-main-provider.cjs")
                )
                .unwrap()
            )
        ),
        fixture["captureSha256"]
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 10);
    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let mut keys = BTreeMap::new();
        if input["saved"]["secret"].is_string() {
            keys.insert(
                input["saved"]["baseUrl"].as_str().unwrap().to_owned(),
                "x".into(),
            );
        }
        let mut host = ProviderHost {
            config: input.get("saved").cloned().unwrap_or_else(|| json!({})),
            keys,
            input: input.clone(),
            requests: vec![],
            publications: 0,
        };
        let before = host.config.clone();
        let result = if input["operation"] == "list" {
            compatible::list_models(&mut host, input["baseUrl"].as_str().unwrap(), None)
        } else {
            compatible::configure(&mut host, input)
        };
        expected(result, case);
        assert_eq!(
            host.requests,
            case["requests"].as_array().unwrap().clone(),
            "{} requests",
            input["name"]
        );
        if case.get("error").is_some() {
            assert_eq!(host.config, before);
            assert_eq!(host.publications, 0);
        }
    }
}
#[test]
fn claude_main_auto_classifier_mode_keeps_handed_back_prompts_and_editor_sandbox() {
    use pm_agents::{Access, ApprovalMode, approvals::Authority, claude};
    for (access, approval, expected) in [
        (Access::Editor, ApprovalMode::Auto, "default"),
        (Access::Project, ApprovalMode::Auto, "auto"),
        (Access::Project, ApprovalMode::Supervised, "acceptEdits"),
    ] {
        let options = claude::Options {
            authority: Authority::new(access, approval, false).unwrap(),
            schema: json!({}),
            model: None,
            effort: None,
            session: None,
            mcp: json!({"mcpServers":{}}),
            extensions: None,
            instructions: String::new(),
            project_tools: vec![],
            editor_tools: vec![],
        };
        let argv = options.argv().unwrap();
        let mode = argv.iter().position(|s| s == "--permission-mode").unwrap();
        assert_eq!(argv[mode + 1], expected);
        assert!(argv.contains(&"--permission-prompt-tool".into()));
        assert!(!argv.contains(&"--dangerously-skip-permissions".into()));
        if access == Access::Project {
            assert!(options.authority.claude_asks());
        }
    }
}

#[test]
fn main_markdown_semantic_trees_streaming_fences_nested_lists_and_inert_links_are_exact() {
    for case in fixture()["markdown"].as_array().unwrap() {
        let text = case["input"]["text"].as_str().unwrap();
        let blocks = pm_agents::markdown::blocks(text).unwrap();
        assert_eq!(
            json!({"plain":pm_agents::markdown::plain(&blocks),"blocks":blocks}),
            case["output"],
            "markdown {text:?}"
        );
    }
}

#[test]
fn main_compatible_streams_keep_utf8_tool_arrival_order_and_failure_semantics() {
    let fixture: Value = serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/main/provider.json"))
            .unwrap(),
    )
    .unwrap();
    for case in fixture["streams"].as_array().unwrap() {
        let mut parser = pm_agents::completion::Completion::default();
        let mut traces = vec![];
        let mut failure = None;
        for chunk in case["chunks"].as_array().unwrap() {
            let bytes = chunk
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>();
            match parser.push(&bytes) {
                Ok(events) => traces.extend(events),
                Err(e) => {
                    failure = Some(e.to_string());
                    break;
                }
            }
        }
        if failure.is_none() {
            match parser.finish() {
                Ok((value, events)) => {
                    traces.extend(events);
                    assert_eq!(value, case["output"]);
                }
                Err(e) => failure = Some(e.to_string()),
            }
        }
        assert_eq!(
            failure,
            case.get("error").and_then(Value::as_str).map(str::to_owned)
        );
        assert_eq!(json!(traces), case["traces"]);
    }
}

#[test]
fn every_main_markdown_unit_input_and_reveal_word_key_matches_source_execution() {
    let fixture = fixture();
    assert!(fixture["markdownSourceTests"].as_u64().unwrap() >= 20);
    for case in fixture["inline"].as_array().unwrap() {
        assert_eq!(
            json!(
                pm_agents::markdown::inline_runs(
                    case["text"].as_str().unwrap(),
                    Some(&case["flags"])
                )
                .unwrap()
            ),
            case["output"],
            "inline {:?}",
            case["text"]
        );
    }
    for case in fixture["words"].as_array().unwrap() {
        assert_eq!(
            json!(pm_agents::markdown::words(case["runs"].as_array().unwrap())),
            case["output"]
        );
    }
}

#[test]
fn main_claude_adapter_permission_modes_and_sandbox_settings_match_actual_source() {
    use pm_agents::{Access, ApprovalMode, approvals::Authority, claude};
    for case in fixture()["claude"].as_array().unwrap() {
        let access: Access = serde_json::from_value(case["input"]["access"].clone()).unwrap();
        let approval: ApprovalMode =
            serde_json::from_value(case["input"]["approval"].clone()).unwrap();
        let options = claude::Options {
            authority: Authority::new(access, approval, access == Access::Computer).unwrap(),
            schema: json!({}),
            model: None,
            effort: None,
            session: None,
            mcp: json!({"mcpServers":{}}),
            extensions: None,
            instructions: String::new(),
            project_tools: vec![],
            editor_tools: vec![],
        };
        let argv = options.argv().unwrap();
        let flag = |name: &str| {
            argv.iter()
                .position(|s| s == name)
                .map(|i| argv[i + 1].clone())
        };
        let settings = flag("--settings").map(|s| serde_json::from_str::<Value>(&s).unwrap());
        assert_eq!(
            json!({"mode":flag("--permission-mode"),"settings":settings,"allowed":flag("--allowedTools"),"disallowed":flag("--disallowedTools"),"skip":argv.contains(&"--dangerously-skip-permissions".into())}),
            case["output"]
        );
    }
}

#[test]
fn markdown_ordered_markers_use_ascii_digits_like_the_main_reference() {
    for marker in ["١", "１", "𝟙", "१२", "9999999999"] {
        let blocks = pm_agents::markdown::blocks(&format!("{marker}. authored text")).unwrap();
        assert_ne!(blocks[0]["kind"], "list", "{marker}");
        assert!(pm_agents::markdown::plain(&blocks).contains(marker));
    }
    let blocks = pm_agents::markdown::blocks("999999999. item").unwrap();
    assert_eq!(blocks[0]["start"], 999999999);
}
