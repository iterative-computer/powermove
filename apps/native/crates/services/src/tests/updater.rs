use crate::error::Result;
use crate::service_bail as bail;
use crate::updater;

#[derive(Default)]
struct FeedDouble {
    calls: Vec<updater::FeedRoute>,
    metadata: Option<updater::FeedMetadata>,
    fail: bool,
}
impl updater::UpdateFeed for FeedDouble {
    fn metadata(&mut self, route: updater::FeedRoute) -> Result<Option<updater::FeedMetadata>> {
        self.calls.push(route);
        if self.fail {
            bail!("feed unavailable")
        }
        Ok(self.metadata.clone())
    }
}
fn eligible() -> updater::TransitionGate {
    updater::TransitionGate {
        packaged_macos: true,
        macos_major: 14,
        migration_complete: true,
        transition_release_verified: true,
    }
}
fn feed(version: &str, os: u32) -> FeedDouble {
    FeedDouble {
        metadata: Some(updater::FeedMetadata {
            version: version.into(),
            name: format!("Powermove {version}"),
            size_bytes: 12_345_678,
            minimum_macos: os,
            notes: None,
        }),
        ..Default::default()
    }
}
#[test]
fn updater_closed_gates_do_not_contact_feed() {
    for gate in [
        updater::TransitionGate::default(),
        updater::TransitionGate {
            migration_complete: false,
            ..eligible()
        },
        updater::TransitionGate {
            transition_release_verified: false,
            ..eligible()
        },
        updater::TransitionGate {
            macos_major: 12,
            ..eligible()
        },
    ] {
        let mut feed = FeedDouble::default();
        assert_eq!(
            updater::check(gate, "1.2.0", &mut feed).unwrap(),
            updater::UpdateStatus::Gated
        );
        assert!(feed.calls.is_empty());
    }
}
#[test]
fn macos_13_routes_only_to_final_electron_compatible_metadata() {
    let gate = updater::TransitionGate {
        packaged_macos: true,
        macos_major: 13,
        ..Default::default()
    };
    let mut feed = feed("1.2.1", 13);
    assert!(matches!(
        updater::check(gate, "1.2.0", &mut feed).unwrap(),
        updater::UpdateStatus::Available(_)
    ));
    assert_eq!(feed.calls, [updater::FeedRoute::ElectronCompatible]);
}
#[test]
fn updater_refuses_downgrades_prereleases_and_unsupported_os() {
    for (current, offered, os) in [
        ("1.2.0", "1.1.9", 14),
        ("1.2.0", "1.2.0", 14),
        ("1.2.0", "2.0.0-beta.1", 14),
        ("1.2.0", "2.0.0", 15),
    ] {
        assert_eq!(
            updater::check(eligible(), current, &mut feed(offered, os)).unwrap(),
            updater::UpdateStatus::Current
        );
    }
}
#[test]
fn updater_allows_beta_lane_and_reports_feed_failure() {
    let mut feed = feed("2.0.0-beta.2", 14);
    assert!(matches!(
        updater::check(eligible(), "2.0.0-beta.1", &mut feed).unwrap(),
        updater::UpdateStatus::Available(_)
    ));
    assert_eq!(feed.calls, [updater::FeedRoute::NativeCutover]);
    feed.fail = true;
    assert!(updater::check(eligible(), "1.2.0", &mut feed).is_err());
}

#[test]
fn available_update_preserves_sheet_facts_and_untrusted_markdown_as_data() {
    let mut feed = feed("2.1.0", 14);
    let metadata = feed.metadata.as_mut().unwrap();
    metadata.name = "Powermove 2.1".into();
    metadata.notes = Some(
        "## Changes\n[Details](https://updates.invalid/notes)\n<script>untrusted()</script>".into(),
    );
    let expected = metadata.clone();
    let roundtrip: updater::FeedMetadata =
        serde_json::from_value(serde_json::to_value(&expected).unwrap()).unwrap();
    assert_eq!(roundtrip, expected);
    assert_eq!(
        updater::check(eligible(), "1.2.0", &mut feed).unwrap(),
        updater::UpdateStatus::Available(expected)
    );
}

#[test]
fn update_notes_limit_counts_utf8_bytes_and_returns_too_large() {
    let mut feed = feed("2.1.0", 14);
    feed.metadata.as_mut().unwrap().notes = Some("é".repeat(32 * 1024));
    assert!(matches!(
        updater::check(eligible(), "1.2.0", &mut feed).unwrap(),
        updater::UpdateStatus::Available(_)
    ));
    feed.metadata
        .as_mut()
        .unwrap()
        .notes
        .as_mut()
        .unwrap()
        .push('x');
    assert_eq!(
        updater::check(eligible(), "1.2.0", &mut feed)
            .unwrap_err()
            .code,
        crate::ServiceErrorCode::TooLarge
    );
}
