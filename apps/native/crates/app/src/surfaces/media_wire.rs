//! Rust → v4 data-only projections. No raw service/provider diagnostic is
//! accepted here. U64 identities are decimal strings (N3/N7: no JS rounding).
use super::media::{self, Action, Target};
use serde_json::{Value, json};
pub fn target(target: &Target) -> Value {
    json!({"owner": {"window": target.owner.window, "project": target.owner.project,
        "document": target.owner.document.to_string(), "revision": target.owner.revision.to_string()},
        "id": target.id, "generation": target.generation.to_string()})
}
pub fn source(source: &media::source::Source) -> Value {
    json!({"target": target(&source.target), "title": source.title, "line": source.line,
        "button": match source.button { Some(Action::Locate) => Some("locate"), Some(Action::Download) => Some("download"), _ => None }})
}
pub fn import_task(import: &media::import::Import) -> Value {
    let row = import.row();
    json!({"target": target(import.target()), "title": row.title, "detail": row.detail,
        "progress": row.completed as f64 / row.total as f64,
        "severity": match row.severity { media::Severity::Info => "info", media::Severity::Warning => "warning", media::Severity::Error => "error" },
        "cancel": row.cancel, "retry": row.retry,
        "disclosure": row.disclosure.into_iter().map(|(file, reason)| json!({"file":file,"reason":reason})).collect::<Vec<_>>()})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_identity_preserves_every_bit_and_source_actions_match_v4() {
        let t = Target {
            owner: media::Owner {
                window: "w".into(),
                project: "p".into(),
                document: u64::MAX,
                revision: u64::MAX - 1,
            },
            id: "a".into(),
            generation: u64::MAX,
        };
        let source =
            media::source::Source::new(t, "a", media::source::Cause::Cloud("iCloud".into()));
        let value = super::source(&source);
        assert_eq!(value["target"]["owner"]["document"], "18446744073709551615");
        assert_eq!(value["target"]["owner"]["revision"], "18446744073709551614");
        assert_eq!(value["target"]["generation"], "18446744073709551615");
        assert_eq!(value["button"], "download");
    }
    #[test]
    fn import_projection_carries_exact_task_copy_measured_progress_and_disclosure() {
        let t = Target {
            owner: media::Owner {
                window: "w".into(),
                project: "p".into(),
                document: 1,
                revision: 0,
            },
            id: "i".into(),
            generation: 0,
        };
        let i = media::import::Import::new(
            t,
            media::import::Origin::Files,
            vec![media::import::File {
                job: "j".into(),
                name: "a.mov".into(),
                asset: "a".into(),
                state: media::import::State::Failed(media::import::Reason::Permission),
                proxy_failed: false,
            }],
        )
        .unwrap();
        let value = import_task(&i);
        assert_eq!(value["title"], "Couldn’t import 1 file");
        assert_eq!(value["progress"], 1.0);
        assert_eq!(value["severity"], "error");
        assert_eq!(
            value["disclosure"][0]["reason"],
            "You don’t have permission to read it"
        );
        assert_eq!(value["retry"], true);
        assert_eq!(value["cancel"], false);
    }
}
