//! Actual file-backed MCP source adapter, using only disposable synthetic input.
use pm_acceptance::fixture::{Fixture, tool_error};
use pm_mcp::extensions::ExtensionHost;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
/// The app's extension host without compilation: forking only copies files.
struct Sources {
    profile: PathBuf,
    builtins: PathBuf,
}
impl ExtensionHost for Sources {
    fn roots(&self) -> Vec<PathBuf> {
        vec![self.profile.join("extensions")]
    }
    fn history(&self) -> PathBuf {
        self.profile.join("agent-change-history")
    }
    fn builtins(&self) -> Option<PathBuf> {
        Some(self.builtins.clone())
    }
    fn manifest(&self, raw: &Value) -> Result<Value, String> {
        pm_compat_runner::manifest::parse(raw)
    }
    fn check(&self, _: &Path, _: &Value) -> Result<(), String> {
        Ok(())
    }
    fn reload(&self, _: &[String]) {}
    fn api_pack(&self) -> Vec<(String, String)> {
        vec![]
    }
}
type Check = Result<(), String>;
fn checked<T, E: std::fmt::Display>(value: Result<T, E>) -> Result<T, String> {
    value.map_err(|e| e.to_string())
}
pub fn fork() -> Check {
    for commit in [false, true] {
        let mut f = Fixture::new()?;
        f.call("finish_session", json!({"commit":true}))?;
        let resources = f.scratch.0.join("resources");
        let source = resources.join("synthetic");
        checked(fs::create_dir_all(source.join("src")))?;
        let manifest = json!({"id":"synthetic","name":"Synthetic","version":"1.2.3","apiVersion":3,"contributes":["effects"],"unknown":{"retained":"λ"}});
        checked(fs::write(
            source.join("manifest.json"),
            manifest.to_string(),
        ))?;
        let bytes = b"export const exact = 1.23456789012345;\n";
        checked(fs::write(source.join("index.ts"), bytes))?;
        checked(fs::write(
            source.join("src/helper.ts"),
            b"export const helper = 'authored';\n",
        ))?;
        f.host.lock().unwrap().extensions = Some(Arc::new(Sources {
            profile: f.scratch.0.join("profile"),
            builtins: resources,
        }));
        let before = f.document()?;
        let revision = f.revision();
        let registries = f
            .host
            .lock()
            .unwrap()
            .kernel()
            .unwrap()
            .state()
            .registries
            .clone();
        let started = f.call(
            "start_session",
            json!({"prompt":"Synthetic source copy","context":if commit {"app"} else {"project"}}),
        )?;
        let stage = PathBuf::from(
            started["stagingDirectory"]
                .as_str()
                .ok_or("missing staging directory")?,
        );
        let result = f.call("fork_builtin_extension", json!({"id":"synthetic"}))?;
        assert_eq!(result["forkId"], "synthetic-fork");
        assert_eq!(result["forkedFrom"], "synthetic@1.2.3");
        let destination = PathBuf::from(result["dir"].as_str().ok_or("missing fork directory")?);
        assert_eq!(destination, stage.join("synthetic-fork"));
        for file in ["index.ts", "src/helper.ts"] {
            let original = checked(fs::read(source.join(file)))?;
            assert_eq!(checked(fs::read(destination.join(file)))?, original);
            assert_eq!(
                checked(fs::read(destination.join(".forked-from").join(file)))?,
                original
            );
        }
        let pristine: Value = checked(serde_json::from_slice(&checked(fs::read(
            destination.join(".forked-from/manifest.json"),
        ))?))?;
        assert_eq!(pristine, manifest);
        let rewritten: Value = checked(serde_json::from_slice(&checked(fs::read(
            destination.join("manifest.json"),
        ))?))?;
        assert_eq!(rewritten["id"], "synthetic-fork");
        assert_eq!(rewritten["forkedFrom"], "synthetic@1.2.3");
        assert_eq!(rewritten["unknown"], manifest["unknown"]);
        assert_eq!(f.document()?, before);
        assert_eq!(f.revision(), revision);
        assert_eq!(
            f.host.lock().unwrap().kernel().unwrap().state().registries,
            registries
        );
        assert_eq!(
            tool_error(
                &mut f.session,
                "fork_builtin_extension",
                json!({"id":"synthetic"})
            )?["code"],
            "invalid_arguments"
        );
        assert_eq!(checked(fs::read(destination.join("index.ts")))?, bytes);
        f.call("finish_session", json!({"commit":commit}))?;
        let run_id = stage
            .parent()
            .and_then(|root| root.file_name())
            .ok_or("missing run identity")?;
        let archive = f.scratch.0.join("profile/mcp-archives").join(run_id);
        assert!(archive.is_dir(), "the exact fork run must be archived");
        let retained = archive.join(if commit {
            "extensions/synthetic-fork"
        } else {
            "workspace/extensions/synthetic-fork"
        });
        assert_eq!(checked(fs::read(retained.join("index.ts")))?, bytes);
        assert_eq!(
            checked(fs::read(retained.join(".forked-from/index.ts")))?,
            bytes
        );
        assert_eq!(checked(fs::read(source.join("index.ts")))?, bytes);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_mcp_fork_retains_source_base_and_archives() {
        super::fork().unwrap();
    }
}
