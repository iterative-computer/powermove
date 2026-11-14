//! Bounded process workers. The UI only enqueues actions and polls events.
//! Pipe writes have their own worker so a runtime which stops reading stdin
//! cannot prevent cancellation, timeouts, or process-group termination.
use crate::{
    Effects, Event, claude, codex, codex_exec,
    delivery::{self, Delivery, Surface},
    protocol::Lines,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}
impl ProcessSpec {
    fn spawn(&self) -> io::Result<Child> {
        let mut c = Command::new(&self.executable);
        c.args(&self.args)
            .current_dir(&self.cwd)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        c.spawn()
    }
}
pub enum Machine {
    Codex(codex::Client),
    Claude(claude::Client),
    CodexExec {
        client: codex_exec::Client,
        output: PathBuf,
    },
}
impl Machine {
    fn surface(&self) -> Surface {
        match self {
            Self::Codex(_) => Surface::Codex,
            Self::Claude(_) => Surface::Claude,
            Self::CodexExec { .. } => Surface::CodexExec,
        }
    }
    fn receive(&mut self, v: Value, now: u64) -> Effects {
        match self {
            Self::Codex(c) => c.receive(v, now),
            Self::Claude(c) => c.receive(v),
            Self::CodexExec { client, .. } => client.receive(v),
        }
    }
    fn tick(&mut self, now: u64) -> Effects {
        match self {
            Self::Codex(c) => c.tick(now),
            Self::Claude(c) => c.tick(now),
            Self::CodexExec { client, .. } => client.tick(now),
        }
    }
    fn stopped(&mut self, reason: &str) -> Effects {
        match self {
            Self::Codex(c) => c.stopped(reason),
            Self::Claude(c) => c.stopped(reason),
            Self::CodexExec { client, .. } => client.stopped(reason),
        }
    }
    fn action(&mut self, a: Action, now: u64) -> (Effects, bool) {
        let warning = |s: String| {
            (
                Effects {
                    events: vec![Event::Warning(s)],
                    ..Effects::default()
                },
                false,
            )
        };
        let optional = |value: Option<Effects>| match value {
            Some(effects) => (effects, true),
            None => (Effects::default(), false),
        };
        let cancel = |effects: Effects| {
            let accepted =
                !effects.events.is_empty() || !effects.outgoing.is_empty() || effects.close_stdin;
            (effects, accepted)
        };
        match a {
            Action::Run(r) => match self {
                Self::Codex(c) => c.run(*r, now).map(|e| (e, true)).unwrap_or_else(warning),
                _ => warning("This process already owns a run".into()),
            },
            Action::Steer {
                run,
                prompt,
                images,
            } => match self {
                Self::Codex(c) => c
                    .steer(&run, &prompt, &images, now)
                    .map(|e| (e, true))
                    .unwrap_or_else(warning),
                _ => warning("Live steering requires App Server".into()),
            },
            Action::Answer { run, item, answers } => match self {
                Self::Codex(c) => optional(c.answer(&run, &item, &answers)),
                Self::Claude(c) if c.run_id() == run => optional(c.answer(&item, &answers)),
                _ => warning("No held question for this run".into()),
            },
            Action::Cancel(run) => match self {
                Self::Codex(c) => optional(c.cancel(&run, now)),
                Self::Claude(c) if c.run_id() == run => cancel(c.cancel()),
                Self::CodexExec { client, .. } if client.run_id() == run => cancel(client.cancel()),
                _ => warning("No active run with this id".into()),
            },
            Action::Shutdown => (Effects::default(), true),
        }
    }
}
pub enum Action {
    Run(Box<codex::Run>),
    Steer {
        run: String,
        prompt: String,
        images: Vec<String>,
    },
    Answer {
        run: String,
        item: String,
        answers: BTreeMap<String, Vec<String>>,
    },
    Cancel(String),
    Shutdown,
}
struct QueuedAction {
    action: Action,
    reply: Option<SyncSender<bool>>,
}
pub struct Handle {
    tx: SyncSender<QueuedAction>,
    events: Receiver<Event>,
    surface: Surface,
    /// Lives as long as the handle (a run's tool grant).
    held: Vec<Box<dyn Send>>,
}
impl Handle {
    /// Keep `value` alive until this handle drops.
    pub fn hold(mut self, value: impl Send + 'static) -> Self {
        self.held.push(Box::new(value));
        self
    }
    pub fn supports_steering(&self) -> bool {
        self.surface == Surface::Codex
    }
    /// On saturation, returns the original action for a later retry.
    pub fn send(&self, a: Action) -> Result<(), TrySendError<Action>> {
        self.enqueue(a, None)
    }
    /// Resolves after the protocol machine accepts/rejects the action and its
    /// effects are enqueued. This is stronger than mailbox admission, but not
    /// a provider RPC acknowledgement (interrupt completion remains an event).
    pub fn send_checked(&self, action: Action) -> Result<Receiver<bool>, TrySendError<Action>> {
        let (reply, pending) = mpsc::sync_channel(1);
        self.enqueue(action, Some(reply))?;
        Ok(pending)
    }
    fn enqueue(
        &self,
        action: Action,
        reply: Option<SyncSender<bool>>,
    ) -> Result<(), TrySendError<Action>> {
        self.tx
            .try_send(QueuedAction { action, reply })
            .map_err(|error| match error {
                TrySendError::Full(queued) => TrySendError::Full(queued.action),
                TrySendError::Disconnected(queued) => TrySendError::Disconnected(queued.action),
            })
    }
    /// Consume the same mailbox as `try_event`, preserving inspection events as
    /// typed observations instead of provider callbacks. Choose one drain API.
    pub fn try_delivery(&self) -> Result<Delivery, mpsc::TryRecvError> {
        self.events
            .try_recv()
            .map(|e| delivery::event(self.surface, e))
    }
    pub fn try_event(&self) -> Result<Event, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.send(Action::Shutdown);
    }
}
/// A run without a runtime process (`compatible`): its worker takes the
/// handle's actions, each with its acceptance reply, and sends its events.
pub(crate) type Queued = (Action, Option<SyncSender<bool>>);
pub(crate) fn worker(surface: Surface) -> (Handle, Receiver<Queued>, SyncSender<Event>) {
    let (tx, rx) = mpsc::sync_channel::<QueuedAction>(128);
    let (etx, events) = mpsc::sync_channel(256);
    let (atx, actions) = mpsc::sync_channel(128);
    thread::spawn(move || {
        for queued in rx {
            if atx.send((queued.action, queued.reply)).is_err() {
                break;
            }
        }
    });
    let handle = Handle {
        tx,
        events,
        surface,
        held: Vec::new(),
    };
    (handle, actions, etx)
}
enum Input {
    Stdout(Vec<u8>),
    Stderr,
    Eof,
    WriteFailed,
}
/// Process startup, signing checks, all IO and shutdown occur off the caller.
pub fn spawn(spec: ProcessSpec, mut machine: Machine, initial: Vec<Value>) -> Handle {
    let surface = machine.surface();
    let (tx, rx) = mpsc::sync_channel::<QueuedAction>(128);
    let (etx, events) = mpsc::sync_channel(256);
    thread::spawn(move || {
        let mut child = match spec.spawn() {
            Ok(c) => c,
            Err(err) => {
                for e in machine
                    .stopped(&format!("Could not start agent runtime: {err}"))
                    .events
                {
                    let _ = etx.send(e);
                }
                return;
            }
        };
        let (itx, irx) = mpsc::sync_channel(32);
        reader(child.stdout.take().unwrap(), itx.clone(), false);
        reader(child.stderr.take().unwrap(), itx.clone(), true);
        let (wtx, wrx) = mpsc::sync_channel::<Option<Vec<u8>>>(32);
        let mut stdin = child.stdin.take().unwrap();
        thread::spawn(move || {
            while let Ok(Some(bytes)) = wrx.recv() {
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    let _ = itx.send(Input::WriteFailed);
                    break;
                }
            }
        });
        let start = Instant::now();
        let now = || start.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let mut lines = Lines::new(1024 * 1024);
        let mut closing = None;
        let mut reason = "Agent runtime stopped.".to_string();
        let mut eof = false;
        let mut initial = Effects {
            outgoing: initial,
            // Exec receives its prompt in argv, but reads non-terminal stdin
            // to EOF before starting. It has no streaming input protocol.
            close_stdin: matches!(&machine, Machine::CodexExec { .. }),
            ..Effects::default()
        };
        if let Machine::Codex(c) = &mut machine {
            initial.append(c.start(0));
        }
        let mut stop = apply(surface, initial, &wtx, &etx, &mut closing).is_err();
        while !stop {
            for _ in 0..128 {
                let action = match rx.try_recv() {
                    Ok(a) => a,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        stop = true;
                        break;
                    }
                };
                if matches!(action.action, Action::Shutdown) {
                    if let Some(reply) = action.reply {
                        let _ = reply.try_send(true);
                    }
                    stop = true;
                    break;
                }
                let (effects, accepted) = machine.action(action.action, now());
                let applied = apply(surface, effects, &wtx, &etx, &mut closing);
                if let Some(reply) = action.reply {
                    let _ = reply.try_send(accepted && applied.is_ok());
                }
                if let Err(err) = applied {
                    reason = err.to_string();
                    stop = true;
                    break;
                }
            }
            if stop {
                break;
            }
            match irx.recv_timeout(Duration::from_millis(10)) {
                Ok(Input::Stdout(bytes)) => {
                    for line in lines.push(&bytes) {
                        let e = match line {
                            Ok(v) => machine.receive(v, now()),
                            Err(w) => Effects {
                                events: vec![Event::Warning(w)],
                                ..Effects::default()
                            },
                        };
                        if let Err(err) = apply(surface, e, &wtx, &etx, &mut closing) {
                            reason = err.to_string();
                            stop = true;
                            break;
                        }
                    }
                }
                Ok(Input::Stderr) => {} // drained on a bounded reader; never forward secrets.
                Ok(Input::WriteFailed) => {
                    reason = "Agent runtime closed its input pipe.".into();
                    stop = true;
                }
                Ok(Input::Eof) => {
                    for v in lines.finish().into_iter().flatten() {
                        let _ = apply(surface, machine.receive(v, now()), &wtx, &etx, &mut closing);
                    }
                    eof = true;
                    stop = true;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => stop = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Err(err) = apply(surface, machine.tick(now()), &wtx, &etx, &mut closing) {
                reason = err.to_string();
                stop = true;
            }
            if closing.is_some_and(|end| Instant::now() >= end) {
                reason = "Agent runtime did not exit after its turn ended.".into();
                stop = true;
            }
        }
        drop(wtx);
        // stdout may close just before process exit. Give it a bounded reap window.
        if eof {
            for _ in 0..20 {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        if let Machine::CodexExec { client, output } = &mut machine
            && eof
        {
            let status = child.try_wait().ok().flatten().and_then(|s| s.code());
            let message = read_output(output).ok();
            for e in client.exited(status, message).events {
                let _ = etx.try_send(e);
            }
        }
        terminate(&mut child);
        // A slow subscriber cannot keep the runtime alive. After termination the
        // final status may wait for its bounded mailbox to drain.
        for event in machine.stopped(&reason).events {
            if etx.send(event).is_err() {
                break;
            }
        }
    });
    Handle {
        tx,
        events,
        surface,
        held: Vec::new(),
    }
}
fn read_output(path: &Path) -> io::Result<String> {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let f = o.open(path)?;
    if !f.metadata()?.is_file() {
        return Err(crate::error("Result is not a regular file"));
    }
    let mut bytes = vec![];
    f.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(crate::error("Agent result too large"));
    }
    String::from_utf8(bytes).map_err(|_| crate::error("Agent result is not UTF-8"))
}
fn reader(mut r: impl Read + Send + 'static, tx: SyncSender<Input>, stderr: bool) {
    thread::spawn(move || {
        let mut buf = [0; 8192];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let v = if stderr {
                        Input::Stderr
                    } else {
                        Input::Stdout(buf[..n].to_vec())
                    };
                    if tx.send(v).is_err() {
                        return;
                    }
                }
            }
        }
        if !stderr {
            let _ = tx.send(Input::Eof);
        }
    });
}
fn apply(
    surface: Surface,
    e: Effects,
    tx: &SyncSender<Option<Vec<u8>>>,
    events: &SyncSender<Event>,
    closing: &mut Option<Instant>,
) -> io::Result<()> {
    for delivery in delivery::ordered(surface, e) {
        match delivery {
            Delivery::Wire(v) => {
                let mut bytes = serde_json::to_vec(&v)?;
                if bytes.len() > 32 * 1024 * 1024 {
                    return Err(crate::error("Agent request exceeds 32 MiB"));
                }
                bytes.push(b'\n');
                tx.try_send(Some(bytes))
                    .map_err(|_| crate::error("Agent runtime is not draining its input"))?;
            }
            Delivery::Callback(event) | Delivery::Inspection(event) => {
                events
                    .try_send(event)
                    .map_err(|_| crate::error("Agent event subscriber is not draining events"))?;
            }
            Delivery::CloseStdin => {
                tx.try_send(None)
                    .map_err(|_| crate::error("Agent runtime is not draining its input"))?;
                *closing = Some(Instant::now() + Duration::from_secs(2));
            }
        }
    }
    Ok(())
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
