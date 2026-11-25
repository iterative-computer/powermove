//! PM-718: a person's Undo or Redo racing an agent's edit. An MCP or Aero
//! edit is a mutation lease (`MutateBegin`, writes, `MutateFinish`) that
//! spans several actor calls; the person's command can arrive before it
//! begins, while it applies, after its last write but before the step is
//! pushed (`MutateFinish`), and after. Each interleaving is a call order
//! here, so every test is deterministic. Undo always takes back exactly the
//! newest finished step; an edit still applying is never cut, folded into
//! or undone in part.

use pm_kernel::tx::{Ack, Call, Caller, EditCall, Handled, HistoryCall, Project, Reply, Request};
use pm_kernel::{Command, DocumentStore, Origin, WriteBody};
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Fixture", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "layer0", "type": "shape", "name": "Layer", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "layer1", "type": "shape", "name": "Other", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {}
}"##;

const AGENT: u32 = 41;

fn project() -> Project {
    Project::new(DocumentStore::new_untitled(parse(PROJECT).unwrap()))
}

fn person() -> Caller {
    Caller::native()
}

fn agent() -> Caller {
    Caller::realm(AGENT, 1, Origin::Mcp)
}

fn call(p: &mut Project, caller: Caller, call: impl Into<Call>) -> Ack {
    p.handle(Request::new(caller, call))
        .reply
        .expect("immediate reply")
}

fn transact(label: &str, command: Command) -> Call {
    Call::Transact {
        label: Some(label.into()),
        body: WriteBody::Commands(vec![command]),
        expected_revision: None,
    }
}

fn opacity(layer: &str, value: f64) -> Command {
    Command::SetStaticOpacity {
        layer_id: layer.into(),
        value,
    }
}

fn rename(layer: &str, name: &str) -> Command {
    Command::SetLayerName {
        layer_id: layer.into(),
        name: name.into(),
    }
}

fn layer(p: &Project, id: &str) -> Value {
    let root = p.store().snapshot().root().unwrap().to_value();
    root.get("comps")
        .and_then(|c| c.get("C1")?.get("layers")?.as_array())
        .and_then(|layers| {
            layers
                .iter()
                .find(|l| l.get("id").and_then(Value::as_str) == Some(id))
        })
        .cloned()
        .expect("layer")
}

fn opacity_of(p: &Project, id: &str) -> f64 {
    let layer = layer(p, id);
    layer
        .get("p")
        .and_then(|p| p.get("opacity")?.get("v")?.as_f64())
        .unwrap()
}

fn name_of(p: &Project, id: &str) -> String {
    layer(p, id)
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned()
}

/// The person's own finished step, below the agent's edit.
fn persons_step(p: &mut Project) {
    let ack = call(p, person(), transact("Opacity", opacity("layer0", 40.0)));
    assert!(matches!(ack.reply, Reply::Applied { .. }));
}

/// The agent's lease, opened as Aero opens it: guarded by the revision it
/// planned against.
fn agent_begins(p: &mut Project, planned: u64) -> Result<u64, Ack> {
    let ack = p
        .handle(
            Request::new(
                agent(),
                EditCall::MutateBegin {
                    label: "Agent edit".into(),
                },
            )
            .outside_revision(planned),
        )
        .reply
        .unwrap();
    match ack.reply {
        Reply::Mutation(lease) => Ok(lease),
        _ => Err(ack),
    }
}

fn agent_begins_now(p: &mut Project) -> u64 {
    let planned = p.store().revision();
    agent_begins(p, planned).unwrap()
}

fn agent_writes(p: &mut Project, command: Command) {
    let ack = call(p, agent(), transact("Agent write", command));
    assert!(matches!(ack.reply, Reply::Applied { .. }), "{ack:?}");
}

fn agent_finishes(p: &mut Project, lease: u64) -> Handled {
    p.handle(Request::new(
        agent(),
        EditCall::MutateFinish {
            lease,
            ops: Vec::new(),
            label: None,
        },
    ))
}

fn finished_ok(handled: &Handled) -> bool {
    matches!(&handled.reply.as_ref().unwrap().reply, Reply::Edit(r) if r.ok)
}

/// The agent's two-write edit: layer0 renamed, layer1 at 50.
fn agent_edit_applied(p: &Project) -> bool {
    name_of(p, "layer0") == "Agent" && opacity_of(p, "layer1") == 50.0
}

fn agent_edit_absent(p: &Project) -> bool {
    name_of(p, "layer0") == "Layer" && opacity_of(p, "layer1") == 100.0
}

