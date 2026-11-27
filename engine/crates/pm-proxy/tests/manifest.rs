use pm_proxy::{SequenceFrame, SequenceManifest, SequenceRate};

fn sequence() -> SequenceManifest {
    SequenceManifest {
        powermove_sequence: 1,
        rate: SequenceRate { num: 24, den: 1 },
        frames: vec![
            SequenceFrame {
                path: "/original/frame1.png".into(),
                hash: "ab".repeat(32),
            },
            SequenceFrame {
                path: "/original/frame2.png".into(),
                hash: "cd".repeat(32),
            },
        ],
    }
}

#[test]
fn sequence_identity_survives_moves_and_hex_case_changes() {
    let original = sequence();
    let mut moved = original.clone();
    for frame in &mut moved.frames {
        frame.path = "/relinked/another-name.png".into();
        frame.hash = frame.hash.to_ascii_uppercase();
    }
    assert_eq!(
        original.content_hash().unwrap(),
        moved.content_hash().unwrap()
    );
}

#[test]
fn sequence_identity_includes_order_rate_and_every_frame_content() {
    let original = sequence();
    let key = original.content_hash().unwrap();
    let mut changed = original.clone();
    changed.frames.reverse();
    assert_ne!(key, changed.content_hash().unwrap());
    changed = original.clone();
    changed.rate.num = 25;
    assert_ne!(key, changed.content_hash().unwrap());
    changed = original;
    changed.frames[1].hash = "ef".repeat(32);
    assert_ne!(key, changed.content_hash().unwrap());
}

#[test]
fn invalid_rate_manifest_and_frame_identity_are_rejected() {
    for (num, den) in [(0, 1), (24, 0), (u32::MAX, 1), (24, u32::MAX)] {
        let mut value = sequence();
        value.rate = SequenceRate { num, den };
        assert!(value.content_hash().is_err());
    }
    for hash in ["bad".into(), "zz".repeat(32)] {
        let mut value = sequence();
        value.frames[0].hash = hash;
        assert!(value.content_hash().is_err());
    }
    let mut value = sequence();
    value.frames[0].path = "relative.png".into();
    assert!(value.content_hash().is_err());
    value = sequence();
    value.frames.clear();
    assert!(value.content_hash().is_err());
    value = sequence();
    value.powermove_sequence = 2;
    assert!(value.content_hash().is_err());
}
