mod common;

use common::temp_dir;

#[test]
fn media_directory_is_removed_after_success_and_unwind() {
    let success_path = {
        let dir = temp_dir("cleanup-success");
        std::fs::write(dir.join("media.wav"), b"fixture").unwrap();
        assert!(dir.join("media.wav").exists());
        dir.to_path_buf()
    };
    assert!(!success_path.exists());

    let panic_dir = temp_dir("cleanup-panic");
    let panic_path = panic_dir.to_path_buf();
    let result = std::panic::catch_unwind(move || {
        std::fs::write(panic_dir.join("media.wav"), b"fixture").unwrap();
        panic!("check temporary media cleanup on unwind");
    });
    assert!(result.is_err());
    assert!(!panic_path.exists());
}
