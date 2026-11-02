use super::*;

/// Exact rational metadata from the worker probe. N3: compare authored values,
/// not their rounded display strings; equivalent time bases compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub numerator: u64,
    pub denominator: u64,
}
impl Ratio {
    pub fn valid(self) -> bool {
        self.denominator != 0
    }
    pub fn equivalent(self, rhs: Self) -> bool {
        self.valid()
            && rhs.valid()
            && u128::from(self.numerator) * u128::from(rhs.denominator)
                == u128::from(rhs.numerator) * u128::from(self.denominator)
    }
    pub fn display(self) -> Option<String> {
        self.valid().then(|| {
            if self.numerator.is_multiple_of(self.denominator) {
                (self.numerator / self.denominator).to_string()
            } else {
                format!("{:.3}", self.numerator as f64 / self.denominator as f64)
                    .trim_end_matches('0')
                    .to_owned()
            }
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub duration: Ratio,
    pub width: u32,
    pub height: u32,
    pub fps: Ratio,
    pub has_audio: bool,
}
pub fn time(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
impl Metadata {
    pub fn differences(&self, original: &Self) -> Option<Vec<String>> {
        if !self.duration.valid()
            || !original.duration.valid()
            || !self.fps.valid()
            || !original.fps.valid()
        {
            return None;
        }
        let mut lines = vec![];
        if !self.duration.equivalent(original.duration) {
            lines.push(format!(
                "It’s {} long; the original was {}.",
                time(self.duration.numerator / self.duration.denominator),
                time(original.duration.numerator / original.duration.denominator)
            ));
        }
        if (self.width, self.height) != (original.width, original.height) {
            lines.push(format!(
                "It’s {}×{}; the original was {}×{}.",
                self.width, self.height, original.width, original.height
            ));
        }
        if !self.fps.equivalent(original.fps) {
            lines.push(format!(
                "It’s {} fps; the original was {} fps.",
                self.fps.display()?,
                original.fps.display()?
            ));
        }
        if self.has_audio != original.has_audio {
            lines.push(
                if self.has_audio {
                    "It has audio; the original didn’t."
                } else {
                    "It has no audio; the original did."
                }
                .into(),
            );
        }
        Some(lines)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Match,
    Mismatch(Alert),
}
pub fn check(
    target: Target,
    chosen: &str,
    probed: &Metadata,
    original: &Metadata,
) -> Option<Check> {
    let differences = probed.differences(original)?;
    if differences.is_empty() {
        return Some(Check::Match);
    }
    Some(Check::Mismatch(Alert {
        target,
        message: format!("“{chosen}” doesn’t match the missing media."),
        informative: differences
            .into_iter()
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
        buttons: vec![Action::UseAnyway, Action::Cancel, Action::ChooseAnother],
        suppression: None,
    }))
}
pub fn respond(target: Target, chosen: String, action: Action) -> Option<Command> {
    let operation = match action {
        Action::UseAnyway => Operation::Relink { chosen },
        Action::ChooseAnother => Operation::Locate,
        _ => return None,
    };
    Some(Command { target, operation })
}
pub fn others(target: Target, count: usize, folder: &str) -> Option<Alert> {
    (count > 0).then(|| Alert {
        target,
        message: if count == 1 {
            format!("Relink 1 other missing file in “{folder}”?")
        } else {
            format!("Relink {count} other missing files in “{folder}”?")
        },
        informative: "They have the same names as missing media in this project.".into(),
        buttons: vec![Action::RelinkAll, Action::JustThisOne],
        suppression: None,
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyOnly {
    pub overlay: &'static str,
    pub symbol: &'static str,
    pub colour: &'static str,
    pub meta: &'static str,
}
pub fn proxy_only() -> ProxyOnly {
    ProxyOnly {
        overlay: "Proxy only",
        symbol: "exclamationmark.triangle.fill",
        colour: "c.warning",
        meta: "Original missing · using proxy",
    }
}
/// Warning, never an export blocker; action targets first missing original.
pub fn export_warning(clips: usize) -> Option<(String, &'static str)> {
    (clips > 0).then(|| {
        (
            format!("{clips} clips use proxies because their originals are missing."),
            "Relink…",
        )
    })
}

#[derive(Clone, Debug)]
pub struct FolderCandidate {
    pub id: String,
    pub name: String,
    pub recorded: Metadata,
    pub probed: Metadata,
}
/// Worker supplies only offline assets resolved by identical name in the chosen
/// folder. Confirmation commits matching IDs only; skipped IDs remain offline.
pub fn matching_folder(
    target: Target,
    folder: String,
    candidates: Vec<FolderCandidate>,
) -> (Option<Command>, Vec<String>) {
    let mut matched = vec![];
    let mut skipped = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.id.clone()) {
            continue;
        }
        if candidate
            .probed
            .differences(&candidate.recorded)
            .is_some_and(|d| d.is_empty())
        {
            matched.push(candidate.id);
        } else {
            skipped.push(candidate.id);
        }
    }
    let command = (!matched.is_empty()).then_some(Command {
        target,
        operation: Operation::RelinkMatching {
            folder,
            assets: matched,
        },
    });
    (command, skipped)
}
