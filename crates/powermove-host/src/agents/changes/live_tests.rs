//! A real Codex run that writes a mod, through the host with a fake renderer
//! window. Spends a little ChatGPT quota, so it only runs on request:
//!
//!   cargo test -p powermove-host --lib agents::changes::live_tests -- --ignored --nocapture
//!
//! Requires `scripts/windows/stage-api-pack.ps1`, `stage-compiler.ps1` and a signed-in Codex.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde_json::{Value, json};

use super::super::service::{Agents, install};
use super::super::tools::tests::{decode_event, fake_window};
use crate::host::{Config, Host};
use crate::router::{Call, Client, Router};

async fn invoke(host: &Arc<Host>, client: &Arc<Client>, channel: &str, arg: Value) -> anyhow::Result<Value> {
    let handler = host.router().get(channel).unwrap();
    let payload = handler(Call { host: host.clone(), client: client.clone(), args: vec![arg], blobs: Arc::new(Vec::<Bytes>::new()) }).await?;
    Ok(payload.json)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spends ChatGPT quota"]
async fn chatgpt_writes_a_mod_that_goes_live() {
    let directory = tempfile::tempdir().unwrap();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = Config {
        user_data: directory.path().join("data"),
        renderer_dir: directory.path().into(),
        builtin_extensions: directory.path().join("builtin"),
        resources_dir: repo.join("target/powermove-resources"),
        ffmpeg: directory.path().into(),
        temp_dir: directory.path().into(),
        version: "test".into(),
        port: 0,
    };
    std::fs::create_dir_all(&config.builtin_extensions).unwrap();
    std::fs::create_dir_all(&config.user_data).unwrap();
    let store = crate::store::Store::load(config.user_data.join("store")).await.unwrap();
    let mut router = Router::default();
    crate::extensions::register(&mut router, &config, store).await.unwrap();
    install(&mut router, &Agents::new(&config, None).await);
    let extensions = crate::extensions::for_profile(&config.user_data).unwrap();
    let mut changed = extensions.subscribe();
    let host = Host::new(config.clone(), "token".into());
    host.install_router(router);

    // The fake renderer answers every editor tool call with a tiny project.
    let (window, mut frames) = fake_window(1);
    let renderer = {
        let (host, window) = (host.clone(), window.clone());
        tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                let (channel, event) = decode_event(frame);
                if channel != "agent-tool:request" {
                    if channel == "codex:event" && event["kind"] == "progress" {
                        println!("progress: {}", event["text"]);
                    }
                    continue;
                }
                println!("tool: {}", event["tool"]);
                let text = if event["tool"] == "__finish_run" { "{}".to_owned() } else { json!({ "revision": 1, "name": "Live test", "layers": [] }).to_string() };
                let response = json!({ "runId": event["runId"], "callId": event["callId"], "ok": true, "changed": false, "content": [{ "type": "text", "text": text }] });
                invoke(&host, &window, "agent-tool:response", response).await.unwrap();
            }
        })
    };
    let request = json!({
        "id": format!("live-{}", uuid::Uuid::new_v4()),
        "provider": "chatgpt",
        "mode": "autonomous",
        "prompt": "Write a minimal Powermove extension (mod) with id `tint`, name \"Tint\": one effect that mixes the image toward a color by an amount parameter. Follow powermove-api/EXTENSIONS.md. Do not edit the project. Report it in extensions with action created.",
        "images": [],
        "model": "gpt-6-luna",
        "reasoningEffort": "low",
        "access": "project",
        "projectId": "live-test",
        "projectName": "Live test",
        "projectJSON": "{\"revision\":1}",
        "attachments": [],
        "consentToken": null,
        "approval": "auto"
    });
    let result = invoke(&host, &window, "codex:run", request).await.unwrap();
    renderer.abort();
    window.close();
    println!("result: {result}");
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["extensions"][0]["id"], "tint", "{result}");
    assert_eq!(result["extensions"][0]["action"], "created", "{result}");
    let change_set = result["extensionChangeSetId"].as_str().expect("an undo record").to_owned();

    let event = loop {
        let event = tokio::time::timeout(Duration::from_secs(30), changed.recv()).await.expect("ext:changed in time").unwrap();
        if event["reason"] == "reload" && event["ids"] == json!(["tint"]) {
            break event;
        }
    };
    println!("ext:changed {event}");
    let record = extensions.list().await.unwrap().into_iter().find(|record| record.id == "tint").expect("the mod is registered");
    println!("record: {}", serde_json::to_string(&record).unwrap());
    assert!(record.bundle_hash.is_some(), "the mod compiled");
    assert!(config.user_data.join("extensions").join("tint").join("manifest.json").is_file());

    // Undo puts the extensions folder back as it was.
    let (window, _frames) = fake_window(2);
    let restored = invoke(&host, &window, "codex:restore-change-set", json!({ "projectId": "live-test", "changeSetId": change_set })).await.unwrap();
    assert_eq!(restored["changeSetId"], change_set);
    assert!(!config.user_data.join("extensions").join("tint").exists());
}
