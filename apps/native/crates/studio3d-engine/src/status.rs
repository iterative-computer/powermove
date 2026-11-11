//! Exact names and copy from design/surfaces/3d-studio.md §§1,3,10.
//! Raw engine details belong in the Error Log, not these strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Softbox,
    Spotlight,
    Bulb,
    Sun,
}
impl LightKind {
    pub fn from_stored_type(value: &str) -> Option<Self> {
        Some(match value {
            "area" => Self::Softbox,
            "spot" => Self::Spotlight,
            "point" => Self::Bulb,
            "sun" => Self::Sun,
            _ => return None,
        })
    }
    pub fn stored_type(self) -> &'static str {
        match self {
            Self::Softbox => "area",
            Self::Spotlight => "spot",
            Self::Bulb => "point",
            Self::Sun => "sun",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Softbox => "Softbox",
            Self::Spotlight => "Spotlight",
            Self::Bulb => "Bulb",
            Self::Sun => "Sun",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Fast,
    Live,
    Preparing {
        completed_triangles: u64,
        total_triangles: u64,
    },
    Refining {
        samples: u32,
        goal: u32,
    },
    Clean,
}
impl Progress {
    /// The host owns the specified 0.5s delay, clean lifetime and accessibility
    /// announcements. No synthetic time or percent is emitted by this model.
    pub fn capsule(self) -> Option<String> {
        match self {
            Self::Fast | Self::Live => None,
            Self::Clean => Some("Clean".into()),
            Self::Refining { samples, goal } if goal > 0 => Some(format!(
                "Refining · {} %",
                u64::from(samples.min(goal)) * 100 / u64::from(goal)
            )),
            Self::Refining { .. } => None,
            Self::Preparing {
                total_triangles: n, ..
            } => Some(format!(
                "Preparing {} triangles",
                if n >= 1_000_000 {
                    format!("{:.1}M", n as f64 / 1_000_000.)
                } else if n >= 1_000 {
                    format!("{}K", n / 1_000)
                } else {
                    n.to_string()
                }
            )),
        }
    }
    pub fn fraction(self) -> Option<f64> {
        match self {
            Self::Preparing {
                completed_triangles: done,
                total_triangles: total,
            } if total > 0 => Some(done.min(total) as f64 / total as f64),
            Self::Refining { samples, goal } if goal > 0 => {
                Some(f64::from(samples.min(goal)) / f64::from(goal))
            }
            Self::Clean => Some(1.),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Use only after the attached Fast path has actually succeeded.
    ModelTooLargeShowingFast,
    /// Use only after the attached Fast path has actually succeeded.
    QualityStoppedShowingFast,
    SceneLoad,
    ExportFrame {
        timecode: String,
    },
}
impl Failure {
    pub fn copy(&self) -> String {
        match self {
            Self::ModelTooLargeShowingFast => {
                "This model is too large for Quality, so it draws in Fast.".into()
            }
            Self::QualityStoppedShowingFast => {
                "Quality preview stopped, so the viewer is showing Fast.".into()
            }
            Self::SceneLoad => "This 3D layer couldn’t be loaded.".into(),
            Self::ExportFrame { timecode } => {
                format!("Couldn’t render 3D at {timecode}. Your project wasn’t changed.")
            }
        }
    }
}

pub const EXPORT_PREPARING: &str = "Preparing 3D model…";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_do_not_change_stored_types() {
        for (stored, name) in [
            ("area", "Softbox"),
            ("spot", "Spotlight"),
            ("point", "Bulb"),
            ("sun", "Sun"),
        ] {
            let kind = LightKind::from_stored_type(stored).unwrap();
            assert_eq!(kind.name(), name);
            assert_eq!(kind.stored_type(), stored);
        }
        assert!(LightKind::from_stored_type("directional").is_none());
    }
    #[test]
    fn status_only_reports_measured_progress_and_approved_copy() {
        assert_eq!(Progress::Live.capsule(), None);
        assert_eq!(Progress::Fast.capsule(), None);
        let refine = Progress::Refining {
            samples: 32,
            goal: 64,
        };
        assert_eq!(refine.capsule().as_deref(), Some("Refining · 50 %"));
        assert_eq!(refine.fraction(), Some(0.5));
        assert_eq!(
            Progress::Refining {
                samples: 0,
                goal: 0
            }
            .fraction(),
            None
        );
        let preparing = Progress::Preparing {
            completed_triangles: 320_000,
            total_triangles: 640_000,
        };
        assert_eq!(
            preparing.capsule().as_deref(),
            Some("Preparing 640K triangles")
        );
        assert_eq!(preparing.fraction(), Some(0.5));
        assert_eq!(
            Progress::Preparing {
                completed_triangles: 0,
                total_triangles: 1_000_000
            }
            .capsule()
            .as_deref(),
            Some("Preparing 1.0M triangles")
        );
        assert_eq!(Progress::Clean.capsule().as_deref(), Some("Clean"));
    }
    #[test]
    fn failure_copy_matches_spec() {
        assert_eq!(
            Failure::SceneLoad.copy(),
            "This 3D layer couldn’t be loaded."
        );
        assert_eq!(
            Failure::ExportFrame {
                timecode: "00:00:01:00".into()
            }
            .copy(),
            "Couldn’t render 3D at 00:00:01:00. Your project wasn’t changed."
        );
        assert_eq!(
            Failure::ModelTooLargeShowingFast.copy(),
            "This model is too large for Quality, so it draws in Fast."
        );
        assert_eq!(
            Failure::QualityStoppedShowingFast.copy(),
            "Quality preview stopped, so the viewer is showing Fast."
        );
        assert_eq!(EXPORT_PREPARING, "Preparing 3D model…");
    }
}
