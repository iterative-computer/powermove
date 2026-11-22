use super::*;
use objc2::{
    AnyThread, DefinedClass, Message, define_class, msg_send, rc::autoreleasepool,
    runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSFilePromiseProvider, NSFilePromiseProviderDelegate, NSPasteboardItem,
    NSPasteboardItemDataProvider, NSPasteboardWriting,
};
use objc2_foundation::{NSBlockOperation, NSData, NSObject, NSObjectProtocol};
use std::sync::atomic::{AtomicUsize, Ordering};

pub const CASES: &[(&str, fn())] = &[
    (
        "finder_list_wins_and_keeps_all_paths",
        finder_list_wins_and_keeps_all_paths,
    ),
    (
        "one_or_many_local_file_urls_are_decoded_in_order",
        one_or_many_local_file_urls_are_decoded_in_order,
    ),
    (
        "finder_reference_url_resolves_to_a_path",
        finder_reference_url_resolves_to_a_path,
    ),
    (
        "web_link_alone_and_remote_file_urls_are_refused",
        web_link_alone_and_remote_file_urls_are_refused,
    ),
    (
        "only_incomplete_drops_are_swept_at_launch",
        only_incomplete_drops_are_swept_at_launch,
    ),
    (
        "image_reads_at_drop_and_ownership_controls_cleanup",
        image_reads_at_drop_and_ownership_controls_cleanup,
    ),
    (
        "image_cap_is_inclusive_and_hover_does_not_load_the_payload",
        image_cap_is_inclusive_and_hover_does_not_load_the_payload,
    ),
    (
        "browser_mix_prefers_durable_file_urls_over_promises",
        browser_mix_prefers_durable_file_urls_over_promises,
    ),
    (
        "promises_survive_late_scheduling_and_isolate_same_names",
        promises_survive_late_scheduling_and_isolate_same_names,
    ),
    (
        "failed_promise_discards_the_entire_batch",
        failed_promise_discards_the_entire_batch,
    ),
    (
        "silent_promise_times_out_and_discards_partial_bytes",
        silent_promise_times_out_and_discards_partial_bytes,
    ),
    ("busy_callback_is_retried", busy_callback_is_retried),
];

struct Board(Retained<NSPasteboard>);

impl Board {
    fn new() -> Self {
        Self(NSPasteboard::pasteboardWithUniqueName())
    }

    fn write(&self, objects: &[Retained<ProtocolObject<dyn NSPasteboardWriting>>]) {
        self.0.clearContents();
        assert!(self.0.writeObjects(&NSArray::from_retained_slice(objects)));
    }

    fn read(&self) -> Option<Source> {
        Source::read(&self.0, true)
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        let _: () = unsafe { msg_send![&*self.0, releaseGlobally] };
    }
}

fn url_item(url: &str, utype: &str) -> Retained<NSPasteboardItem> {
    let item = NSPasteboardItem::new();
    assert!(item.setString_forType(&NSString::from_str(url), &NSString::from_str(utype)));
    item
}

fn set_image(item: &NSPasteboardItem, utype: &str, bytes: &[u8]) {
    assert!(item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(utype)));
}

fn background() -> BackgroundExecutor {
    BackgroundExecutor::new(Arc::new(crate::MacDispatcher::new()))
}

