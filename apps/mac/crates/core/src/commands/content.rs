//! Layer content (`d`): text, shape geometry, media fit and trim, audio
//! levels, extension parameters.
//!
//! Ports `setContent` (with `setAudioContent`, `setExtensionContent`,
//! `normalizeAudioLevelPatch`, `resizeVideoForSpeed`) of
//! `legacy/core/editing.ts`, used by the inspector's content section and the
//! mixer.

use pm_model::Map;

use super::Cx;
use crate::error::CommandError;

/// Content commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ContentCommand {
    /// `set_content` (`setContent`): merges `patch` into `d`. A field whose
    /// channel is animated is keyed at `time` instead (as `set_property`
    /// `c.<key>`). Audio layers accept only `asset`, `trim`, `gain`,
    /// `fadeIn`, `fadeOut` (clamped); extension layers only `w`, `h`,
    /// `data`, `parameters`; `audioGain` on video/pre-comp is clamped to 0–4
    /// (`null` clears it); a constant video `speed` change rescales the
    /// layer's duration. Keys `__proto__`, `prototype`, `constructor` are
    /// refused. Label "Edit Content".
    Set {
        layer_id: String,
        patch: Map,
        time: f64,
    },
}

impl ContentCommand {
    pub fn label(&self) -> String {
        "Edit Content".to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented("set_content"))
    }
}
