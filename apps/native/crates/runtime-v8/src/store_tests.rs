use super::*;
#[test]
fn protocol_bounds_version_truncation_and_round_trip() {
    let good = json!({"v":PROTOCOL,"kind":"eval","id":7,"source":"😀"});
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &good).unwrap();
    assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), good);
    for len in [0u32, (FRAME_LIMIT + 1) as u32, u32::MAX] {
        assert!(matches!(
            read_frame(&mut len.to_le_bytes().as_slice()),
            Err(Error::Quota(_))
        ));
    }
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &json!({"v":2})).unwrap();
    assert!(read_frame(&mut bytes.as_slice()).is_err());
    assert!(read_frame(&mut [4, 0, 0, 0, b'{'].as_slice()).is_err());
}
#[test]
fn generations_are_cluster_and_session_fenced() {
    let mut g = Generation {
        realm: "cluster".into(),
        number: 3,
    };
    validate_generation(&mut g, "cluster", 7).unwrap();
    assert_eq!(g.number, (7 << 32) | 3);
    assert!(validate_generation(&mut g, "cluster", 8).is_err());
    assert!(
        validate_generation(
            &mut Generation {
                realm: "other".into(),
                number: 1
            },
            "cluster",
            7
        )
        .is_err()
    );
    assert!(
        validate_generation(
            &mut Generation {
                realm: "cluster".into(),
                number: 0
            },
            "cluster",
            7
        )
        .is_err()
    );
}
#[test]
fn seatbelt_profile_has_no_user_or_network_grants() {
    assert!(STORE_SEATBELT.contains("(deny default)"));
    assert!(STORE_SEATBELT.contains("(allow dynamic-code-generation)"));
    for grant in [
        "(allow network",
        "(allow process-exec",
        "(allow file-write",
        "(subpath \"/Users",
    ] {
        assert!(!STORE_SEATBELT.contains(grant));
    }
}