fn finish(pending: Pending) -> Option<ReceivedDrop> {
    futures::executor::block_on(pending.finish(&background()))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn storage(&self) -> Storage {
        Storage::new(self.0.join("received"), background())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn await_removed(path: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!path.exists(), "unclaimed or failed drop left a directory");
}

fn finder_list_wins_and_keeps_all_paths() {
    autoreleasepool(|_| {
        let board = Board::new();
        let finder = NSString::from_str(FINDER_LIST);
        let url = NSString::from_str(FILE_URL);
        let types = NSArray::from_slice(&[&*finder, &*url]);
        let paths = NSArray::from_retained_slice(&[
            NSString::from_str("/incoming/first.png"),
            NSString::from_str("/incoming/second.mov"),
        ]);
        unsafe {
            board.0.declareTypes_owner(&types, None);
            assert!(board.0.setPropertyList_forType(&paths, &finder));
        }
        assert!(
            board
                .0
                .setString_forType(&NSString::from_str("file:///other.png"), &url)
        );
        let Some(Source::Paths(paths)) = board.read() else {
            panic!("Finder paths")
        };
        assert_eq!(
            paths.paths(),
            &[
                PathBuf::from("/incoming/first.png"),
                PathBuf::from("/incoming/second.mov")
            ]
        );
    });
}

fn one_or_many_local_file_urls_are_decoded_in_order() {
    autoreleasepool(|_| {
        let board = Board::new();
        let first = || {
            ProtocolObject::from_retained(url_item("file:///incoming/first%20take.mov", FILE_URL))
        };
        board.write(&[first()]);
        let Some(Source::Paths(paths)) = board.read() else {
            panic!("file URL")
        };
        assert_eq!(paths.paths(), &[PathBuf::from("/incoming/first take.mov")]);
        board.write(&[
            first(),
            ProtocolObject::from_retained(url_item("file:///incoming/second.png", FILE_URL)),
        ]);
        let Some(Source::Paths(paths)) = board.read() else {
            panic!("file URLs")
        };
        assert_eq!(
            paths.paths(),
            &[
                PathBuf::from("/incoming/first take.mov"),
                PathBuf::from("/incoming/second.png")
            ]
        );
    });
}

fn web_link_alone_and_remote_file_urls_are_refused() {
    autoreleasepool(|_| {
        let board = Board::new();
        board.write(&[ProtocolObject::from_retained(url_item(
            "https://example.com/image.png",
            "public.url",
        ))]);
        assert!(board.read().is_none());
        board.write(&[ProtocolObject::from_retained(url_item(
            "file://example.com/file.mov",
            FILE_URL,
        ))]);
        assert!(board.read().is_none());
    });
}

fn finder_reference_url_resolves_to_a_path() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let path = root.0.join("a photo.mov");
        fs::write(&path, b"sample").unwrap();
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let reference = url.fileReferenceURL().unwrap();
        let text = reference.absoluteString().unwrap().to_string();
        assert!(text.starts_with("file:///.file/id="), "{text}");
        let board = Board::new();
        board.write(&[ProtocolObject::from_retained(url_item(&text, FILE_URL))]);
        let Some(Source::Paths(paths)) = board.read() else {
            panic!("file reference")
        };
        assert_eq!(paths.paths(), &[path]);
    });
}

fn only_incomplete_drops_are_swept_at_launch() {
    let root = Scratch::new();
    let storage = root.storage();
    assert!(!storage.root.exists(), "launch reserves no folders");
    let incoming = storage.root.join(".incoming");
    fs::create_dir_all(incoming.join("interrupted")).unwrap();
    fs::write(incoming.join("interrupted/movie.mov"), b"partial").unwrap();
    let kept = storage.root.join("registered/movie.mov");
    fs::create_dir(kept.parent().unwrap()).unwrap();
    fs::write(&kept, b"complete").unwrap();
    clear_incoming_drops(&storage.root).unwrap();
    assert!(!incoming.exists());
    assert_eq!(fs::read(kept).unwrap(), b"complete");
    clear_incoming_drops(&storage.root).unwrap();
}

