//! S096. v1.2.0 panels/ShaderPanel.svelte and shader/ShaderEditor.svelte.
//! Source drafts, debounce bursts and revision-bound compile work, without views.
use super::{DocumentStamp, Flight, Ticket};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const WRITE_DEBOUNCE_MS: u64 = 420;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub id: String,
    pub kind: String,
    pub code: String,
}
pub fn selected_shader<'a>(layers: &'a [Layer], selection: &[String]) -> Option<&'a Layer> {
    selection
        .first()
        .and_then(|id| layers.iter().find(|l| &l.id == id && l.kind == "shader"))
        .or_else(|| layers.iter().find(|l| l.kind == "shader"))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Problem {
    Stale,
    Busy,
    NotFocused,
    Dirty,
    InvalidSelection,
    Rejected(String),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Write {
    pub ticket: Ticket,
    pub stamp: DocumentStamp,
    pub layer_id: String,
    pub code: String,
    pub command: Value,
}
#[derive(Debug)]
pub struct Editor {
    pub layer_id: String,
    pub stamp: DocumentStamp,
    pub draft: String,
    pub focused: bool,
    pub problem: Option<Problem>,
    written: String,
    due: Option<u64>,
    tab_exit_armed: bool,
    flight: Flight,
    pending: Option<Write>,
}
impl Editor {
    pub fn new(stamp: DocumentStamp, layer_id: String, code: String) -> Self {
        Self {
            layer_id,
            stamp,
            draft: code.clone(),
            written: code,
            focused: false,
            problem: None,
            due: None,
            tab_exit_armed: false,
            flight: Flight::default(),
            pending: None,
        }
    }
    pub fn focus(&mut self) {
        self.focused = true;
    }
    pub fn input(&mut self, code: String, now: u64) -> Result<(), Problem> {
        if !self.focused {
            return Err(Problem::NotFocused);
        }
        self.draft = code;
        self.due = Some(now.saturating_add(WRITE_DEBOUNCE_MS));
        self.tab_exit_armed = false;
        Ok(())
    }
    pub fn dirty(&self) -> bool {
        self.draft != self.written
    }
    /// Publication ticks never clobber a focused or rejected/unsaved draft.
    pub fn publication(&mut self, stamp: DocumentStamp, layer: &Layer) -> Result<(), Problem> {
        if stamp.project != self.stamp.project
            || stamp.session != self.stamp.session
            || layer.id != self.layer_id
        {
            return Err(Problem::Stale);
        }
        if !self.focused && !self.dirty() && self.pending.is_none() {
            self.draft = layer.code.clone();
            self.written = layer.code.clone();
            self.stamp = stamp;
        }
        Ok(())
    }
    /// One command = one undo burst, no open lease held during an IO wait.
    /// `force` is used for blur/unmount/compile; returned work still needs ACK.
    pub fn flush(&mut self, now: u64, force: bool) -> Option<Write> {
        if self.flight.current().is_some()
            || !self.dirty()
            || !force && self.due.is_none_or(|due| now < due)
        {
            return None;
        }
        self.due = None;
        let write = Write {
            ticket: self.flight.start(),
            stamp: self.stamp.clone(),
            layer_id: self.layer_id.clone(),
            code: self.draft.clone(),
            command: json!({"type":"set_content","target":self.layer_id,"patch":{"code":self.draft}}),
        };
        self.pending = Some(write.clone());
        Some(write)
    }
    /// Only an applied kernel result advances written text/revision. A stale or
    /// rejected command keeps its exact draft for retry/recovery.
    pub fn acknowledged(&mut self, ticket: Ticket, result: Result<DocumentStamp, String>) -> bool {
        if !self.flight.finish(ticket) {
            return false;
        }
        let write = self.pending.take().expect("write paired with flight");
        match result {
            Ok(stamp)
                if stamp.project == write.stamp.project
                    && stamp.session == write.stamp.session
                    && stamp.revision >= write.stamp.revision =>
            {
                self.written = write.code;
                self.stamp = stamp;
                self.problem = None;
            }
            Ok(_) => self.problem = Some(Problem::Stale),
            Err(code) => self.problem = Some(Problem::Rejected(code)),
        }
        true
    }
    /// Host must retain this controller until its final flush is acknowledged.
    pub fn blur(&mut self, now: u64) -> Option<Write> {
        self.focused = false;
        self.tab_exit_armed = false;
        self.flush(now, true)
    }
    pub fn can_retire(&self) -> bool {
        !self.dirty() && self.pending.is_none()
    }
    pub fn escape(&mut self) {
        self.tab_exit_armed = true;
    }
    /// UTF-16 offsets match NSTextView/legacy textarea, including astral text.
    /// Invalid/split-surrogate ranges are refused without changing authored text.
    pub fn tab(&mut self, start: usize, end: usize, now: u64) -> Result<Option<usize>, Problem> {
        if self.tab_exit_armed {
            self.tab_exit_armed = false;
            return Ok(None);
        }
        if !self.focused {
            return Err(Problem::NotFocused);
        }
        let units: Vec<_> = self.draft.encode_utf16().collect();
        if start > end || end > units.len() {
            return Err(Problem::InvalidSelection);
        }
        let left = String::from_utf16(&units[..start]).map_err(|_| Problem::InvalidSelection)?;
        let right = String::from_utf16(&units[end..]).map_err(|_| Problem::InvalidSelection)?;
        self.input(format!("{left}  {right}"), now)?;
        Ok(Some(start + 2))
    }
    pub fn compile(&self) -> Result<CompileInput, Problem> {
        if self.pending.is_some() {
            return Err(Problem::Busy);
        }
        if self.dirty() {
            return Err(Problem::Dirty);
        }
        Ok(CompileInput {
            stamp: self.stamp.clone(),
            layer_id: self.layer_id.clone(),
            code: self.written.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileInput {
    pub stamp: DocumentStamp,
    pub layer_id: String,
    pub code: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compiled {
    pub diagnostics: Vec<Diagnostic>,
    pub uniform_definitions: Value,
}
pub trait Compiler: Send + 'static {
    /// Worker must invalidate the exact program, compile this source and return
    /// actual diagnostics/uniforms. No renderer success is inferred from parsing.
    fn compile(&mut self, input: &CompileInput) -> crate::Result<Compiled>;
}
#[derive(Debug)]
pub struct CompileWork {
    pub ticket: Ticket,
    pub input: CompileInput,
}
pub struct CompileReply {
    pub ticket: Ticket,
    pub input: CompileInput,
    pub result: crate::Result<Compiled>,
}
impl CompileWork {
    pub fn run(self, compiler: &mut impl Compiler) -> CompileReply {
        let result = compiler.compile(&self.input);
        CompileReply {
            ticket: self.ticket,
            input: self.input,
            result,
        }
    }
}
#[derive(Debug, Default)]
pub struct Compilation {
    flight: Flight,
    expected: Option<CompileInput>,
    pub result: Option<Compiled>,
    pub error: Option<crate::ServiceErrorCode>,
}
impl Compilation {
    pub fn start(&mut self, input: CompileInput) -> CompileWork {
        self.result = None;
        self.error = None;
        self.expected = Some(input.clone());
        CompileWork {
            ticket: self.flight.start(),
            input,
        }
    }
    pub fn receive(&mut self, reply: CompileReply, current: &CompileInput) -> bool {
        if !self.flight.accepts(reply.ticket) {
            return false;
        }
        self.flight.finish(reply.ticket);
        if self.expected.as_ref() != Some(&reply.input) || current != &reply.input {
            self.expected = None;
            return false;
        }
        match reply.result {
            Ok(result) => self.result = Some(result),
            Err(error) => self.error = Some(error.code),
        }
        true
    }
    pub fn cancel(&mut self) -> Option<Ticket> {
        self.expected = None;
        self.flight.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp() -> DocumentStamp {
        DocumentStamp {
            project: "p".into(),
            session: 1,
            revision: 1,
        }
    }
    fn editor() -> Editor {
        Editor::new(stamp(), "shader".into(), "ab".into())
    }
    #[test]
    fn fallback_uses_only_first_selected_then_first_shader() {
        let layers = vec![
            Layer {
                id: "a".into(),
                kind: "shader".into(),
                code: "a".into(),
            },
            Layer {
                id: "b".into(),
                kind: "shader".into(),
                code: "b".into(),
            },
        ];
        assert_eq!(selected_shader(&layers, &["b".into()]).unwrap().id, "b");
        assert_eq!(
            selected_shader(&layers, &["missing".into(), "b".into()])
                .unwrap()
                .id,
            "a"
        );
        assert!(selected_shader(&[], &[]).is_none());
    }
    #[test]
    fn debounce_is_420_ms_and_new_input_restarts_clock() {
        let mut e = editor();
        e.focus();
        e.input("first".into(), 0).unwrap();
        e.input("second".into(), 100).unwrap();
        assert!(e.flush(519, false).is_none());
        let work = e.flush(520, false).unwrap();
        assert_eq!(
            work.command,
            json!({"type":"set_content","target":"shader","patch":{"code":"second"}})
        );
        assert!(e.flush(999, true).is_none());
    }
    #[test]
    fn writes_wait_for_real_ack_and_do_not_lose_newer_typing() {
        let mut e = editor();
        e.focus();
        e.input("one".into(), 0).unwrap();
        let one = e.flush(420, false).unwrap();
        e.input("two".into(), 421).unwrap();
        let mut next = stamp();
        next.revision = 2;
        e.acknowledged(one.ticket, Ok(next.clone()));
        assert!(e.dirty());
        let two = e.flush(841, false).unwrap();
        assert_eq!(two.stamp, next);
        assert_eq!(two.code, "two");
        assert!(!e.acknowledged(one.ticket, Ok(stamp())));
    }
    #[test]
    fn focused_draft_and_rejected_blur_are_retained() {
        let mut e = editor();
        e.focus();
        e.input("local".into(), 0).unwrap();
        let layer = Layer {
            id: "shader".into(),
            kind: "shader".into(),
            code: "foreign".into(),
        };
        let mut newer = stamp();
        newer.revision = 2;
        e.publication(newer.clone(), &layer).unwrap();
        assert_eq!(e.draft, "local");
        let write = e.blur(1).unwrap();
        e.acknowledged(write.ticket, Err("stale".into()));
        e.publication(newer, &layer).unwrap();
        assert_eq!(e.draft, "local");
        assert!(!e.can_retire());
    }
    #[test]
    fn no_change_no_write_and_unmount_flushes_immediately() {
        let mut e = editor();
        e.focus();
        assert!(e.blur(0).is_none());
        assert!(e.can_retire());
        e.focus();
        e.input("pending".into(), 1).unwrap();
        let work = e.blur(1).unwrap();
        assert_eq!(work.code, "pending");
        assert!(!e.can_retire());
        e.acknowledged(work.ticket, Ok(stamp()));
        assert!(e.can_retire());
    }
    #[test]
    fn utf16_tab_and_escape_tab_keep_native_offsets() {
        let mut e = editor();
        e.focus();
        e.input("😀ab".into(), 0).unwrap();
        assert_eq!(e.tab(2, 3, 1), Ok(Some(4)));
        assert_eq!(e.draft, "😀  b");
        assert_eq!(e.tab(1, 1, 2), Err(Problem::InvalidSelection));
        e.escape();
        assert_eq!(e.tab(0, 0, 3), Ok(None));
        assert_eq!(e.draft, "😀  b");
    }
    #[test]
    fn compile_waits_for_flushed_source() {
        let mut e = editor();
        e.focus();
        e.input("new".into(), 0).unwrap();
        assert_eq!(e.compile(), Err(Problem::Dirty));
        let write = e.flush(0, true).unwrap();
        assert_eq!(e.compile(), Err(Problem::Busy));
        e.acknowledged(write.ticket, Ok(stamp()));
        assert_eq!(e.compile().unwrap().code, "new");
    }
    #[test]
    fn compiler_adapter_retains_full_diagnostics_and_uniforms() {
        struct TestCompiler;
        impl Compiler for TestCompiler {
            fn compile(&mut self, input: &CompileInput) -> crate::Result<Compiled> {
                assert_eq!(input.code, "ab");
                Ok(Compiled {
                    diagnostics: vec![Diagnostic {
                        message: "error ".repeat(100),
                        line: Some(7),
                        column: Some(2),
                    }],
                    uniform_definitions: json!([{"name":"uSpeed","future":1}]),
                })
            }
        }
        let mut c = Compilation::default();
        let input = editor().compile().unwrap();
        let reply = c.start(input.clone()).run(&mut TestCompiler);
        assert!(c.receive(reply, &input));
        assert!(c.result.as_ref().unwrap().diagnostics[0].message.len() > 90);
        assert_eq!(c.result.unwrap().uniform_definitions[0]["future"], 1);
    }
    #[test]
    fn compile_completion_cannot_cross_source_or_session() {
        struct TestCompiler;
        impl Compiler for TestCompiler {
            fn compile(&mut self, _: &CompileInput) -> crate::Result<Compiled> {
                Ok(Compiled {
                    diagnostics: vec![],
                    uniform_definitions: json!([]),
                })
            }
        }
        let mut c = Compilation::default();
        let input = editor().compile().unwrap();
        let old = c.start(input.clone());
        let current = c.start(input.clone());
        assert!(!c.receive(old.run(&mut TestCompiler), &input));
        let mut changed = input;
        changed.stamp.session += 1;
        assert!(!c.receive(current.run(&mut TestCompiler), &changed));
        assert!(c.result.is_none());
    }
}
