//! A save's transaction record, and recovery from it.
//!
//! Before a save moves any file it writes a small record beside the project,
//! `.<file name>.<uuid>.save`, and syncs it: which file is being replaced,
//! who is saving (machine, user, process, profile), the private names of the
//! staged file and of the file moved aside, the hashes of the version that
//! was read and of the staged file, and then, line by line, each step right
//! before it is taken. The record goes when the save has nothing left
//! beside the project.
//!
//! A save that was cut short leaves its record. Recovery ([`settle`]) acts
//! only on files a record names, and only once their bytes were read and
//! found to be what the record says: the last saved version goes back under
//! the project's name, an unfinished save gets a name the person finds, an
//! incomplete copy is taken back. Every move takes a name exclusively
//! ([`place_new`]), is recorded before it is made, and can be made again, so
//! a second crash in the middle of recovery resumes. What recovery cannot
//! decide (bytes that match nothing, two records of one file, a record that
//! cannot be read) it does not move: the person chooses
//! ([`leftovers`], [`use_leftover`]). A private file no record names is
//! never touched.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use pm_model::json::{Map, Value, parse, to_canonical_string};

use super::durable::{
    FileVersion, Host, Placed, PublishHooks, begins, is_uuid_v4, keep_beside, lstat_file,
    place_new, sync_directory, sync_file, withdraw,
};
use crate::error::{ProjectIoError, io_err};

/// Extensions of the private names a save uses beside a project file
/// (`.<file name>.<uuid>.<extension>`).
pub(super) const STAGED: &str = "tmp";
pub(super) const ASIDE: &str = "aside";
pub(super) const PARTIAL: &str = "partial";
const RECORD: &str = "save";

/// Who wrote a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    /// The machine's hardware identifier.
    pub host: String,
    /// The machine's name, for a person.
    pub computer: String,
    pub user: String,
    pub pid: u32,
    /// When that process started (microseconds since 1970): what tells it
    /// from a later process that was given the same number.
    pub started: u64,
    /// The profile folder of that instance of the app.
    pub profile: String,
}

impl Owner {
    /// This process, saving for the profile at `profile`.
    pub fn this(profile: Option<&Path>) -> Owner {
        let pid = std::process::id();
        Owner {
            host: host_id(),
            computer: computer_name(),
            user: std::env::var("USER").unwrap_or_default(),
            pid,
            started: started_of(pid).unwrap_or(0),
            profile: profile.map_or(String::new(), |p| p.display().to_string()),
        }
    }
    /// Whether this is a process of this machine.
    pub fn is_here(&self) -> bool {
        self.host == host_id()
    }
    /// Whether the process that wrote the record is running on this machine
    /// now: the same number and the same start.
    pub fn is_alive_here(&self) -> bool {
        self.is_here() && self.started != 0 && started_of(self.pid) == Some(self.started)
    }
    /// Whether it is the calling process.
    pub fn is_this_process(&self) -> bool {
        self.is_here()
            && self.pid == std::process::id()
            && started_of(self.pid) == Some(self.started)
    }
    fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("host", Value::from(self.host.as_str()));
        m.insert("computer", Value::from(self.computer.as_str()));
        m.insert("user", Value::from(self.user.as_str()));
        m.insert("pid", Value::from(self.pid as f64));
        m.insert("started", Value::from(self.started.to_string()));
        m.insert("profile", Value::from(self.profile.as_str()));
        Value::Object(m)
    }
    fn from_value(v: &Value) -> Option<Owner> {
        let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Owner {
            host: text("host")?,
            computer: text("computer")?,
            user: text("user")?,
            pid: v.get("pid").and_then(Value::as_f64)? as u32,
            started: text("started")?.parse().ok()?,
            profile: text("profile")?,
        })
    }
}

#[cfg(target_os = "macos")]
fn host_id() -> String {
    unsafe extern "C" {
        fn gethostuuid(id: *mut u8, wait: *const libc::timespec) -> libc::c_int;
    }
    let mut id = [0u8; 16];
    let wait = libc::timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    // SAFETY: a 16-byte buffer and a valid timeout, as the call asks.
    if unsafe { gethostuuid(id.as_mut_ptr(), &wait) } == 0 {
        super::durable::hex(&id)
    } else {
        computer_name()
    }
}
#[cfg(not(target_os = "macos"))]
fn host_id() -> String {
    computer_name()
}