fn image_reads_at_drop_and_ownership_controls_cleanup() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let storage = root.storage();
        let board = Board::new();
        for (utype, extension) in IMAGES {
            let item = NSPasteboardItem::new();
            set_image(&item, utype, b"image bytes");
            board.write(&[ProtocolObject::from_retained(item)]);
            let source = board.read().unwrap();
            assert!(matches!(source, Source::Image(_)));
            let received = finish(storage.begin(source, &board.0).unwrap()).unwrap();
            let [path] = received.paths().paths() else {
                panic!("one image")
            };
            assert_eq!(
                path.parent().unwrap().parent(),
                Some(storage.root.as_path())
            );
            assert_eq!(path.extension().unwrap(), extension);
            assert_eq!(fs::read(path).unwrap(), b"image bytes");
            let path = path.clone();
            if extension == "png" {
                // A consumed event alone releases the path. Taking ownership
                // keeps it available until the async import has finished.
                let owner = ReceivedDrop::for_paths(std::slice::from_ref(&path)).unwrap();
                drop(received);
                assert!(path.is_file());
                drop(owner);
                await_removed(path.parent().unwrap());
            } else {
                received.keep(std::slice::from_ref(&path));
                drop(received);
                assert!(path.is_file());
            }
            assert!(ReceivedDrop::for_paths(&[path]).is_none());
        }
    });
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = AtomicUsize]
    struct LazyImage;

    unsafe impl NSObjectProtocol for LazyImage {}

    unsafe impl NSPasteboardItemDataProvider for LazyImage {
        #[unsafe(method(pasteboard:item:provideDataForType:))]
        fn provide(&self, _: Option<&NSPasteboard>, item: &NSPasteboardItem, kind: &NSString) {
            self.ivars().fetch_add(1, Ordering::Relaxed);
            item.setData_forType(&NSData::with_bytes(b"12345"), kind);
        }
    }
);

fn image_cap_is_inclusive_and_hover_does_not_load_the_payload() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let storage = root.storage();
        let board = Board::new();
        let item = NSPasteboardItem::new();
        let provider = LazyImage::alloc().set_ivars(AtomicUsize::new(0));
        let provider: Retained<LazyImage> = unsafe { msg_send![super(provider), init] };
        assert!(item.setDataProvider_forTypes(
            ProtocolObject::from_ref(&*provider),
            &NSArray::from_retained_slice(&[NSString::from_str(IMAGES[0].0)]),
        ));
        board.write(&[ProtocolObject::from_retained(item)]);
        let Some(Source::Image(kind)) = board.read() else {
            panic!("advertised image")
        };
        assert!(!storage.root.exists());
        assert_eq!(provider.ivars().load(Ordering::Relaxed), 0);
        let name = board.0.name().to_string();
        let change = board.0.changeCount();
        assert!(storage.read_image(&name, change, kind, 5).is_some());
        assert_eq!(provider.ivars().load(Ordering::Relaxed), 1);
        assert!(storage.read_image(&name, change, kind, 4).is_none());
        let item = NSPasteboardItem::new();
        set_image(&item, IMAGES[0].0, b"");
        board.write(&[ProtocolObject::from_retained(item)]);
        assert!(
            storage
                .read_image(&name, change, kind, IMAGE_LIMIT)
                .is_none()
        );
        assert!(finish(storage.begin(board.read().unwrap(), &board.0).unwrap()).is_none());
        assert_eq!(IMAGE_LIMIT, 536_870_912);
    });
}

struct Provider {
    name: &'static str,
    bytes: &'static [u8],
    fail: bool,
    calls: AtomicUsize,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Provider]
    struct TestProvider;

    unsafe impl NSObjectProtocol for TestProvider {}

    unsafe impl NSFilePromiseProviderDelegate for TestProvider {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn file_name(&self, _: &NSFilePromiseProvider, _: &NSString) -> Retained<NSString> {
            NSString::from_str(self.ivars().name)
        }

        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn operation_queue(&self, _: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            NSOperationQueue::new()
        }

        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _: &NSFilePromiseProvider,
            url: &NSURL,
            complete: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            self.ivars().calls.fetch_add(1, Ordering::Relaxed);
            let write = fs::write(local_path(url).unwrap(), self.ivars().bytes);
            if self.ivars().fail || write.is_err() {
                let error = unsafe {
                    NSError::errorWithDomain_code_userInfo(
                        &NSString::from_str("DropSourceTest"),
                        1,
                        None,
                    )
                };
                complete.call((Retained::as_ptr(&error).cast_mut(),));
            } else {
                complete.call((std::ptr::null_mut(),));
            }
        }
    }
);

