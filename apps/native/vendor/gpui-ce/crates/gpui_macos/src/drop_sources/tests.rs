use super::*;
use objc2::{
    AnyThread, DefinedClass, define_class, msg_send, rc::autoreleasepool, runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSFilePromiseProvider, NSFilePromiseProviderDelegate, NSPasteboardItem, NSPasteboardWriting,
};
use objc2_foundation::{NSObject, NSObjectProtocol};
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
        "web_link_alone_and_remote_file_urls_are_refused",
        web_link_alone_and_remote_file_urls_are_refused,
    ),
    (
        "png_and_tiff_land_in_distinct_durable_directories",
        png_and_tiff_land_in_distinct_durable_directories,
    ),
    (
        "image_cap_is_inclusive_and_rejects_oversize_and_empty_data",
        image_cap_is_inclusive_and_rejects_oversize_and_empty_data,
    ),
    (
        "browser_mix_prefers_existing_file_urls_then_promises_without_duplicates",
        browser_mix_prefers_existing_file_urls_then_promises_without_duplicates,
    ),
    (
        "failed_promise_discards_the_entire_batch",
        failed_promise_discards_the_entire_batch,
    ),
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

fn finish(pending: Pending) -> Option<Received> {
    futures::executor::block_on(pending.finish(&background()))
}

fn scratch() -> ReceiveDirectory {
    ReceiveDirectory::new(&std::env::temp_dir()).unwrap()
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

fn png_and_tiff_land_in_distinct_durable_directories() {
    autoreleasepool(|_| {
        let board = Board::new();
        let root = scratch();
        for (utype, extension) in IMAGES {
            let item = NSPasteboardItem::new();
            set_image(&item, utype, b"image bytes");
            board.write(&[ProtocolObject::from_retained(item)]);
            let Some(Source::Image(image)) = board.read() else {
                panic!("image source")
            };
            let received = finish(Pending::Image(root.path.clone(), image)).unwrap();
            let [path] = received.paths.paths() else {
                panic!("one image")
            };
            assert_eq!(path.parent().unwrap().parent(), Some(root.path.as_path()));
            assert_eq!(path.extension().unwrap(), extension);
            assert_eq!(fs::read(path).unwrap(), b"image bytes");
            let path = path.clone();
            received.keep();
            assert!(path.is_file());
        }
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 2);
    });
}

fn image_cap_is_inclusive_and_rejects_oversize_and_empty_data() {
    autoreleasepool(|_| {
        let board = Board::new();
        let item = NSPasteboardItem::new();
        set_image(&item, IMAGES[0].0, b"12345");
        board.write(&[ProtocolObject::from_retained(item)]);
        assert!(matches!(
            Source::read_with_image_limit(&board.0, true, 5),
            Some(Source::Image(_))
        ));
        assert!(Source::read_with_image_limit(&board.0, true, 4).is_none());
        let item = NSPasteboardItem::new();
        set_image(&item, IMAGES[0].0, b"");
        board.write(&[ProtocolObject::from_retained(item)]);
        assert!(board.read().is_none());
        assert_eq!(IMAGE_LIMIT, 536_870_912);
    });
}

