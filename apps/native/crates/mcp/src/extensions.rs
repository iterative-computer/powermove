//! Worker-only source operations. Staging does not activate an extension.
use crate::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedBuiltin {
    pub fork_id: String,
    pub dir: PathBuf,
    pub files: Vec<String>,
    pub forked_from: String,
}
pub trait ExtensionSources: Send + Sync {
    /// The host supplies its shipped resource root, never a user profile. The
    /// destination is the current run's isolated staging directory. Preserve
    /// pristine sources and retain incomplete work on failure (N4).
    fn fork_builtin(
        &self,
        id: &str,
        fork_id: Option<&str>,
        staging: &Path,
    ) -> Result<ForkedBuiltin>;
}