fn promised_file(
    name: &'static str,
    bytes: &'static [u8],
    fail: bool,
) -> (Retained<TestProvider>, Retained<NSFilePromiseProvider>) {
    let delegate = TestProvider::alloc().set_ivars(Provider {
        name,
        bytes,
        fail,
        calls: AtomicUsize::new(0),
    });
    let delegate: Retained<TestProvider> = unsafe { msg_send![super(delegate), init] };
    let provider = NSFilePromiseProvider::initWithFileType_delegate(
        NSFilePromiseProvider::alloc(),
        &NSString::from_str("public.png"),
        ProtocolObject::from_ref(&*delegate),
    );
    (delegate, provider)
}

fn write_promises(board: &Board, providers: &[Retained<NSFilePromiseProvider>]) {
    let writers: Vec<_> = providers
        .iter()
        .map(|provider| ProtocolObject::from_retained(provider.clone()))
        .collect();
    board.write(&writers);
}

// AppKit disallows fulfillment outside an active NSDraggingSession. Decode
// the real named-board receivers and drive begin through the one injected
// operation AppKit owns. Its read, staging, timeout and cleanup are unchanged.
fn begin_fixture(
    board: &Board,
    storage: &Storage,
    providers: &[Retained<NSFilePromiseProvider>],
    silent: bool,
) -> (
    PromiseBatch,
    PathBuf,
    Vec<RcBlock<dyn Fn(*mut NSURL, *mut NSError)>>,
) {
    let source = board.read().unwrap();
    storage.prepare(&source);
    let Source::Promises { directory, .. } = &source else {
        panic!("promises")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let staging_path = loop {
        if let Some(directory) = directory.lock().unwrap().as_ref() {
            break directory.path.clone();
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut providers = providers.iter();
    let mut held_readers = Vec::new();
    let pending = storage.begin_inner(source, &board.0, |_, destination, queue, reader| {
        let provider = providers.next().expect("named board receiver").clone();
        let delegate = provider.delegate().unwrap();
        let file_name: Retained<NSString> = unsafe {
            msg_send![&*delegate, filePromiseProvider: &*provider, fileNameForType: &*provider.fileType()]
        };
        let path = local_path(destination).unwrap().join(file_name.to_string());
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let reader = reader.copy();
        if silent {
            held_readers.push(reader.clone());
        }
        let done = RcBlock::new(move |error: *mut NSError| {
            if !silent {
                reader.call((Retained::as_ptr(&url).cast_mut(), error));
            }
        });
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let write = RcBlock::new(move || {
            assert!(objc2::MainThreadMarker::new().is_none());
            delegate.filePromiseProvider_writePromiseToURL_completionHandler(&provider, &url, &done);
        });
        // SAFETY: Test provider/delegate state and file writes can run on a worker.
        let op = unsafe { NSBlockOperation::blockOperationWithBlock(&write) };
        let queue = queue.retain();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            queue.addOperation(&op);
        });
        1
    }).unwrap();
    assert!(providers.next().is_none(), "every provider must decode");
    let Pending::Promises(batch) = pending else {
        panic!("started a promise batch")
    };
    (batch, staging_path, held_readers)
}

fn browser_mix_prefers_durable_file_urls_over_promises() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let existing = std::env::current_exe().unwrap();
        assert!(usable_file_url(&existing));
        let temporary = root.0.join("temporary image.png");
        fs::write(&temporary, b"placeholder").unwrap();
        assert!(!usable_file_url(&temporary));
        let temporary_alias = root.0.join("image alias.png");
        std::os::unix::fs::symlink(&existing, &temporary_alias).unwrap();
        assert!(!usable_file_url(&temporary_alias));
        assert!(temporary_path(
            Path::new("/Volumes/sample/TemporaryItems/image.png"),
            &std::env::temp_dir()
        ));
        let board = Board::new();
        for (path, durable) in [
            (existing, true),
            (temporary, false),
            (temporary_alias, false),
        ] {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
            let item = url_item(&url.absoluteString().unwrap().to_string(), FILE_URL);
            assert!(item.setString_forType(
                &NSString::from_str("https://example.com/image.png"),
                &NSString::from_str("public.url")
            ));
            set_image(&item, IMAGES[1].0, b"raster fallback");
            let (_provider, writer) = promised_file("promised.png", b"promise", false);
            board.write(&[
                ProtocolObject::from_retained(item),
                ProtocolObject::from_retained(writer),
            ]);
            let source = board.read().unwrap();
            assert!(matches!(source, Source::Promises { .. }));
            futures::executor::block_on(source.probe_urls(&background()).unwrap());
            if durable {
                assert_eq!(source.into_paths().ok().unwrap().paths(), &[path]);
            } else {
                assert!(matches!(source.into_paths(), Err(Source::Promises { .. })));
            }
        }
        assert!(!root.storage().root.exists());
    });
}

