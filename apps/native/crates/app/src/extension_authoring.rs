//! The app side of agent extension authoring (`pm_mcp::extensions`): where
//! extensions live, the shipped built-in sources, and the isolated check that
//! compiles and activates staged code before it is promoted. Runs on MCP
//! workers, never on the main thread.
use pm_builtins::{BUILTIN_PANELS, Builtins};
use pm_compat_runner::{CompatRunner, manifest};
use pm_mcp::extensions::ExtensionHost;
use pm_runtime_v8::{Evaluation, Ticket};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// What an agent reads before writing an extension (`api/` in its workspace).
const PACK: &[(&str, &str)] = &[
    (
        "EXTENSIONS.md",
        include_str!("../../../js/v4/api-pack/EXTENSIONS.md"),
    ),
    (
        "v4/builtin.ts",
        include_str!("../../../js/v4/src/builtin.ts"),
    ),
    ("v4/ui.ts", include_str!("../../../js/v4/src/ui.ts")),
    ("v4/hooks.ts", include_str!("../../../js/v4/src/hooks.ts")),
    (
        "v4/protocol.ts",
        include_str!("../../../js/v4/src/protocol.ts"),
    ),
    (
        "v4/host-api.ts",
        include_str!("../../../js/v4/src/host-api.ts"),
    ),
    (
        "v4/schemas.ts",
        include_str!("../../../js/v4/src/schemas.ts"),
    ),
    (
        "samples/hello-panel/manifest.json",
        include_str!("../../../js/v4/api-pack/samples/hello-panel/manifest.json"),
    ),
    (
        "samples/hello-panel/index.tsx",
        include_str!("../../../js/v4/api-pack/samples/hello-panel/index.tsx"),
    ),
    // The shipped app's pack for apiVersion 1-3, unchanged: those extensions
    // load here too, and it is the API for commands, keys, menus and effects.
    (
        "v3/EXTENSIONS.md",
        include_str!("../../../../desktop/docs/EXTENSIONS.md"),
    ),
    (
        "v3/api.ts",
        include_str!("../../../../desktop/src/renderer/src/kernel/api.ts"),
    ),
    (
        "v3/extensions.ts",
        include_str!("../../../../../packages/registry/src/manifest.ts"),
    ),
    (
        "v3/project.ts",
        include_str!("../../../../desktop/src/renderer/src/core/types/project.ts"),
    ),
    (
        "v3/commands.ts",
        include_str!("../../../../desktop/src/renderer/src/core/types/commands.ts"),
    ),
    (
        "v3/samples/gradient-tint/manifest.json",
        include_str!("../../../../desktop/docs/samples/gradient-tint/manifest.json"),
    ),
    (
        "v3/samples/gradient-tint/index.ts",
        include_str!("../../../../desktop/docs/samples/gradient-tint/index.ts"),
    ),
    (
        "v3/samples/gradient-tint/README.md",
        include_str!("../../../../desktop/docs/samples/gradient-tint/README.md"),
    ),
    (
        "v3/samples/media-browser/manifest.json",
        include_str!("../../../../desktop/docs/samples/media-browser/manifest.json"),
    ),
    (
        "v3/samples/media-browser/index.ts",
        include_str!("../../../../desktop/docs/samples/media-browser/index.ts"),
    ),
    (
        "v3/samples/media-browser/MediaBrowserPanel.svelte",
        include_str!("../../../../desktop/docs/samples/media-browser/MediaBrowserPanel.svelte"),
    ),
    (
        "v3/samples/media-browser/README.md",
        include_str!("../../../../desktop/docs/samples/media-browser/README.md"),
    ),
];
const ACTIVATION: Duration = Duration::from_secs(10);

pub struct AppExtensions {
    profile: PathBuf,
    roots: Vec<PathBuf>,
}
impl AppExtensions {
    pub fn new(profile: PathBuf) -> Self {
        Self {
            roots: crate::extensions::roots(&profile),
            profile,
        }
    }
}
impl ExtensionHost for AppExtensions {
    fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
    fn history(&self) -> PathBuf {
        self.profile.join("agent-change-history")
    }
    fn builtins(&self) -> Option<PathBuf> {
        pm_builtins::builtin_sources(&self.profile.join("builtin-extensions")).ok()
    }
    fn manifest(&self, raw: &Value) -> Result<Value, String> {
        manifest::parse(raw)
    }
    fn check(&self, dir: &Path, manifest: &Value) -> Result<(), String> {
        check(dir, manifest)
    }
    fn reload(&self, _: &[String]) {
        pm_compat_runner::host::rescan_all();
    }
    fn api_pack(&self) -> Vec<(String, String)> {
        PACK.iter()
            .map(|(n, t)| ((*n).into(), (*t).into()))
            .collect()
    }
}

fn settle(ticket: Ticket<Evaluation>) -> Result<(), String> {
    let deadline = Instant::now() + ACTIVATION;
    loop {
        if let Some(result) = ticket.try_take() {
            return result.map(drop).map_err(|e| e.to_string());
        }
        if Instant::now() > deadline {
            return Err(format!(
                "activation did not finish within {}s",
                ACTIVATION.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
/// The built-in a fork draws in place of, if any.
pub fn replaced_panel(manifest: &Value) -> Option<&'static str> {
    let replaces = manifest["replaces"].as_array()?;
    BUILTIN_PANELS
        .iter()
        .map(|p| p.id.as_str())
        .find(|panel| replaces.iter().any(|r| r == panel))
}
/// Compile with the app's compiler, then activate in throwaway runtimes on a
/// synthetic project: v1–3 in a trusted realm, v4 in its own panel isolate.
pub fn check(dir: &Path, manifest: &Value) -> Result<(), String> {
    let api = manifest["apiVersion"].as_u64().unwrap_or(1);
    let entry = manifest["entry"].as_str().unwrap_or("index.ts");
    let runner = CompatRunner::new(&pm_compat_runner::initial()).map_err(|e| e.to_string())?;
    let graph = runner.compile(dir, entry, api).map_err(|e| match e {
        pm_runtime_v8::Error::Host(message) => message,
        other => other.to_string(),
    })?;
    if api < 4 {
        let installed = runner
            .install(manifest, &graph.to_json())
            .map_err(|e| e.to_string())?;
        return match installed["ok"].as_bool() {
            Some(true) => Ok(()),
            _ => Err(installed["error"]["message"]
                .as_str()
                .unwrap_or("activation failed")
                .to_owned()),
        };
    }
    drop(runner);
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
    let result = (|| {
        let document = pm_model::json::parse(&pm_compat_runner::initial()["document"].to_string())
            .map_err(|_| "synthetic project".to_owned())?;
        actor
            .handle()
            .open("extension-check", pm_kernel::OpenSource::Untitled(document))
            .wait()
            .ok_or("kernel stopped")?
            .map_err(|e| e.to_string())?;
        let panels =
            Builtins::new(actor.handle(), "extension-check", || {}).map_err(|e| e.to_string())?;
        let id = manifest["id"].as_str().unwrap_or_default();
        let ticket = match replaced_panel(manifest) {
            Some(panel) => panels.replace(panel, id, &graph.to_json()),
            None => panels.mount_extension(id, &graph.to_json(), None),
        };
        let settled = ticket.map_err(|e| e.to_string()).and_then(settle);
        let errors = panels.take_errors();
        panels.close();
        settled?;
        match errors.first() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    })();
    actor.shutdown();
    result
}

#[cfg(test)]
#[path = "extension_authoring_tests.rs"]
mod tests;