fn computer_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: a buffer of the stated length; the name is NUL-terminated
    // when it fits.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } != 0 {
        return String::new();
    }
    let len = buf.iter().position(|b| *b == 0).unwrap_or(0);
    let name = String::from_utf8_lossy(&buf[..len]).into_owned();
    name.strip_suffix(".local").unwrap_or(&name).to_owned()
}

/// When the process `pid` of this machine started, if it runs.
#[cfg(target_os = "macos")]
fn started_of(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: output storage of the stated size, read only when filled.
    let info = unsafe {
        if libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        ) != size
        {
            return None;
        }
        info.assume_init()
    };
    Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}
#[cfg(not(target_os = "macos"))]
fn started_of(pid: u32) -> Option<u64> {
    crate::locks::pid_alive(pid).then_some(1)
}

/// How a save gives the project its new file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// The name held nothing (first save, Save As to a new name).
    New,
    /// The atomic exchange of the staged file and the project file.
    Exchange,
    /// The project file is moved aside, the staged file installed.
    Aside,
}
impl Route {
    fn name(self) -> &'static str {
        match self {
            Route::New => "new",
            Route::Exchange => "exchange",
            Route::Aside => "aside",
        }
    }
}

/// A record as read back.
#[derive(Clone, Debug)]
pub(crate) struct Record {
    /// The record file.
    pub path: PathBuf,
    pub owner: Owner,
    /// When the save began (milliseconds since 1970).
    pub at: u64,
    exchange: bool,
    /// The hash of the version the save was replacing (`None`: a new file).
    old: Option<String>,
    /// The hash and size of the staged file.
    new: String,
    size: u64,
    /// The staged file's private name, and the name the replaced file is
    /// moved aside to.
    pub staged: PathBuf,
    pub aside: PathBuf,
    steps: Vec<Value>,
}

fn line(value: &Value) -> String {
    let text = to_canonical_string(value);
    format!(
        "{} {text}\n",
        &pm_model::hash::sha256_hex(text.as_bytes())[..16]
    )
}

/// The whole lines of a record: everything before the first line that is
/// torn or does not carry its own checksum.
fn lines(text: &str) -> Vec<Value> {
    text.split_inclusive('\n')
        .map_while(|l| {
            let (sum, json) = l.strip_suffix('\n')?.split_once(' ')?;
            (pm_model::hash::sha256_hex(json.as_bytes())[..16] == *sum)
                .then(|| parse(json).ok())
                .flatten()
        })
        .collect()
}

/// `.<name>.<uuid>.<extension>` beside `destination`, for the save `uuid`.
fn private_name(destination: &Path, uuid: &str, extension: &str) -> PathBuf {
    let name = destination.file_name().unwrap_or(OsStr::new("project"));
    destination
        .parent()
        .unwrap_or(Path::new(""))
        .join(format!(".{}.{uuid}.{extension}", name.to_string_lossy()))
}

/// The save a private name beside `destination` belongs to, and the name's
/// extension: `.<file name>.<uuid v4>.<extension>`, exactly.
fn private_parts<'a>(entry: &'a str, destination: &Path) -> Option<(&'a str, &'a str)> {
    let name = destination.file_name()?.to_str()?;
    let rest = entry
        .strip_prefix('.')?
        .strip_prefix(name)?
        .strip_prefix('.')?;
    let (uuid, extension) = (rest.get(..36)?, rest.get(36..)?.strip_prefix('.')?);
    is_uuid_v4(uuid).then_some((uuid, extension))
}

