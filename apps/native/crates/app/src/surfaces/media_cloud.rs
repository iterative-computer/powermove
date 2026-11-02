use super::*;
pub const PREFERENCE: &str = "Automatically download cloud media when opening projects";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileState {
    InCloud,
    Downloading { bytes: u64, total: Option<u64> },
    Downloaded,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub id: String,
    pub name: String,
    pub state: FileState,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cloud {
    target: Target,
    provider: String,
    files: Vec<File>,
    attempts: BTreeMap<String, u64>,
    consent: bool,
    decided: bool,
    cancelled: bool,
}
/// App/window session owns this across every batch in one document open.
#[derive(Default)]
pub struct CloudNotices(std::collections::BTreeSet<(String, String, u64)>);
impl CloudNotices {
    fn claim(&mut self, owner: &Owner) -> bool {
        self.0
            .insert((owner.window.clone(), owner.project.clone(), owner.document))
    }
    pub fn close_document(&mut self, owner: &Owner) {
        self.0
            .remove(&(owner.window.clone(), owner.project.clone(), owner.document));
    }
}
impl Cloud {
    pub fn new(
        target: Target,
        provider: String,
        files: Vec<File>,
        automatic: bool,
    ) -> Option<Self> {
        let mut ids = std::collections::BTreeSet::new();
        if files.is_empty()
            || files.iter().any(|f| {
                f.id.is_empty() || !ids.insert(f.id.clone()) || f.state != FileState::InCloud
            })
        {
            return None;
        }
        let attempts = files
            .iter()
            .map(|f| (f.id.clone(), target.generation))
            .collect();
        Some(Self {
            attempts,
            target,
            provider,
            files,
            consent: automatic,
            decided: false,
            cancelled: false,
        })
    }
    pub fn files(&self) -> &[File] {
        &self.files
    }
    pub fn prompt(&self) -> Option<Alert> {
        if self.consent || self.decided {
            return None;
        }
        let n = self.files.len();
        let mut info = "These files are stored in the cloud and need to be downloaded to use them in this project.\n\n".to_string();
        info.push_str(
            &self
                .files
                .iter()
                .take(8)
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if n > 8 {
            info.push_str(&format!("\n…and {} more", n - 8));
        }
        Some(Alert {
            target: self.target.clone(),
            message: if n == 1 {
                "Download cloud media?".into()
            } else {
                format!("Download {n} cloud media files?")
            },
            informative: info,
            buttons: vec![Action::Download, Action::NotNow],
            suppression: Some(PREFERENCE),
        })
    }
    /// Checkbox starts unchecked. Only an explicit checked response sets it.
    pub fn respond(
        &mut self,
        target: &Target,
        action: Action,
        checked: bool,
        notices: &mut CloudNotices,
        host: &mut impl MediaHost,
    ) -> Option<Banner> {
        if target != &self.target
            || self.decided
            || !matches!(action, Action::Download | Action::NotNow)
        {
            return None;
        }
        self.decided = true;
        if checked {
            host.enqueue(Command {
                target: self.target.clone(),
                operation: Operation::SetAutomaticCloudDownload,
            });
        }
        if action == Action::Download {
            self.consent = true;
            self.start(host);
            None
        } else {
            self.deferred_banner(notices)
        }
    }
    fn deferred_banner(&mut self, notices: &mut CloudNotices) -> Option<Banner> {
        if !notices.claim(&self.target.owner) {
            return None;
        }
        Some(Banner {
            text: "Some media is still in the cloud.".into(),
            severity: Severity::Info,
            action: Some(Action::Download),
        })
    }
    /// Also used for the banner's explicit Download action. Never called merely
    /// because a project opened unless automatic consent was already supplied.
    pub fn download_action(&mut self, target: &Target, host: &mut impl MediaHost) -> bool {
        if target != &self.target {
            return false;
        }
        self.consent = true;
        self.start(host)
    }
    pub fn start(&mut self, host: &mut impl MediaHost) -> bool {
        if !self.consent {
            return false;
        }
        let assets = self
            .files
            .iter_mut()
            .filter_map(|f| {
                if f.state == FileState::InCloud {
                    f.state = FileState::Downloading {
                        bytes: 0,
                        total: None,
                    };
                    Some(f.id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if assets.is_empty() {
            return false;
        }
        for id in &assets {
            self.attempts.insert(id.clone(), self.target.generation);
        }
        self.cancelled = false;
        host.enqueue(Command {
            target: self.target.clone(),
            operation: Operation::Download { assets },
        });
        true
    }
    pub fn observe(&mut self, target: &Target, id: &str, state: FileState) -> bool {
        if target.owner != self.target.owner
            || target.id != self.target.id
            || self.attempts.get(id).copied() != Some(target.generation)
            || self.cancelled
        {
            return false;
        }
        let Some(file) = self.files.iter_mut().find(|f| f.id == id) else {
            return false;
        };
        let FileState::Downloading {
            bytes: old,
            total: old_total,
        } = file.state
        else {
            return false;
        };
        if let FileState::Downloading { bytes, total } = state {
            if bytes < old
                || total.is_some_and(|t| t == 0 || bytes > t)
                || old_total.is_some_and(|t| total != Some(t))
            {
                return false;
            }
        } else if !matches!(state, FileState::Downloaded | FileState::Failed) {
            return false;
        }
        file.state = state;
        true
    }
    fn next_generation(&self) -> Option<u64> {
        self.attempts
            .values()
            .copied()
            .chain([self.target.generation])
            .max()?
            .checked_add(1)
    }
    pub fn file_target(&self, id: &str) -> Option<Target> {
        let mut target = self.target.clone();
        target.generation = *self.attempts.get(id)?;
        Some(target)
    }
    /// Retry one failed card immediately, while unrelated downloads continue.
    /// A per-file generation rejects a late completion from its failed attempt.
    pub fn retry_file(&mut self, target: &Target, id: &str, host: &mut impl MediaHost) -> bool {
        if self.file_target(id).as_ref() != Some(target) {
            return false;
        }
        let Some(next) = target.generation.checked_add(1) else {
            return false;
        };
        let Some(file) = self
            .files
            .iter_mut()
            .find(|f| f.id == id && f.state == FileState::Failed)
        else {
            return false;
        };
        file.state = FileState::Downloading {
            bytes: 0,
            total: None,
        };
        self.attempts.insert(id.to_owned(), next);
        self.cancelled = false;
        let mut target = target.clone();
        target.generation = next;
        host.enqueue(Command {
            target,
            operation: Operation::Download {
                assets: vec![id.to_owned()],
            },
        });
        true
    }
    pub fn cancel(&mut self, host: &mut impl MediaHost) -> bool {
        let Some(next) = self.next_generation() else {
            return false;
        };
        let jobs = self
            .files
            .iter_mut()
            .filter_map(|f| {
                if matches!(f.state, FileState::Downloading { .. }) {
                    f.state = FileState::InCloud;
                    Some(f.id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if jobs.is_empty() {
            return false;
        }
        self.cancelled = true;
        self.target.generation = next;
        host.enqueue(Command {
            target: self.target.clone(),
            operation: Operation::CancelRemaining {
                jobs,
                undo_label: None,
            },
        });
        true
    }
    pub fn retry(&mut self, host: &mut impl MediaHost) -> bool {
        if self
            .files
            .iter()
            .any(|f| matches!(f.state, FileState::Downloading { .. }))
        {
            return false;
        }
        let failed = self.files.iter().any(|f| f.state == FileState::Failed);
        if !failed {
            return false;
        }
        let Some(next) = self.next_generation() else {
            return false;
        };
        self.target.generation = next;
        let assets = self
            .files
            .iter_mut()
            .filter_map(|f| {
                if f.state == FileState::Failed {
                    f.state = FileState::Downloading {
                        bytes: 0,
                        total: None,
                    };
                    Some(f.id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for id in &assets {
            self.attempts.insert(id.clone(), self.target.generation);
        }
        self.cancelled = false;
        host.enqueue(Command {
            target: self.target.clone(),
            operation: Operation::Download { assets },
        });
        true
    }

    pub fn target(&self) -> &Target {
        &self.target
    }
    pub fn task_title(&self) -> String {
        if self
            .files
            .iter()
            .any(|f| matches!(f.state, FileState::Downloading { .. }))
        {
            format!(
                "Downloading {} files from {}",
                self.files.len(),
                self.provider
            )
        } else {
            format!(
                "{} of {} downloaded",
                self.files
                    .iter()
                    .filter(|f| f.state == FileState::Downloaded)
                    .count(),
                self.files.len()
            )
        }
    }
    pub fn card(state: &FileState) -> (&'static str, Option<&'static str>, Option<f64>) {
        match state {
            FileState::InCloud => ("In cloud", None, None),
            FileState::Downloading { bytes, total } => (
                "Downloading…",
                None,
                total
                    .filter(|t| *t > 0 && *bytes <= *t)
                    .map(|t| *bytes as f64 / t as f64),
            ),
            FileState::Failed => ("Download failed", Some("Retry Download"), None),
            FileState::Downloaded => ("", None, None),
        }
    }
}
