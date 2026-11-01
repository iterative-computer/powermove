use pm_agents::conversation;
use serde_json::{Value, json};
#[test]
fn bounded_context_matches_unmodified_ts_thread_model() {
    let f: Value = serde_json::from_str(include_str!("../fixtures/thread-context.json")).unwrap();
    let m = f["messages"].as_array().unwrap();
    assert_eq!(conversation::context(m, "thread-1"), f["context"]);
    assert_eq!(
        serde_json::to_value(m.iter().map(conversation::message_text).collect::<Vec<_>>()).unwrap(),
        f["messageText"]
    );
}
#[test]
fn original_brief_remains_with_decisions_after_long_conversation() {
    let mut messages = vec![json!({"role":"user","text":"Authored original direction"})];
    for _ in 0..40 {
        messages.push(json!({"role":"assistant","text":"Long progress ".repeat(1000)}));
    }
    let excerpts = conversation::for_agent(&messages);
    assert_eq!(excerpts[0].text, "Authored original direction");
    assert!(excerpts.iter().skip(1).any(|v| v.truncated));
    assert_eq!(messages[0]["text"], "Authored original direction");
}
#[test]
fn question_decisions_and_visual_artifacts_are_readable_by_id() {
    let q = json!({"role":"trace","steps":[{"kind":"question","status":"closed","questions":[{"id":"q","question":"Continue?"}]}]});
    assert_eq!(
        conversation::message_text(&q),
        "You asked: Continue?\n(Unanswered)"
    );
    let m = json!({"role":"assistant","text":"Ready","visualReplies":[{"title":"Frame","id":"frame-1"}]});
    assert_eq!(
        conversation::message_text(&m),
        "Ready\n[Visual reply: Frame; artifact frame-1]"
    );
}
#[test]
fn title_normalizes_whitespace_without_splitting_unicode() {
    assert_eq!(
        conversation::title(&[json!({"role":"user","text":"  Synthetic\n  direction "})]),
        "Synthetic direction"
    );
    assert_eq!(conversation::title(&[]), "New thread");
    let title = conversation::title(&[json!({"role":"user","text":"🎞".repeat(40)})]);
    assert_eq!(title.encode_utf16().count(), 64);
}
