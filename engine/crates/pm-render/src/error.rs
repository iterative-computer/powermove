//! Render errors and per-layer failures.

use crate::capability::Unsupported;

/// Why a frame could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The composition uses something this renderer cannot draw yet. The
    /// tracer and export lanes check [`crate::unsupported`] first; this is
    /// the same answer found while rendering (for example a media file
    /// whose format only shows when it is opened).
    #[error("unsupported: {}", .0.iter().map(|u| u.to_string()).collect::<Vec<_>>().join("; "))]
    Unsupported(Vec<Unsupported>),
    #[error("composition {0:?} not found")]
    Composition(String),
    #[error("invalid request: {0}")]
    Request(String),
    #[error("{0}")]
    Device(#[from] pm_gpu::DeviceError),
    #[error("readback: {0}")]
    Readback(#[from] pm_gpu::readback::ReadbackError),
    #[error("GPU: {0}")]
    Gpu(String),
    #[error("media: {0}")]
    Media(String),
    /// Layers drawn by the decision 19 fallback runner
    /// (`crate::shader_layers::fallback`) whose pixels have not arrived:
    /// the caller asks the runner for them and renders the frame again.
    /// The targets are left untouched.
    #[error("waiting for the shader fallback runner ({} layer draws)", .0.len())]
    ShaderFallbackPending(Vec<crate::shader_layers::FallbackRequest>),
    /// Layers a compatibility producer draws (`crate::compat`) whose answer
    /// has not arrived: the caller asks the renderer and renders the frame
    /// again. The targets are left untouched.
    #[error("waiting for {} compatibility rasters", .0.len())]
    CompatPending(Vec<crate::compat::CompatNeed>),
    /// The hang guard holds a program this frame draws: a frame that drew
    /// it was seen running past the hang budget and has not ended
    /// (`effects::watchdog`, interactive policy). Nothing was presented; the
    /// caller renders again once `Renderer::watchdog_holding` is false.
    #[error("waiting for the GPU watchdog to judge a frame that has not finished")]
    WatchdogPending,
    /// A still this frame draws is still being read on the prefetch worker
    /// past the frame's wait, with no earlier picture of it to show
    /// (`media::MediaFailure::Pending`). Nothing was presented; the caller
    /// renders again once `Renderer::media_landed` changes.
    #[error("an image file is being read")]
    MediaPending,
    /// A program only the WebGL fallback runner draws could not be drawn
    /// by a runner the renderer waits on (a job: no window to run it, its
    /// context was lost); the message names it.
    #[error("{0}")]
    Fallback(String),
    /// A job renderer's watchdog stopped after a runaway program
    /// (`effects::watchdog::Policy::Job`): the message names it.
    #[error("{0}")]
    Watchdog(String),
    /// A frame drawn in slices gave way to something newer
    /// (`crate::slices`). The targets are left untouched.
    #[error("the frame gave way before it was finished")]
    Abandoned,
}

/// An SVG image layer drawn by the declared resvg fallback (RGBA8 at
/// output resolution) instead of the float painter: it rendered, at
/// today's precision, for the named reason (an unsupported feature such as
/// `feTurbulence`, a device or work limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgFallback {
    pub composition: String,
    pub layer: String,
    pub path: std::path::PathBuf,
    pub reason: String,
}

/// A source frame that could not be read, named by its file and its time in
/// it, with why: only [`ReadCause::Damaged`] says the file itself is bad.
/// It travels as a [`LayerFailure`]'s message, so every reader of failures
/// keeps working, and [`UnreadFrame::parse`] reads it back for the person's
/// sentence.
#[derive(Debug, Clone, PartialEq)]
pub struct UnreadFrame {
    pub file: std::path::PathBuf,
    /// The frame's time in the file, in seconds (`None`: the file is one
    /// frame, as an image sequence's files are).
    pub seconds: Option<f64>,
    pub cause: ReadCause,
    /// What the reader said.
    pub reason: String,
}

/// Why a source frame could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadCause {
    /// The file's data is corrupt or not what it claims (a bad copy).
    Damaged,
    /// Its volume is not there (a network or removable volume dropped out).
    Unavailable,
    /// Powermove may not read it.
    NoPermission,
    /// It is no longer there.
    Missing,
    OutOfMemory,
    /// Anything else, said by `reason`.
    Failed,
}