impl Record {
    fn read(path: &Path, destination: &Path) -> Option<Record> {
        let (uuid, _) = private_parts(path.file_name()?.to_str()?, destination)?;
        let steps = lines(&fs::read_to_string(path).ok()?);
        let begin = steps.first()?;
        let text = |key: &str| begin.get(key).and_then(Value::as_str);
        // The record names its own files and no others.
        let staged = private_name(destination, uuid, STAGED);
        let aside = private_name(destination, uuid, ASIDE);
        let named = |key: &str, path: &Path| {
            text(key).is_some_and(|n| Some(OsStr::new(n)) == path.file_name())
        };
        if begin.get("v").and_then(Value::as_f64) != Some(1.0)
            || text("step") != Some("begin")
            || !named("file", destination)
            || !named("staged", &staged)
            || !named("aside", &aside)
        {
            return None;
        }
        Some(Record {
            path: path.to_owned(),
            owner: Owner::from_value(begin.get("owner")?)?,
            at: text("at")?.parse().ok()?,
            exchange: text("route")? == Route::Exchange.name(),
            old: text("old").map(str::to_owned),
            new: text("new")?.to_owned(),
            size: text("size")?.parse().ok()?,
            staged,
            aside,
            steps,
        })
    }
    fn has(&self, step: &str) -> bool {
        self.steps
            .iter()
            .any(|s| s.get("step").and_then(Value::as_str) == Some(step))
    }
}

/// A save's record while the save runs.
pub(super) struct Transaction {
    file: File,
    record: PathBuf,
    destination: PathBuf,
    pub hooks: PublishHooks,
}

impl Transaction {
    /// Writes and syncs the record of a save of `destination` whose staged
    /// file (`size` bytes, hash `new`) is at `staged`, before that save
    /// moves anything. `old` is the hash of the version it replaces.
    pub fn begin(
        destination: &Path,
        staged: &Path,
        route: Route,
        old: Option<&str>,
        new: &str,
        size: u64,
        host: &Host,
    ) -> Result<Transaction, ProjectIoError> {
        let record = staged.with_extension(RECORD);
        let name = |path: &Path| {
            Value::from(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let mut begin = Map::new();
        begin.insert("v", Value::from(1.0));
        begin.insert("step", Value::from("begin"));
        begin.insert("file", name(destination));
        begin.insert("route", Value::from(route.name()));
        begin.insert("owner", Owner::this(host.profile).to_value());
        begin.insert("at", Value::from(crate::locks::now_ms().to_string()));
        if let Some(old) = old {
            begin.insert("old", Value::from(old));
        }
        begin.insert("new", Value::from(new));
        begin.insert("size", Value::from(size.to_string()));
        begin.insert("staged", name(staged));
        begin.insert("aside", name(&staged.with_extension(ASIDE)));
        let mut options = OpenOptions::new();
        options.append(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&record).map_err(|e| io_err(&record, e))?;
        let written = file
            .write_all(line(&Value::Object(begin)).as_bytes())
            .and_then(|()| sync_file(&file))
            .map_err(|e| io_err(&record, e))
            // Its name must be there after a power cut too.
            .and_then(|()| sync_directory(destination.parent().unwrap_or(Path::new("."))));
        if let Err(e) = written {
            let _ = fs::remove_file(&record);
            return Err(e);
        }
        let tx = Transaction {
            file,
            record,
            destination: destination.to_owned(),
            hooks: host.hooks.clone(),
        };
        tx.reached("begin");
        Ok(tx)
    }

    /// Continues the record of a save that was cut short.
    fn resume(record: &Record, destination: &Path, hooks: &PublishHooks) -> io::Result<Self> {
        Ok(Transaction {
            file: OpenOptions::new().append(true).open(&record.path)?,
            record: record.path.clone(),
            destination: destination.to_owned(),
            hooks: hooks.clone(),
        })
    }

    fn reached(&self, step: &str) {
        if let Some(hook) = &self.hooks.step {
            hook(step, &self.destination);
        }
    }

    /// Appends `step` and syncs it: a step is recorded right before it is
    /// taken, so a record never claims less than was done.
    pub fn step(&mut self, step: &str) -> Result<(), ProjectIoError> {
        self.step_with(step, Map::new())
    }

    fn step_with(&mut self, step: &str, mut members: Map) -> Result<(), ProjectIoError> {
        members.insert("step", Value::from(step));
        self.file
            .write_all(line(&Value::Object(members)).as_bytes())
            .and_then(|()| sync_file(&self.file))
            .map_err(|e| io_err(&self.record, e))?;
        self.reached(step);
        Ok(())
    }

    /// Records that the file under the private name `from` is about to be
    /// placed under `to`, a name the person finds, and why.
    pub fn keeping(&mut self, from: &Path, to: &Path, why: &str) -> Result<(), ProjectIoError> {
        let name = |path: &Path| {
            Value::from(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let mut members = Map::new();
        members.insert("from", name(from));
        members.insert("to", name(to));
        members.insert("why", Value::from(why));
        self.step_with("keep", members)
    }

    /// The save left nothing beside the project: its record goes.
    pub fn close(self) {
        let _ = fs::remove_file(&self.record);
        self.reached("closed");
    }
}

/// The records beside `destination`: those that can be read, and the files
/// named like one that cannot.
fn records_of(destination: &Path) -> (Vec<Record>, Vec<PathBuf>) {
    let (mut read, mut unreadable) = (Vec::new(), Vec::new());
    for (path, extension) in private_files(destination) {
        if extension == RECORD {
            match Record::read(&path, destination) {
                Some(record) => read.push(record),
                None => unreadable.push(path),
            }
        }
    }
    read.sort_by_key(|r| r.at);
    (read, unreadable)
}

/// The regular files beside `destination` under one of a save's private
/// names, with the name's extension.
fn private_files(destination: &Path) -> Vec<(PathBuf, String)> {
    let parent = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let (_, extension) = private_parts(name.to_str()?, destination)?;
            let path = parent.join(&name);
            fs::symlink_metadata(&path)
                .is_ok_and(|m| m.is_file())
                .then(|| (path, extension.to_owned()))
        })
        .collect();
    found.sort();
    found
}

/// Whether `path` is one of the private names a save uses beside a project
/// file. It is never a project's own name: a session whose file sits under
/// one is not following a rename.
pub(super) fn is_private_name(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            let Some((rest, extension)) = name.rsplit_once('.') else {
                return false;
            };
            name.starts_with('.')
                && [STAGED, ASIDE, PARTIAL, RECORD].contains(&extension)
                && rest
                    .len()
                    .checked_sub(36)
                    .and_then(|at| rest.get(at..))
                    .is_some_and(is_uuid_v4)
        })
}

