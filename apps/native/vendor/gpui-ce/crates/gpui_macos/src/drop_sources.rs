//! Powermove P-DROP-SOURCES, adapted from NativeGPUI patch 0001 (Apache-2.0).
use block2::RcBlock;
use futures::{FutureExt, StreamExt, channel::mpsc, future::Either};
use gpui::{BackgroundExecutor, ExternalPaths, Task};
use objc2::{
    ClassType, msg_send,
    rc::{Retained, autoreleasepool},
};
use objc2_app_kit::{NSFilePromiseReceiver, NSPasteboard};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSOperationQueue, NSString, NSURL};
use std::{
    collections::HashMap,
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        Arc, LazyLock, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const FINDER_LIST: &str = "NSFilenamesPboardType";
// AppKit stores the legacy pasteboard tag as a dynamic UTI conforming to public.data.
const FINDER_LIST_UTI: &str = "dyn.ah62d4rv4gu8y6y4grf0gn5xbrzw1gydcr7u1e3cytf2gn";
const FILE_URL: &str = "public.file-url";
const IMAGES: [(&str, &str); 2] = [("public.png", "png"), ("public.tiff", "tiff")];
const IMAGE_LIMIT: usize = 512 * 1024 * 1024;

pub(crate) fn registered_types(receive_media: bool) -> Retained<NSArray<NSString>> {
    let mut types = vec![
        NSString::from_str(FINDER_LIST),
        NSString::from_str(FILE_URL),
    ];
    if receive_media {
        types.extend(NSFilePromiseReceiver::readableDraggedTypes().to_vec());
        types.extend(IMAGES.map(|(utype, _)| NSString::from_str(utype)));
    }
    NSArray::from_retained_slice(&types)
}

pub(crate) enum Source {
    Paths(ExternalPaths),
    Promises {
        urls: ExternalPaths,
        usable_urls: Arc<AtomicBool>,
        directory: Arc<Mutex<Option<ReceiveDirectory>>>,
    },
    Image(ImageKind),
}

#[derive(Clone, Copy)]
pub(crate) struct ImageKind {
    utype: &'static str,
    extension: &'static str,
}

impl Source {
    pub(crate) fn read(board: &NSPasteboard, receive_media: bool) -> Option<Self> {
        let items = board
            .pasteboardItems()
            .map(|items| items.to_vec())
            .unwrap_or_default();
        let finder_type = NSString::from_str(FINDER_LIST);
        let finder_uti = NSString::from_str(FINDER_LIST_UTI);
        // Reading the legacy property directly would also synthesize a Finder
        // list from file URLs, stripping the host and masking promise placeholders.
        let has_finder_list = items.is_empty()
            || items.iter().any(|item| {
                let types = item.types();
                types.containsObject(&finder_type) || types.containsObject(&finder_uti)
            });
        if has_finder_list
            && let Some(paths) = board
                .propertyListForType(&finder_type)
                .and_then(|list| list.downcast::<NSArray>().ok())
        {
            let paths = ExternalPaths(
                paths
                    .to_vec()
                    .into_iter()
                    .filter_map(|item| item.downcast::<NSString>().ok())
                    .map(|path| PathBuf::from(path.to_string()))
                    .filter(|path| path.is_absolute())
                    .collect(),
            );
            if !paths.paths().is_empty() {
                return Some(Self::Paths(paths));
            }
        }

        let urls = ExternalPaths(
            items
                .into_iter()
                .filter_map(|item| item.stringForType(&NSString::from_str(FILE_URL)))
                .filter_map(|text| NSURL::URLWithString(&text))
                .filter_map(|url| local_path(&url))
                .collect(),
        );
        // SAFETY: NSFilePromiseReceiver implements NSPasteboardReading.
        if receive_media && unsafe { board.canReadObjectForClasses_options(&promise_class(), None) }
        {
            return Some(Self::Promises {
                urls,
                usable_urls: Arc::new(AtomicBool::new(false)),
                directory: Arc::default(),
            });
        }
        if !urls.paths().is_empty() {
            return Some(Self::Paths(urls));
        }
        if receive_media {
            let types = board.types()?;
            return IMAGES.into_iter().find_map(|(utype, extension)| {
                types
                    .containsObject(&NSString::from_str(utype))
                    .then_some(Self::Image(ImageKind { utype, extension }))
            });
        }
        None
    }

    /// Give new representations the same hover type as Finder files. Consumers
    /// only receive the real paths in their drop handler, once the files exist.
    pub(crate) fn hover_paths(&self) -> ExternalPaths {
        match self {
            Self::Paths(paths) => paths.clone(),
            Self::Promises { urls, .. } => urls.clone(),
            Self::Image(_) => ExternalPaths::default(),
        }
    }

    /// Some promise sources also advertise a placeholder file URL. Resolve that
    /// distinction while hovering so the actual AppKit drop callback can choose.
    pub(crate) fn probe_urls(&self, background: &BackgroundExecutor) -> Option<Task<()>> {
        if let Self::Promises {
            urls, usable_urls, ..
        } = self
            && !urls.paths().is_empty()
        {
            let urls = urls.clone();
            let usable_urls = usable_urls.clone();
            return Some(background.spawn(async move {
                usable_urls.store(
                    urls.paths().iter().all(|path| usable_file_url(path)),
                    Ordering::Release,
                );
            }));
        }
        None
    }

    /// Select already materialized files before doing any receiving work.
    pub(crate) fn into_paths(self) -> Result<ExternalPaths, Self> {
        match self {
            Self::Paths(paths) => Ok(paths),
            Self::Promises {
                urls, usable_urls, ..
            } if usable_urls.load(Ordering::Acquire) => Ok(urls),
            source => Err(source),
        }
    }
}

fn usable_file_url(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    if temporary_path(path, &temp) {
        return false;
    }
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    let temp = fs::canonicalize(&temp).unwrap_or(temp);
    !temporary_path(&path, &temp)
}

fn temporary_path(path: &Path, temp: &Path) -> bool {
    path.starts_with(temp)
        || path
            .components()
            .any(|part| part.as_os_str() == "TemporaryItems")
}

/// Set by the host before the editor opens. Promise directories are prepared
/// only for a hovering promise, since AppKit needs one inside the drop callback.
#[derive(Clone)]
pub(crate) struct Storage {
    root: PathBuf,
    background: BackgroundExecutor,
}

impl Storage {
    pub(crate) fn new(root: PathBuf, background: BackgroundExecutor) -> Self {
        Self { root, background }
    }

    pub(crate) fn prepare(&self, source: &Source) {
        if let Source::Promises { directory, .. } = source {
            let directory = directory.clone();
            let storage = self.clone();
            self.background
                .spawn(async move {
                    let prepared = ReceiveDirectory::new(&storage).ok();
                    *directory.lock().unwrap() = prepared;
                })
                .detach();
        }
    }

    /// Called from AppKit's performDragOperation for this pasteboard.
    pub(crate) fn begin(&self, source: Source, board: &NSPasteboard) -> Option<Pending> {
        self.begin_inner(source, board, PromiseBatch::start_receiver)
    }

    fn begin_inner(
        &self,
        source: Source,
        board: &NSPasteboard,
        start: impl FnMut(&NSFilePromiseReceiver, &NSURL, &NSOperationQueue, &PromiseReader) -> usize,
    ) -> Option<Pending> {
        match source {
            Source::Promises { directory, .. } => {
                let receivers = promise_receivers(board);
                let directory = directory.lock().unwrap().take()?;
                Some(Pending::Promises(PromiseBatch::begin(
                    receivers, directory, start,
                )?))
            }
            Source::Image(kind) => {
                let board_name = board.name().to_string();
                let change = board.changeCount();
                let storage = self.clone();
                Some(Pending::Image(self.background.spawn(async move {
                    storage.read_image(&board_name, change, kind, IMAGE_LIMIT)
                })))
            }
            Source::Paths(_) => None,
        }
    }

    fn read_image(
        &self,
        board_name: &str,
        change: isize,
        kind: ImageKind,
        limit: usize,
    ) -> Option<ReceivedDrop> {
        autoreleasepool(|_| {
            let board = NSPasteboard::pasteboardWithName(&NSString::from_str(board_name));
            if board.changeCount() != change {
                return None;
            }
            let data = board.dataForType(&NSString::from_str(kind.utype))?;
            if board.changeCount() != change || !(1..=limit).contains(&data.len()) {
                return None;
            }
            let directory = ReceiveDirectory::new(self).ok()?;
            let path = directory.path.join(format!("image.{}", kind.extension));
            // SAFETY: Keep the pasteboard data and its byte slice on this worker.
            fs::write(&path, unsafe { data.as_bytes_unchecked() }).ok()?;
            directory.publish(smallvec::smallvec![path]).ok()
        })
    }
}

pub(crate) enum Pending {
    Promises(PromiseBatch),
    Image(Task<Option<ReceivedDrop>>),
}

impl Pending {
    pub(crate) async fn finish(self, background: &BackgroundExecutor) -> Option<ReceivedDrop> {
        match self {
            Self::Promises(promises) => promises.finish(background, Duration::from_secs(600)).await,
            Self::Image(task) => task.await,
        }
    }
}

fn local_path(url: &NSURL) -> Option<PathBuf> {
    if !url.isFileURL()
        || url
            .host()
            .is_some_and(|h| !h.is_empty() && h.to_string() != "localhost")
        || url.path().is_none()
    {
        return None;
    }
    let resolved;
    let url = if url.isFileReferenceURL() {
        resolved = url.filePathURL()?;
        &*resolved
    } else {
        url
    };
    crate::platform::ns_url_to_path(url)
        .ok()
        .filter(|p| p.is_absolute())
}

fn promise_class() -> Retained<NSArray<objc2::runtime::AnyClass>> {
    NSArray::from_slice(&[NSFilePromiseReceiver::class()])
}

fn promise_receivers(board: &NSPasteboard) -> Vec<Retained<NSFilePromiseReceiver>> {
    // SAFETY: NSFilePromiseReceiver implements NSPasteboardReading.
    let objects = unsafe { board.readObjectsForClasses_options(&promise_class(), None) };
    objects
        .into_iter()
        .flat_map(|objects| objects.to_vec())
        .filter_map(|object| object.downcast::<NSFilePromiseReceiver>().ok())
        .collect()
}

type PromiseReader = block2::DynBlock<dyn Fn(*mut NSURL, *mut NSError)>;
type Callback = (usize, Option<PathBuf>);

pub(crate) struct PromiseBatch {
    directory: Option<ReceiveDirectory>,
    queue: Retained<NSOperationQueue>,
    // AppKit may keep scheduling operations after begin returns.
    _receivers: Vec<Retained<NSFilePromiseReceiver>>,
    expected: Vec<usize>,
    callbacks: mpsc::UnboundedReceiver<Callback>,
}

impl PromiseBatch {
    fn start_receiver(
        receiver: &NSFilePromiseReceiver,
        destination: &NSURL,
        queue: &NSOperationQueue,
        reader: &PromiseReader,
    ) -> usize {
        let options: Retained<NSDictionary> = NSDictionary::new();
        // SAFETY: This runs in performDragOperation. The reader takes a raw URL
        // pointer since AppKit may omit the URL when it supplies an error.
        unsafe {
            let _: () = msg_send![
                receiver,
                receivePromisedFilesAtDestination: destination,
                options: &*options,
                operationQueue: queue,
                reader: reader,
            ];
        }
        receiver.fileNames().count()
    }

    fn begin(
        receivers: Vec<Retained<NSFilePromiseReceiver>>,
        directory: ReceiveDirectory,
        mut start: impl FnMut(
            &NSFilePromiseReceiver,
            &NSURL,
            &NSOperationQueue,
            &PromiseReader,
        ) -> usize,
    ) -> Option<Self> {
        if receivers.is_empty() {
            return None;
        }
        let path = CString::new(directory.path.as_os_str().as_bytes()).ok()?;
        // SAFETY: This filesystem representation lives through the initializer.
        let destination = unsafe {
            NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                NonNull::new(path.as_ptr().cast_mut())?,
                true,
                None,
            )
        };
        let queue = NSOperationQueue::new();
        // Move each file out of AppKit's shared input directory in its
        // coordinated read, before running the next reader.
        queue.setMaxConcurrentOperationCount(1);
        let (tx, callbacks) = mpsc::unbounded();
        let expected = receivers
            .iter()
            .enumerate()
            .map(|(index, receiver)| {
                let parent = directory.path.clone();
                let tx = tx.clone();
                let reader = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
                    let path = if error.is_null() && !tx.is_closed() {
                        // SAFETY: AppKit's successful result stays live for the reader.
                        unsafe { url.as_ref() }
                            .and_then(local_path)
                            .and_then(|path| isolate(&parent, index, path).ok())
                    } else {
                        None
                    };
                    let _ = tx.unbounded_send((index, path));
                });
                // AppKit requires the same destination URL for every receiver.
                // The reader isolates completed files into per-receiver subfolders.
                start(receiver, &destination, &queue, &reader)
            })
            .collect();
        Some(Self {
            directory: Some(directory),
            queue,
            _receivers: receivers,
            expected,
            callbacks,
        })
    }

    async fn finish(
        mut self,
        background: &BackgroundExecutor,
        timeout: Duration,
    ) -> Option<ReceivedDrop> {
        let collect = collect_callbacks(&self.expected, &mut self.callbacks).fuse();
        let timer = background.timer(timeout).fuse();
        let paths = match futures::future::select(Box::pin(collect), Box::pin(timer)).await {
            Either::Left((paths, _)) => paths?,
            Either::Right(_) => return None,
        };
        let directory = self.directory.take()?;
        background
            .spawn(async move { directory.publish(paths).ok() })
            .await
    }
}

