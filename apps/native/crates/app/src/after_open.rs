//! Project commands asked for while a project arrives (the menu, `--open`
//! arguments): each belongs to the open under way in its window and runs
//! only when that open settles there, on that project. A later open, a
//! failed open, or the window showing another project drops it, and the
//! window says so.

type Command = (String, Vec<serde_json::Value>);

#[derive(Default)]
pub(crate) struct AfterOpen {
    /// The newest open this window began, and the project it brings.
    open: u64,
    name: String,
    queued: Vec<(u64, Command)>,
}

impl AfterOpen {
    /// A new open of `name` begins. Returns it, and the note for commands of
    /// the earlier open it dropped.
    pub(crate) fn begin(&mut self, name: &str) -> (u64, Option<String>) {
        let note = self.drop_all();
        self.open += 1;
        self.name = name.to_owned();
        (self.open, note)
    }

    /// Waits for the open under way.
    pub(crate) fn defer(&mut self, name: String, args: Vec<serde_json::Value>) {
        self.queued.push((self.open, (name, args)));
    }

    /// `open` settled; `shown`: the window still shows the project it
    /// brought. Returns the commands to run now, and the note for any
    /// dropped instead.
    pub(crate) fn settle(&mut self, open: u64, shown: bool) -> (Vec<Command>, Option<String>) {
        if open != self.open || !shown {
            return (Vec::new(), self.drop_all());
        }
        let run = std::mem::take(&mut self.queued)
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        (run, None)
    }

    /// The open brought no project: its failure alert says why, so its
    /// commands go quietly.
    pub(crate) fn fail(&mut self) {
        self.queued.clear();
    }

    fn drop_all(&mut self) -> Option<String> {
        let what = match std::mem::take(&mut self.queued).len() {
            0 => return None,
            1 => "A command".to_owned(),
            n => format!("{n} commands"),
        };
        Some(format!(
            "{what} asked for while “{}” opened didn’t run: another project took its place.",
            self.name
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(run: &[Command]) -> Vec<&str> {
        run.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Opening project B while A settles: A's commands never run against B.
    #[test]
    fn a_command_for_an_earlier_open_never_runs_on_the_next_project() {
        let mut q = AfterOpen::default();
        let (a, _) = q.begin("Launch Film");
        q.defer("exportVideo".into(), vec![]);
        let (b, note) = q.begin("Teaser");
        assert_eq!(
            note.as_deref(),
            Some(
                "A command asked for while “Launch Film” opened didn’t run: another project took its place."
            )
        );
        q.defer("compositionSettings".into(), vec![]);
        let (run, note) = q.settle(a, true);
        assert!(run.is_empty(), "A settling late runs nothing of B's");
        assert!(note.unwrap().contains("“Teaser”"));
        q.defer("undo".into(), vec![]);
        let (run, note) = q.settle(b, true);
        assert_eq!(names(&run), ["undo"]);
        assert_eq!(note, None);
    }

    #[test]
    fn commands_run_when_their_open_settles_on_their_project() {
        let mut q = AfterOpen::default();
        let (a, note) = q.begin("Launch Film");
        assert_eq!(note, None);
        q.defer("exportVideo".into(), vec![]);
        q.defer("undo".into(), vec![]);
        let (run, note) = q.settle(a, true);
        assert_eq!(names(&run), ["exportVideo", "undo"]);
        assert_eq!(note, None);
    }

    #[test]
    fn a_window_showing_another_project_drops_them() {
        let mut q = AfterOpen::default();
        let (a, _) = q.begin("Launch Film");
        q.defer("exportVideo".into(), vec![]);
        q.defer("undo".into(), vec![]);
        let (run, note) = q.settle(a, false);
        assert!(run.is_empty());
        assert!(
            note.unwrap()
                .starts_with("2 commands asked for while “Launch Film”")
        );
        let (b, _) = q.begin("Teaser");
        q.defer("undo".into(), vec![]);
        q.fail();
        assert!(q.settle(b, true).0.is_empty());
    }
}
