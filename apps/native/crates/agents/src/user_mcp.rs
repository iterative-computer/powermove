//! User MCP registrations. Paths and environment are supplied by the host;
//! tests never discover a home or start a registered server.
use crate::{Access, Provider};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Default)]
pub struct Loaded {
    pub servers: serde_json::Map<String, Value>,
    pub warnings: Vec<String>,
}

/// Uses v1's global locations, before the runtime's isolated home replaces them.
/// An absent explicit HOME means no discovery (fixture environments default shut).
pub fn load(provider: Provider, access: Access, vars: &BTreeMap<String, String>) -> Loaded {
    let mut out = Loaded::default();
    // v1 allowed these integrations for all three Claude/Codex access levels.
    // Launch checks fresh Computer consent before loading; compatible runs
    // have no corresponding v1 integration and must not discover registrations.
    if !matches!(
        (provider, access),
        (
            Provider::Claude | Provider::Chatgpt,
            Access::Editor | Access::Project | Access::Computer
        )
    ) {
        return out;
    }
    let nonempty = |key: &str| {
        vars.get(key)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    let path = match provider {
        Provider::Claude => nonempty("CLAUDE_CONFIG_DIR")
            .or_else(|| nonempty("HOME"))
            .map(|p| p.join(".claude.json")),
        Provider::Chatgpt => nonempty("CODEX_HOME")
            .or_else(|| nonempty("HOME").map(|p| p.join(".codex")))
            .map(|p| p.join("config.toml")),
        Provider::Compatible => None,
    };
    let Some(path) = path else {
        return out;
    };
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(_) => {
            out.warnings.push(
                "Cannot read user MCP configuration. Continuing with Powermove tools.".into(),
            );
            return out;
        }
    };
    let config = match parse(provider, &source) {
        Some(c) => c,
        None => {
            out.warnings
                .push("Invalid user MCP configuration. Continuing with Powermove tools.".into());
            return out;
        }
    };
    let key = if provider == Provider::Claude {
        "mcpServers"
    } else {
        "mcp_servers"
    };
    let Some(servers) = config.get(key) else {
        return out;
    };
    let Some(servers) = servers.as_object() else {
        out.warnings
            .push("Invalid user MCP server list. Continuing with Powermove tools.".into());
        return out;
    };
    for (name, server) in servers {
        if name == "powermove"
            || server["enabled"] == false
            || config["disabledMcpServers"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == name))
        {
            continue;
        }
        let Some(mut server) = server.as_object().cloned() else {
            out.warnings.push(format!(
                "User MCP server {} has invalid launch settings; skipped.",
                safe_name(name)
            ));
            continue;
        };
        // Invalid optional registrations must never invalidate the whole launch.
        let text = |key: &str| {
            server
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        };
        fn supported(v: &Value) -> bool {
            match v {
                Value::Null => false,
                Value::Array(a) => a.iter().all(supported),
                Value::Object(o) => o.values().all(supported),
                _ => true,
            }
        }
        let valid = !name.trim().is_empty()
            && !name.chars().any(char::is_control)
            && server.values().all(supported)
            && (provider != Provider::Claude
                || server.get("type").is_none_or(|v| {
                    if server.contains_key("command") {
                        v == "stdio"
                    } else {
                        v == "http" || v == "sse"
                    }
                }))
            && (server.contains_key("command") != server.contains_key("url"))
            && (text("command") || text("url"))
            && ["cwd", "bearer_token_env_var", "type"]
                .iter()
                .all(|k| server.get(*k).is_none_or(Value::is_string))
            && ["enabled_tools", "disabled_tools", "env_vars", "scopes"]
                .iter()
                .all(|k| {
                    server.get(*k).is_none_or(|v| {
                        v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
                    })
                })
            && [
                "startup_timeout_sec",
                "startup_timeout_ms",
                "tool_timeout_sec",
            ]
            .iter()
            .all(|k| {
                server
                    .get(*k)
                    .is_none_or(|v| v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.))
            })
            && server
                .get("args")
                .is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
            && ["env", "headers", "http_headers", "env_http_headers"]
                .iter()
                .all(|k| {
                    server.get(*k).is_none_or(|v| {
                        v.as_object()
                            .is_some_and(|o| o.values().all(Value::is_string))
                    })
                })
            && server.get("url").is_none_or(|v| {
                v.as_str()
                    .and_then(|s| url::Url::parse(s).ok())
                    .is_some_and(|u| matches!(u.scheme(), "http" | "https"))
            });
        if !valid {
            out.warnings.push(format!(
                "User MCP server {} has invalid launch settings; skipped.",
                safe_name(name)
            ));
            continue;
        }
        server.remove("enabled");
        if provider == Provider::Claude {
            let kind = if server.contains_key("command") {
                "stdio"
            } else {
                "http"
            };
            server.entry("type").or_insert_with(|| json!(kind));
        }
        if provider == Provider::Chatgpt {
            server.insert("required".into(), json!(false));
            let timeout = server
                .get("startup_timeout_sec")
                .and_then(Value::as_f64)
                .or_else(|| {
                    server
                        .get("startup_timeout_ms")
                        .and_then(Value::as_f64)
                        .map(|n| n / 1000.)
                })
                .unwrap_or(10.);
            server.remove("startup_timeout_ms");
            server.insert("startup_timeout_sec".into(), json!(timeout));
        }
        out.servers.insert(name.clone(), Value::Object(server));
    }
    out
}
fn safe_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || "-_.".contains(*c))
        .take(80)
        .collect()
}
fn parse(provider: Provider, source: &str) -> Option<Value> {
    if provider == Provider::Claude {
        return serde_json::from_str(source).ok().filter(Value::is_object);
    }
    let document: toml_edit::DocumentMut = source.parse().ok()?;
    fn item(i: &toml_edit::Item) -> Value {
        if let Some(t) = i.as_table_like() {
            return Value::Object(t.iter().map(|(k, v)| (k.into(), item(v))).collect());
        }
        if let Some(v) = i.as_value() {
            return value(v);
        }
        Value::Null
    }
    fn value(v: &toml_edit::Value) -> Value {
        match v {
            toml_edit::Value::String(v) => json!(v.value()),
            toml_edit::Value::Integer(v) => json!(v.value()),
            toml_edit::Value::Float(v) => json!(v.value()),
            toml_edit::Value::Boolean(v) => json!(v.value()),
            toml_edit::Value::Array(a) => Value::Array(a.iter().map(value).collect()),
            toml_edit::Value::InlineTable(t) => {
                Value::Object(t.iter().map(|(k, v)| (k.into(), value(v))).collect())
            }
            _ => Value::Null,
        }
    }
    Some(item(document.as_item()))
}
impl Loaded {
    pub fn codex_argv(&self) -> Vec<String> {
        fn inline(v: &Value) -> String {
            match v {
                Value::Object(o) => format!(
                    "{{{}}}",
                    o.iter()
                        .map(|(k, v)| format!("{}={}", json!(k), inline(v)))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Value::Array(a) => {
                    format!("[{}]", a.iter().map(inline).collect::<Vec<_>>().join(","))
                }
                _ => v.to_string(),
            }
        }
        self.servers
            .iter()
            .flat_map(|(name, s)| {
                [
                    "--config".into(),
                    format!("mcp_servers.{}={}", json!(name), inline(s)),
                ]
            })
            .collect()
    }
}
fn startup_message(name: &str) -> String {
    format!(
        "User MCP server {} could not start. The run can continue with the other tools.",
        safe_name(name)
    )
}

/// Exec does not publish every MCP startup transition as JSON. Inspect stderr
/// only at the drain boundary; return safe notices, never the raw diagnostic.
#[derive(Default)]
pub(crate) struct StderrFailures {
    line: Vec<u8>,
    overflow: bool,
    seen: std::collections::BTreeSet<String>,
}
impl StderrFailures {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = vec![];
        for &byte in bytes {
            if byte == b'\n' {
                if let Some(message) = self.flush() {
                    out.push(message);
                }
            } else if !self.overflow {
                if self.line.len() < 16 * 1024 {
                    self.line.push(byte);
                } else {
                    self.line.clear();
                    self.overflow = true;
                }
            }
        }
        out
    }
    pub(crate) fn finish(&mut self) -> Vec<String> {
        self.flush().into_iter().collect()
    }
    fn flush(&mut self) -> Option<String> {
        let message = if !self.overflow && self.seen.len() < 64 {
            Self::message(&String::from_utf8_lossy(&self.line))
        } else {
            None
        };
        self.line.clear();
        self.overflow = false;
        message.filter(|message| self.seen.insert(message.clone()))
    }
    fn message(line: &str) -> Option<String> {
        for (prefix, suffix) in [
            ("MCP client for `", "` failed to start"),
            ("MCP client for '", "' failed to start"),
            ("mcp: ", " failed:"),
        ] {
            if let Some((_, rest)) = line.split_once(prefix)
                && let Some((name, _)) = rest.split_once(suffix)
            {
                return (name != "powermove").then(|| startup_message(name));
            }
        }
        (line.contains("MCP startup failed") || line.contains("MCP startup: incomplete"))
            .then(|| "One or more MCP servers could not start. Check their settings.".into())
    }
}