impl Drop for PromiseBatch {
    fn drop(&mut self) {
        if self.directory.is_some() {
            self.callbacks.close();
            self.queue.cancelAllOperations();
        }
    }
}

async fn collect_callbacks(
    expected: &[usize],
    callbacks: &mut mpsc::UnboundedReceiver<Callback>,
) -> Option<smallvec::SmallVec<[PathBuf; 2]>> {
    if expected.contains(&0) {
        return None;
    }
    let mut batches = vec![Vec::new(); expected.len()];
    let mut remaining = expected
        .iter()
        .try_fold(0usize, |total, count| total.checked_add(*count))?;
    while remaining != 0 {
        let (index, path) = callbacks.next().await?;
        let batch = batches.get_mut(index)?;
        if batch.len() == expected[index] {
            return None;
        }
        batch.push(path?);
        remaining -= 1;
    }
    Some(batches.into_iter().flatten().collect())
}

fn isolate(parent: &Path, index: usize, path: PathBuf) -> std::io::Result<PathBuf> {
    if path.parent() != Some(parent) || !fs::symlink_metadata(&path)?.is_file() {
        return Err(std::io::Error::other("Invalid promised file"));
    }
    let folder = parent.join(index.to_string());
    match fs::create_dir(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let destination = folder.join(
        path.file_name()
            .ok_or_else(|| std::io::Error::other("Missing file name"))?,
    );
    // A hard link refuses an occupied destination. Both names are in staging
    // on the same volume. The promised bytes never need a second copy.
    fs::hard_link(&path, &destination)?;
    fs::remove_file(path)?;
    Ok(destination)
}

static RECEIVED: LazyLock<Mutex<HashMap<PathBuf, Weak<ReceivedFiles>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Ownership of materialized drop files while a host handler or import needs them.
#[derive(Clone)]
pub struct ReceivedDrop(Arc<ReceivedFiles>);

struct ReceivedFiles {
    paths: ExternalPaths,
    directory: ReceiveDirectory,
    retained: Mutex<std::collections::HashSet<PathBuf>>,
}

impl std::fmt::Debug for ReceivedDrop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("ReceivedDrop")
            .field(&self.0.paths)
            .finish()
    }
}