/// Whether a save of `destination` left a record: something to settle
/// before the file is opened or saved again.
pub(crate) fn pending(destination: &Path) -> bool {
    private_files(destination)
        .iter()
        .any(|(_, extension)| extension == RECORD)
}

/// What [`settle`] did with a file a cut-short save left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeftAside {
    /// The project's path held nothing: the file moved aside (the version
    /// that save was replacing) is the project file again.
    Restored,
    /// The path holds a file: the one moved aside is kept beside it here.
    Kept(PathBuf),
    /// The finished file of the save that was cut short (it holds the work
    /// that save was writing), kept beside the project here.
    Unfinished(PathBuf),
}

/// Why recovery left a record's files where they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undecided {
    /// A file the record names does not hold what the record says, or the
    /// version the save was replacing is nowhere.
    Unknown,
    /// More than one save of this file was cut short.
    Several,
    /// The record itself cannot be read.
    Unreadable,
}

/// A file recovery did not move, for the person to decide about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    pub path: PathBuf,
    /// `previous`: the file that was under the project's name when the save
    /// began to replace it. `saving`: the file the save had finished
    /// writing. `record`: a record that cannot be read.
    pub role: &'static str,
    pub why: Undecided,
    /// Who was saving and when (milliseconds since 1970), when known.
    pub owner: Option<Owner>,
    pub at: u64,
    pub size: u64,
}

/// What [`settle`] found and did.
#[derive(Debug, Default)]
pub struct Settled {
    /// What was moved, in order.
    pub left: Vec<LeftAside>,
    /// Files left where they are for the person to choose from.
    pub undecided: Vec<Leftover>,
    /// Private files no record names: never touched.
    pub unrecorded: Vec<PathBuf>,
}

/// What a file a record names holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Holds {
    Nothing,
    /// The version the save was replacing.
    Old,
    /// The file the save wrote.
    New,
    Other,
}

