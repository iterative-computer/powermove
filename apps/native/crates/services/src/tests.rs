use crate::storage::ServiceStorage;

fn test_storage() -> (tempfile::TempDir, ServiceStorage) {
    let temp = tempfile::tempdir().unwrap();
    let storage = ServiceStorage::open(temp.path().join("services")).unwrap();
    (temp, storage)
}

mod diagnostics;
mod foundation;
mod media;
mod worker;