impl ReceivedDrop {
    /// Obtain ownership from the exact paths handed to a native drop handler.
    pub fn for_paths(paths: &[PathBuf]) -> Option<Self> {
        let files = RECEIVED.lock().unwrap();
        paths
            .iter()
            .find_map(|path| files.get(path)?.upgrade().map(Self))
    }

    pub(crate) fn paths(&self) -> &ExternalPaths {
        &self.0.paths
    }

    /// Whether this lease owns a particular file.
    pub fn owns(&self, path: &Path) -> bool {
        self.0.paths.paths().iter().any(|owned| owned == path)
    }

    /// Keep files only when the host has registered a path as a durable source.
    pub fn keep(&self, paths: &[PathBuf]) {
        self.0
            .retained
            .lock()
            .unwrap()
            .extend(paths.iter().filter(|path| self.owns(path)).cloned());
    }
}

impl Drop for ReceivedFiles {
    fn drop(&mut self) {
        let mut files = RECEIVED.lock().unwrap();
        for path in self.paths.paths() {
            files.remove(path);
        }
        self.directory.retained = std::mem::take(self.retained.get_mut().unwrap());
    }
}

/// Remove unfinished transfers once the app has acquired its profile lock.
pub fn clear_incoming_drops(root: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(root.join(".incoming")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub(crate) struct ReceiveDirectory {
    path: PathBuf,
    background: BackgroundExecutor,
    retained: std::collections::HashSet<PathBuf>,
}

impl ReceiveDirectory {
    fn new(storage: &Storage) -> std::io::Result<Self> {
        let root = storage.root.join(".incoming");
        fs::create_dir_all(&root)?;
        let path = fs::canonicalize(&root)?.join(uuid::Uuid::new_v4().to_string());
        let mut builder = fs::DirBuilder::new();
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700).create(&path)?;
        Ok(Self {
            path,
            background: storage.background.clone(),
            retained: Default::default(),
        })
    }

    fn publish(mut self, paths: smallvec::SmallVec<[PathBuf; 2]>) -> std::io::Result<ReceivedDrop> {
        let final_path = self
            .path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(self.path.file_name().unwrap());
        let paths = ExternalPaths(
            paths
                .into_iter()
                .map(|path| {
                    final_path.join(
                        path.strip_prefix(&self.path)
                            .expect("drop file under staging root"),
                    )
                })
                .collect(),
        );
        fs::rename(&self.path, &final_path)?;
        self.path = final_path;
        let files = Arc::new(ReceivedFiles {
            paths,
            directory: self,
            retained: Default::default(),
        });
        {
            let mut registered = RECEIVED.lock().unwrap();
            for path in files.paths.paths() {
                registered.insert(path.clone(), Arc::downgrade(&files));
            }
        }
        Ok(ReceivedDrop(files))
    }
}

impl Drop for ReceiveDirectory {
    fn drop(&mut self) {
        let path = std::mem::take(&mut self.path);
        let retained = std::mem::take(&mut self.retained);
        self.background
            .spawn(async move {
                if retained.is_empty() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    remove_unregistered(&path, &retained);
                }
            })
            .detach();
    }
}

fn remove_unregistered(dir: &Path, retained: &std::collections::HashSet<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                if retained.iter().any(|file| file.starts_with(&path)) {
                    remove_unregistered(&path, retained);
                } else {
                    let _ = fs::remove_dir_all(path);
                }
            } else if !retained.contains(&path) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod tests;
