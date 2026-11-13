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
    FileStat, FileVersion, Folder, Host, Placed, PublishHooks, Stray, begins, begins_file, copy_of,
    dispose, dispose_captured, folder_sync, head_held_back, holds_hash, is_backup_name, is_uuid_v4,
    local_stamp, lstat_file, place_new, probing, rename_private, sync_file, trash_or_leave,
};
use crate::error::{ProjectIoError, io_err};

/// Extensions of the private names a save uses beside a project file
/// (`.<file name>.<uuid>.<extension>`).
pub(super) const STAGED: &str = "tmp";
pub(super) const ASIDE: &str = "aside";
pub(super) const PARTIAL: &str = "partial";
/// The name a version is captured under while it is disposed of.
const GONE: &str = "gone";
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
    /// The files retained under a private name beside the project and the
    /// hash each held when it was.
    retained: Vec<(PathBuf, String)>,
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

/// The length of a record's whole lines: the bytes up to the first line that
/// is torn or does not carry its own checksum ([`lines`]).
fn whole_len(text: &[u8]) -> usize {
    let mut len = 0;
    for l in text.split_inclusive(|b| *b == b'\n') {
        let whole = std::str::from_utf8(l)
            .ok()
            .and_then(|l| l.strip_suffix('\n')?.split_once(' '))
            .is_some_and(|(sum, json)| {
                pm_model::hash::sha256_hex(json.as_bytes())[..16] == *sum && parse(json).is_ok()
            });
        if !whole {
            break;
        }
        len += l.len();
    }
    len
}

