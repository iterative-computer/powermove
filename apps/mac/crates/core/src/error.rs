//! Error types of the core. `Display` is a sentence for the person who
//! clicked; where the old app threw a message, it is the same sentence.

use std::fmt;

use pm_model::container::ContainerError;
use pm_model::patch::PatchError;

/// Why a command was refused. Commands are validated against the scratch
/// document of their transaction, before anything is published.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandError {
    /// No layer has this id (in the open composition or any other).
    LayerNotFound(String),
    /// The layer has no such property (`Property “x” was not found on “name”`).
    PropertyNotFound {
        layer_id: String,
        name: String,
        channel: String,
    },
    /// The layer, or a group above it, is locked. The text is the old app's.
    Locked { layer_id: String, name: String },
    /// A value that must be finite (and within a range) was not.
    InvalidValue { what: &'static str, reason: String },
    /// The property is animated or driven by an expression; the first command
    /// lane (static values only) refuses instead of guessing a keyframe.
    Animated { layer_id: String, channel: String },
    /// The property carries a hand-edit marker and the origin may not
    /// overwrite it (`preserveHandEdits` in the old app).
    PreservedHandEdit { layer_id: String, channel: String },
    /// The stored document does not have the expected shape.
    Malformed(String),
    /// The request is invalid; the text is the old app's message where it had
    /// one (`Unknown blend mode: x`, `Parenting would create a cycle`, ...).
    Invalid(String),
    /// A keyframe id that no channel holds any more.
    KeyframeNotFound(String),
    /// No effect on the layer has this id (or type).
    EffectNotFound(String),
    /// No composition has this id.
    CompNotFound(String),
    /// The command is in the catalog but not ported yet (see
    /// docs/gpui/core-api.md for its status). Nothing changed.
    NotImplemented(&'static str),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::LayerNotFound(_) => f.write_str("Layer not found"),
            CommandError::PropertyNotFound { name, channel, .. } => {
                write!(f, "Property “{channel}” was not found on “{name}”")
            }
            CommandError::Locked { name, .. } => write!(f, "Layer “{name}” is locked"),
            CommandError::InvalidValue { what, reason } => write!(f, "Invalid {what}: {reason}"),
            CommandError::Animated { channel, .. } => write!(
                f,
                "“{channel}” is animated; edit its keyframes instead of its static value"
            ),
            CommandError::PreservedHandEdit { channel, .. } => write!(
                f,
                "Preserved hand-edited {channel}; explicitly allow overwrite to change it"
            ),
            CommandError::Malformed(m) => write!(f, "The project is malformed: {m}"),
            CommandError::Invalid(m) => f.write_str(m),
            CommandError::KeyframeNotFound(_) => {
                f.write_str("The selected keyframes are no longer available")
            }
            CommandError::EffectNotFound(e) => write!(f, "Effect not found: {e}"),
            CommandError::CompNotFound(_) => f.write_str("Composition not found"),
            CommandError::NotImplemented(c) => {
                write!(f, "“{c}” is not available in this version yet")
            }
        }
    }
}

impl std::error::Error for CommandError {}

/// Why a transaction, undo, redo, or gesture step did not publish. In every
/// case the published document is exactly what it was before the call.
#[derive(Clone, Debug, PartialEq)]
pub enum TransactionError {
    /// A command was refused (see [`CommandError`]).
    Command(CommandError),
    /// An operation did not apply to the scratch clone.
    Patch(String),
    /// Whole-transaction validation failed after the operations applied.
    Invalid(String),
    /// A gesture is open; only gesture steps may write until it ends.
    GestureInProgress,
    /// The gesture id is unknown, already ended, or not the open gesture.
    NoSuchGesture,
    /// `expected_revision` did not match the current revision.
    Stale {
        expected: u64,
        current: u64,
    },
    /// The store holds no project tree.
    NoDocument,
    /// Nothing to undo / redo.
    NothingToUndo,
    NothingToRedo,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransactionError::Command(e) => e.fmt(f),
            TransactionError::Patch(m) => write!(f, "The edit could not be applied: {m}"),
            TransactionError::Invalid(m) => write!(f, "The edit was rejected: {m}"),
            TransactionError::GestureInProgress => {
                f.write_str("A drag is in progress; finish it before making another edit")
            }
            TransactionError::NoSuchGesture => f.write_str("That drag is not in progress"),
            TransactionError::Stale { expected, current } => write!(
                f,
                "The project changed (expected revision {expected}, now {current})"
            ),
            TransactionError::NoDocument => f.write_str("No project is open"),
            TransactionError::NothingToUndo => f.write_str("Nothing to undo"),
            TransactionError::NothingToRedo => f.write_str("Nothing to redo"),
        }
    }
}

