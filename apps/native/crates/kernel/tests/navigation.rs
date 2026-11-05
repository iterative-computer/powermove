use pm_kernel::{navigation::Flowchart, *};
use pm_model::json::parse;
fn project() -> Project {
    Project::new(DocumentStore::new_untitled(parse(r##"{
        "id":"P","compId":"A","compName":"Main","w":640,"h":360,"fps":30,"dur":10,
        "assets":{},"layers":[
            {"id":"p1","type":"precomp","d":{"comp":"B"}},
            {"id":"p2","type":"precomp","d":{"comp":"B"}},
            {"id":"missing","type":"precomp","d":{"comp":"gone"}},
            {"id":"ordinary","type":"shape","d":{"comp":"C"}}],
        "comps":{"B":{"id":"B","name":"Child","w":320,"h":180,"fps":24,"dur":2,"layers":[]},
                 "C":{"id":"C","name":"Parent","layers":[{"id":"use","type":"precomp","d":{"comp":"A"}}]}}
    }"##).unwrap()))
}
fn call(p: &mut Project, c: Call) -> Ack {
    p.handle(Request::new(Caller::native(), c)).reply.unwrap()
}
fn navigate(p: &mut Project, id: &str) -> Ack {
    let document = p.store().doc_id();
    call(
        p,
        Call::OpenComposition {
            comp_id: id.into(),
            document,
        },
    )
}
#[test]
fn relationships_ignore_missing_and_non_precomp_links_and_deduplicate_instances() {
    let p = project();
    let chart = Flowchart::read(&p.store().snapshot()).unwrap();
    assert_eq!(chart.current.id, "A");
    assert_eq!(
        chart
            .children
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
    assert_eq!(
        chart
            .parents
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["C"]
    );
}
#[test]
fn navigation_publishes_real_composition_without_undo_or_content_changes() {
    let mut p = project();
    let before = p.store().root().unwrap().get("comps").unwrap().clone();
    let history = p.store().history().len();
    let ack = navigate(&mut p, "B");
    assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
    assert_eq!(p.store().open_comp().as_deref(), Some("B"));
    assert_eq!(p.store().root().unwrap().get("comps"), Some(&before));
    assert_eq!(p.store().history().len(), history);
    let chart = Flowchart::read(&p.store().snapshot()).unwrap();
    assert_eq!(chart.current.name, "Child");
    assert!(chart.children.is_empty());
    assert_eq!(chart.parents[0].id, "A");
    let revision = p.store().revision();
    assert!(matches!(navigate(&mut p, "gone").reply, Reply::Throw(_)));
    assert_eq!(p.store().revision(), revision);
    assert!(matches!(
        navigate(&mut p, "B").reply,
        Reply::Applied { changed: false, .. }
    ));
}
#[test]
fn navigation_preserves_an_in_progress_edit() {
    let mut p = project();
    call(
        &mut p,
        Call::History(HistoryCall::Begin {
            label: "Editing".into(),
        }),
    );
    let revision = p.store().revision();
    assert!(matches!(navigate(&mut p, "B").reply, Reply::Throw(_)));
    assert_eq!(p.store().open_comp().as_deref(), Some("A"));
    assert_eq!(p.store().revision(), revision);
}

#[test]
fn revisiting_restores_only_valid_layer_selection_and_same_comp_is_a_noop() {
    let mut p = project();
    call(
        &mut p,
        Call::Selection(SelectionCall::Layers {
            ids: vec!["p1".into()],
            add: false,
        }),
    );
    let revision = p.store().revision();
    navigate(&mut p, "A");
    assert_eq!(p.store().revision(), revision);
    assert_eq!(p.store().selection().layers(), ["p1"]);
    navigate(&mut p, "B");
    assert!(p.store().selection().is_empty());
    navigate(&mut p, "A");
    assert_eq!(p.store().selection().layers(), ["p1"]);
}

#[test]
fn queued_navigation_cannot_switch_a_replaced_document_with_the_same_ids() {
    let mut p = project();
    let revision = p.store().revision();
    let document = p.store().doc_id() + 1;
    assert!(matches!(
        call(
            &mut p,
            Call::OpenComposition {
                comp_id: "B".into(),
                document
            }
        )
        .reply,
        Reply::Throw(_)
    ));
    assert_eq!(p.store().revision(), revision);
    assert_eq!(p.store().open_comp().as_deref(), Some("A"));
}
