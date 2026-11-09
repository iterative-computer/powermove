use crate::storage::ServiceStorage;

fn test_storage() -> (tempfile::TempDir, ServiceStorage) {
    let temp = tempfile::tempdir().unwrap();
    let storage = ServiceStorage::open(temp.path().join("services")).unwrap();
    (temp, storage)
}

mod diagnostics;
mod entrypoints;
mod errors;
mod foundation;
mod store;
mod updater;
mod worker;