impl std::error::Error for TransactionError {}

impl From<CommandError> for TransactionError {
    fn from(e: CommandError) -> Self {
        TransactionError::Command(e)
    }
}

impl From<PatchError> for TransactionError {
    fn from(e: PatchError) -> Self {
        TransactionError::Patch(e.to_string())
    }
}

/// Why a project could not be opened or saved.
#[derive(Debug)]
pub enum ProjectIoError {
    /// The container reader rejected the file (message of
    /// `decodeProjectContainer`).
    Container(ContainerError),
    /// The operating system refused (path, then the OS message).
    Io {
        path: String,
        source: std::io::Error,
    },
    /// The file parsed but is not a Powermove project.
    NotAProject,
    /// The file changed on disk since it was opened or last saved by us. The
    /// open document is untouched; use Save As to keep both versions.
    ChangedOnDisk,
    /// The destination exists and this was a first save (no overwrite).
    DestinationExists(String),
    /// A media entry could not be read or failed its checksum.
    Media { id: String, reason: String },
    /// Bad arguments (duplicate media ids, oversized fields, ...).
    Invalid(String),
    /// Another writer changed the file while we published. Neither version
    /// was lost: ours is kept at `recovery_path`.
    Conflict {
        message: String,
        recovery_path: std::path::PathBuf,
    },
    /// Our file was published but the final directory sync failed. Your
    /// edits stay open (and dirty); save again to confirm.
    Finalize(String),
    /// A save of this document is already running.
    SaveInProgress,
    /// No destination: use Save As.
    NoDestination,
    /// The receipt belongs to another document session.
    WrongDocument,
}

impl fmt::Display for ProjectIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectIoError::Container(e) => e.fmt(f),
            ProjectIoError::Io { path, source } => write!(f, "{path}: {source}"),
            ProjectIoError::NotAProject => f.write_str("This is not a Powermove project."),
            ProjectIoError::ChangedOnDisk => f.write_str(
                "The project file was moved, deleted, or changed outside Powermove. Your edits \
                 are still open. Use Save As with a different name to keep both versions.",
            ),
            ProjectIoError::DestinationExists(p) => {
                write!(f, "{p} already exists. Choose another name.")
            }
            ProjectIoError::Media { id, reason } => {
                write!(f, "Could not save media “{id}”: {reason}")
            }
            ProjectIoError::Invalid(m) => f.write_str(m),
            ProjectIoError::Conflict {
                message,
                recovery_path,
            } => write!(
                f,
                "{message} The other version was preserved at {}",
                recovery_path.display()
            ),
            ProjectIoError::Finalize(m) => write!(
                f,
                "The project file was written, but save finalization failed. Your edits are \
                 still open. Try Save again to confirm, or Save As to keep a separate copy. {m}"
            ),
            ProjectIoError::SaveInProgress => {
                f.write_str("A save to this project file is already in progress.")
            }
            ProjectIoError::NoDestination => f.write_str("Choose a destination with Save As."),
            ProjectIoError::WrongDocument => {
                f.write_str("That save belongs to a project that is no longer open.")
            }
        }
    }
}

impl std::error::Error for ProjectIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProjectIoError::Io { source, .. } => Some(source),
            ProjectIoError::Container(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ContainerError> for ProjectIoError {
    fn from(e: ContainerError) -> Self {
        ProjectIoError::Container(e)
    }
}

pub(crate) fn io_err(path: &std::path::Path, source: std::io::Error) -> ProjectIoError {
    ProjectIoError::Io {
        path: path.display().to_string(),
        source,
    }
}