const UNREAD: &str = "an unread frame ";
const UNREAD_AT: &str = "at ";
const UNREAD_OF: &str = " s of ";
const UNREAD_FILE: &str = "of ";
const UNREAD_WHY: &str = " could not be read (";

impl ReadCause {
    const ALL: [ReadCause; 6] = [
        Self::Damaged,
        Self::Unavailable,
        Self::NoPermission,
        Self::Missing,
        Self::OutOfMemory,
        Self::Failed,
    ];
    fn tag(self) -> &'static str {
        match self {
            Self::Damaged => "damaged",
            Self::Unavailable => "unavailable",
            Self::NoPermission => "no permission",
            Self::Missing => "missing",
            Self::OutOfMemory => "out of memory",
            Self::Failed => "failed",
        }
    }
    /// Why a read of the media failed, from its typed error.
    pub fn of(error: &pm_media::MediaError) -> Self {
        use pm_media::MediaError as E;
        match error {
            E::Io(e) => Self::of_io(e),
            // FFmpeg's errors are negative errno values, or its own tags.
            E::Ffmpeg { code, .. } => match -code {
                c if c == FFMPEG_INVALIDDATA => Self::Damaged,
                c if (1..=255).contains(&c) => Self::of_errno(c),
                _ => Self::Failed,
            },
            // kVTVideoDecoderBadDataErr: the sample's data is bad.
            E::VideoToolbox { status: -12909, .. } => Self::Damaged,
            E::Invalid(_) | E::Decode(_) | E::Dav1d { .. } | E::OpenExr { .. } | E::Image(_) => {
                Self::Damaged
            }
            E::OutOfMemory(_) => Self::OutOfMemory,
            E::VideoToolbox { .. }
            | E::CoreVideo { .. }
            | E::Unsupported(_)
            | E::Time(_)
            | E::Cancelled => Self::Failed,
        }
    }
    pub fn of_io(error: &std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        match error.kind() {
            K::PermissionDenied => Self::NoPermission,
            K::NotFound => Self::Missing,
            K::OutOfMemory => Self::OutOfMemory,
            K::InvalidData | K::UnexpectedEof => Self::Damaged,
            K::TimedOut | K::NotConnected | K::StaleNetworkFileHandle => Self::Unavailable,
            _ => error.raw_os_error().map_or(Self::Failed, Self::of_errno),
        }
    }
    fn of_errno(errno: i32) -> Self {
        // EPERM, EACCES / ENOENT / ENOMEM / EIO, ENXIO, ENODEV, ENOTCONN,
        // ETIMEDOUT, EHOSTDOWN, EHOSTUNREACH, ESTALE (Darwin numbering;
        // Linux shares the first ones).
        match errno {
            1 | 13 => Self::NoPermission,
            2 => Self::Missing,
            12 => Self::OutOfMemory,
            5 | 6 | 19 | 57 | 60 | 64 | 65 | 70 => Self::Unavailable,
            _ => Self::Failed,
        }
    }
    /// Why a file that is one frame (an image sequence's) did not read:
    /// opening and reading it again says whether it can be read at all; a
    /// file that reads is damaged.
    pub fn of_file(path: &std::path::Path) -> Self {
        use std::io::Read;
        match std::fs::File::open(path).and_then(|mut f| f.read(&mut [0u8; 1])) {
            Ok(_) => Self::Damaged,
            Err(e) => Self::of_io(&e),
        }
    }
}

/// FFmpeg's AVERROR_INVALIDDATA, `-MKTAG('I','N','D','A')`.
const FFMPEG_INVALIDDATA: i32 = 0x4144_4E49;

impl UnreadFrame {
    /// "an unread frame at 0.292 s of /clips/take 3.mov could not be read
    /// (damaged: decode: …)", or for a file that is one frame "an unread
    /// frame of /frames/shot.0004.png could not be read (no permission: …)".
    pub fn message(&self) -> String {
        let at = self.seconds.map_or_else(
            || UNREAD_FILE.to_owned(),
            |s| format!("{UNREAD_AT}{s:.3}{UNREAD_OF}"),
        );
        format!(
            "{UNREAD}{at}{}{UNREAD_WHY}{}: {})",
            self.file.display(),
            self.cause.tag(),
            self.reason
        )
    }

