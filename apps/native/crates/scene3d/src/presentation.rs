//! Model copy from Opus's 2026-11-04 3D Studio spec, §§1,3,7,9,10.
//! No colors, layout, timing-token values or panels are defined here.
use crate::light_tool::Kind;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    WrongType,
    GlbTooBig,
    Triangles,
    Parts,
    Splats,
    NoGeometry,
    CadUnreadable,
    CadConversion,
    Texture,
    Splat,
    Scene,
    QualityStopped,
    QualityTooLarge,
}
impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Self::WrongType => "Choose an OBJ, GLB, glTF, STEP, IGES, BREP, PLY or .splat file.",
            Self::GlbTooBig => "This GLB is over the 512 MB limit.",
            Self::Triangles => {
                "This model has more than 10 million triangles, so it wasn’t imported."
            }
            Self::Parts => "This model has more than 65,536 parts, so it wasn’t imported.",
            Self::Splats => "This file has more than 16 million splats, so it wasn’t imported.",
            Self::NoGeometry => "This model has no geometry Powermove can use.",
            Self::CadUnreadable => {
                "This CAD file couldn’t be read. Export it again as STEP and try again."
            }
            Self::CadConversion => "This CAD model couldn’t be converted.",
            Self::Texture => "A selected texture couldn’t be read, so the model wasn’t imported.",
            Self::Splat => "This file isn’t a splat or point cloud Powermove can read.",
            Self::Scene => "This 3D layer couldn’t be loaded.",
            Self::QualityStopped => "Quality preview stopped, so the viewer is showing Fast.",
            Self::QualityTooLarge => "This model is too large for Quality, so it draws in Fast.",
        }
    }
}
pub fn light_name(k: Kind) -> &'static str {
    match k {
        Kind::Area => "Softbox",
        Kind::Spot => "Spotlight",
        Kind::Point => "Bulb",
        Kind::Sun => "Sun",
    }
}
pub const UPGRADED: &str = "3D layers were upgraded, so they may look different.";
pub const MISSING_MODEL: &str = "The model file is missing.";
pub const ANIMATED_BACKGROUND: &str =
    "Its background is animated. Move that to a separate layer, then try again.";
pub const LONG_ANIMATION: &str =
    "It’s animated for more than 2,398 frames. Split or trim it, then try again.";
pub fn upgrade_error(reason: &str) -> String {
    format!("This 3D layer couldn’t be upgraded. {reason}")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preview {
    Fast,
    Quality,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    Idle,
    Live,
    Refining {
        completed: u32,
        total: u32,
    },
    Clean,
    Preparing {
        triangles: u64,
        completed: u64,
        total: u64,
    },
    Importing {
        file: String,
    },
    Converting {
        file: String,
    },
    ExportWaiting,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub text: String,
    pub fraction: Option<f64>,
}
impl Progress {
    /// `delay_elapsed` and `clean_visible` are resolved by the host using the
    /// spec's delay and pm-design motion tokens; no invented token numbers.
    pub fn status(
        &self,
        preview: Preview,
        delay_elapsed: bool,
        clean_visible: bool,
    ) -> Option<Status> {
        let fraction = |n: u64, d: u64| {
            if d > 0 && n <= d {
                Some(n as f64 / d as f64)
            } else {
                None
            }
        };
        match self {
            Self::Idle | Self::Live => None,
            Self::Refining { completed, total } if preview == Preview::Quality && delay_elapsed => {
                let f = fraction(*completed as u64, *total as u64)?;
                Some(Status {
                    text: format!("Refining · {} %", (f * 100.).floor() as u32),
                    fraction: Some(f),
                })
            }
            Self::Clean if preview == Preview::Quality && clean_visible => Some(Status {
                text: "Clean".into(),
                fraction: None,
            }),
            Self::Preparing {
                triangles,
                completed,
                total,
            } => Some(Status {
                text: format!("Preparing {} triangles", triangle_count(*triangles)),
                fraction: fraction(*completed, *total),
            }),
            Self::Importing { file } if delay_elapsed => Some(Status {
                text: format!("Importing {file}…"),
                fraction: None,
            }),
            Self::Converting { file } if delay_elapsed => Some(Status {
                text: format!("Converting {file}…"),
                fraction: None,
            }),
            Self::ExportWaiting => Some(Status {
                text: "Preparing 3D model…".into(),
                fraction: None,
            }),
            _ => None,
        }
    }
}
fn triangle_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.)
    } else if n >= 1000 {
        format!("{}K", n / 1000)
    } else {
        n.to_string()
    }
}