struct Provider {
    name: &'static str,
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
            let write = fs::write(local_path(url).unwrap(), b"promised bytes");
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
    fail: bool,
) -> (Retained<TestProvider>, Retained<NSFilePromiseProvider>) {
    let delegate = TestProvider::alloc().set_ivars(Provider {
        name,
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

// AppKit refuses receivePromisedFiles... outside a real drag callback. Named
// boards prove its receiver decoding; drive its provider delegate on the queue
// to exercise the same completion, validation and cleanup as a real transfer.
fn finish_fixture_batch(
    board: &Board,
    providers: &[Retained<NSFilePromiseProvider>],
    root: &Path,
) -> Option<Received> {
    let receivers = promise_receivers(&board.0);
    assert_eq!(receivers.len(), providers.len());
    let directory = ReceiveDirectory::new(root).unwrap();
    let queue = NSOperationQueue::new();
    let mut batches = Vec::new();
    for provider in providers {
        let delegate = provider.delegate().unwrap();
        let name: Retained<NSString> = unsafe {
            msg_send![&*delegate, filePromiseProvider: &**provider, fileNameForType: &*provider.fileType()]
        };
        let path = directory.path.join(name.to_string());
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let results = PromiseFiles::default();
        let provider = provider.clone();
        let completion = RcBlock::new({
            let results = results.clone();
            move |error: *mut NSError| {
                results
                    .lock()
                    .unwrap()
                    .push(error.is_null().then(|| path.clone()));
            }
        });
        let write = RcBlock::new(move || {
            assert!(objc2::MainThreadMarker::new().is_none());
            delegate.filePromiseProvider_writePromiseToURL_completionHandler(
                &provider,
                &url,
                &completion,
            );
        });
        // SAFETY: these provider/delegate instances are worker-safe fixtures.
        unsafe { queue.addOperationWithBlock(&write) };
        batches.push((1, results));
    }
    finish(Pending::Promises(PromiseBatch {
        directory,
        queue,
        receivers,
        batches,
    }))
}

fn browser_mix_prefers_existing_file_urls_then_promises_without_duplicates() {
    autoreleasepool(|_| {
        let root = scratch();
        let existing = root.path.join("existing image.png");
        fs::write(&existing, b"existing").unwrap();
        let board = Board::new();
        let existing_url = NSURL::fileURLWithPath(&NSString::from_str(existing.to_str().unwrap()));
        let item = url_item(
            &existing_url.absoluteString().unwrap().to_string(),
            FILE_URL,
        );
        assert!(item.setString_forType(
            &NSString::from_str("https://example.com/image.png"),
            &NSString::from_str("public.url")
        ));
        set_image(&item, IMAGES[1].0, b"raster fallback");
        let (delegate, promise) = promised_file("promise.png", false);
        board.write(&[
            ProtocolObject::from_retained(item),
            ProtocolObject::from_retained(promise.clone()),
        ]);
        let source = board.read().unwrap();
        assert!(matches!(source, Source::Promises { .. }));
        futures::executor::block_on(source.probe_urls(&background()).unwrap());
        let paths = source
            .into_paths()
            .ok()
            .expect("existing local URLs take precedence");
        assert_eq!(paths.paths(), &[existing.clone()]);
        assert_eq!(delegate.ivars().calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1);

        fs::remove_file(existing).unwrap();
        let source = board.read().unwrap();
        futures::executor::block_on(source.probe_urls(&background()).unwrap());
        assert!(matches!(source.into_paths(), Err(Source::Promises { .. })));
        let received = finish_fixture_batch(&board, &[promise], &root.path).unwrap();
        let [path] = received.paths.paths() else {
            panic!("one promise")
        };
        assert_eq!(path.file_name().unwrap(), "promise.png");
        assert_eq!(fs::read(path).unwrap(), b"promised bytes");
        assert_eq!(delegate.ivars().calls.load(Ordering::Relaxed), 1);
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1);
        let path = path.clone();
        received.keep();
        assert!(path.is_file());
    });
}

fn failed_promise_discards_the_entire_batch() {
    autoreleasepool(|_| {
        let root = scratch();
        let board = Board::new();
        let (good, first) = promised_file("complete.png", false);
        let (bad, second) = promised_file("partial.png", true);
        board.write(&[
            ProtocolObject::from_retained(first.clone()),
            ProtocolObject::from_retained(second.clone()),
        ]);
        assert!(matches!(board.read(), Some(Source::Promises { .. })));
        assert!(finish_fixture_batch(&board, &[first, second], &root.path).is_none());
        assert_eq!(good.ivars().calls.load(Ordering::Relaxed), 1);
        assert_eq!(bad.ivars().calls.load(Ordering::Relaxed), 1);
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 0);
    });
}
