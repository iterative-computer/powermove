//! Caption layers.
//!
//! Ports `addCaptions`, `editCaptions` and `setCaptionsContent` of
//! `captions/commands.ts` (cue lists are `{ id, start, end, text, words? }`
//! in layer-local seconds; styles fill in from their preset).

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// A cue list or text to import (`cues`, or `text` in `format`).
#[derive(Clone, Debug, PartialEq)]
pub enum CueSource {
    /// Cue objects `{ id?, start, end, text, words? }`.
    Cues(Value),
    /// SubRip, WebVTT or plain text, shifted by `offset` seconds.
    Text {
        text: String,
        format: CaptionFormat,
        offset: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionFormat {
    Srt,
    Vtt,
    Text,
}

/// `add_captions`.
#[derive(Clone, Debug, PartialEq)]
pub struct AddCaptions {
    pub id: String,
    pub name: Option<String>,
    pub source: CueSource,
    /// Partial style over the preset (`style.preset`, default `classic`).
    pub style: Option<Value>,
    pub language: Option<String>,
    pub from: Option<f64>,
    pub duration: Option<f64>,
    pub index: Option<usize>,
    pub comp: Option<String>,
}

/// `edit_captions` operations (`op`).
#[derive(Clone, Debug, PartialEq)]
pub enum CaptionEdit {
    /// Replace every cue.
    Replace(CueSource),
    /// Insert cues (sorted by start).
    Insert(Value),
    /// Patch cues by id (text, start, end, words): the cue drag and text edit.
    Update(Value),
    /// Delete cues by id.
    Delete(Vec<String>),
    /// Split a cue at layer-local `at` seconds.
    Split { id: String, at: f64 },
    /// Merge cues into the first.
    Merge(Vec<String>),
    /// Shift cues by `by` seconds.
    Move { ids: Vec<String>, by: f64 },
    /// Import text, replacing (default) or adding.
    Import { source: CueSource, replace: bool },
    /// Patch the style (`preset` resets to that preset first) and language.
    Style {
        style: Value,
        language: Option<String>,
    },
}

/// Caption commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CaptionCommand {
    /// `add_captions` (`addCaptions`). Label "Add Captions".
    Add(Box<AddCaptions>),
    /// `edit_captions` (`editCaptions`); the layer's duration grows to fit
    /// its cues. Label "Edit Captions".
    Edit { layer_id: String, edit: CaptionEdit },
}

impl CaptionCommand {
    pub fn label(&self) -> String {
        match self {
            CaptionCommand::Add(_) => "Add Captions",
            CaptionCommand::Edit { .. } => "Edit Captions",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            CaptionCommand::Add(_) => "add_captions",
            CaptionCommand::Edit { .. } => "edit_captions",
        }))
    }
}
