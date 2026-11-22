//! Powermove P-DROP-SOURCES, adapted from NativeGPUI patch 0001 (Apache-2.0).
use block2::RcBlock;
use gpui::{BackgroundExecutor, ExternalPaths, Task};
use objc2::{ClassType, rc::Retained};
use objc2_app_kit::{NSFilePromiseReceiver, NSPasteboard};
use objc2_foundation::{
    NSArray, NSCopying, NSData, NSDictionary, NSError, NSOperationQueue, NSString, NSURL,
};
use std::{
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
    },
    Image(Image),
}

pub(crate) struct Image {
    data: Retained<NSData>,
    extension: &'static str,
}

// SAFETY: The pasteboard result is copied as immutable NSData. Immutable
// Foundation data can be read and released on a worker.
unsafe impl Send for Image {}

impl Image {
    fn write(self, directory: &Path) -> std::io::Result<PathBuf> {
        let path = directory.join(format!("image.{}", self.extension));
        // SAFETY: This is an immutable snapshot.
        fs::write(&path, unsafe { self.data.as_bytes_unchecked() })?;
        Ok(path)
    }
}

impl Source {
    pub(crate) fn read(board: &NSPasteboard, receive_media: bool) -> Option<Self> {
        Self::read_with_image_limit(board, receive_media, IMAGE_LIMIT)
    }

