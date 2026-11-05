//! Actual file-backed MCP source adapter, using only disposable synthetic input.
use pm_acceptance::fixture::{Fixture, tool_error};
use pm_mcp_backends::extensions::BuiltinSources;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};
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
        f.host.lock().unwrap().extension_sources =
            Some(Arc::new(checked(BuiltinSources::new(resources))?));
        let before = f.document()?;
        let revision = f.revision();
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
        assert!(
            f.host
                .lock()
                .unwrap()
                .kernel()
                .state()
                .registries
                .values()
                .all(|r| r.is_empty())
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
        let archive = checked(fs::read_dir(f.scratch.0.join("profile/mcp-archives")))?
            .next()
            .ok_or("missing archived run")?
            .map_err(|e| e.to_string())?
            .path();
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