/// A person's command that has to wait: no reply yet, a ticket instead.
fn waits(p: &mut Project, command: HistoryCall) -> u64 {
    let handled = p.handle(Request::new(person(), command));
    assert!(
        handled.reply.is_none(),
        "answered while the agent's edit was applying: {:?}",
        handled.reply
    );
    handled.deferred.expect("a deferred answer")
}

fn resolved(handled: &Handled, ticket: u64) -> Reply {
    handled
        .resolved
        .iter()
        .find(|(t, _)| *t == ticket)
        .map(|(_, ack)| ack.reply.clone())
        .unwrap_or_else(|| panic!("ticket {ticket} not answered: {:?}", handled.resolved))
}

fn undo(p: &mut Project) -> Reply {
    call(p, person(), HistoryCall::Undo).reply
}

fn redo(p: &mut Project) -> Reply {
    call(p, person(), HistoryCall::Redo).reply
}

#[test]
fn undo_before_the_agent_edit_begins_reverts_the_persons_step_and_refuses_the_stale_edit() {
    let mut p = project();
    persons_step(&mut p);
    let planned = p.store().revision();
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
    // The agent planned against the step the person just took back.
    assert!(agent_begins(&mut p, planned).is_err());
    assert!(p.open_entry().is_none());
    assert!(agent_edit_absent(&p));
    // Nothing of the agent's reached history: Redo still brings back the
    // person's step.
    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);

    // Replanned on the current revision, the edit lands as one step.
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    agent_writes(&mut p, opacity("layer1", 50.0));
    assert!(finished_ok(&agent_finishes(&mut p, lease)));
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
}