    /// The unread frame a failure's message names, if it names one.
    pub fn parse(message: &str) -> Option<UnreadFrame> {
        let rest = message.strip_prefix(UNREAD)?;
        let (seconds, rest) = match rest.strip_prefix(UNREAD_AT) {
            Some(timed) => {
                let (seconds, rest) = timed.split_once(UNREAD_OF)?;
                (Some(seconds.parse().ok()?), rest)
            }
            None => (None, rest.strip_prefix(UNREAD_FILE)?),
        };
        let at = rest.rfind(UNREAD_WHY)?;
        let why = rest[at + UNREAD_WHY.len()..].strip_suffix(')')?;
        let (cause, reason) = ReadCause::ALL
            .into_iter()
            .find_map(|c| Some((c, why.strip_prefix(c.tag())?.strip_prefix(": ")?)))?;
        Some(UnreadFrame {
            file: rest[..at].into(),
            seconds,
            cause,
            reason: reason.to_owned(),
        })
    }
}

/// A layer that failed while the rest of the frame rendered (section 8,
/// "Isolation": a failing node marks its layer failed and outputs
/// transparency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerFailure {
    /// The composition holding the layer.
    pub composition: String,
    pub layer: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::{ReadCause, UnreadFrame};

    #[test]
    fn an_unread_frame_reads_back_from_its_message() {
        let damaged = UnreadFrame {
            file: "/clips/take 3: final.mov".into(),
            seconds: Some(12.5),
            cause: ReadCause::Damaged,
            reason: "decode: sample 300 (pts 3584) failed to decode: status -12909".into(),
        };
        assert_eq!(
            damaged.message(),
            "an unread frame at 12.500 s of /clips/take 3: final.mov could not be read (damaged: decode: sample 300 (pts 3584) failed to decode: status -12909)"
        );
        assert_eq!(UnreadFrame::parse(&damaged.message()), Some(damaged));
        for cause in ReadCause::ALL {
            let member = UnreadFrame {
                file: "/frames/shot.0004.png".into(),
                seconds: None,
                cause,
                reason: "i/o: Operation not permitted (os error 1)".into(),
            };
            assert_eq!(UnreadFrame::parse(&member.message()), Some(member));
        }
        assert_eq!(UnreadFrame::parse("media for asset A is offline"), None);
    }

    /// Only corrupt data is damage; access, a volume that dropped out and
    /// memory say what they are.
    #[test]
    fn a_read_failure_keeps_its_cause() {
        use pm_media::MediaError as E;
        use std::io::{Error, ErrorKind};
        let io = |e: Error| ReadCause::of(&E::Io(e));
        assert_eq!(
            io(Error::from(ErrorKind::PermissionDenied)),
            ReadCause::NoPermission
        );
        assert_eq!(io(Error::from(ErrorKind::NotFound)), ReadCause::Missing);
        assert_eq!(io(Error::from_raw_os_error(5)), ReadCause::Unavailable);
        assert_eq!(io(Error::from_raw_os_error(6)), ReadCause::Unavailable);
        assert_eq!(io(Error::from_raw_os_error(70)), ReadCause::Unavailable);
        assert_eq!(
            io(Error::from(ErrorKind::UnexpectedEof)),
            ReadCause::Damaged
        );
        let ffmpeg = |code| {
            ReadCause::of(&E::Ffmpeg {
                what: "read".into(),
                code,
                message: String::new(),
            })
        };
        assert_eq!(ffmpeg(-0x4144_4E49), ReadCause::Damaged);
        assert_eq!(ffmpeg(-5), ReadCause::Unavailable);
        assert_eq!(ffmpeg(-13), ReadCause::NoPermission);
        assert_eq!(ffmpeg(-12), ReadCause::OutOfMemory);
        assert_eq!(
            ReadCause::of(&E::Invalid("bad box".into())),
            ReadCause::Damaged
        );
        assert_eq!(
            ReadCause::of(&E::Decode("frame 3".into())),
            ReadCause::Damaged
        );
        assert_eq!(
            ReadCause::of(&E::VideoToolbox {
                what: "decode".into(),
                status: -12909
            }),
            ReadCause::Damaged
        );
        assert_eq!(
            ReadCause::of(&E::OutOfMemory(1 << 30)),
            ReadCause::OutOfMemory
        );
        assert_eq!(ReadCause::of(&E::Cancelled), ReadCause::Failed);
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.png");
        assert_eq!(ReadCause::of_file(&gone), ReadCause::Missing);
        std::fs::write(&gone, b"not a png").unwrap();
        assert_eq!(ReadCause::of_file(&gone), ReadCause::Damaged);
    }
}
