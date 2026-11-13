//! Profile keys only the native app writes (Settings › Shortcuts' keymap):
//! accepted as settings, kept in the store, read back on the next launch.
mod common;
use pm_kernel::library::{NATIVE_KEYS, StoreKey, parse_key};
use pm_kernel::persistence::{DiskFormat, LibraryWriter};
use pm_kernel::state::{StateOp, StateSnapshot, StateTransaction, is_setting};
use pm_model::json::parse;

#[test]
fn the_keymap_is_a_setting_that_is_stored_and_read_back() {
    assert_eq!(NATIVE_KEYS, ["keymap"]);
    assert!(is_setting("keymap"));
    assert_eq!(parse_key("keymap"), Some(StoreKey::Static("keymap".into())));
    let keymap = parse(
        r#"{"preset":"premiere-pro","bindings":[{"command":"extendEdit","args":[],"keys":["e"]}]}"#,
    )
    .unwrap();
    let (next, _) = StateSnapshot::seeded()
        .stage(StateTransaction {
            ops: vec![StateOp::Setting {
                key: "keymap".into(),
                value: Some(keymap.clone()),
            }],
            ..Default::default()
        })
        .expect("a keymap is a setting");
    assert_eq!(next.settings.get("keymap"), Some(&keymap));
    let dir = common::TempDir::new("native-keys");
    let writer = LibraryWriter::open(&dir.0, DiskFormat::V1_2, None).unwrap();
    writer.set("keymap", keymap.clone()).unwrap();
    writer.flush().unwrap();
    assert!(dir.path("store/keymap.json").exists());
    let loaded = StateSnapshot::load(writer.store()).unwrap();
    assert_eq!(loaded.settings.get("keymap"), Some(&keymap));
}
