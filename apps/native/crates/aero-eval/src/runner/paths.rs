//! Every read, walk and write the runner makes on a run's files is anchored
//! at the reserved run folder: the folder is opened once, and each path
//! below it is reached one component at a time with `openat(…, O_NOFOLLOW)`
//! from the open folder above it. No path is checked and then opened again
//! by name, so a component swapped for a symlink between a check and an open
//! is refused, never followed, and a path the host merely names is read only
//! when it lies inside the run.
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// Largest file the runner reads back (a saved project, a frame).
pub const MAX_READ: u64 = 512 << 20;

/// Create a file that must not exist yet: an existing file or a link at
/// `path` (one a run component left in the run folder) is refused, never
/// written through. For files directly in the run folder, whose own path
/// the run cannot change.
pub fn create_new(path: &Path) -> Result<File, String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// What an entry of a folder is, without following it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

/// An open folder that every access below it goes through.
#[derive(Debug)]
pub struct Anchor {
    fd: OwnedFd,
    /// The folder's path as given (for messages and for mapping paths the
    /// host names onto it).
    pub path: PathBuf,
}

fn cstr(name: &OsStr) -> Result<CString, String> {
    CString::new(name.as_bytes()).map_err(|_| format!("{} has a NUL byte", name.display()))
}

fn err(at: &Path, e: std::io::Error) -> String {
    if e.raw_os_error() == Some(libc::ELOOP) || e.raw_os_error() == Some(libc::ENOTDIR) {
        format!(
            "{} is a symlink or not a folder; the runner does not follow links",
            at.display()
        )
    } else {
        format!("{}: {e}", at.display())
    }
}

