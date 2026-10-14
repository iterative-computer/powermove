//! Errors from mixing, decoding, and the output device.

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("media: {0}")]
    Media(#[from] pm_media::MediaError),
    #[error("composition: {0}")]
    Comp(#[from] pm_eval::CompError),
    /// The legacy evaluator would have thrown (a malformed document).
    #[error("evaluation: {0}")]
    Eval(String),
    /// A layer names an asset the host cannot resolve to a file.
    #[error("the media for \u{201c}{layer}\u{201d} is missing (asset {asset})")]
    MissingMedia { layer: String, asset: String },
    #[error("audio device: {0}")]
    Device(String),
    #[error("invalid: {0}")]
    Invalid(String),
    /// A feature this phase does not implement (named in the message).
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

impl From<pm_eval::JsThrow> for AudioError {
    fn from(e: pm_eval::JsThrow) -> Self {
        AudioError::Eval(e.to_string())
    }
}

impl AudioError {
    pub fn invalid(m: impl std::fmt::Display) -> Self {
        AudioError::Invalid(m.to_string())
    }
}

pub type Result<T, E = AudioError> = std::result::Result<T, E>;
