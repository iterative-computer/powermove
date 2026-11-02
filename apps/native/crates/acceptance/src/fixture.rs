//! Disposable synthetic profiles and read-only corpus inputs. Source bytes are
//! copied before opening: locks, saves and recovery never touch the corpus.
use pm_mcp::{Automation, McpSession};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(pm_kernel::new_id("pm-acceptance-synthetic"));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
    pub fn copy(&self, source: &Path, ordinal: usize) -> std::io::Result<PathBuf> {
        if !std::fs::symlink_metadata(source)?.file_type().is_file() {
            return Err(std::io::Error::other(
                "corpus input must be an ordinary file",
            ));
        }
        let destination = self.0.join(format!("corpus-{ordinal:04}.pmv"));
        // create_new prevents an accidental overwrite even inside our scratch.
        let mut input = std::fs::File::open(source)?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?;
        std::io::copy(&mut input, &mut out)?;
        Ok(destination)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub struct Fixture {
    pub session: McpSession,
    pub host: Arc<Mutex<Automation>>,
    pub id: String,
    // Drop session and host before releasing the scratch directory.
    pub scratch: Scratch,
}
impl Fixture {
    pub fn new() -> Result<Self, String> {
        Self::configured(|_| Ok(()))
    }
    pub fn configured(configure: impl FnOnce(&Path) -> Result<(), String>) -> Result<Self, String> {
        let scratch = Scratch::new().map_err(|e| e.to_string())?;
        configure(&scratch.0.join("profile"))?;
        let host = Arc::new(Mutex::new(
            Automation::new(scratch.0.join("profile"), scratch.0.join("exports"))
                .map_err(|e| e.to_string())?,
        ));
        let mut session = McpSession::new(host.clone());
        let created = tool(
            &mut session,
            "create_project",
            json!({"name":"Synthetic W1","width":320,"height":180,"fps":30,"duration":12}),
        )?;
        let id = created["projectId"]
            .as_str()
            .ok_or("missing project ID")?
            .to_owned();
        tool(
            &mut session,
            "start_session",
            json!({"prompt":"Synthetic acceptance"}),
        )?;
        Ok(Self {
            host,
            session,
            id,
            scratch,
        })
    }
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value, String> {
        tool(&mut self.session, name, args)
    }
    pub fn edit(&mut self, commands: Value) -> Result<Value, String> {
        self.call("apply_commands", json!({"commands":commands}))
    }
    pub fn add(&mut self, id: &str) -> Result<Value, String> {
        self.edit(json!([{"type":"add_layer","id":id,"layerType":"solid","name":"Synthetic solid","duration":6}]))
    }
    pub fn document(&self) -> Result<Value, String> {
        self.host
            .lock()
            .unwrap()
            .read_document(&self.id)
            .map_err(|e| e.to_string())
    }
    pub fn verify_saved(&mut self) -> Result<(), String> {
        let before = without_revision(self.document()?);
        let result = self.call("save_project", json!({}))?;
        let opened = pm_kernel::project_io::open_project(
            Path::new(result["path"].as_str().ok_or("missing save path")?),
            Default::default(),
        )
        .map_err(|e| e.to_string())?;
        let store = pm_kernel::DocumentStore::open(opened);
        let realized = pm_model::canonical::realize_project(
            store
                .snapshot()
                .document
                .to_value()
                .ok_or("missing saved document")?,
        );
        let saved: Value = serde_json::from_str(&pm_model::json::to_canonical_string(&realized))
            .map_err(|e| e.to_string())?;
        assert_eq!(without_revision(saved), before);
        Ok(())
    }
    pub fn revision(&self) -> u64 {
        self.host.lock().unwrap().revision(&self.id).unwrap()
    }
}
pub fn tool(session: &mut McpSession, name: &str, args: Value) -> Result<Value, String> {
    let response = session.call(name, args);
    let text = response["content"][0]["text"]
        .as_str()
        .ok_or("MCP missing text content")?;
    if response["isError"] != false {
        return Err(format!("{name}: {text}"));
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}
pub fn tool_error(session: &mut McpSession, name: &str, args: Value) -> Result<Value, String> {
    let response = session.call(name, args);
    if response["isError"] != true {
        return Err(format!("{name} unexpectedly succeeded"));
    }
    serde_json::from_str(
        response["content"][0]["text"]
            .as_str()
            .ok_or("missing error content")?,
    )
    .map_err(|e| e.to_string())
}
pub fn layer<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .expect("fixture layer")
}
pub fn without_revision(mut doc: Value) -> Value {
    doc.as_object_mut().unwrap().remove("revision");
    doc
}

/// No media decoder is exercised by this fixture: it describes exact authored
/// source timing and animation; decoded frame/audio oracles remain macOS gates.
pub fn clip_project() -> Value {
    json!({"id":"P-synthetic","name":"Synthetic source handles","compId":"C-main","compName":"Main","w":320,"h":180,"fps":30,"dur":12,"bg":"#000000","work":[0,12],"markers":[],"comps":{},"assets":{"A-video":{"id":"A-video","kind":"video","name":"synthetic-frames.mov","dur":12}},"layers":[{"id":"L-clip","type":"video","name":"Synthetic numbered frames","from":2,"dur":3,"on":true,"d":{"asset":"A-video","fit":"cover","trim":1,"speed":2,"w":320,"h":180},"p":{"opacity":{"v":100,"kf":[{"i":"K-a","t":0,"v":25},{"i":"K-b","t":2,"v":75}],"expr":null}}}]})
}

/// These are public engine regression fixtures, never profile/project samples.
/// Include bytes at compile time so the installed runner needs no source tree.
pub const CORPUS: &[(&str, &[u8])] = &[
    (
        "json",
        include_bytes!(
            "../../../../../engine/crates/pm-model/tests/fixtures/containers/json-plain.pmv"
        ),
    ),
    (
        "pmv3",
        include_bytes!("../../../../../engine/crates/pm-model/tests/fixtures/containers/pmv3.pmv"),
    ),
    (
        "pmv4",
        include_bytes!("../../../../../engine/crates/pm-model/tests/fixtures/containers/pmv4.pmv"),
    ),
];
pub fn corpus_fixture(name: &str) -> Result<(), String> {
    let (_, bytes) = CORPUS
        .iter()
        .find(|(id, _)| *id == name)
        .ok_or("unknown public corpus fixture")?;
    let source = Scratch::new().map_err(|e| e.to_string())?;
    let path = source.0.join("public-synthetic.pmv");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    crate::scenarios::corpus_copy(&path, 0)?;
    assert_eq!(std::fs::read(path).map_err(|e| e.to_string())?, *bytes);
    Ok(())
}
