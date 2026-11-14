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

/// A source frame that could not be read: a damaged file (a bad copy, a
/// failing card), named by its file and its time in it. It travels as a
/// [`LayerFailure`]'s message, so every reader of failures keeps working,
/// and [`DamagedFrame::parse`] reads it back for the person's sentence.
#[derive(Debug, Clone, PartialEq)]
pub struct DamagedFrame {
    pub file: std::path::PathBuf,
    /// The frame's time in the file, in seconds (`None`: the file is one
    /// frame, as an image sequence's files are).
    pub seconds: Option<f64>,
    /// What the decoder said.
    pub reason: String,
}

const DAMAGED: &str = "a damaged frame ";
const DAMAGED_AT: &str = "at ";
const DAMAGED_OF: &str = " s of ";
const DAMAGED_FILE: &str = "of ";
const DAMAGED_WHY: &str = " could not be read (";

impl DamagedFrame {
    /// "a damaged frame at 0.292 s of /clips/take 3.mov could not be read
    /// (decode: …)", or for a file that is one frame "a damaged frame of
    /// /frames/shot.0004.png could not be read (…)".
    pub fn message(&self) -> String {
        let at = self.seconds.map_or_else(
            || DAMAGED_FILE.to_owned(),
            |s| format!("{DAMAGED_AT}{s:.3}{DAMAGED_OF}"),
        );
        format!(
            "{DAMAGED}{at}{}{DAMAGED_WHY}{})",
            self.file.display(),
            self.reason
        )
    }

    /// The damaged frame a failure's message names, if it names one.
    pub fn parse(message: &str) -> Option<DamagedFrame> {
        let rest = message.strip_prefix(DAMAGED)?;
        let (seconds, rest) = match rest.strip_prefix(DAMAGED_AT) {
            Some(timed) => {
                let (seconds, rest) = timed.split_once(DAMAGED_OF)?;
                (Some(seconds.parse().ok()?), rest)
            }
            None => (None, rest.strip_prefix(DAMAGED_FILE)?),
        };
        let at = rest.rfind(DAMAGED_WHY)?;
        let reason = rest[at + DAMAGED_WHY.len()..].strip_suffix(')')?;
        Some(DamagedFrame {
            file: rest[..at].into(),
            seconds,
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
    use super::DamagedFrame;

    #[test]
    fn a_damaged_frame_reads_back_from_its_message() {
        let damaged = DamagedFrame {
            file: "/clips/take 3: final.mov".into(),
            seconds: Some(12.5),
            reason: "decode: sample 300 (pts 3584) failed to decode: status -12909".into(),
        };
        assert_eq!(
            damaged.message(),
            "a damaged frame at 12.500 s of /clips/take 3: final.mov could not be read (decode: sample 300 (pts 3584) failed to decode: status -12909)"
        );
        assert_eq!(DamagedFrame::parse(&damaged.message()), Some(damaged));
        let member = DamagedFrame {
            file: "/frames/shot.0004.png".into(),
            seconds: None,
            reason: "invalid media: frame 0 of 0".into(),
        };
        assert_eq!(
            member.message(),
            "a damaged frame of /frames/shot.0004.png could not be read (invalid media: frame 0 of 0)"
        );
        assert_eq!(DamagedFrame::parse(&member.message()), Some(member));
        assert_eq!(DamagedFrame::parse("media for asset A is offline"), None);
    }
}
