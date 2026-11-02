use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Unsupported,
    Damaged,
    Missing,
    DiskFull,
    Permission,
}
impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unsupported => "Not a supported format",
            Self::Damaged => "The file may be damaged",
            Self::Missing => "The file is missing",
            Self::DiskFull => "Not enough disk space",
            Self::Permission => "You don’t have permission to read it",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    Imported,
    Failed(Reason),
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub job: String,
    pub name: String,
    pub asset: String,
    pub state: State,
    /// A failed proxy does not change the original's Imported state.
    pub proxy_failed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Files,
    Folder(String),
    Paste,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Converting(String),
    Proxy(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub detail: Option<String>,
    pub severity: Severity,
    pub completed: usize,
    pub total: usize,
    pub cancel: bool,
    pub retry: bool,
    pub disclosure: Vec<(String, String)>,
    pub banner: Option<Banner>,
}
#[derive(Clone, Debug)]
pub struct Import {
    target: Target,
    origin: Origin,
    files: Vec<File>,
    step: Option<Step>,
    stopped: bool,
    disk_full: bool,
}
impl Import {
    pub fn new(target: Target, origin: Origin, files: Vec<File>) -> Option<Self> {
        let mut ids = std::collections::BTreeSet::new();
        if files.is_empty()
            || files
                .iter()
                .any(|f| f.job.is_empty() || !ids.insert(f.job.clone()))
        {
            return None;
        }
        Some(Self {
            target,
            origin,
            files,
            step: None,
            stopped: false,
            disk_full: false,
        })
    }
    pub fn files(&self) -> &[File] {
        &self.files
    }
    pub fn target(&self) -> &Target {
        &self.target
    }
    pub fn set_step(&mut self, target: &Target, step: Option<Step>) -> bool {
        if target != &self.target || self.stopped || self.terminal() {
            return false;
        }
        self.step = step;
        true
    }
    pub fn observe(
        &mut self,
        target: &Target,
        job: &str,
        state: State,
        proxy_failed: bool,
        host: &mut impl MediaHost,
    ) -> bool {
        if target != &self.target || self.stopped {
            return false;
        }
        let Some(file) = self.files.iter_mut().find(|f| f.job == job) else {
            return false;
        };
        if !matches!(file.state, State::Queued | State::Running)
            || matches!(state, State::Queued)
            || (proxy_failed && state != State::Imported)
        {
            return false;
        }
        file.state = state.clone();
        file.proxy_failed = proxy_failed;
        if state == State::Failed(Reason::DiskFull) {
            self.disk_full = true;
            self.stop_remaining(host);
        }
        true
    }
    fn stop_remaining(&mut self, host: &mut impl MediaHost) -> bool {
        let jobs = self
            .files
            .iter_mut()
            .filter_map(|f| {
                if matches!(f.state, State::Queued | State::Running) {
                    f.state = State::Cancelled;
                    Some(f.job.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        self.stopped = true;
        self.step = None;
        // Fence in-flight probes; completed/admitted assets remain in the batch.
        self.target.generation += 1;
        if jobs.is_empty() {
            return false;
        }
        host.enqueue(Command {
            target: self.target.clone(),
            operation: Operation::CancelRemaining {
                jobs,
                undo_label: Some("Import Media"),
            },
        });
        true
    }
    pub fn cancel(&mut self, target: &Target, host: &mut impl MediaHost) -> bool {
        if target != &self.target || self.terminal() || self.stopped {
            return false;
        }
        self.stop_remaining(host)
    }
    pub fn key(
        &mut self,
        target: &Target,
        key: Key,
        focused: bool,
        host: &mut impl MediaHost,
    ) -> bool {
        key == Key::Escape && focused && self.cancel(target, host)
    }
    pub fn retry(&mut self, target: &Target, host: &mut impl MediaHost) -> bool {
        if target != &self.target || !self.terminal() {
            return false;
        }
        let jobs = self
            .files
            .iter()
            .filter(|f| matches!(f.state, State::Failed(r) if r != Reason::Unsupported))
            .map(|f| f.job.clone())
            .collect::<Vec<_>>();
        let assets = self
            .files
            .iter()
            .filter(|f| f.proxy_failed)
            .map(|f| f.asset.clone())
            .collect::<Vec<_>>();
        if jobs.is_empty() && assets.is_empty() {
            return false;
        }
        self.target.generation += 1;
        self.stopped = false;
        self.disk_full = false;
        if !jobs.is_empty() {
            host.enqueue(Command {
                target: self.target.clone(),
                operation: Operation::RetryImports { jobs: jobs.clone() },
            });
        }
        // Proxy result is delivered to its separate proxy task; no reimport.
        if !assets.is_empty() {
            host.enqueue(Command {
                target: self.target.clone(),
                operation: Operation::RetryProxies { assets },
            });
        }
        for f in &mut self.files {
            if jobs.contains(&f.job) {
                f.state = State::Queued;
            }
            f.proxy_failed = false;
        }
        true
    }
    pub fn terminal(&self) -> bool {
        self.files
            .iter()
            .all(|f| !matches!(f.state, State::Queued | State::Running))
    }
    pub fn row(&self) -> Row {
        let total = self.files.len();
        let imported = self
            .files
            .iter()
            .filter(|f| f.state == State::Imported)
            .count();
        let completed = self
            .files
            .iter()
            .filter(|f| !matches!(f.state, State::Queued | State::Running))
            .count();
        let failed = self
            .files
            .iter()
            .any(|f| matches!(f.state, State::Failed(_)) || f.proxy_failed);
        let title = if self.disk_full {
            "There isn’t enough disk space.".into()
        } else if self.stopped {
            format!("Import stopped · {imported} of {total} imported")
        } else if self.terminal() && imported == 0 {
            format!("Couldn’t import {}", files_noun(total))
        } else if self.terminal() && failed {
            format!("{imported} of {total} files imported")
        } else {
            match &self.origin {
                Origin::Files => format!("Importing {}", files_noun(total)),
                Origin::Folder(folder) => format!("Importing “{folder}”"),
                Origin::Paste => "Pasting media".into(),
            }
        };
        let detail = if self.disk_full {
            Some("Free some space and try again.".into())
        } else if !self.terminal() {
            Some(match &self.step {
                Some(Step::Converting(file)) => format!("Converting “{file}”…"),
                Some(Step::Proxy(file)) => format!("Creating proxy for “{file}”…"),
                None => format!("{completed} of {total}"),
            })
        } else {
            None
        };
        let disclosure = self
            .files
            .iter()
            .filter_map(|f| {
                if let State::Failed(reason) = f.state {
                    Some((f.name.clone(), reason.label().into()))
                } else if f.proxy_failed {
                    Some((
                        f.name.clone(),
                        format!("Couldn’t create a proxy for “{}”.", f.name),
                    ))
                } else {
                    None
                }
            })
            .collect();
        Row {
            title,
            detail,
            severity: if self.disk_full || (self.terminal() && imported == 0 && !self.stopped) {
                Severity::Error
            } else if failed {
                Severity::Warning
            } else {
                Severity::Info
            },
            completed,
            total,
            cancel: !self.terminal() && !self.stopped,
            retry: self.terminal()
                && self.files.iter().any(|f| {
                    matches!(f.state, State::Failed(r) if r != Reason::Unsupported)
                        || f.proxy_failed
                }),
            disclosure,
            banner: (self.terminal() && imported == 0 && !self.stopped).then(|| Banner {
                text: "No media was imported.".into(),
                severity: Severity::Warning,
                action: Some(Action::ShowDetails),
            }),
        }
    }
}

/// `1 file` / `{n} files`.
fn files_noun(n: usize) -> String {
    if n == 1 {
        "1 file".into()
    } else {
        format!("{n} files")
    }
}
