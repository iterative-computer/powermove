use super::*;
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub target: Target,
    frames: Vec<String>,
    frame_numbers: Vec<u64>,
    pub frame_count: u64,
    pub missing: u64,
    pub mixed_sizes: bool,
    pub fps: f64,
    pub replace: bool,
    pub required: bool,
    closed: bool,
}
impl Sequence {
    /// Worker supplies naturally ordered frame paths and the inclusive numbered
    /// indices. Missing frames hold the previous image in the engine, not dropped.
    pub fn new(
        target: Target,
        frames: Vec<(u64, String)>,
        mixed_sizes: bool,
        project_fps: f64,
        replace: bool,
        required: bool,
    ) -> Option<Self> {
        if frames.is_empty()
            || frames.iter().any(|(_, path)| path.is_empty())
            || frames.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        {
            return None;
        }
        let frame_count = frames
            .last()?
            .0
            .checked_sub(frames.first()?.0)?
            .checked_add(1)?;
        let (frame_numbers, frames): (Vec<u64>, Vec<String>) = frames.into_iter().unzip();
        Some(Self {
            target,
            missing: frame_count - frames.len() as u64,
            frames,
            frame_numbers,
            frame_count,
            mixed_sizes,
            fps: project_fps,
            replace,
            required,
            closed: false,
        })
    }
    pub fn title(&self) -> &'static str {
        if self.replace {
            "Replace Image Sequence"
        } else {
            "Import Image Sequence"
        }
    }
    pub fn explanation(&self) -> &'static str {
        "Turn your numbered images into one clip."
    }
    pub fn image_count(&self) -> String {
        format!("{} images", self.frames.len())
    }
    pub fn range(&self) -> String {
        format!(
            "{} to {}",
            self.frames.first().unwrap(),
            self.frames.last().unwrap()
        )
    }
    pub fn validation(&self) -> Option<&'static str> {
        (!self.fps.is_finite() || !(1.0..=240.0).contains(&self.fps))
            .then_some("Enter a frame rate from 1 to 240 fps.")
    }
    pub fn duration(&self) -> Option<String> {
        if self.validation().is_some() {
            return None;
        }
        Some(format!(
            "{} · {} frames",
            super::relink::time((self.frame_count as f64 / self.fps).floor() as u64),
            self.frame_count
        ))
    }
    pub fn warnings(&self) -> Vec<String> {
        let mut out = vec![];
        if self.missing > 0 {
            out.push(format!(
                "{} numbers are missing from the sequence; those frames hold the previous image.",
                self.missing
            ));
        }
        if self.mixed_sizes {
            out.push("The images aren’t all the same size; each is fitted to the first.".into());
        }
        out
    }
    pub fn buttons(&self) -> Vec<Action> {
        let mut out = vec![];
        if !self.replace && !self.required {
            out.push(Action::IndividualImages);
        }
        out.extend([Action::Cancel, self.default_action()]);
        out
    }
    pub fn default_action(&self) -> Action {
        if self.replace {
            Action::Replace
        } else {
            Action::ImportSequence
        }
    }
    pub fn key(&self, key: Key) -> Option<Action> {
        match key {
            Key::Return if self.validation().is_none() => Some(self.default_action()),
            Key::Escape => Some(Action::Cancel),
            _ => None,
        }
    }
    pub fn respond(&mut self, target: &Target, action: Action) -> Option<Command> {
        if self.closed || target != &self.target || !self.buttons().contains(&action) {
            return None;
        }
        if action == Action::Cancel {
            self.closed = true;
            return None;
        }
        let operation = if action == Action::IndividualImages {
            Operation::IndividualImages {
                frames: self.frames.clone(),
            }
        } else if action == self.default_action() && self.validation().is_none() {
            Operation::Sequence {
                frames: self.frames.clone(),
                frame_numbers: self.frame_numbers.clone(),
                frame_count: self.frame_count,
                hold_missing_frames: true,
                fit_to_first: true,
                fps: self.fps,
                replace: self.replace,
            }
        } else {
            return None;
        };
        self.closed = true;
        Some(Command {
            target: target.clone(),
            operation,
        })
    }
    pub fn target_disappeared(&mut self, object_label: Option<&str>) -> Option<Banner> {
        if self.closed {
            return None;
        }
        self.closed = true;
        // N3: the review's noun table is authoritative, including articles.
        Some(Banner {
            text: format!(
                "{} was changed, so the dialog closed.",
                if matches!(object_label, Some("The project" | "Project")) {
                    "The project"
                } else {
                    "The media"
                }
            ),
            severity: Severity::Info,
            action: None,
        })
    }
    pub fn checking(completed: u64, total: u64) -> String {
        format!("Checking frames · {completed} of {total}")
    }
    pub fn creating() -> &'static str {
        "Creating image sequence…"
    }
    /// NS sheet layout/AX identifiers; reading order, first field selected.
    pub fn fields() -> [&'static str; 2] {
        ["Frame Rate", "Duration"]
    }
    pub fn tokens() -> [&'static str; 8] {
        [
            "m.sheet_width",
            "m.sheet_inset",
            "m.sheet_value_column",
            "m.row",
            "t.heading",
            "t.reading",
            "c.secondary",
            "m.symbol_inline",
        ]
    }
}