/// Provider diagnostics are untrusted and can contain headers or environment
/// values. Report identity/status only; never forward the raw startup error.
pub fn startup_warnings(message: &Value) -> Vec<crate::Event> {
    let mut events = vec![];
    let mut failed = |name: &str| {
        if name != "powermove" {
            events.push(crate::Event::Warning(startup_message(name)));
        }
    };
    if message["type"] == "system" && message["subtype"] == "init" {
        for s in message["mcp_servers"].as_array().into_iter().flatten() {
            if matches!(s["status"].as_str(), Some("failed" | "needs-auth")) {
                failed(s["name"].as_str().unwrap_or("unknown"));
            }
        }
    }
    if matches!(
        message["method"].as_str(),
        Some("mcpServer/startupStatus/updated" | "codex/event/mcp_startup_update")
    ) {
        let p = message["params"].get("msg").unwrap_or(&message["params"]);
        if matches!(p["status"].as_str(), Some("failed" | "error")) {
            failed(
                p["server"]
                    .as_str()
                    .or(p["name"].as_str())
                    .unwrap_or("unknown"),
            );
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "user-mcp-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn exec_stderr_failure_notices_handle_chunks_without_forwarding_secrets() {
        let mut reader = StderrFailures::default();
        assert!(reader.push(b"MCP client for `st").is_empty());
        let messages = reader.push(b"dio` failed to start: sensitive-fixture\nMCP client for `stdio` failed to start: another-sensitive-fixture\nordinary sensitive stderr\n");
        assert_eq!(messages, vec![startup_message("stdio")]);
        assert!(!messages[0].contains("sensitive"));
        assert!(
            reader
                .push(b"MCP client for `powermove` failed to start: private\n")
                .is_empty()
        );
        let generic =
            reader.push(b"ERROR rmcp::transport::worker: MCP startup failed: sensitive-fixture\n");
        assert_eq!(generic.len(), 1);
        assert!(!generic[0].contains("sensitive"));
        assert!(
            reader
                .push(b"mcp: remote failed: sensitive-fixture")
                .is_empty()
        );
        assert_eq!(reader.finish(), vec![startup_message("remote")]);
        assert!(reader.push(&vec![b'x'; 20 * 1024]).is_empty());
        assert!(reader.finish().is_empty());
    }

    #[test]
    fn claude_disabled_servers_and_all_access_levels_match_v1() {
        let root = temp();
        let vars = BTreeMap::from([("CLAUDE_CONFIG_DIR".into(), root.display().to_string())]);
        std::fs::write(root.join(".claude.json"),json!({"mcpServers":{
            "stdio":{"type":"stdio","command":"fixture","args":["x"],"env":{"MODE":"test"}},
            "remote":{"type":"http","url":"https://example.test/mcp","headers":{"X-Fixture":"fixture"}},
            "disabled":{"command":"fixture"},"powermove":{"command":"wrong"}},
            "disabledMcpServers":["disabled"],"projects":{"private":{"mcpServers":{"ignored":{"command":"fixture"}}}}}).to_string()).unwrap();
        for access in [Access::Editor, Access::Project, Access::Computer] {
            let loaded = load(Provider::Claude, access, &vars);
            assert_eq!(loaded.servers.len(), 2);
            assert_eq!(loaded.servers["remote"]["headers"]["X-Fixture"], "fixture");
        }
        assert!(
            load(Provider::Compatible, Access::Computer, &vars)
                .servers
                .is_empty()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn broken_config_and_registrations_are_nonfatal_and_diagnostics_are_safe() {
        let root = temp();
        let vars = BTreeMap::from([("CLAUDE_CONFIG_DIR".into(), root.display().to_string())]);
        assert!(
            load(Provider::Claude, Access::Editor, &vars)
                .warnings
                .is_empty()
        );
        std::fs::write(root.join(".claude.json"), "{sensitive-fixture-invalid").unwrap();
        let loaded = load(Provider::Claude, Access::Editor, &vars);
        assert!(loaded.servers.is_empty());
        assert_eq!(loaded.warnings.len(), 1);
        assert!(!loaded.warnings[0].contains("sensitive-fixture"));
        std::fs::write(
            root.join(".claude.json"),
            json!({"mcpServers":{"bad":{"command":7},"malformed":"sensitive-fixture","good":{"command":"fixture"}}}).to_string(),
        )
        .unwrap();
        let loaded = load(Provider::Claude, Access::Editor, &vars);
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.warnings.len(), 2);
        assert!(
            loaded
                .warnings
                .iter()
                .all(|s| !s.contains("sensitive-fixture"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn startup_failure_is_a_safe_warning_and_provider_can_finish() {
        let authority =
            crate::approvals::Authority::new(Access::Editor, crate::ApprovalMode::Auto, false)
                .unwrap();
        let mut client = crate::claude::Client::new("fixture".into(), authority, 0);
        let effects=client.receive(json!({"type":"system","subtype":"init","mcp_servers":[{"name":"fixture","status":"failed","error":"sensitive-fixture"}]}));
        assert!(
            matches!(&effects.events[0],crate::Event::Warning(s) if s.contains("fixture") && !s.contains("sensitive-fixture"))
        );
        assert!(!effects.close_stdin);
        let effects =
            client.receive(json!({"type":"result","subtype":"success","result":"continued"}));
        assert!(effects.events.iter().any(|e| matches!(
            e,
            crate::Event::Completed {
                result: crate::RunResult::Success { .. },
                ..
            }
        )));
        let warnings = startup_warnings(
            &json!({"method":"mcpServer/startupStatus/updated","params":{"server":"fixture","status":"failed","error":"sensitive-fixture"}}),
        );
        assert_eq!(warnings.len(), 1);
    }
    #[test]
    fn fixture_loads_only_enabled_global_registrations_and_reloads() {
        let root = std::env::temp_dir().join(format!(
            "user-mcp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let vars = BTreeMap::from([("CODEX_HOME".into(), root.display().to_string())]);
        let file = root.join("config.toml");
        std::fs::write(&file, "model='unrelated'\n[mcp_servers.'with.dots']\ncommand='fixture-server'\nargs=['--fixture']\nrequired=true\n[mcp_servers.remote]\nurl='https://example.test/mcp'\nbearer_token_env_var='FIXTURE_TOKEN'\nstartup_timeout_sec=90\ntool_timeout_sec=240\n[mcp_servers.off]\ncommand='off'\nenabled=false\n[mcp_servers.powermove]\ncommand='wrong'\n").unwrap();
        let loaded = load(Provider::Chatgpt, Access::Editor, &vars);
        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(loaded.servers["with.dots"]["required"], false);
        assert_eq!(
            loaded.servers["remote"]["bearer_token_env_var"],
            "FIXTURE_TOKEN"
        );
        assert_eq!(loaded.servers["remote"]["startup_timeout_sec"], json!(90.));
        assert_eq!(loaded.servers["remote"]["tool_timeout_sec"], json!(240));
        let argv = loaded.codex_argv();
        let doc: toml_edit::DocumentMut = argv
            .chunks(2)
            .map(|p| p[1].as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .parse()
            .unwrap();
        assert_eq!(
            doc["mcp_servers"]["with.dots"]["command"].as_str(),
            Some("fixture-server")
        );
        std::fs::write(&file, "model='changed'").unwrap();
        assert!(
            load(Provider::Chatgpt, Access::Project, &vars)
                .servers
                .is_empty()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