fn openat(
    dir: libc::c_int,
    name: &CString,
    flags: libc::c_int,
    mode: libc::c_uint,
) -> std::io::Result<OwnedFd> {
    // SAFETY: a valid folder fd and a NUL-terminated name; the fd returned is
    // owned by the caller.
    let fd = unsafe {
        libc::openat(
            dir,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: a fresh fd from openat.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

const DIR_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY;

/// The plain components of a relative path (no `..`, no root, no `.`).
fn parts(rel: &Path) -> Result<Vec<&OsStr>, String> {
    if rel.as_os_str().is_empty() {
        return Ok(Vec::new());
    }
    rel.components()
        .map(|c| match c {
            Component::Normal(n) => Ok(n),
            _ => Err(format!(
                "{} is not a plain path inside the run",
                rel.display()
            )),
        })
        .collect()
}

impl Anchor {
    /// Open a folder (its last component must not be a symlink).
    pub fn open(dir: &Path) -> Result<Self, String> {
        let c = cstr(dir.as_os_str())?;
        let fd = openat(libc::AT_FDCWD, &c, DIR_FLAGS, 0).map_err(|e| err(dir, e))?;
        Ok(Self {
            fd,
            path: dir.to_path_buf(),
        })
    }

    /// The open folder `rel` below this one, every component unfollowed.
    pub fn dir(&self, rel: &Path) -> Result<Anchor, String> {
        let mut fd = self.fd.try_clone().map_err(|e| e.to_string())?;
        let mut at = self.path.clone();
        for n in parts(rel)? {
            at.push(n);
            fd = openat(fd.as_raw_fd(), &cstr(n)?, DIR_FLAGS, 0).map_err(|e| err(&at, e))?;
        }
        Ok(Anchor { fd, path: at })
    }

    /// The folder holding `rel` and its last component.
    fn parent_of<'a>(&self, rel: &'a Path) -> Result<(Anchor, &'a OsStr), String> {
        let p = parts(rel)?;
        let (last, head) = p
            .split_last()
            .ok_or_else(|| format!("{}: an empty path", self.path.display()))?;
        let parent: PathBuf = head.iter().collect();
        Ok((self.dir(&parent)?, last))
    }

    /// Read the regular file `rel` (at most [`MAX_READ`] bytes).
    pub fn read(&self, rel: &Path) -> Result<Vec<u8>, String> {
        let (dir, name) = self.parent_of(rel)?;
        let at = dir.path.join(name);
        let fd = openat(
            dir.fd.as_raw_fd(),
            &cstr(name)?,
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        )
        .map_err(|e| err(&at, e))?;
        let mut f = File::from(fd);
        let m = f.metadata().map_err(|e| err(&at, e))?;
        if !m.is_file() {
            return Err(format!("{} is not a regular file", at.display()));
        }
        if m.len() > MAX_READ {
            return Err(format!("{} is larger than {MAX_READ} bytes", at.display()));
        }
        let mut out = Vec::with_capacity(m.len() as usize);
        Read::by_ref(&mut f)
            .take(MAX_READ + 1)
            .read_to_end(&mut out)
            .map_err(|e| err(&at, e))?;
        if out.len() as u64 > MAX_READ {
            return Err(format!("{} is larger than {MAX_READ} bytes", at.display()));
        }
        Ok(out)
    }

    /// Create the file `rel`, which must not exist yet (nor be a link).
    pub fn create_new(&self, rel: &Path) -> Result<File, String> {
        let (dir, name) = self.parent_of(rel)?;
        let at = dir.path.join(name);
        openat(
            dir.fd.as_raw_fd(),
            &cstr(name)?,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o644,
        )
        .map(File::from)
        .map_err(|e| err(&at, e))
    }

    /// Write the new file `rel` and sync it.
    pub fn write_new(&self, rel: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut f = self.create_new(rel)?;
        f.write_all(bytes)
            .and_then(|()| f.sync_all())
            .map_err(|e| format!("{}: {e}", self.path.join(rel).display()))
    }

    /// Replace the file `rel` with `bytes`: written to a new file beside it,
    /// then renamed over it (a link at `rel` is replaced, never followed).
    pub fn replace(&self, rel: &Path, bytes: &[u8]) -> Result<(), String> {
        let (dir, name) = self.parent_of(rel)?;
        let mut tmp = name.to_os_string();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        dir.write_new(&tmp, bytes)?;
        // SAFETY: both names are NUL-terminated and relative to the same open folder.
        let r = unsafe {
            libc::renameat(
                dir.fd.as_raw_fd(),
                cstr(tmp.as_os_str())?.as_ptr(),
                dir.fd.as_raw_fd(),
                cstr(name)?.as_ptr(),
            )
        };
        if r != 0 {
            return Err(format!(
                "{}: {}",
                dir.path.join(name).display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    /// Make the folder `name` in this folder (a single component).
    pub fn mkdir(&self, name: &OsStr) -> Result<(), String> {
        parts(Path::new(name))?;
        // SAFETY: a valid folder fd and a NUL-terminated name.
        let r = unsafe { libc::mkdirat(self.fd.as_raw_fd(), cstr(name)?.as_ptr(), 0o755) };
        if r != 0 {
            return Err(err(&self.path.join(name), std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// The entries of this folder, sorted by name, each classified without
    /// following it.
    pub fn entries(&self) -> Result<Vec<(String, Kind)>, String> {
        let dup = self.fd.try_clone().map_err(|e| e.to_string())?;
        // SAFETY: fdopendir takes ownership of the duplicated fd; closedir
        // releases it.
        let dirp = unsafe { libc::fdopendir(std::os::fd::IntoRawFd::into_raw_fd(dup)) };
        if dirp.is_null() {
            return Err(err(&self.path, std::io::Error::last_os_error()));
        }
        // The duplicate shares the offset: start from the beginning.
        // SAFETY: dirp is a valid stream.
        unsafe { libc::rewinddir(dirp) };
        let mut names = Vec::new();
        loop {
            // SAFETY: dirp is valid until closedir; the entry is copied out at once.
            let e = unsafe { libc::readdir(dirp) };
            if e.is_null() {
                break;
            }
            // SAFETY: d_name is NUL-terminated.
            let name = unsafe { std::ffi::CStr::from_ptr((*e).d_name.as_ptr()) };
            let b = name.to_bytes();
            if b != b"." && b != b".." {
                names.push(name.to_owned());
            }
        }
        // SAFETY: closes the stream and its fd.
        unsafe { libc::closedir(dirp) };
        let mut out = Vec::new();
        for n in names {
            // SAFETY: a zeroed stat is a valid out-parameter.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: valid fd, NUL-terminated name, valid out-pointer.
            let r = unsafe {
                libc::fstatat(
                    self.fd.as_raw_fd(),
                    n.as_ptr(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            let name = String::from_utf8_lossy(n.to_bytes()).into_owned();
            if r != 0 {
                // Gone between the listing and the look: skip it.
                continue;
            }
            let kind = match st.st_mode & libc::S_IFMT {
                libc::S_IFREG => Kind::File,
                libc::S_IFDIR => Kind::Dir,
                libc::S_IFLNK => Kind::Symlink,
                _ => Kind::Other,
            };
            out.push((name, kind));
        }
        out.sort();
        Ok(out)
    }

    /// Every regular file below this folder, relative and sorted; a symlink
    /// anywhere is an error (nothing behind it is read).
    pub fn files(&self) -> Result<Vec<PathBuf>, String> {
        fn walk(a: &Anchor, rel: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
            for (name, kind) in a.entries()? {
                let r = rel.join(&name);
                match kind {
                    Kind::Symlink => {
                        return Err(format!("{} is a symlink", a.path.join(&name).display()));
                    }
                    Kind::Dir => walk(&a.dir(Path::new(&name))?, &r, out)?,
                    Kind::File => out.push(r),
                    Kind::Other => {}
                }
            }
            Ok(())
        }
        let mut out = Vec::new();
        walk(self, Path::new(""), &mut out)?;
        out.sort();
        Ok(out)
    }

    /// Hash of every file below this folder (dot files and folders skipped),
    /// as `generate.py` computes it: sha256 over sorted `rel\0sha256\n` lines.
    pub fn tree_hash(&self) -> Result<String, String> {
        let mut lines = Vec::new();
        for rel in self.files()? {
            if rel
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            {
                continue;
            }
            let bytes = self.read(&rel)?;
            lines.push((
                rel.to_string_lossy().replace('\\', "/"),
                crate::sha256_hex(&bytes),
            ));
        }
        lines.sort();
        let text: String = lines.iter().map(|(r, s)| format!("{r}\0{s}\n")).collect();
        Ok(crate::sha256_hex(text.as_bytes()))
    }

    /// A path the host named, as a plain relative path inside this folder
    /// (lexically; the open that follows is anchored and unfollowed). `also`
    /// is the folder's canonical spelling (`/var` is `/private/var`).
    pub fn relative(&self, path: &Path, also: &Path) -> Result<PathBuf, String> {
        let rel = path
            .strip_prefix(&self.path)
            .or_else(|_| path.strip_prefix(also))
            .map_err(|_| format!("{} is outside {}", path.display(), self.path.display()))?;
        parts(rel)?;
        Ok(rel.to_path_buf())
    }
}
