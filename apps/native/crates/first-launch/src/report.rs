//! Migration Report data (N003). Private to this Mac: never telemetry or logs.
//! Decrypted values, secret counts on the gate, and item names stay out of it.
use crate::{
    ConflictKind, State, disk,
    secrets::{self, SecretStep},
    surfaces::migration::{Report, ReportKind, ReportRow},
};
use anyhow::Result;
use pm_kernel::{LibraryStore, library::StoreKey};
use pm_native_migrate::migration::Status;
use std::path::Path;

fn project_name(store: &LibraryStore, id: &str) -> String {
    store
        .load(id)
        .ok()
        .flatten()
        .and_then(|p| {
            p.project
                .get("name")
                .and_then(pm_model::json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Untitled".into())
}
/// `env/<extension>.env/<KEY>` shows as its extension. Keys stay private.
fn sign_in_name(id: &str) -> String {
    if id == "cloud/session" || id.starts_with("cloud/human/") {
        "Powermove Cloud".into()
    } else if id == "agent-provider" {
        "Agent provider".into()
    } else {
        id.strip_prefix("env/")
            .and_then(|rest| rest.split('/').next())
            .map(|file| file.trim_end_matches(".env").to_owned())
            .unwrap_or_else(|| "Extension".into())
    }
}
pub fn load(native: &Path) -> Result<Report> {
    let state: State = disk::json(&native.join("migration.json"))?;
    let library = LibraryStore::open(native);
    let checkpoint = native.join("first-launch-checkpoint");
    let source = LibraryStore::open(&checkpoint.join("profile"));
    let mut rows = Vec::new();
    let mut push = |id: String, display_name: String, kind| {
        let row = ReportRow {
            id,
            display_name,
            kind,
        };
        if !rows.contains(&row) {
            rows.push(row);
        }
    };
    for id in &state.history_refusals {
        let copy = state.receipts.get(id).map_or(id, |r| &r.native);
        push(
            copy.clone(),
            project_name(&library, copy),
            ReportKind::History,
        );
    }
    for id in &state.missing_projects {
        push(
            id.clone(),
            project_name(&source, id),
            ReportKind::MissingProjects,
        );
    }
    let passes: serde_json::Value = disk::json(&native.join("Media Store/migration.json"))?;
    if let Some(pass) = passes
        .get("passes")
        .and_then(serde_json::Value::as_object)
        .and_then(|passes| {
            passes
                .values()
                .find(|p| p["snapshot"].as_str() == state.latest_pass.as_deref())
        })
    {
        if let Some(owners) = pass.get("blockedOwners").and_then(|v| v.as_array()) {
            for owner in owners.iter().filter_map(serde_json::Value::as_str) {
                let name = state
                    .receipts
                    .get(owner)
                    .map(|r| project_name(&library, &r.native))
                    .unwrap_or_else(|| project_name(&source, owner));
                push(owner.to_owned(), name, ReportKind::Media);
            }
        }
        if let Some(closure) = pass.get("closure").and_then(|v| v.as_array()) {
            for entry in closure {
                let status = serde_json::from_value::<Status>(entry["status"].clone());
                if matches!(status, Ok(Status::ProxyOnly)) {
                    let key = entry["reference"]["key"].as_str().unwrap_or("Media");
                    let name = entry["reference"]["sourcePath"]
                        .as_str()
                        .and_then(|p| Path::new(p).file_name())
                        .map_or(key.into(), |n| n.to_string_lossy());
                    push(key.to_owned(), name.into_owned(), ReportKind::Proxies);
                }
            }
        }
    }
    if checkpoint.join("profile/store").is_dir() {
        for key in source.keys()? {
            if let StoreKey::Quarantined(name) | StoreKey::Unregistered(name) = key {
                push(name.clone(), name, ReportKind::Quarantined);
            }
        }
    }
    for conflict in &state.conflicts {
        let kind = match conflict.kind {
            ConflictKind::Project => "project",
            ConflictKind::Setting => "setting",
            ConflictKind::LocalStorage => "storage",
            ConflictKind::File => "file",
        };
        push(
            format!("{kind}:{}", conflict.id),
            conflict.id.clone(),
            ReportKind::Conflicts,
        );
    }
    if state.secrets == SecretStep::Skipped {
        let payloads = secrets::inventory(&checkpoint.join("profile"))?;
        let mut names: Vec<_> = payloads.iter().map(|p| sign_in_name(&p.id)).collect();
        names.sort();
        names.dedup();
        for name in names {
            push(format!("sign-in:{name}"), name, ReportKind::SignIns);
        }
    }
    Ok(Report { rows })
}
