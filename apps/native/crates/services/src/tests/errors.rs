use crate::{Result, ServiceError, ServiceErrorCode as Code, error::Context};
use std::io;

#[test]
fn io_codes_are_typed_and_survive_context_without_message_matching() {
    for (kind, code) in [
        (io::ErrorKind::NotFound, Code::NotFound),
        (io::ErrorKind::PermissionDenied, Code::PermissionDenied),
        (io::ErrorKind::StorageFull, Code::DiskFull),
        (io::ErrorKind::NetworkUnreachable, Code::Offline),
        (io::ErrorKind::Other, Code::Unknown),
    ] {
        let result: io::Result<()> = Err(io::Error::new(kind, "synthetic diagnostic"));
        let error = result.context("operation context").unwrap_err();
        assert_eq!(error.code, code);
        assert!(format!("{:#}", error.diagnostic()).contains("synthetic diagnostic"));
        let wrapped = ServiceError::from(anyhow::Error::new(error).context("host context"));
        assert_eq!(wrapped.code, code);
        assert_eq!(
            serde_json::from_str::<Code>(&serde_json::to_string(&code).unwrap()).unwrap(),
            code
        );
    }
}

#[test]
fn service_validations_expose_codes_for_reviewed_developer_messages() {
    let mut library = crate::library::LibraryService(RefusingKernel);
    assert_eq!(
        library
            .apply(crate::library::LibraryAction::Duplicate {
                id: "project".into(),
                new_id: "project".into(),
            })
            .unwrap_err()
            .code,
        Code::InvalidInput
    );
    assert_eq!(
        library
            .apply(crate::library::LibraryAction::Open {
                id: "project".into(),
            })
            .unwrap_err()
            .code,
        Code::PermissionDenied
    );
    let mut queue = crate::entrypoints::EntryQueue::default();
    assert_eq!(
        queue.push_file("/tmp/example.txt".into()).unwrap_err().code,
        Code::InvalidProject
    );
    assert_eq!(
        queue
            .push_link(&"x".repeat(16 * 1024 + 1))
            .unwrap_err()
            .code,
        Code::TooLarge
    );
    assert_eq!(
        crate::entrypoints::AuthLink::parse("invalid")
            .err()
            .unwrap()
            .code,
        Code::InvalidInput
    );
    let (_temp, mut storage) = super::test_storage();
    assert_eq!(
        crate::diagnostics::Diagnostics::log(
            &mut storage,
            &mut crate::diagnostics::OmitText,
            crate::diagnostics::Level::Error,
            "synthetic",
            &"x".repeat(64 * 1024 + 1),
        )
        .unwrap_err()
        .code,
        Code::TooLarge
    );
    assert_eq!(
        crate::storage::ServiceStorage::open("relative")
            .err()
            .unwrap()
            .code,
        Code::InvalidInput
    );
    assert_eq!(
        crate::media::ArtifactCache::new(0).err().unwrap().code,
        Code::InvalidInput
    );
}
struct RefusingKernel;
impl crate::library::LibraryHook for RefusingKernel {
    fn list(&mut self) -> Result<Vec<crate::library::ProjectEntry>> {
        Ok(vec![])
    }
    fn recents(&mut self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    fn apply(&mut self, _: crate::library::LibraryAction) -> Result<()> {
        Err(ServiceError::message(
            Code::PermissionDenied,
            "kernel diagnostic",
        ))
    }
}

#[test]
fn worker_returns_the_original_initialization_and_task_codes() {
    use std::time::{Duration, Instant};
    fn wait<T>(pending: crate::worker::Pending<T>) -> ServiceError {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match pending.try_take() {
                Err(error) => return error,
                Ok(None) => assert!(Instant::now() < deadline),
                Ok(Some(_)) => panic!("expected an error"),
            }
            std::thread::yield_now();
        }
    }
    let worker = crate::worker::ServiceWorker::<()>::start(|| {
        Err(ServiceError::message(Code::DiskFull, "init diagnostic"))
    })
    .unwrap();
    assert_eq!(
        wait(worker.submit(|_| Ok(())).unwrap()).code,
        Code::DiskFull
    );
    let worker = crate::worker::ServiceWorker::start(|| Ok(())).unwrap();
    assert_eq!(
        wait(
            worker
                .submit::<()>(|_| { Err(ServiceError::message(Code::NotFound, "task diagnostic")) })
                .unwrap()
        )
        .code,
        Code::NotFound
    );
}

#[test]
fn storage_stream_failure_preserves_disk_full_code_and_previous_backup() {
    struct Full;
    impl io::Read for Full {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::StorageFull))
        }
    }
    let (_temp, mut storage) = super::test_storage();
    let before = storage.backup("project", &b"authored"[..]).unwrap();
    assert_eq!(
        storage.backup("project", Full).unwrap_err().code,
        Code::DiskFull
    );
    assert_eq!(storage.backups("project").unwrap(), vec![before]);
}
