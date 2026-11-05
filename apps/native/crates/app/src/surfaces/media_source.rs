use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    Unsupported(Option<String>),
    Damaged,
    Offline,
    Cloud(String),
    DecodeDuringPlayback,
    AudioUnreadable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub target: Target,
    pub title: Option<String>,
    pub line: Option<String>,
    pub button: Option<Action>,
    pub banner: Option<Banner>,
    pub keep_last_good_frame: bool,
    pub video_continues: bool,
}
impl Source {
    pub fn new(target: Target, file: &str, cause: Cause) -> Self {
        let (line, button, banner, keep_last_good_frame, video_continues) = match cause {
            Cause::Unsupported(codec) => (
                Some(codec.map_or_else(
                    || "Powermove can’t decode this format.".into(),
                    |codec| format!("Powermove can’t decode {codec} files."),
                )),
                None,
                None,
                false,
                false,
            ),
            Cause::Damaged => (
                Some("The file may be damaged.".into()),
                None,
                None,
                false,
                false,
            ),
            Cause::Offline => (
                Some("The file is missing.".into()),
                Some(Action::Locate),
                None,
                false,
                false,
            ),
            Cause::Cloud(provider) => (
                Some(format!("The file is stored in {provider}.")),
                Some(Action::Download),
                None,
                false,
                false,
            ),
            Cause::DecodeDuringPlayback => (
                None,
                None,
                Some(Banner {
                    text: "Couldn’t decode part of this clip.".into(),
                    severity: Severity::Warning,
                    action: None,
                }),
                true,
                false,
            ),
            Cause::AudioUnreadable => (
                None,
                None,
                Some(Banner {
                    text: "This clip’s audio can’t be played.".into(),
                    severity: Severity::Info,
                    action: None,
                }),
                false,
                true,
            ),
        };
        Self {
            target,
            title: line.as_ref().map(|_| format!("Can’t Preview “{file}”")),
            line,
            button,
            banner,
            keep_last_good_frame,
            video_continues,
        }
    }
    pub fn activate(&self, target: &Target, action: Action) -> Option<Command> {
        if target != &self.target || self.button != Some(action) {
            return None;
        }
        Some(Command {
            target: target.clone(),
            operation: match action {
                Action::Locate => Operation::Locate,
                Action::Download => Operation::Download {
                    assets: vec![target.id.clone()],
                },
                _ => return None,
            },
        })
    }
    // The source bar and Back to Composition remain in the existing source host.
    pub fn empty_style(&self) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
        self.title.as_ref().map(|_| {
            (
                "exclamationmark.triangle",
                "2 * m.symbol_panel",
                "c.muted",
                "t.heading",
            )
        })
    }
}