fn holds(path: &Path, record: &Record) -> Holds {
    // A save that changed nothing writes the bytes it read: the file moved
    // aside is then the old version, whatever else has those bytes.
    let old_first = path == record.aside;
    match lstat_file(path) {
        Ok(None) => Holds::Nothing,
        Err(_) => Holds::Other,
        Ok(Some(_)) => match FileVersion::read(path) {
            Ok(Some(version)) if old_first && Some(&version.hash) == record.old.as_ref() => {
                Holds::Old
            }
            Ok(Some(version)) if version.hash == record.new && version.stat.size == record.size => {
                Holds::New
            }
            Ok(Some(version)) if Some(&version.hash) == record.old.as_ref() => Holds::Old,
            Ok(None) => Holds::Nothing,
            _ => Holds::Other,
        },
    }
}

/// Settles what saves of `destination` that were cut short left beside it
/// (see the module). Nothing is removed that was not read at that moment and
/// found to be a file the record describes, and nothing is placed over a
/// file. Call it while holding the project's lock, so no save of this
/// project is in the middle of its steps.
pub(crate) fn settle(destination: &Path, host: &Host) -> Settled {
    let mut settled = Settled::default();
    let (records, unreadable) = records_of(destination);
    let mut named: Vec<PathBuf> = Vec::new();
    for path in unreadable {
        named.push(path.clone());
        settled
            .undecided
            .push(leftover(&path, "record", Undecided::Unreadable, None));
    }
    let several = records.len() > 1;
    for record in &records {
        named.extend([
            record.path.clone(),
            record.staged.clone(),
            record.aside.clone(),
        ]);
        let unresolved = if several {
            Some(Undecided::Several)
        } else {
            settle_one(record, destination, host, &mut settled.left)
        };
        if let Some(why) = unresolved {
            for (path, role) in [(&record.aside, "previous"), (&record.staged, "saving")] {
                if matches!(lstat_file(path), Ok(Some(_)) | Err(_)) {
                    settled
                        .undecided
                        .push(leftover(path, role, why, Some(record)));
                }
            }
        }
    }
    settled.unrecorded = private_files(destination)
        .into_iter()
        .filter(|(path, _)| !named.contains(path) && fs::metadata(path).is_ok_and(|m| m.len() > 0))
        .map(|(path, _)| path)
        .collect();
    if !records.is_empty() {
        let _ = sync_directory(destination.parent().unwrap_or(Path::new(".")));
    }
    settled
}

fn leftover(path: &Path, role: &'static str, why: Undecided, record: Option<&Record>) -> Leftover {
    Leftover {
        path: path.to_owned(),
        role,
        why,
        owner: record.map(|r| r.owner.clone()),
        at: record.map_or(0, |r| r.at),
        size: fs::symlink_metadata(path).map_or(0, |m| m.len()),
    }
}