/// Opens the record at `path` to append to it, first cut back to its last
/// whole line and synced: a line appended behind a torn one (a crash in the
/// middle of writing it) would never be read.
fn append_to(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().read(true).append(true).open(path)?;
    let text = fs::read(path)?;
    let whole = whole_len(&text);
    if whole < text.len() {
        file.set_len(whole as u64)?;
        sync_file(&file)?;
    }
    Ok(file)
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

/// Whether `step`, a line after a record's first, is one a save or a
/// recovery writes, and every name in it is one that record may name: its
/// own staged or moved-aside file (`staged`, `aside`), or a name a file is
/// kept under beside the project (`<stem> (<label> <time>).<extension>`, a
/// single file name in the project's folder). A record with any other line
/// (a crafted or damaged one that still carries a valid checksum) is not
/// read at all, so recovery never acts on a name outside its own folder.
fn known_step(step: &Value, destination: &Path, staged: &Path, aside: &Path) -> bool {
    let text = |key: &str| step.get(key).and_then(Value::as_str);
    // The name an unreadable file at the project's name is moved to before
    // a version the person chose takes the name ([`use_leftover`]), and the
    // name a version is captured under while it is disposed of.
    let (partial, gone) = (staged.with_extension(PARTIAL), staged.with_extension(GONE));
    let own = |key: &str| {
        text(key).is_some_and(|n| {
            [staged, aside, &partial, &gone]
                .iter()
                .any(|p| p.file_name() == Some(OsStr::new(n)))
        })
    };
    let kept = |key: &str| text(key).is_some_and(|n| is_kept_name(n, destination));
    match text("step") {
        Some(
            "aside" | "install" | "exchanged" | "installed" | "restore" | "restored" | "chosen",
        ) => true,
        // A version from the project's backups found changed is kept beside
        // the project too: its source is named by its backup file name.
        Some("keep") => (own("from") || text("from").is_some_and(is_backup_name)) && kept("to"),
        Some("kept") => kept("to"),
        Some("retained") => own("name"),
        Some("dispose") => own("name") && text("hash").is_some(),
        Some("left") => own("name") && matches!(text("where"), Some("trash" | "hidden")),
        // A file moved off a public name (the project's, or a kept file's)
        // to the record's own, and the file it was a copy of, if any.
        Some("withdraw") => {
            (text("from").map(OsStr::new) == destination.file_name() || kept("from"))
                && (step.get("whole").is_none() || own("whole"))
        }
        Some("settled") => step.get("holds").is_none() || text("holds").is_some(),
        Some("seen") => step.get("to").is_none() || kept("to") || own("to"),
        _ => false,
    }
}

/// Whether `name` is a name [`Transaction::keep_file`] gives a file beside
/// `destination`: one plain file name (no folder, no `..`), the project's
/// name with a label in brackets, and the project's extension.
fn is_kept_name(name: &str, destination: &Path) -> bool {
    let (Some(stem), extension) = (
        destination.file_stem().and_then(OsStr::to_str),
        destination.extension().and_then(OsStr::to_str),
    ) else {
        return false;
    };
    let tail = match extension {
        Some(extension) => format!(").{extension}"),
        None => ")".to_owned(),
    };
    !name.contains(['/', '\\', '\0'])
        && Path::new(name).file_name() == Some(OsStr::new(name))
        && name
            .strip_prefix(stem)
            .and_then(|rest| rest.strip_prefix(" ("))
            .is_some_and(|rest| rest.ends_with(&tail) && rest.len() > tail.len())
}

impl Record {
    fn read(path: &Path, destination: &Path) -> Option<Record> {
        let (uuid, _) = private_parts(path.file_name()?.to_str()?, destination)?;
        // A tail torn inside a character is not a whole line either.
        let steps = lines(&String::from_utf8_lossy(&fs::read(path).ok()?));
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
            || !steps[1..]
                .iter()
                .all(|step| known_step(step, destination, &staged, &aside))
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
            // The last line about a file says what it was retained with.
            retained: [
                &staged,
                &aside,
                &staged.with_extension(PARTIAL),
                &staged.with_extension(GONE),
            ]
            .into_iter()
            .filter_map(|path| {
                let hash = steps.iter().rev().find_map(|s| {
                    let text = |key: &str| s.get(key).and_then(Value::as_str);
                    (text("step") == Some("retained")
                        && text("name").map(OsStr::new) == path.file_name())
                    .then(|| text("hash"))
                    .flatten()
                })?;
                Some((path.clone(), hash.to_owned()))
            })
            .collect(),
            staged,
            aside,
            steps,
        })
    }
    /// Whether `path` is retained under its private name.
    fn retains(&self, path: &Path) -> bool {
        self.retained.iter().any(|(retained, _)| retained == path)
    }
    /// The private names this record may name a file by: the staged file,
    /// the file moved aside, a file taken back from under a public name, and
    /// a version captured to be disposed of.
    fn names(&self) -> [PathBuf; 4] {
        [
            self.staged.clone(),
            self.aside.clone(),
            self.path.with_extension(PARTIAL),
            self.gone(),
        ]
    }
    fn gone(&self) -> PathBuf {
        self.path.with_extension(GONE)
    }
    /// Whether everything this record still names is a retained file:
    /// nothing to settle, only to prune later.
    fn only_retains(&self) -> bool {
        (!self.retained.is_empty() || !self.notices().is_empty())
            && self
                .names()
                .iter()
                .all(|p| self.retains(p) || matches!(lstat_file(p), Ok(None)))
    }
    fn partial(&self) -> PathBuf {
        self.path.with_extension(PARTIAL)
    }
    /// The last file this record moved off a public name: the name, and the
    /// file it was an incomplete copy of (none for a file moved off because
    /// it does not open).
    fn withdrawing(&self) -> Option<(PathBuf, Option<PathBuf>)> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let step = self
            .steps
            .iter()
            .rev()
            .find(|s| s.get("step").and_then(Value::as_str) == Some("withdraw"))?;
        let name = |key: &str| step.get(key).and_then(Value::as_str).map(|n| dir.join(n));
        Some((name("from")?, name("whole")))
    }
    /// What the version captured under [`Self::gone`] was to hold: the last
    /// disposal this record began.
    fn disposing(&self) -> Option<(String, Option<u64>)> {
        let step = self
            .steps
            .iter()
            .rev()
            .find(|s| s.get("step").and_then(Value::as_str) == Some("dispose"))?;
        Some((
            step.get("hash")?.as_str()?.to_owned(),
            step.get("size")
                .and_then(Value::as_str)
                .and_then(|size| size.parse().ok()),
        ))
    }
    /// What this record has to tell the person and has not been seen yet:
    /// each file a save or a recovery kept beside the project (why, and
    /// where), and `restored` (no path) when the last saved version was put
    /// back under the project's name. A file counts as kept from the line
    /// written once it had its name; or, if the process died right then,
    /// from the line written before, when the file is under that name and
    /// gone from its private one.
    fn notices(&self) -> Vec<(String, Option<PathBuf>)> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let text =
            |step: &Value, key: &str| step.get(key).and_then(Value::as_str).map(str::to_owned);
        let is = |step: &Value, name: &str| step.get("step").and_then(Value::as_str) == Some(name);
        let mut told: Vec<(String, Option<PathBuf>)> = Vec::new();
        let mut push = |why: String, path: Option<PathBuf>| {
            if !told.iter().any(|(_, known)| *known == path) {
                told.push((why, path));
            }
        };
        for (at, step) in self.steps.iter().enumerate() {
            if is(step, "kept") {
                if let (Some(why), Some(to)) = (text(step, "why"), text(step, "to")) {
                    push(why, Some(dir.join(to)));
                }
            } else if is(step, "restored") {
                push("restored".into(), None);
            } else if is(step, "left") {
                // Could not be kept beside the project: in the Trash, or
                // still under its private name. The line is written before
                // the move to the Trash; a file still under its name never
                // got there. A later attempt to keep it, or a later word on
                // where it is, is what counts.
                let (Some(name), Some(place)) = (text(step, "name"), text(step, "where")) else {
                    continue;
                };
                let superseded = self.steps[at + 1..].iter().any(|later| {
                    (is(later, "keep") || is(later, "left"))
                        && [text(later, "from"), text(later, "name")].contains(&Some(name.clone()))
                });
                if superseded {
                    continue;
                }
                let still_there = !matches!(lstat_file(&dir.join(&name)), Ok(None));
                let why = if place == "trash" && !still_there {
                    "trashed"
                } else {
                    "hidden"
                };
                push(why.into(), Some(dir.join(name)));
            } else if is(step, "keep") {
                let (Some(from), Some(to), Some(why)) =
                    (text(step, "from"), text(step, "to"), text(step, "why"))
                else {
                    continue;
                };
                // The last word about that file: no later attempt to keep
                // it, and it was not given up on (`left`).
                let last = !self.steps[at + 1..].iter().any(|later| {
                    (is(later, "keep") && text(later, "from").as_ref() == Some(&from))
                        || (is(later, "left") && text(later, "name").as_ref() == Some(&from))
                });
                if last
                    && matches!(lstat_file(&dir.join(&from)), Ok(None))
                    && matches!(lstat_file(&dir.join(&to)), Ok(Some(_)))
                {
                    push(why, Some(dir.join(to)));
                }
            }
        }
        told.retain(|(_, path)| {
            !self.steps.iter().any(|step| {
                is(step, "seen")
                    && match path {
                        Some(path) => {
                            text(step, "to").map(|to| dir.join(to)).as_ref() == Some(path)
                        }
                        None => text(step, "to").is_none(),
                    }
            })
        });
        told
    }
    /// Whether the file now under the project's name (`hash`) is one this
    /// record knows: the version its save replaced, the one it wrote, or
    /// what the name held when the record was last settled. Its notices are
    /// about that project, not about whatever else may be put under the
    /// name later.
    fn knows(&self, hash: &str) -> bool {
        self.old.as_deref() == Some(hash)
            || self.new == hash
            || self.steps.iter().rev().any(|step| {
                step.get("step").and_then(Value::as_str) == Some("settled")
                    && step.get("holds").and_then(Value::as_str) == Some(hash)
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
    hooks: PublishHooks,
    /// A file this record names is retained under its private name, or
    /// could not be given the name it should have: the record stays with it.
    retains: bool,
    /// The record holds something the person has not been told (a file
    /// kept beside the project, the last saved version put back): it stays
    /// until that was seen ([`kept_seen`]).
    tells: bool,
    /// Whether this is the save itself (not a later recovery from its
    /// record): another version it finds at the path arrived during it.
    live: bool,
    /// What the project's folder answered when it was synced as the save
    /// began ([`Self::folder`]).
    folder: Folder,
    /// The folder failed to sync after the install: why.
    pub unsynced: Option<String>,
    /// The hash and size of the file this save staged.
    new: (String, u64),
}

/// What became of a file [`Transaction::dispose`] did not remove.
#[derive(Debug, Default)]
pub(super) struct Kept {
    /// Kept beside the project under a plain name; the record tells.
    pub beside: Vec<PathBuf>,
    /// Could not be kept there: where each is now (the Trash, or the
    /// private name the record still names it by).
    pub elsewhere: Vec<PathBuf>,
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
        let mut owner = Owner::this(host.profile);
        if let Some(other) = &host.hooks.owner_host {
            (owner.host, owner.computer) = (other.clone(), other.clone());
        }
        begin.insert("owner", owner.to_value());
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
            // Its name must be there after a power cut too. What the folder
            // answers here is what it is held to after the install.
            .and_then(|()| folder_sync(destination.parent().unwrap_or(Path::new(".")), host.hooks));
        let folder = match written {
            Ok(folder) => folder,
            Err(e) => {
                let _ = fs::remove_file(&record);
                return Err(e);
            }
        };
        let tx = Transaction {
            file,
            record,
            destination: destination.to_owned(),
            hooks: host.hooks.clone(),
            retains: false,
            tells: false,
            live: true,
            folder,
            unsynced: None,
            new: (new.to_owned(), size),
        };
        tx.reached("begin");
        Ok(tx)
    }

    /// Continues the record of a save that was cut short.
    fn resume(record: &Record, destination: &Path, hooks: &PublishHooks) -> io::Result<Self> {
        let mut tx = Transaction {
            file: append_to(&record.path)?,
            record: record.path.clone(),
            destination: destination.to_owned(),
            hooks: hooks.clone(),
            retains: !record.retained.is_empty(),
            tells: !record.notices().is_empty(),
            live: false,
            unsynced: None,
            folder: folder_sync(destination.parent().unwrap_or(Path::new(".")), hooks)
                .map_err(|_| io::Error::other("the folder cannot be synced"))?,
            new: (record.new.clone(), record.size),
        };
        // A disposal cut short after the version was captured: judged now
        // as it would have been then.
        let gone = record.gone();
        if !record.retains(&gone) && !matches!(lstat_file(&gone), Ok(None)) {
            let judged = record.disposing().is_some_and(|(hash, size)| {
                dispose_captured(&gone, &gone, &holds_hash(&hash, size), hooks)
            });
            if !judged {
                tx.keep_changed(vec![gone], None);
            }
        }
        // A file moved off a public name and not judged yet: judged now.
        let partial = record.partial();
        if !record.retains(&partial) && !matches!(lstat_file(&partial), Ok(None)) {
            match record.withdrawing() {
                Some((from, whole)) => tx.judge_withdrawn(&from, whole.as_deref(), None),
                None => {
                    tx.keep_changed(vec![partial], None);
                }
            }
        }
        Ok(tx)
    }

    /// Puts the file `from` under the name `to`, which nothing may hold
    /// ([`place_new`]). A copy that failed part way is taken back from
    /// under the name ([`Self::withdraw`]).
    pub fn place(
        &mut self,
        from: &Path,
        to: &Path,
        hash: Option<&str>,
    ) -> Result<Placed, ProjectIoError> {
        let hooks = self.hooks.clone();
        place_new(from, to, hash, &hooks).map_err(|unplaced| {
            if unplaced.partial {
                self.withdraw(to, Some(from), unplaced.created.as_ref());
            }
            unplaced.error
        })
    }

    /// Moves the file under the public name `to` to the record's own
    /// private name, written into the record before the move, and says
    /// where it is now. `whole` is the file it is an incomplete copy of,
    /// when that is why it is moved. `None`: nothing was moved (an earlier
    /// file is still under that private name, the record cannot be written,
    /// or the name holds nothing).
    pub fn capture(&mut self, to: &Path, whole: Option<&Path>) -> Option<PathBuf> {
        let partial = self.record.with_extension(PARTIAL);
        if !matches!(lstat_file(&partial), Ok(None)) {
            self.strand();
            return None;
        }
        let name = |path: &Path| {
            Value::from(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let mut members = Map::new();
        members.insert("from", name(to));
        // Named when it is one of the record's own files; another file (a
        // version from the backups) is known only while this call runs.
        if let Some(whole) = whole.filter(|whole| {
            [STAGED, ASIDE, GONE]
                .iter()
                .any(|e| self.record.with_extension(e) == **whole)
        }) {
            members.insert("whole", name(whole));
        }
        self.step_with("withdraw", members).ok()?;
        rename_private(to, &partial, &self.hooks).ok()?;
        if let Some(hook) = &self.hooks.after_withdrawn {
            hook(to);
        }
        Some(partial)
    }

    /// Takes back the incomplete copy of `whole` under the public name
    /// `to`, a name that held nothing before the copy began. Nothing is
    /// removed by that name: whatever it holds is first moved to the
    /// record's private name ([`Self::capture`]) and judged there
    /// ([`Self::judge_withdrawn`]). `created` is the file the copy made,
    /// when this process made it.
    pub fn withdraw(&mut self, to: &Path, whole: Option<&Path>, created: Option<&FileStat>) {
        if self.capture(to, whole).is_some() {
            self.judge_withdrawn(to, whole, created);
        }
    }

    /// Judges the file moved off the public name `to` to the record's
    /// private name. It is the incomplete copy of `whole` only if it is the
    /// very file the copy created (`created`) or carries this save's mark,
    /// and its bytes are a beginning of `whole`'s: then it is removed
    /// (`whole` has all of them). Anything else landed there or was written
    /// into since: it is kept beside the project and told.
    fn judge_withdrawn(&mut self, to: &Path, whole: Option<&Path>, created: Option<&FileStat>) {
        let partial = self.record.with_extension(PARTIAL);
        let hooks = self.hooks.clone();
        let ours = whole.is_some_and(|whole| {
            let made_here = created.is_some_and(|created| {
                matches!(lstat_file(&partial), Ok(Some(now)) if now.same_identity(created))
            });
            (made_here || copy_of(&partial, whole))
                && dispose_captured(&partial, to, &|part, _| begins_file(whole, part), &hooks)
        });
        if !ours {
            self.keep_changed(vec![partial], None);
        }
    }

    /// How far the project's folder could be synced when the save began.
    pub fn folder(&self) -> Folder {
        self.folder
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
        if self
            .hooks
            .record_fails
            .as_ref()
            .is_some_and(|fails| fails(step))
        {
            return Err(io_err(
                &self.record,
                io::Error::other("the record cannot be written"),
            ));
        }
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

    /// A file this save's record names stays under its private name though
    /// it should not (it could not be put back, or given a name the person
    /// finds): the record stays, and the next open settles it.
    pub fn strand(&mut self) {
        self.retains = true;
    }

    /// The name this save moves the replaced file aside to.
    pub fn aside(&self) -> PathBuf {
        self.record.with_extension(ASIDE)
    }

    /// The kept file `path` needs no telling any more.
    pub fn seen(&mut self, path: &Path) {
        let mut members = Map::new();
        if let Some(name) = path.file_name() {
            members.insert("to", Value::from(name.to_string_lossy().into_owned()));
        }
        let _ = self.step_with("seen", members);
    }

    /// The name this save staged its new file under.
    pub fn staged(&self) -> PathBuf {
        self.record.with_extension(STAGED)
    }

    /// Records that the file under the private name `path` stays there,
    /// holding `hash` (read now when the caller did not): a version a save
    /// replaced, or the original of a file that could only be copied to its
    /// new name. It is not removed now; a later save's retention pass reads
    /// it again and removes it only if it still holds exactly that
    /// ([`prune_retained`]).
    pub fn retain(&mut self, path: &Path, hash: Option<&str>) {
        let Some(hash) = hash
            .map(str::to_owned)
            .or_else(|| super::durable::hash_file(path).ok())
        else {
            // Not readable now: it stays, named by the record, unpruned.
            self.retains = true;
            return;
        };
        let mut members = Map::new();
        members.insert(
            "name",
            Value::from(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        members.insert("hash", Value::from(hash));
        let _ = self.step_with("retained", members);
        self.retains = true;
    }

    /// Gives `held`, a file under a private name, a name the person finds
    /// beside the project ([`Self::keep_file`]), recording each name before it
    /// is tried. `label` is `other version` (a file that was under the
    /// project's name and is not the version the save could replace) or
    /// `recovered` (this save's own finished file). Where the name could
    /// only be given to a copy, another version's original stays under its
    /// private name, retained; the save's own staged file goes.
    pub fn keep(&mut self, held: PathBuf, label: &str) -> Result<PathBuf, PathBuf> {
        let (to, placed) = self
            .keep_file(held.clone(), label)
            .inspect_err(|_| self.strand())?;
        if placed == Placed::Copied {
            if label == "recovered" {
                // The save's own finished file, whose bytes the new name
                // holds: it goes only if it still holds exactly those.
                let (new, size) = self.new.clone();
                self.dispose(&held, &new, Some(size));
            } else {
                self.retain(&held, None);
            }
        }
        Ok(to)
    }

    /// Disposes of `path`, a file this record names that should hold
    /// exactly `hash` (and be `size` long), recording it before the file is
    /// captured ([`dispose`]). Whatever is not removed (the file, changed or
    /// unreadable, or one that landed under its name) is never unlinked: it
    /// is kept beside the project as "other version" and told, else it goes
    /// to the Trash, else it stays under a private name this record names.
    /// The result is empty when the file is gone.
    pub fn dispose(&mut self, path: &Path, hash: &str, size: Option<u64>) -> Kept {
        let gone = self.record.with_extension(GONE);
        let unchanged = holds_hash(hash, size);
        // A file retained under the capture name itself (it could only be
        // copied to its kept name) is read where it is.
        if path == gone {
            let hooks = self.hooks.clone();
            return match dispose_captured(&gone, &gone, &unchanged, &hooks) {
                true => Kept::default(),
                false => self.keep_changed(vec![gone], None),
            };
        }
        let mut members = Map::new();
        members.insert(
            "name",
            Value::from(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        members.insert("hash", Value::from(hash));
        if let Some(size) = size {
            members.insert("size", Value::from(size.to_string()));
        }
        if self.step_with("dispose", members).is_err() {
            // Not recorded, so not begun: the file stays as it is.
            self.strand();
            return Kept {
                elsewhere: vec![path.to_owned()],
                ..Default::default()
            };
        }
        let hooks = self.hooks.clone();
        let left = dispose(path, &gone, &unchanged, &hooks);
        self.keep_changed(left, Some(path))
    }

    /// Keeps the files a disposal did not remove (see [`Self::dispose`]).
    /// One captured under the record's capture name whose kept name holds a
    /// copy goes back under `own`, the private name it had, where that is
    /// free, and stays retained there.
    fn keep_changed(&mut self, left: Vec<PathBuf>, own: Option<&Path>) -> Kept {
        let mut kept = Kept::default();
        let gone = self.record.with_extension(GONE);
        for file in left {
            match self.keep_as(file.clone(), "other version", "changed") {
                Ok((beside, placed)) => {
                    kept.beside.push(beside);
                    if placed == Placed::Copied {
                        let back = own.filter(|own| {
                            file == gone
                                && matches!(lstat_file(own), Ok(None))
                                && rename_private(&file, own, &self.hooks).is_ok()
                        });
                        self.retain(back.unwrap_or(&file), None);
                    }
                }
                Err(file) => {
                    // Where it goes is recorded before it goes, so a crash
                    // right after the move still tells it. A file whose
                    // fate cannot be recorded stays under the private name
                    // the record already gives it, where the next open
                    // takes it up again.
                    let name = |file: &Path| {
                        Value::from(
                            file.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        )
                    };
                    let left = |place: &str| {
                        let mut members = Map::new();
                        members.insert("name", name(&file));
                        members.insert("where", Value::from(place));
                        members
                    };
                    let to_trash = self.hooks.trash.is_some();
                    let recorded = self
                        .step_with("left", left(if to_trash { "trash" } else { "hidden" }))
                        .is_ok();
                    let now = if recorded {
                        trash_or_leave(file.clone(), &self.hooks)
                    } else {
                        file.clone()
                    };
                    let trashed = now != file;
                    if !trashed {
                        self.strand();
                        if recorded && to_trash {
                            // Told as hidden even where this line is lost:
                            // the file is still under its name.
                            let _ = self.step_with("left", left("hidden"));
                        }
                    }
                    self.tells |= recorded;
                    if let Ok(mut strays) = self.hooks.strays.lock() {
                        strays.push(Stray {
                            project: self.destination.clone(),
                            path: now.clone(),
                            trashed,
                            recorded,
                        });
                    }
                    kept.elsewhere.push(now);
                }
            }
        }
        kept
    }

    /// [`Self::keep`] without deciding what becomes of the file under its
    /// old name when the new one holds a copy: says how it was placed.
    pub fn keep_file(&mut self, held: PathBuf, label: &str) -> Result<(PathBuf, Placed), PathBuf> {
        let why = match label {
            "recovered" => "unfinished",
            // Found at the path by the save itself: it arrived during it.
            _ if self.live => "arrived",
            _ => "other",
        };
        self.keep_as(held, label, why)
    }

    /// [`Self::keep_file`] with the reason the person is told: `changed`
    /// for a version that was to be removed and was found written into.
    fn keep_as(
        &mut self,
        held: PathBuf,
        label: &str,
        why: &str,
    ) -> Result<(PathBuf, Placed), PathBuf> {
        let kept = self.name_beside(held, label, why)?;
        // It has its name: the person is to be told, and the record keeps
        // that until they were.
        let mut members = Map::new();
        members.insert(
            "to",
            Value::from(
                kept.0
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        members.insert("why", Value::from(why));
        let _ = self.step_with("kept", members);
        self.tells = true;
        Ok(kept)
    }

    /// Gives `held` the name `<project> (<label> <date time>).<ext>` beside
    /// the project, never over anything ([`Self::place`]): each name is
    /// recorded right before it is tried, and a taken one is passed over.
    /// `Err` with its private name when it could not be given another.
    fn name_beside(
        &mut self,
        held: PathBuf,
        label: &str,
        why: &str,
    ) -> Result<(PathBuf, Placed), PathBuf> {
        let destination = self.destination.clone();
        let dir = destination.parent().unwrap_or(Path::new("."));
        let stem = destination
            .file_stem()
            .unwrap_or("project".as_ref())
            .to_string_lossy();
        let stamp = local_stamp();
        for n in 1..=50 {
            let mut name = format!("{stem} ({label} {stamp}");
            if n > 1 {
                name.push_str(&format!(" {n}"));
            }
            name.push(')');
            if let Some(extension) = destination.extension() {
                name.push('.');
                name.push_str(&extension.to_string_lossy());
            }
            let to = dir.join(name);
            if self.keeping(&held, &to, why).is_err() {
                break;
            }
            match self.place(&held, &to, None) {
                Ok(placed) => return Ok((to, placed)),
                Err(ProjectIoError::DestinationExists(_)) => continue,
                Err(_) => break,
            }
        }
        Err(held)
    }

    /// The last saved version is back under the project's name: the person
    /// is to be told.
    pub fn restored(&mut self) {
        let _ = self.step("restored");
        self.tells = true;
    }

    /// Everything this record names is where it will stay. `holds` is what
    /// the project's name holds now, which is the project the record's
    /// notices are about ([`Record::knows`]).
    pub fn settled(&mut self, holds: Option<&str>) {
        if self.tells {
            let mut members = Map::new();
            if let Some(holds) = holds {
                members.insert("holds", Value::from(holds));
            }
            let _ = self.step_with("settled", members);
        }
    }

    /// The save has nothing left to do: its record goes, unless a file it
    /// names is retained under its private name.
    pub fn finish(self) {
        if !self.retains && !self.tells {
            let _ = fs::remove_file(&self.record);
        }
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
                && [STAGED, ASIDE, PARTIAL, GONE, RECORD].contains(&extension)
                && rest
                    .len()
                    .checked_sub(36)
                    .and_then(|at| rest.get(at..))
                    .is_some_and(is_uuid_v4)
        })
}

/// Whether a save of `destination` was cut short and left its record:
/// something to settle before the file is opened or saved again. (A record
/// that only keeps a replaced version, or a notice, is at rest.)
pub(crate) fn pending(destination: &Path) -> bool {
    let (records, unreadable) = records_of(destination);
    !unreadable.is_empty() || records.iter().any(|r| !r.only_retains())
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
    /// The save is going on right now: the process that began it is
    /// running on this machine. Its files are not for anyone else to move.
    Active,
    /// The save was begun on another machine. From here it cannot be told
    /// whether that machine crashed or is still at it (its files reach this
    /// one on a shared volume, or in a folder on this machine's own disk
    /// that a sync client fills), so nothing is settled unasked: the person
    /// is told who and since when, and chooses.
    Elsewhere,
    /// Not undecided: a whole version a save's record proves (read now, it
    /// holds exactly the bytes the record names), offered when the file at
    /// the project's name does not open.
    Proven,
}

/// Whose save `record` is, where that keeps recovery from acting on it.
/// Another machine's is never acted on unasked, whatever the volume: that
/// a folder is on a local disk does not say no other machine writes it (a
/// sync client does), and nothing read here says that machine has stopped.
fn held_by_another(record: &Record) -> Option<Undecided> {
    if record.owner.is_alive_here() && !record.owner.is_this_process() {
        Some(Undecided::Active)
    } else if !record.owner.is_here() {
        Some(Undecided::Elsewhere)
    } else {
        None
    }
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
    /// Read now and found to hold exactly the bytes the save's record
    /// names for it (then `role` says which version by those bytes). Only
    /// such a file is a version to offer; any other is a file that could
    /// not be identified.
    pub proven: bool,
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
    /// The folder could not be synced after recovery's moves: they may not
    /// survive a power cut. Whoever acts on the result says so.
    pub unsynced: Option<String>,
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
    for record in &records {
        named.push(record.path.clone());
        named.extend(record.names());
    }
    // A record that only keeps a replaced version is not a cut-short save.
    let records: Vec<_> = records.into_iter().filter(|r| !r.only_retains()).collect();
    let several = records.len() > 1;
    // Recovery's moves are made inside a coordinated write where the system
    // grants one; nothing is asked for when there is nothing to settle.
    let settle_all = |left: &mut Vec<LeftAside>, undecided: &mut Vec<Leftover>| {
        for record in &records {
            let unresolved = if let Some(held) = held_by_another(record) {
                Some(held)
            } else if several {
                Some(Undecided::Several)
            } else {
                settle_one(record, destination, host, left)
            };
            if let Some(why) = unresolved {
                for (path, role) in [(&record.aside, "previous"), (&record.staged, "saving")] {
                    if !record.retains(path) && matches!(lstat_file(path), Ok(Some(_)) | Err(_)) {
                        undecided.push(leftover(path, role, why, Some(record)));
                    }
                }
            }
        }
    };
    if !records.is_empty() {
        let (mut left, mut undecided) = (Vec::new(), Vec::new());
        let mut once = Some(());
        host.coordinated(destination, &mut |_| {
            if once.take().is_some() {
                probing(|| settle_all(&mut left, &mut undecided));
            }
        });
        settled.left = left;
        settled.undecided.extend(undecided);
    }
    settled.unrecorded = private_files(destination)
        .into_iter()
        .filter(|(path, _)| !named.contains(path) && fs::metadata(path).is_ok_and(|m| m.len() > 0))
        .map(|(path, _)| path)
        .collect();
    if !records.is_empty()
        && let Err(e) = folder_sync(destination.parent().unwrap_or(Path::new(".")), host.hooks)
    {
        settled.unsynced = Some(e.to_string());
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
        proven: false,
    }
}

/// [`leftover`] for one of `record`'s own files: proven by its bytes where
/// they are what the record says, and then named by them.
fn own_leftover(path: &Path, role: &'static str, why: Undecided, record: &Record) -> Leftover {
    let (role, proven) = match proves(path, record) {
        Holds::New => ("saving", true),
        Holds::Old => ("previous", true),
        _ => (role, false),
    };
    Leftover {
        proven,
        ..leftover(path, role, why, Some(record))
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
    take_back_copies(record, destination, &mut tx);
    let mut at_path = holds(destination, record);
    let aside = holds(&record.aside, record);
    let staged = holds(&record.staged, record);
    let old = record.old.as_deref();
    let mut unresolved = None;

    // The last saved version goes back under a name that holds nothing.
    let mut aside_settled = aside == Holds::Nothing || record.retains(&record.aside);
    if !aside_settled && at_path == Holds::Nothing && aside == Holds::Old {
        if tx.step("restore").is_err() {
            return Some(Undecided::Unknown);
        }
        match tx.place(&record.aside, destination, old) {
            Ok(placed) => {
                // Back as a copy: the file itself stays, for a later save
                // to read again and prune.
                if placed == Placed::Copied {
                    tx.retain(&record.aside, old);
                }
                tx.restored();
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
        tx.restored();
        left.push(LeftAside::Restored);
    }
    if !aside_settled {
        aside_settled = match (at_path, aside) {
            // The version the save replaced (the path holds the save), or a
            // second copy of what the path holds: retained, not removed.
            (Holds::Old | Holds::New, Holds::Old) => {
                tx.retain(&record.aside, old);
                true
            }
            // The path holds the save, or something else took it: a file
            // that was under the project's name is kept where it is found.
            (Holds::New, _) | (Holds::Other, Holds::Old) => tx
                .keep(record.aside.clone(), "other version")
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
        _ if record.retains(&record.staged) => true,
        // Installed by a copy: the save's own staged file, whose bytes the
        // path holds. Removed if it still holds exactly those when it goes;
        // otherwise kept beside the project and told.
        (Holds::New, Holds::New) => {
            let kept = tx.dispose(&record.staged, &record.new, Some(record.size));
            left.extend(kept.beside.into_iter().map(LeftAside::Kept));
            true
        }
        // The exchange displaced the version the save replaced: retained,
        // under the name a replaced file has.
        (Holds::New | Holds::Old, Holds::Old) => {
            let free = matches!(lstat_file(&record.aside), Ok(None));
            if free && fs::rename(&record.staged, &record.aside).is_ok() {
                tx.retain(&record.aside, old);
            } else {
                tx.retain(&record.staged, old);
            }
            true
        }
        // An exchange hands back whatever was under the name.
        (Holds::New, Holds::Other) | (Holds::Other, Holds::Old) if record.exchange => tx
            .keep(record.staged.clone(), "other version")
            .map(|kept| left.push(LeftAside::Kept(kept)))
            .is_ok(),
        // The version the save was replacing is nowhere: not decided here.
        (Holds::Nothing, Holds::New) if record.old.is_some() => false,
        (_, Holds::New) if aside_settled => tx
            .keep(record.staged.clone(), "recovered")
            .map(|kept| left.push(LeftAside::Unfinished(kept)))
            .is_ok(),
        _ => false,
    };
    if !staged_settled {
        unresolved = Some(Undecided::Unknown);
    }
    if unresolved.is_none() {
        let holds = match at_path {
            Holds::Old => record.old.clone(),
            Holds::New => Some(record.new.clone()),
            Holds::Other => super::durable::hash_file(destination).ok(),
            Holds::Nothing => None,
        };
        tx.settled(holds.as_deref());
        tx.finish();
    }
    unresolved
}

/// A save of the project stands: the file under its name went from `old`
/// to `new`. Records beside it that still have something to tell and knew
/// the old file learn the new one, so their notices stay this project's
/// until they were seen.
pub(super) fn carry_notices(destination: &Path, current: &Transaction, old: &str, new: &str) {
    for record in records_of(destination).0 {
        if record.path == current.record || record.notices().is_empty() || !record.knows(old) {
            continue;
        }
        if let Ok(mut file) = append_to(&record.path) {
            let mut members = Map::new();
            members.insert("step", Value::from("settled"));
            members.insert("holds", Value::from(new));
            let _ = file
                .write_all(line(&Value::Object(members)).as_bytes())
                .and_then(|()| sync_file(&file));
        }
    }
}

/// What the records beside `destination` have to tell the person and have
/// not been seen: (why, where) of each file a save or a recovery kept
/// beside the project (`arrived`, `other`, `unfinished`, `changed`,
/// `trashed`, `hidden`), and `restored`.
///
/// A record that knows the file now under the project's name (`hash`; see
/// [`Record::knows`]) tells it as it happened. One that does not (another
/// program changed the project since, or another project was put under the
/// name) still has a kept file nobody was told about: its notices are told
/// as `earlier`, a file an earlier save under this name kept, without a
/// claim about the file that is open. Nothing goes untold for want of a
/// matching hash.
pub(crate) fn kept_notices(
    destination: &Path,
    hash: Option<&str>,
) -> Vec<(String, Option<PathBuf>)> {
    let mut notices = Vec::new();
    for record in records_of(destination).0 {
        let knows = hash.is_none_or(|hash| record.knows(hash));
        notices.extend(record.notices().into_iter().map(|(why, path)| {
            if knows {
                (why, path)
            } else {
                ("earlier".to_owned(), path)
            }
        }));
    }
    notices
}

/// The person has seen the notices about the kept files `paths` (and, with
/// `restored`, that the last saved version was put back). Each is marked
/// seen in its record; a record with nothing left to tell and nothing
/// retained goes.
pub(crate) fn kept_seen(destination: &Path, paths: &[PathBuf], restored: bool) {
    for record in records_of(destination).0 {
        let told = record.notices();
        let seen: Vec<_> = told
            .iter()
            .filter(|(_, path)| match path {
                Some(path) => paths.contains(path),
                None => restored,
            })
            .collect();
        if seen.is_empty() {
            continue;
        }
        let Ok(mut file) = append_to(&record.path) else {
            continue;
        };
        for (_, path) in &seen {
            let mut members = Map::new();
            members.insert("step", Value::from("seen"));
            if let Some(name) = path.as_ref().and_then(|p| p.file_name()) {
                members.insert("to", Value::from(name.to_string_lossy().into_owned()));
            }
            let _ = file.write_all(line(&Value::Object(members)).as_bytes());
        }
        let _ = sync_file(&file);
        let left = record
            .names()
            .iter()
            .any(|p| !matches!(lstat_file(p), Ok(None)));
        if seen.len() == told.len() && !left {
            let _ = fs::remove_file(&record.path);
        }
    }
}

/// The retention pass a save runs once it stands: every version an earlier
/// save of `destination` replaced and retained beside it (and every
/// original of a file that could only be copied to its name) is read
/// again. One that still holds exactly the bytes it was retained with is
/// removed in the same call as that read ([`dispose`]). One that changed
/// since (something wrote into it after it was replaced) holds work nobody
/// read: it is kept beside the project as "other version" (recorded in the
/// record that names it) and returned; where it cannot be
/// kept there it goes to the Trash (or stays) and is reported as a
/// [`Stray`]. `current` is the record of the save that runs the pass.
pub(super) fn prune_retained(
    destination: &Path,
    current: &Transaction,
    hooks: &PublishHooks,
) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    let (records, _) = records_of(destination);
    for record in records {
        if record.path == current.record || !record.only_retains() || record.retained.is_empty() {
            continue;
        }
        let Ok(mut tx) = Transaction::resume(&record, destination, hooks) else {
            continue;
        };
        for (path, hash) in &record.retained {
            if matches!(lstat_file(path), Ok(None)) {
                continue;
            }
            // Read again as it goes, and removed only if unchanged. One
            // that something wrote into since it was retained holds work
            // nobody read: kept beside the project and told.
            changed.extend(tx.dispose(path, hash, None).beside);
        }
        // The record goes with its last file, unless it has something to
        // tell that was not seen yet (a file just kept is one).
        let rest = Record::read(&record.path, destination).is_none_or(|r| {
            !r.notices().is_empty() || r.names().iter().any(|p| !matches!(lstat_file(p), Ok(None)))
        });
        if !rest {
            let _ = fs::remove_file(&record.path);
        }
    }
    changed
}

/// A copy that was cut short: the record says a file was about to be placed
/// under a name (the project's, by `install` or `restore`; a kept file's,
/// by `keep`), and that name holds a beginning of the file. It is taken
/// back ([`Transaction::withdraw`]); the whole file is still under its private name.
fn take_back_copies(record: &Record, destination: &Path, tx: &mut Transaction) {
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
        // Shorter than the whole file, or as long and still without its
        // first bytes, which a copy gets last.
        let shorter = match (fs::symlink_metadata(&to), fs::symlink_metadata(whole)) {
            (Ok(part), Ok(whole)) => {
                part.is_file()
                    && (part.len() < whole.len()
                        || (part.len() == whole.len() && head_held_back(&to)))
            }
            _ => false,
        };
        // The file under the name carries this save's mark, the whole file
        // is proven to be what the record says it is (the staged file, or
        // the version the save replaced), and the name holds a beginning of
        // it: only then is it a copy this save was making. Anything else
        // there is not this save's to take back.
        if shorter
            && copy_of(&to, whole)
            && matches!(holds(whole, record), Holds::New | Holds::Old)
            && begins(whole, &to)
        {
            tx.withdraw(&to, Some(whole), None);
        }
    }
}

/// The files recovery left for the person to choose from ([`Settled`]'s
/// `undecided`), without moving anything.
pub(crate) fn leftovers(destination: &Path) -> Vec<Leftover> {
    let (records, unreadable) = records_of(destination);
    let records: Vec<_> = records.into_iter().filter(|r| !r.only_retains()).collect();
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
        let why = held_by_another(record).unwrap_or(why);
        for (path, role) in [(&record.aside, "previous"), (&record.staged, "saving")] {
            if !record.retains(path) && matches!(lstat_file(path), Ok(Some(_))) {
                found.push(own_leftover(path, role, why, record));
            }
        }
    }
    // Whole versions any record proves, wherever they are now (under a
    // private name, retained, or kept beside the project), for when the file
    // under the project's name does not open.
    for record in records_of(destination).0 {
        if held_by_another(&record) == Some(Undecided::Active) {
            continue;
        }
        for path in proven_candidates(&record) {
            if found.iter().any(|l| l.path == path) {
                continue;
            }
            let row = own_leftover(&path, "", Undecided::Proven, &record);
            if row.proven {
                found.push(row);
            }
        }
    }
    found
}

/// Which of `record`'s two versions `path` holds, by its bytes alone: a
/// version offered to the person is what it holds, and a file a share is
/// still settling the times of right after it was written is that too.
fn proves(path: &Path, record: &Record) -> Holds {
    let Ok(Some(stat)) = lstat_file(path) else {
        return Holds::Nothing;
    };
    match super::durable::hash_file(path) {
        Ok(hash) if hash == record.new && stat.size == record.size => Holds::New,
        Ok(hash) if Some(&hash) == record.old.as_ref() => Holds::Old,
        _ => Holds::Other,
    }
}

/// The files `record` names that may hold one of its two versions: its own
/// private files and the names it gave kept files beside the project.
fn proven_candidates(record: &Record) -> Vec<PathBuf> {
    let dir = record.path.parent().unwrap_or(Path::new("."));
    let mut paths = vec![record.aside.clone(), record.staged.clone()];
    for step in &record.steps {
        let text = |key: &str| step.get(key).and_then(Value::as_str);
        if matches!(text("step"), Some("keep" | "kept"))
            && let Some(to) = text("to")
        {
            let path = dir.join(to);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
        .into_iter()
        .filter(|p| matches!(lstat_file(p), Ok(Some(_))))
        .collect()
}

/// The person chose `chosen`, a file a record of `destination` names, to
/// be the project file: it takes the project's name, which must hold
/// nothing, and the other files that record names get names the person
/// finds. Nothing is removed. Call it while holding the project's lock.
///
/// `chosen` must be a version: read now, it holds exactly the bytes the
/// record names (proven again right before it is placed). A file that does
/// not is made the project only with `unproven`, which says the person was
/// told it could not be identified and chose it all the same.
pub(crate) fn use_leftover(
    destination: &Path,
    chosen: &Path,
    unproven: bool,
    host: &Host,
) -> Result<Vec<LeftAside>, ProjectIoError> {
    let mut used = None;
    host.coordinated(destination, &mut |_| {
        if used.is_none() {
            used = Some(probing(|| {
                use_leftover_now(destination, chosen, unproven, host)
            }));
        }
    });
    used.unwrap_or_else(|| {
        Err(ProjectIoError::Invalid(
            "The file coordinator did not run.".into(),
        ))
    })
}

fn use_leftover_now(
    destination: &Path,
    chosen: &Path,
    unproven: bool,
    host: &Host,
) -> Result<Vec<LeftAside>, ProjectIoError> {
    let hooks = host.hooks;
    let (records, _) = records_of(destination);
    let record = records
        .iter()
        .find(|r| {
            r.staged == chosen
                || r.aside == chosen
                || (proven_candidates(r).iter().any(|p| p == chosen)
                    && matches!(proves(chosen, r), Holds::Old | Holds::New))
        })
        .ok_or_else(|| {
            ProjectIoError::Invalid("That file is not one a save of this project left.".into())
        })?;
    // A save that is going on right now is not for anyone to choose from.
    if held_by_another(record) == Some(Undecided::Active) {
        return Err(ProjectIoError::Invalid(
            "This project is being saved right now by another copy of Powermove on this Mac."
                .into(),
        ));
    }
    // What the chosen file must hold to be the version it is offered as.
    let version = |record: &Record| match proves(chosen, record) {
        Holds::New => Some(record.new.clone()),
        Holds::Old => record.old.clone(),
        _ => None,
    };
    let unidentified = || {
        ProjectIoError::Invalid(
            "That file does not hold what the save that left it recorded, so it is not a \
             version Powermove can vouch for. Nothing was changed."
                .into(),
        )
    };
    if version(record).is_none() && !unproven {
        return Err(unidentified());
    }
    let mut tx =
        Transaction::resume(record, destination, hooks).map_err(|e| io_err(&record.path, e))?;
    take_back_copies(record, destination, &mut tx);
    let mut left = Vec::new();
    // The name holds a file that does not open as a project (a copy cut
    // short and changed since, or a file no save of this one made): it is
    // not this save's to remove. It is moved to the record's own private
    // name and kept beside the project, and the chosen version takes the
    // name. A file there that opens is the project: nothing is changed.
    if lstat_file(destination)?.is_some() {
        if super::open_project(destination, super::OpenOptions::default()).is_ok() {
            return Err(ProjectIoError::Invalid(
                "The project's file opens as it is; nothing was changed.".into(),
            ));
        }
        // Recorded before it is moved, so a crash right then leaves a
        // file the record names.
        let Some(unreadable) = tx.capture(destination, None) else {
            return Err(ProjectIoError::Invalid(
                "The file under the project's name could not be moved aside; nothing was \
                 changed."
                    .into(),
            ));
        };
        match tx.keep(unreadable.clone(), "unreadable") {
            Ok(kept) => left.push(LeftAside::Kept(kept)),
            Err(_) => {
                // Still under the record's private name, which it names.
                tx.retain(&unreadable, None);
            }
        }
    }
    // Proven again at the last moment: it is placed as the bytes read now.
    let expected = version(record);
    if expected.is_none() && !unproven {
        return Err(unidentified());
    }
    tx.step("chosen")?;
    if tx.place(chosen, destination, expected.as_deref())? == Placed::Copied {
        // The name holds a verified copy: the file itself stays, for a
        // later save to read again and prune.
        tx.retain(chosen, None);
    }
    // A kept file that is the project now is nothing more to tell.
    if chosen != record.aside && chosen != record.staged {
        tx.seen(chosen);
    }
    let mut settled = true;
    for (path, label) in [
        (&record.aside, "other version"),
        (&record.staged, "recovered"),
    ] {
        if path != chosen && !record.retains(path) && matches!(lstat_file(path), Ok(Some(_))) {
            match tx.keep(path.clone(), label) {
                Ok(kept) if label == "recovered" => left.push(LeftAside::Unfinished(kept)),
                Ok(kept) => left.push(LeftAside::Kept(kept)),
                // Still under its private name, which the record names.
                Err(_) => settled = false,
            }
        }
    }
    if settled {
        tx.finish();
    }
    folder_sync(destination.parent().unwrap_or(Path::new(".")), hooks)?;
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
        for extension in [STAGED, ASIDE, PARTIAL, GONE, RECORD] {
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
