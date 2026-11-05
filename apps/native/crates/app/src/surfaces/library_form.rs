//! S018 typed drafts. Invalid input stays visible and never reaches the kernel.
use super::model::Failure;

pub const PRESETS: &[(&str, u32, u32)] = &[
    ("4K UHD · 3840×2160", 3840, 2160),
    ("HD 1080p · 1920×1080", 1920, 1080),
    ("HD 720p · 1280×720", 1280, 720),
    ("Vertical HD · 1080×1920", 1080, 1920),
    ("Portrait 4:5 · 1080×1350", 1080, 1350),
    ("Square · 1080×1080", 1080, 1080),
];
pub const FORM_ROWS: &[&str] = &[
    "Name",
    "Resolution",
    "Size",
    "Frame Rate",
    "Duration",
    "Background",
];
pub const FIELD_LABELS: &[&str] = &[
    "Name",
    "Resolution",
    "Width",
    "Height",
    "Frame Rate",
    "Duration",
    "Background",
];
pub const NAME_ERROR: &str = "Enter a name.";
pub const SIZE_ERROR: &str = "Enter a size from 16 to 8192 pixels.";
pub const FPS_ERROR: &str = "Enter a frame rate from 1 to 240 fps.";
pub const DURATION_ERROR: &str = "Enter a duration up to 1 hour.";
pub const SHEET_TITLE: &str = "New Project";
pub const SHEET_BUTTONS: &[&str] = &["Cancel", "Create"];
pub const CUSTOM_PRESET: &str = "Custom";
pub const UNITS: &[&str] = &["px", "fps", "s"];
pub const INITIAL_FOCUS: &str = "Name";
pub const SELECT_INITIAL_CONTENTS: bool = true;

#[derive(Clone, Debug, PartialEq)]
pub struct Format {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    /// Authored colour string, not a converted/rounded rendering value.
    pub background: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewProject {
    pub name: String,
    pub format: Format,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validation {
    pub name: Option<&'static str>,
    pub size: Option<&'static str>,
    pub fps: Option<&'static str>,
    pub duration: Option<&'static str>,
}
impl Validation {
    pub fn valid(&self) -> bool {
        self.name.is_none() && self.size.is_none() && self.fps.is_none() && self.duration.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewProjectForm {
    pub name: String,
    pub width: String,
    pub height: String,
    pub fps: String,
    pub duration: String,
    pub background: String,
    pub preset: Option<usize>,
    /// No invented default for the unspecified toggle. Host must resolve GAPS.
    pub linked: Option<bool>,
    /// Stale-target guard: the originating window/project generation.
    pub owner: String,
    pub submitting: bool,
}
impl NewProjectForm {
    /// Current format wins; default is supplied by app settings, never invented
    /// in the UI. All names (including Trash) participate in uniquing.
    pub fn new(
        owner: String,
        current: Option<&Format>,
        default: &Format,
        names: &[String],
    ) -> Self {
        let seed = current.unwrap_or(default);
        Self {
            name: unique("Untitled", names),
            width: seed.width.to_string(),
            height: seed.height.to_string(),
            fps: seed.fps.to_string(),
            duration: seed.duration.to_string(),
            background: seed.background.clone(),
            preset: PRESETS
                .iter()
                .position(|(_, w, h)| *w == seed.width && *h == seed.height),
            linked: None,
            owner,
            submitting: false,
        }
    }
    pub fn choose_preset(&mut self, index: Option<usize>) -> Result<(), Failure> {
        if self.submitting {
            return Err(Failure::Busy);
        }
        if let Some(i) = index {
            let (_, w, h) = PRESETS.get(i).ok_or(Failure::Stale)?;
            self.width = w.to_string();
            self.height = h.to_string();
        }
        self.preset = index;
        Ok(())
    }
    /// AppKit text notifications call this after native editing. Editing either
    /// dimension always selects Custom, even when it matches a preset exactly.
    pub fn edit_size(&mut self, width: String, height: String) -> Result<(), Failure> {
        if self.submitting {
            return Err(Failure::Busy);
        }
        self.width = width;
        self.height = height;
        self.preset = None;
        Ok(())
    }
    pub fn validation(&self) -> Validation {
        let size_valid = [&self.width, &self.height].iter().all(|s| {
            s.trim()
                .parse::<u32>()
                .is_ok_and(|n| (16..=8192).contains(&n))
        });
        let fps = finite(&self.fps).filter(|n| (1.0..=240.0).contains(n));
        let duration = fps.and_then(|fps| parse_duration(&self.duration, fps));
        Validation {
            name: self.name.trim().is_empty().then_some(NAME_ERROR),
            size: (!size_valid).then_some(SIZE_ERROR),
            fps: fps.is_none().then_some(FPS_ERROR),
            // Avoid blaming an untouched duration for an invalid frame rate.
            duration: (fps.is_some() && !duration.is_some_and(|n| (0.1..=3600.0).contains(&n)))
                .then_some(DURATION_ERROR),
        }
    }
    pub fn values(&self) -> Option<NewProject> {
        if !self.validation().valid() {
            return None;
        }
        let fps = finite(&self.fps)?;
        Some(NewProject {
            name: self.name.trim().into(),
            format: Format {
                width: self.width.trim().parse().ok()?,
                height: self.height.trim().parse().ok()?,
                fps,
                duration: parse_duration(&self.duration, fps)?,
                background: self.background.clone(),
            },
        })
    }
    pub fn can_create(&self) -> bool {
        !self.submitting && self.values().is_some()
    }
}

fn finite(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Seconds or HH:MM:SS:FF non-drop timecode. Never truncates a numeric prefix.
/// Frame count uses nominal rounded-up rate; frame duration uses exact rate.
pub fn parse_duration(input: &str, fps: f64) -> Option<f64> {
    if !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
        return None;
    }
    let s = input.trim();
    if !s.contains(':') {
        return finite(s);
    }
    let parts: Vec<_> = s.split(':').collect();
    if parts.len() != 4
        || parts
            .iter()
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let n: Vec<u32> = parts
        .iter()
        .map(|s| s.parse::<u32>())
        .collect::<Result<_, _>>()
        .ok()?;
    if n[1] >= 60 || n[2] >= 60 || f64::from(n[3]) >= fps.ceil() {
        return None;
    }
    // Timecode counts frames, including at 30000/1001. N3: preserve source time.
    let nominal_frames = (f64::from(n[0]) * 3600.0 + f64::from(n[1]) * 60.0 + f64::from(n[2]))
        * fps.ceil()
        + f64::from(n[3]);
    Some(nominal_frames / fps)
}

pub fn unique(base: &str, names: &[String]) -> String {
    let names: std::collections::BTreeSet<_> = names.iter().map(String::as_str).collect();
    if !names.contains(base) {
        return base.into();
    }
    for suffix in 2_u64.. {
        let name = format!("{base} {suffix}");
        if !names.contains(name.as_str()) {
            return name;
        }
    }
    unreachable!("finite library cannot contain every u64 suffix")
}