/// One record. `None` when everything it names is settled (the record is
/// gone); otherwise why its files were left.
fn settle_one(
    record: &Record,
    destination: &Path,
    host: &Host,
    left: &mut Vec<LeftAside>,
) -> Option<Undecided> {
    let hooks = host.hooks;
    let Ok(mut tx) = Transaction::resume(record, destination, hooks) else {
        return Some(Undecided::Unreadable);
    };
    take_back_copies(record, destination, hooks);
    let mut at_path = holds(destination, record);
    let aside = holds(&record.aside, record);
    let staged = holds(&record.staged, record);
    let mut unresolved = None;

    // The last saved version goes back under a name that holds nothing.
    let mut aside_settled = aside == Holds::Nothing;
    if at_path == Holds::Nothing && aside == Holds::Old {
        if tx.step("restore").is_err() {
            return Some(Undecided::Unknown);
        }
        match place_new(&record.aside, destination, record.old.as_deref(), hooks) {
            Ok(placed) => {
                if placed == Placed::Copied {
                    let _ = fs::remove_file(&record.aside);
                }
                let _ = tx.step("restored");
                left.push(LeftAside::Restored);
                (at_path, aside_settled) = (Holds::Old, true);
            }
            // A version landed there at that instant: it stays.
            Err(ProjectIoError::DestinationExists(_)) => at_path = Holds::Other,
            Err(_) => return Some(Undecided::Unknown),
        }
    } else if at_path == Holds::Old
        && record.has("restore")
        && !record.has("restored")
        && aside != Holds::Other
    {
        // Put back right before the crash that interrupted that recovery.
        let _ = tx.step("restored");
        left.push(LeftAside::Restored);
    }
    if !aside_settled {
        aside_settled = match (at_path, aside) {
            // The path holds the same bytes: the one aside is a duplicate.
            (Holds::Old, Holds::Old) | (Holds::New, Holds::Old) => {
                fs::remove_file(&record.aside).is_ok()
            }
            // The path holds the save, or something else took it: a file
            // that was under the project's name is kept where it is found.
            (Holds::New, _) | (Holds::Other, Holds::Old) => keep_beside(
                record.aside.clone(),
                destination,
                "other version",
                hooks,
                &mut |from, to| tx.keeping(from, to, "other"),
            )
            .map(|kept| left.push(LeftAside::Kept(kept)))
            .is_ok(),
            // Unknown bytes, and the path does not hold this save.
            _ => false,
        };
        if !aside_settled {
            unresolved = Some(Undecided::Unknown);
        }
    }

    // What the staged name holds: the finished file of a save that did not
    // take the path, the file an exchange displaced, or a duplicate.
    let staged_settled = match (at_path, staged) {
        (_, Holds::Nothing) => true,
        // Installed by a copy; or the exchange displaced the old version.
        (Holds::New, Holds::New) | (Holds::New | Holds::Old, Holds::Old) => {
            fs::remove_file(&record.staged).is_ok()
        }
        // An exchange hands back whatever was under the name.
        (Holds::New, Holds::Other) | (Holds::Other, Holds::Old) if record.exchange => keep_beside(
            record.staged.clone(),
            destination,
            "other version",
            hooks,
            &mut |from, to| tx.keeping(from, to, "other"),
        )
        .map(|kept| left.push(LeftAside::Kept(kept)))
        .is_ok(),
        // The version the save was replacing is nowhere: not decided here.
        (Holds::Nothing, Holds::New) if record.old.is_some() => false,
        (_, Holds::New) if aside_settled => keep_beside(
            record.staged.clone(),
            destination,
            "recovered",
            hooks,
            &mut |from, to| tx.keeping(from, to, "unfinished"),
        )
        .map(|kept| left.push(LeftAside::Unfinished(kept)))
        .is_ok(),
        _ => false,
    };
    if !staged_settled {
        unresolved = Some(Undecided::Unknown);
    }
    if unresolved.is_none() {
        tx.close();
    }
    unresolved
}

/// A copy that was cut short: the record says a file was about to be placed
/// under a name (the project's, by `install` or `restore`; a kept file's,
/// by `keep`), and that name holds a beginning of the file. It is taken
/// back ([`withdraw`]); the whole file is still under its private name.
fn take_back_copies(record: &Record, destination: &Path, hooks: &PublishHooks) {
    let dir = destination.parent().unwrap_or(Path::new("."));
    let mut copies: Vec<(PathBuf, &Path)> = Vec::new();
    for step in &record.steps {
        let text = |key: &str| step.get(key).and_then(Value::as_str);
        match text("step") {
            Some("install") => copies.push((destination.to_owned(), &record.staged)),
            Some("restore") => copies.push((destination.to_owned(), &record.aside)),
            Some("keep") => {
                let whole = [&record.staged, &record.aside]
                    .into_iter()
                    .find(|p| p.file_name().and_then(OsStr::to_str) == text("from"));
                if let (Some(whole), Some(to)) = (whole, text("to")) {
                    copies.push((dir.join(to), whole));
                }
            }
            _ => {}
        }
    }
    for (to, whole) in copies {
        let shorter = match (fs::symlink_metadata(&to), fs::symlink_metadata(whole)) {
            (Ok(part), Ok(whole)) => part.is_file() && part.len() < whole.len(),
            _ => false,
        };
        if shorter && begins(whole, &to) {
            withdraw(&to, whole, hooks);
        }
    }
}

/// The files recovery left for the person to choose from ([`Settled`]'s
/// `undecided`), without moving anything.
pub(crate) fn leftovers(destination: &Path) -> Vec<Leftover> {
    let (records, unreadable) = records_of(destination);
    let mut found: Vec<_> = unreadable
        .iter()
        .map(|path| leftover(path, "record", Undecided::Unreadable, None))
        .collect();
    let why = if records.len() > 1 {
        Undecided::Several
    } else {
        Undecided::Unknown
    };
    for record in &records {
        for (path, role) in [(&record.aside, "previous"), (&record.staged, "saving")] {
            if matches!(lstat_file(path), Ok(Some(_))) {
                found.push(leftover(path, role, why, Some(record)));
            }
        }
    }
    found
}

