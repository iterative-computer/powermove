//! Source-derived cases: v1.2.0 inline-prompt.ts:46–73,196–368;
//! attachments.ts:18–55; Composer.svelte:145–179. Synthetic inputs only.
use pm_panel_agentpanel::{
    Error,
    composer::{Attachment, Draft, Part},
};
use serde_json::{Value, json};
fn file(id: &str, name: &str) -> Attachment {
    Attachment::from_record(json!({"id":id,"name":name,"type":"application/octet-stream","size":3,"dataBase64":"AP+A","future":{"retain":true}})).unwrap()
}
#[test]
fn utf16_offsets_preserve_unicode_token_order_and_unknown_metadata() {
    let a = json!({"id":"a","name":"a.bin","dataBase64":"AP8=","promptOffset":2,"future":[1,2]});
    let b = json!({"id":"b","name":"b.bin","dataBase64":"","promptOffset":2});
    let draft = Draft::from_legacy("😀\nlast", &[a.clone(), b.clone()]).unwrap();
    assert_eq!(
        draft.request_text(),
        "😀[Attachment: a.bin][Attachment: b.bin]\nlast"
    );
    assert_eq!(draft.legacy(), ("😀\nlast".into(), vec![a, b]));
    assert_eq!(draft.selection(), 9..9);
}
#[test]
fn invalid_utf16_and_duplicate_ids_fail_without_normalizing() {
    for offset in [1, 99] {
        assert!(
            Draft::from_legacy("😀", &[json!({"id":"a","name":"A","promptOffset":offset})])
                .is_err()
        );
    }
    let item = json!({"id":"a","name":"A"});
    assert!(Draft::from_legacy("", &[item.clone(), item]).is_err());
    let mut draft = Draft::from_legacy("😀", &[]).unwrap();
    assert_eq!(draft.select(1..1), Err(Error::Invalid("split surrogate")));
    assert_eq!(
        draft.select(std::ops::Range { start: 2, end: 1 }),
        Err(Error::Invalid("selection order"))
    );
}
#[test]
fn file_only_draft_and_following_text_stay_on_one_line() {
    let mut d = Draft::default();
    d.add_files(vec![file("a", "a.bin")], 0).unwrap();
    assert!(d.sendable());
    d.insert_text("then", 1, true);
    assert_eq!(d.request_text(), "[Attachment: a.bin]then");
    assert_eq!(d.legacy().0, "then");
}
#[test]
fn two_step_delete_is_repeat_safe_and_undo_restores_bytes() {
    let mut d = Draft::default();
    d.add_files(vec![file("a", "a.bin")], 0).unwrap();
    assert!(d.delete(true, false, 1));
    assert_eq!(d.armed(), Some("a"));
    assert!(d.delete(true, true, 2));
    assert_eq!(d.attachments().count(), 1);
    assert!(d.delete(true, false, 3));
    assert_eq!(d.attachments().count(), 0);
    assert!(d.undo(false));
    assert_eq!(d.attachment("a").unwrap().record()["dataBase64"], "AP+A");
    assert!(d.undo(true));
    assert!(!d.sendable());
}
#[test]
fn forward_delete_and_pointer_disarm_require_another_arm() {
    let mut d = Draft::default();
    d.add_files(vec![file("a", "a")], 0).unwrap();
    d.select(0..0).unwrap();
    d.delete(false, false, 1);
    d.disarm();
    d.delete(false, false, 2);
    assert_eq!(d.attachments().count(), 1);
    d.delete(false, false, 3);
    assert!(!d.sendable());
}
#[test]
fn selected_tokens_survive_paste_and_typing() {
    let mut d = Draft::from_legacy(
        "left right",
        &[json!({"id":"a","name":"A","promptOffset":5,"dataBase64":""})],
    )
    .unwrap();
    d.select_all();
    d.insert_text("\nnew", 1, false);
    assert_eq!(d.request_text(), "left [Attachment: A]right\nnew");
}
#[test]
fn native_word_deletion_does_not_invent_empty_line_breaks() {
    let mut d = Draft::from_legacy("one\ntwo\nthree", &[]).unwrap();
    d.delete_range(4..7, 1).unwrap();
    assert_eq!(d.legacy().0, "one\n\nthree");
    d.delete_range(3..5, 2).unwrap();
    assert_eq!(d.legacy().0, "onethree");
    assert_eq!(d.selection(), 3..3);
    d.select_all();
    d.delete(true, false, 3);
    assert_eq!(d.legacy().0, "");
    assert_eq!(d.selection(), 0..0);
}
#[test]
fn invalid_native_delete_never_removes_tokens() {
    let mut d = Draft::default();
    d.add_files(vec![file("a", "a")], 0).unwrap();
    assert!(d.delete_range(0..1, 1).is_err());
    assert!(d.sendable());
}
#[test]
fn clipboard_cut_paste_moves_each_token_once_and_preserves_plain_fallback() {
    let mut d = Draft::from_legacy(
        "LR",
        &[json!({"id":"a","name":"A","promptOffset":1,"dataBase64":"AA=="})],
    )
    .unwrap();
    d.select_all();
    let clip = d.clipboard(true, 1);
    assert_eq!(clip.plain, "LAR");
    assert!(!d.sendable());
    d.paste_parts(&clip.parts, 2).unwrap();
    assert_eq!(d.request_text(), "L[Attachment: A]R");
    d.paste_parts(&[Part::File("a".into()), Part::File("a".into())], 3)
        .unwrap();
    assert_eq!(d.attachments().count(), 1);
    assert_eq!(d.request_text(), "LR[Attachment: A]");
}
#[test]
fn invalid_clipboard_preserves_selection_and_draft() {
    let mut d = Draft::from_legacy("keep", &[]).unwrap();
    d.select(0..2).unwrap();
    let before = d.legacy();
    assert!(
        d.paste_parts(
            &[Part::Text("replace".into()), Part::File("unknown".into())],
            0
        )
        .is_err()
    );
    assert_eq!(d.legacy(), before);
    assert_eq!(d.selection(), 0..2);
}
#[test]
fn attachment_drag_preserves_utf16_text_and_is_one_undo() {
    let mut d = Draft::from_legacy("😀xy", &[]).unwrap();
    d.add_files(vec![file("a", "A")], 0).unwrap();
    d.move_file("a", 2, 1).unwrap();
    assert_eq!(d.request_text(), "😀[Attachment: A]xy");
    assert!(d.undo(false));
    assert_eq!(d.request_text(), "😀xy[Attachment: A]");
}
#[test]
fn typing_coalesces_only_inside_600ms_and_redo_branch_is_replaced() {
    let mut d = Draft::default();
    d.insert_text("a", 1, true);
    d.insert_text("b", 600, true);
    d.insert_text("c", 1200, true);
    assert!(d.undo(false));
    assert_eq!(d.request_text(), "ab");
    assert!(d.undo(false));
    assert_eq!(d.request_text(), "");
    d.insert_text("other", 1300, true);
    assert!(!d.undo(true));
    assert_eq!(d.request_text(), "other");
}
#[test]
fn local_history_does_not_cross_sent_draft_boundary() {
    let mut d = Draft::from_legacy("private draft", &[]).unwrap();
    d.clear_after_send();
    assert!(!d.undo(false));
    assert!(!d.undo(true));
    assert!(!d.sendable());
}
#[test]
fn attachment_identity_conflict_is_atomic() {
    let mut d = Draft::default();
    d.add_files(vec![file("a", "old")], 0).unwrap();
    let old = d.request_text();
    assert!(
        d.add_files(vec![file("b", "B"), file("a", "changed")], 1)
            .is_err()
    );
    assert_eq!(d.request_text(), old);
    assert!(d.attachment("b").is_none());
}
#[test]
fn zero_byte_files_have_payload_and_missing_payloads_remain_explicit() {
    let zero = Attachment::from_record(json!({"id":"z","name":"zero","dataBase64":""})).unwrap();
    let missing =
        Attachment::from_record(json!({"id":"m","name":"missing","payloadStored":true})).unwrap();
    assert!(zero.payload_available());
    assert!(!missing.payload_available());
    assert_eq!(missing.record()["payloadStored"], true);
}
#[test]
fn preview_types_match_tag_and_svg_bytes_still_survive() {
    for (mime, expected) in [
        ("image/png", true),
        ("image/SVG+xml", true),
        ("image/avif", true),
        ("image/tiff", false),
        ("text/plain", false),
    ] {
        let v: Value = json!({"id":"a","name":"x","type":mime,"dataUrl":"data:image/png;base64,AA==","dataBase64":"AA=="});
        assert_eq!(Attachment::from_record(v).unwrap().previewable(), expected);
    }
}