#[test]
fn undo_while_the_agent_edit_applies_waits_and_reverts_that_edit_whole() {
    let mut p = project();
    persons_step(&mut p);
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    let ticket = waits(&mut p, HistoryCall::Undo);
    // The person's step is untouched while the agent is still writing.
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
    agent_writes(&mut p, opacity("layer1", 50.0));
    let finished = agent_finishes(&mut p, lease);
    assert!(finished_ok(&finished), "the agent's edit is not cut");
    assert!(matches!(resolved(&finished, ticket), Reply::Bool(true)));
    assert!(agent_edit_absent(&p), "the whole agent edit is undone");
    assert_eq!(opacity_of(&p, "layer0"), 40.0, "the step below is kept");
    assert!(p.open_entry().is_none());

    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_applied(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
    assert!(agent_edit_absent(&p));
}

#[test]
fn undo_after_the_agent_edits_last_write_before_its_step_is_pushed_waits_for_the_step() {
    let mut p = project();
    persons_step(&mut p);
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    agent_writes(&mut p, opacity("layer1", 50.0));
    // Everything is written; the step is pushed only when the lease ends.
    let ticket = waits(&mut p, HistoryCall::Undo);
    assert!(agent_edit_applied(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
    let finished = agent_finishes(&mut p, lease);
    assert!(finished_ok(&finished));
    assert!(matches!(resolved(&finished, ticket), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
}

#[test]
fn undo_after_the_agent_edit_lands_reverts_exactly_it() {
    let mut p = project();
    persons_step(&mut p);
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    agent_writes(&mut p, opacity("layer1", 50.0));
    assert!(finished_ok(&agent_finishes(&mut p, lease)));
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_applied(&p));
}

#[test]
fn an_aborted_agent_edit_leaves_the_waiting_undo_the_persons_step() {
    let mut p = project();
    persons_step(&mut p);
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    let ticket = waits(&mut p, HistoryCall::Undo);
    let aborted = p.handle(Request::new(agent(), EditCall::MutateAbort { lease }));
    assert!(matches!(
        aborted.reply.as_ref().unwrap().reply,
        Reply::Bool(true)
    ));
    // Nothing of the agent's is on the stack: the newest finished step is
    // the person's own.
    assert!(matches!(resolved(&aborted, ticket), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
}

#[test]
fn redo_while_the_agent_edit_applies_waits_and_never_folds_into_it() {
    let mut p = project();
    persons_step(&mut p);
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    let ticket = waits(&mut p, HistoryCall::Redo);
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
    agent_writes(&mut p, opacity("layer1", 50.0));
    let finished = agent_finishes(&mut p, lease);
    assert!(finished_ok(&finished));
    // The agent's new step ended the redo stack, as any new step does; the
    // person's step is not re-applied inside the agent's.
    assert!(matches!(resolved(&finished, ticket), Reply::Bool(false)));
    assert!(agent_edit_applied(&p));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
}

/// The editor's Undo command first seals an edit its owner left open (a
/// drag that never sent its end). An agent's edit that is applying is never
/// one: it says so, so the editor sends only the Undo, which waits.
#[test]
fn an_agent_edit_that_is_applying_is_never_an_edit_left_open() {
    let mut p = project();
    persons_step(&mut p);
    assert!(!p.edit_applying());
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    assert!(p.open_entry().is_some() && p.edit_applying());
    let ticket = waits(&mut p, HistoryCall::Undo);
    agent_writes(&mut p, opacity("layer1", 50.0));
    let finished = agent_finishes(&mut p, lease);
    assert!(finished_ok(&finished), "the agent's edit finished whole");
    assert!(!p.edit_applying());
    assert!(matches!(resolved(&finished, ticket), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_applied(&p));
}

/// The agent's own Undo inside its own edit keeps today's nesting: it is
/// not held back by the edit it is part of.
#[test]
fn the_agents_own_undo_inside_its_edit_is_not_held() {
    let mut p = project();
    persons_step(&mut p);
    let lease = agent_begins_now(&mut p);
    let own = p.handle(Request::new(agent(), HistoryCall::Undo));
    assert!(own.reply.is_some(), "answered at once");
    assert!(finished_ok(&agent_finishes(&mut p, lease)));
}

#[test]
fn several_undos_while_the_agent_edit_applies_run_in_order_after_it() {
    let mut p = project();
    persons_step(&mut p);
    let lease = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    let first = waits(&mut p, HistoryCall::Undo);
    let second = waits(&mut p, HistoryCall::Undo);
    let third = waits(&mut p, HistoryCall::Undo);
    agent_writes(&mut p, opacity("layer1", 50.0));
    let finished = agent_finishes(&mut p, lease);
    assert!(matches!(resolved(&finished, first), Reply::Bool(true)));
    assert!(matches!(resolved(&finished, second), Reply::Bool(true)));
    assert!(matches!(resolved(&finished, third), Reply::Bool(false)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 100.0);
    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert!(matches!(redo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_applied(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
}

/// An in-app agent run: the turn is fenced (`AgentTurn`), each tool call is
/// its own lease, and the run's steps become one "Agent" step at its end.
/// During the turn the person's Undo takes back the person's own newest step
/// (the coedit rule); arriving while a lease applies, it waits for it, and
/// the agent's steps stay whole.
#[test]
fn a_multi_step_agent_turn_keeps_each_edit_whole_around_the_persons_undo() {
    let mut p = project();
    persons_step(&mut p);
    let mark = match call(&mut p, agent(), HistoryCall::Mark).reply {
        Reply::Mark(mark) => mark,
        other => panic!("{other:?}"),
    };
    call(&mut p, agent(), HistoryCall::AgentTurn { active: true });

    let first = agent_begins_now(&mut p);
    agent_writes(&mut p, rename("layer0", "Agent"));
    assert!(finished_ok(&agent_finishes(&mut p, first)));

    let second = agent_begins_now(&mut p);
    agent_writes(&mut p, opacity("layer1", 50.0));
    let ticket = waits(&mut p, HistoryCall::Undo);
    agent_writes(&mut p, opacity("layer1", 60.0));
    let finished = agent_finishes(&mut p, second);
    assert!(finished_ok(&finished));
    assert!(matches!(resolved(&finished, ticket), Reply::Applied { .. }));
    assert_eq!(opacity_of(&p, "layer0"), 100.0, "the person's own step");
    assert_eq!(name_of(&p, "layer0"), "Agent");
    assert_eq!(opacity_of(&p, "layer1"), 60.0);

    // The run ends: its steps are squashed before the turn is unfenced, so
    // no plain Undo can land on one tool call of the run.
    let revision = p.store().revision();
    let squashed = call(
        &mut p,
        agent(),
        HistoryCall::SquashSegments {
            mark,
            label: "Agent · Edit project".into(),
            expected_revision: revision,
        },
    );
    assert!(matches!(squashed.reply, Reply::Entry(Some(_))));
    call(&mut p, agent(), HistoryCall::AgentTurn { active: false });

    // The person's "Undo Opacity" is the newest step; then the whole run.
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
    assert_eq!(name_of(&p, "layer0"), "Agent");
    assert!(matches!(undo(&mut p), Reply::Bool(true)));
    assert!(agent_edit_absent(&p));
    assert_eq!(opacity_of(&p, "layer0"), 40.0);
}