/// The person chose `chosen`, a file a record of `destination` names, to
/// be the project file: it takes the project's name, which must hold
/// nothing, and the other files that record names get names the person
/// finds. Nothing is removed. Call it while holding the project's lock.
pub(crate) fn use_leftover(
    destination: &Path,
    chosen: &Path,
    host: &Host,
) -> Result<Vec<LeftAside>, ProjectIoError> {
    let hooks = host.hooks;
    let (records, _) = records_of(destination);
    let record = records
        .iter()
        .find(|r| r.staged == chosen || r.aside == chosen)
        .ok_or_else(|| {
            ProjectIoError::Invalid("That file is not one a save of this project left.".into())
        })?;
    let mut tx =
        Transaction::resume(record, destination, hooks).map_err(|e| io_err(&record.path, e))?;
    take_back_copies(record, destination, hooks);
    tx.step("chosen")?;
    if place_new(chosen, destination, None, hooks)? == Placed::Copied {
        // The copy was read back and holds the same bytes.
        let _ = fs::remove_file(chosen);
    }
    let mut left = Vec::new();
    let mut settled = true;
    for (path, label, why) in [
        (&record.aside, "other version", "other"),
        (&record.staged, "recovered", "unfinished"),
    ] {
        if path != chosen && matches!(lstat_file(path), Ok(Some(_))) {
            match keep_beside(path.clone(), destination, label, hooks, &mut |from, to| {
                tx.keeping(from, to, why)
            }) {
                Ok(kept) if why == "other" => left.push(LeftAside::Kept(kept)),
                Ok(kept) => left.push(LeftAside::Unfinished(kept)),
                // Still under its private name, which the record names.
                Err(_) => settled = false,
            }
        }
    }
    if settled {
        tx.close();
    }
    let _ = sync_directory(destination.parent().unwrap_or(Path::new(".")));
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_read_up_to_its_first_torn_line() {
        let mut m = Map::new();
        m.insert("step", Value::from("begin"));
        let whole = line(&Value::Object(m));
        let mut torn = whole.clone();
        torn.push_str(&whole[..whole.len() - 3]);
        assert_eq!(lines(&torn).len(), 1);
        assert_eq!(lines(&format!("{whole}{whole}")).len(), 2);
        assert_eq!(lines(&whole.replace("begin", "bogus")).len(), 0);
    }

    #[test]
    fn this_process_is_alive_here_and_a_dead_one_is_not() {
        let me = Owner::this(None);
        assert!(me.is_alive_here() && me.is_this_process());
        let reused = Owner {
            started: me.started + 1,
            ..me.clone()
        };
        assert!(!reused.is_alive_here(), "the same number, another start");
        let elsewhere = Owner {
            host: "another machine".into(),
            ..me
        };
        assert!(!elsewhere.is_here() && !elsewhere.is_alive_here());
        assert_eq!(Owner::from_value(&elsewhere.to_value()), Some(elsewhere));
    }

    #[test]
    fn only_a_saves_own_private_names_are_private_names() {
        let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e";
        for extension in [STAGED, ASIDE, PARTIAL, RECORD] {
            assert!(is_private_name(Path::new(&format!(
                "/job/.cut.pmv.{uuid}.{extension}"
            ))));
        }
        assert!(!is_private_name(Path::new(&format!(
            "/job/.cut.pmv.{uuid}.pmv"
        ))));
        assert!(!is_private_name(Path::new(&format!(
            "/job/cut.pmv.{uuid}.tmp"
        ))));
        assert!(!is_private_name(Path::new("/job/.cut.pmv.tmp")));
        let destination = Path::new("/job/cut.pmv");
        assert_eq!(
            private_parts(&format!(".cut.pmv.{uuid}.save"), destination),
            Some((uuid, "save"))
        );
        assert_eq!(
            private_parts(&format!(".other.pmv.{uuid}.save"), destination),
            None
        );
    }
}
