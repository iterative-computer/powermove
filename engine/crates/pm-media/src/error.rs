//! Errors from probing, indexing, decoding, and reading images.

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The file is malformed or not what its container claims.
    #[error("invalid media: {0}")]
    Invalid(String),
    /// A feature this file uses that a code path does not handle; callers
    /// may fall back (for example to a demux scan) and report the reason.
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("FFmpeg {what}: {message} ({code})")]
    Ffmpeg { what: String, code: i32, message: String },
    #[error("VideoToolbox {what}: OSStatus {status}")]
    VideoToolbox { what: String, status: i32 },
    #[error("CoreVideo {what}: CVReturn {status}")]
    CoreVideo { what: String, status: i32 },
    #[error("dav1d {what}: {code}")]
    Dav1d { what: String, code: i32 },
    #[error("OpenEXR {what}: {message} ({code})")]
    OpenExr { what: String, code: i32, message: String },
    #[error("image: {0}")]
    Image(String),
    #[error("out of memory allocating {0} bytes of frame memory")]
    OutOfMemory(usize),
    /// A decoded frame that was required did not arrive.
    #[error("decode: {0}")]
    Decode(String),
    #[error("{0}")]
    Time(#[from] pm_time::TimeError),
    /// The operation was cancelled (a superseded request or a shutdown).
    #[error("cancelled")]
    Cancelled,
}

impl MediaError {
    pub fn invalid(m: impl fmt::Display) -> Self {
        MediaError::Invalid(m.to_string())
    }
    pub fn unsupported(m: impl fmt::Display) -> Self {
        MediaError::Unsupported(m.to_string())
    }
    pub fn decode(m: impl fmt::Display) -> Self {
        MediaError::Decode(m.to_string())
    }
    pub fn is_unsupported(&self) -> bool {
        matches!(self, MediaError::Unsupported(_))
    }
}

pub type Result<T, E = MediaError> = std::result::Result<T, E>;
