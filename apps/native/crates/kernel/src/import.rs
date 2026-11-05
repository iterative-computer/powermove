//! Worker preparation and single-transaction admission for Finder, folder, cloud
//! and generated imports. Callers materialize cloud/Figma output before capture;
//! no network, UI, source hydration or source deletion occurs in the kernel.
use crate::{
    Command, MediaCommand,
    error::CommandError,
    project_io::{MediaEntry, MediaSource},
};
use pm_model::{Value, json::to_canonical_string};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ImportInput {
    pub asset_id: String,
    /// Complete authored metadata (including folder, cloud and future fields).
    pub asset: Value,
    /// A caller-owned immutable output, never a borrowed source-link capability.
    pub owned_path: PathBuf,
    pub mime: String,
    pub revision: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedImport {
    job: String,
    document: Option<u64>,
    identity: String,
    asset_identity: String,
    assets: Vec<ImportInput>,
    folders: Value,
}
impl PreparedImport {
    /// Worker-only. Every output is verified before the actor sees a command.
    pub fn capture(job: String, assets: Vec<ImportInput>) -> Result<Self, CommandError> {
        let invalid = |s: &str| CommandError::Invalid(s.into());
        if job.is_empty() || job.len() > 256 || assets.is_empty() {
            return Err(invalid("Invalid import job"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut hash = Sha256::new();
        for a in &assets {
            if a.asset_id.is_empty()
                || !ids.insert(&a.asset_id)
                || a.asset.as_object().is_none()
                || !a.owned_path.is_absolute()
                || a.revision.is_empty()
                || a.mime.is_empty()
                || a.sha256.len() != 64
                || !a
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid("Invalid imported asset"));
            }
            let io = |_| invalid("Imported output is unavailable");
            let before = fs::symlink_metadata(&a.owned_path).map_err(io)?;
            #[cfg(target_os = "macos")]
            {
                use std::os::macos::fs::MetadataExt;
                if before.st_flags() & 0x40000000 != 0 {
                    return Err(invalid("Imported output is not local"));
                }
            }
            if !before.is_file() || before.len() != a.size {
                return Err(invalid("Imported output changed"));
            }
            let mut f = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&a.owned_path)
                .map_err(io)?;
            let mut sha = Sha256::new();
            let mut count = 0u64;
            let mut buf = [0; 65536];
            loop {
                let n = f.read(&mut buf).map_err(io)?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > a.size {
                    return Err(invalid("Imported output changed"));
                }
                sha.update(&buf[..n]);
            }
            let stamp = |m: &fs::Metadata| {
                (
                    m.dev(),
                    m.ino(),
                    m.len(),
                    m.mtime(),
                    m.mtime_nsec(),
                    m.ctime(),
                    m.ctime_nsec(),
                )
            };
            if count != a.size
                || format!("{:x}", sha.finalize()) != a.sha256
                || stamp(&before) != stamp(&f.metadata().map_err(io)?)
                || stamp(&before) != stamp(&fs::symlink_metadata(&a.owned_path).map_err(io)?)
            {
                return Err(invalid("Imported output changed"));
            }
            // Length-delimited JSON binds metadata, order and byte identity, but
            // excludes scratch paths so replay can use another verified copy.
            let fields = serde_json::json!([
                a.asset_id,
                to_canonical_string(&a.asset),
                a.mime,
                a.revision,
                a.size,
                a.sha256
            ]);
            hash.update(
                serde_json::to_vec(&fields).map_err(|_| invalid("Invalid import metadata"))?,
            );
            hash.update([0]);
        }
        let identity = format!("{:x}", hash.finalize());
        Ok(Self {
            job,
            document: None,
            asset_identity: identity.clone(),
            identity,
            assets,
            folders: Value::Object(pm_model::Map::new()),
        })
    }
    /// Combine verified folder/Finder/cloud/generated results as one admission.
    /// Re-verifies retained outputs on the worker before handing them to the actor.
    pub fn combine(job: String, inputs: Vec<Self>) -> Result<Self, CommandError> {
        let mut assets = Vec::new();
        for input in inputs {
            if !input.folders.as_object().is_some_and(|m| m.is_empty()) {
                return Err(CommandError::Invalid(
                    "Supply the complete folder map on the combined import".into(),
                ));
            }
            assets.extend(input.assets);
        }
        Self::capture(job, assets)
    }
    /// Complete imported folder metadata. No folder name, layout or appearance
    /// is invented. Existing destination folders are compared on the actor.
    pub fn with_folders(mut self, folders: Value) -> Result<Self, CommandError> {
        let map = folders
            .as_object()
            .ok_or_else(|| CommandError::Invalid("Invalid imported folders".into()))?;
        if map.values().any(|v| v.as_object().is_none()) {
            return Err(CommandError::Invalid("Invalid imported folder".into()));
        }
        self.identity = self.asset_identity.clone();
        if !map.is_empty() {
            self.identity = format!(
                "{:x}",
                Sha256::digest(
                    format!("{}\0{}", self.asset_identity, to_canonical_string(&folders))
                        .as_bytes()
                )
            );
        }
        self.folders = folders;
        Ok(self)
    }
    pub(crate) fn folders(&self) -> &Value {
        &self.folders
    }
    pub fn for_document(mut self, document: u64) -> Self {
        self.document = Some(document);
        self
    }
    pub(crate) fn expected_document(&self) -> Option<u64> {
        self.document
    }
    pub fn command(self) -> Command {
        MediaCommand::AdmitImport {
            prepared: std::sync::Arc::new(self),
        }
        .into()
    }
    pub(crate) fn job(&self) -> &str {
        &self.job
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn assets(&self) -> &[ImportInput] {
        &self.assets
    }
    pub(crate) fn media(&self) -> Vec<MediaEntry> {
        self.assets
            .iter()
            .map(|a| MediaEntry {
                id: a.asset_id.as_str().into(),
                media_type: a.mime.as_str().into(),
                revision: a.revision.as_str().into(),
                source: MediaSource::File {
                    path: a.owned_path.clone(),
                    length: a.size,
                    sha256: Some(a.sha256.clone()),
                },
            })
            .collect()
    }
}

impl crate::KernelHandle {
    /// Worker-prepared import through the existing single-writer actor. Binding
    /// to the queued document session refuses a completion after close/reopen,
    /// even when the project id and revision happen to be the same.
    pub fn admit_import(
        &self,
        project: &str,
        document: u64,
        prepared: PreparedImport,
    ) -> crate::Pending<Result<crate::Ack, crate::KernelFailure>> {
        self.call(
            project,
            crate::Request::new(
                crate::Caller::native(),
                crate::Call::Transact {
                    label: Some("Import".into()),
                    body: crate::WriteBody::Commands(vec![
                        prepared.for_document(document).command(),
                    ]),
                    expected_revision: None,
                },
            ),
        )
    }
}