    fn read_with_image_limit(
        board: &NSPasteboard,
        receive_media: bool,
        image_limit: usize,
    ) -> Option<Self> {
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
            });
        }
        if !urls.paths().is_empty() {
            return Some(Self::Paths(urls));
        }
        if receive_media {
            for (utype, extension) in IMAGES {
                if let Some(data) = board.dataForType(&NSString::from_str(utype))
                    && (1..=image_limit).contains(&data.len())
                {
                    return Some(Self::Image(Image {
                        data: data.copy(),
                        extension,
                    }));
                }
            }
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
        if let Self::Promises { urls, usable_urls } = self
            && !urls.paths().is_empty()
        {
            let urls = urls.clone();
            let usable_urls = usable_urls.clone();
            return Some(background.spawn(async move {
                usable_urls.store(
                    urls.paths().iter().all(|path| path.exists()),
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
            Self::Promises { urls, usable_urls } if usable_urls.load(Ordering::Acquire) => Ok(urls),
            source => Err(source),
        }
    }
}

/// AppKit only allows starting a promise inside its drag-destination callback.
/// Keep empty, unique directories ready so that callback need not touch disk.
#[derive(Clone)]
pub(crate) struct Storage(Arc<StorageState>);

struct StorageState {
    root: PathBuf,
    reserved: Mutex<Vec<ReceiveDirectory>>,
    background: BackgroundExecutor,
}

impl Storage {
    pub(crate) fn new(root: PathBuf, background: BackgroundExecutor) -> Self {
        let storage = Self(Arc::new(StorageState {
            root,
            reserved: Mutex::new(Vec::new()),
            background,
        }));
        storage.replenish(2);
        storage
    }

    fn replenish(&self, count: usize) {
        let storage = self.clone();
        self.0
            .background
            .spawn(async move {
                for _ in 0..count {
                    let Ok(directory) = ReceiveDirectory::new(&storage.0.root) else {
                        break;
                    };
                    let mut reserved = storage.0.reserved.lock().unwrap();
                    if reserved.len() < 2 {
                        reserved.push(directory);
                    }
                }
            })
            .detach();
    }

    /// Called from AppKit's performDragOperation for this pasteboard.
    pub(crate) fn begin(&self, source: Source, board: &NSPasteboard) -> Option<Pending> {
        match source {
            Source::Promises { .. } => {
                let receivers = promise_receivers(board);
                if receivers.is_empty() {
                    return None;
                }
                let directory = self.0.reserved.lock().unwrap().pop();
                self.replenish(1);
                Some(Pending::Promises(PromiseBatch::begin(
                    receivers, directory?,
                )?))
            }
            Source::Image(image) => Some(Pending::Image(self.0.root.clone(), image)),
            Source::Paths(_) => None,
        }
    }
}

impl Drop for StorageState {
    fn drop(&mut self) {
        let reserved = std::mem::take(self.reserved.get_mut().unwrap());
        self.background
            .spawn(async move { drop(reserved) })
            .detach();
    }
}

pub(crate) enum Pending {
    Promises(PromiseBatch),
    Image(PathBuf, Image),
}

impl Pending {
    pub(crate) async fn finish(self, background: &BackgroundExecutor) -> Option<Received> {
        match self {
            Self::Promises(promises) => promises.finish(background).await,
            Self::Image(root, image) => {
                background
                    .spawn(async move {
                        let directory = ReceiveDirectory::new(&root).ok()?;
                        let path = image.write(&directory.path).ok()?;
                        Some(Received {
                            paths: ExternalPaths(smallvec::smallvec![path]),
                            directory,
                        })
                    })
                    .await
            }
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

type PromiseFiles = Arc<Mutex<Vec<Option<PathBuf>>>>;

pub(crate) struct PromiseBatch {
    directory: ReceiveDirectory,
    queue: Retained<NSOperationQueue>,
    receivers: Vec<Retained<NSFilePromiseReceiver>>,
    batches: Vec<(usize, PromiseFiles)>,
}

impl PromiseBatch {
    fn begin(
        receivers: Vec<Retained<NSFilePromiseReceiver>>,
        directory: ReceiveDirectory,
    ) -> Option<Self> {
        let options = NSDictionary::new();
        let path = CString::new(directory.path.as_os_str().as_bytes()).ok()?;
        // SAFETY: The null-terminated path remains live during this initializer.
        let destination = unsafe {
            NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                NonNull::new(path.as_ptr().cast_mut())?,
                true,
                None,
            )
        };
        let queue = NSOperationQueue::new();
        let mut batches = Vec::new();
        for receiver in &receivers {
            let results = PromiseFiles::default();
            let reader = RcBlock::new({
                let results = results.clone();
                move |url: NonNull<NSURL>, error: *mut NSError| {
                    // SAFETY: AppKit supplies a valid URL for the successful callback.
                    let path = if error.is_null() {
                        local_path(unsafe { url.as_ref() })
                    } else {
                        None
                    };
                    results.lock().unwrap().push(path);
                }
            });
            // SAFETY: Called in performDragOperation with receivers read from
            // its own pasteboard. The existing directory and worker queue live
            // through all of the AppKit callbacks.
            unsafe {
                receiver.receivePromisedFilesAtDestination_options_operationQueue_reader(
                    &destination,
                    &options,
                    &queue,
                    &reader,
                );
            }
            // Legacy promises can produce several files despite listing just
            // one file type. AppKit populates fileNames when receiving starts.
            batches.push((receiver.fileNames().count(), results));
        }
        Some(Self {
            directory,
            queue,
            receivers,
            batches,
        })
    }

    async fn finish(self, background: &BackgroundExecutor) -> Option<Received> {
        let Self {
            directory,
            queue,
            receivers,
            batches,
        } = self;
        let received = background
            .spawn_dedicated(move |_| async move {
                queue.waitUntilAllOperationsAreFinished();
                let mut paths = ExternalPaths::default();
                for (count, results) in batches {
                    let mut results = results.lock().unwrap();
                    if count == 0 || results.len() != count {
                        return None;
                    }
                    for result in results.drain(..) {
                        let path = result?;
                        if path.parent() != Some(&directory.path)
                            || !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file())
                            || paths.0.contains(&path)
                        {
                            return None;
                        }
                        paths.0.push(path);
                    }
                }
                Some(Received { paths, directory })
            })
            .await;
        drop(receivers);
        received
    }
}

pub(crate) struct Received {
    pub(crate) paths: ExternalPaths,
    directory: ReceiveDirectory,
}

impl Received {
    pub(crate) fn keep(mut self) {
        // An accepted drop may become a project source, a relink or a sequence.
        self.directory.keep = true;
    }
}

struct ReceiveDirectory {
    path: PathBuf,
    keep: bool,
}

impl ReceiveDirectory {
    fn new(root: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(root)?;
        let path = fs::canonicalize(root)?.join(uuid::Uuid::new_v4().to_string());
        let mut builder = fs::DirBuilder::new();
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700).create(&path)?;
        Ok(Self { path, keep: false })
    }
}

impl Drop for ReceiveDirectory {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