fn promises_survive_late_scheduling_and_isolate_same_names() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let storage = root.storage();
        let board = Board::new();
        let (_first, first) = promised_file("same.png", b"first", false);
        let (_second, second) = promised_file("same.png", b"second", false);
        let providers = [first, second];
        write_promises(&board, &providers);
        let (batch, staging, _held_readers) = begin_fixture(&board, &storage, &providers, false);
        assert!(staging.starts_with(storage.root.join(".incoming")));
        let received =
            futures::executor::block_on(batch.finish(&background(), Duration::from_secs(2)))
                .unwrap();
        assert!(!staging.exists());
        let paths = received.paths().paths();
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0].parent(), paths[1].parent());
        for (path, expected) in paths
            .iter()
            .zip([b"first".as_slice(), b"second".as_slice()])
        {
            assert_eq!(path.file_name().unwrap(), "same.png");
            assert_eq!(fs::read(path).unwrap(), expected);
        }
        let keep = paths[0].clone();
        let release = paths[1].parent().unwrap().to_owned();
        received.keep(std::slice::from_ref(&keep));
        drop(received);
        await_removed(&release);
        assert_eq!(fs::read(keep).unwrap(), b"first");
    });
}

fn failed_promise_discards_the_entire_batch() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let storage = root.storage();
        let board = Board::new();
        let (_good, first) = promised_file("complete.png", b"complete", false);
        let (bad, second) = promised_file("partial.png", b"partial", true);
        let providers = [first, second];
        write_promises(&board, &providers);
        let (batch, staging, _held_readers) = begin_fixture(&board, &storage, &providers, false);
        assert!(
            futures::executor::block_on(batch.finish(&background(), Duration::from_secs(2)))
                .is_none()
        );
        assert_eq!(bad.ivars().calls.load(Ordering::Relaxed), 1);
        await_removed(&staging);
    });
}

fn silent_promise_times_out_and_discards_partial_bytes() {
    autoreleasepool(|_| {
        let root = Scratch::new();
        let storage = root.storage();
        let board = Board::new();
        let (delegate, provider) = promised_file("partial.png", b"partial", false);
        let providers = [provider];
        write_promises(&board, &providers);
        let (batch, staging, _held_readers) = begin_fixture(&board, &storage, &providers, true);
        fs::write(staging.join("interrupted.mov"), b"interrupted").unwrap();
        let timeout = Duration::from_millis(150);
        let start = std::time::Instant::now();
        assert!(futures::executor::block_on(batch.finish(&background(), timeout)).is_none());
        assert!(start.elapsed() >= timeout, "the sender never completed");
        assert_eq!(delegate.ivars().calls.load(Ordering::Relaxed), 1);
        await_removed(&staging);
    });
}

fn busy_callback_is_retried() {
    let mut attempts = 0;
    futures::executor::block_on(crate::window::retry_file_drop(&background(), || {
        attempts += 1;
        attempts == 3
    }));
    assert_eq!(attempts, 3);
}
