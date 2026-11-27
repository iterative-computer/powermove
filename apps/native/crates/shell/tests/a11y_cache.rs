#![cfg(target_os = "macos")]
//! P-A11Y-CACHE: with an assistive client connected, a frame that redraws a
//! parent reuses an unchanged cached view (no render) and replays its
//! accessibility nodes, focus and actions; the adapter receives only the
//! nodes that changed, and applying those updates gives the whole tree.
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, StyleRefinement,
    Styled as _, TestAppContext, Window,
    accesskit::{Action, Node, NodeId, Role, TreeUpdate},
    div, px,
};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

struct Child {
    renders: Rc<Cell<usize>>,
    presses: Rc<Cell<usize>>,
    focus: FocusHandle,
    label: &'static str,
}

impl Render for Child {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let presses = self.presses.clone();
        div()
            .id("child")
            .role(Role::Group)
            .aria_label("Child")
            .child(
                div()
                    .id("button")
                    .role(Role::Button)
                    .aria_label(self.label)
                    .size(px(20.))
                    .track_focus(&self.focus)
                    .on_click(move |_, _, _| presses.set(presses.get() + 1)),
            )
    }
}

struct Parent {
    child: Entity<Child>,
    tick: usize,
}

impl Render for Parent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut style = StyleRefinement::default();
        style.size.width = Some(px(80.).into());
        style.size.height = Some(px(80.).into());
        div()
            .id("root")
            .role(Role::Group)
            .size_full()
            .child(
                div()
                    .id("tick")
                    .role(Role::Label)
                    .aria_label(format!("Tick {}", self.tick)),
            )
            .child(self.child.clone().cached(style))
    }
}

/// The adapter's view of the tree: updates applied in order, keeping only
/// the nodes reachable from the root, as AccessKit does.
#[derive(Default)]
struct Adapter(HashMap<NodeId, Node>, Option<NodeId>);

impl Adapter {
    fn apply(&mut self, update: TreeUpdate) {
        if let Some(tree) = update.tree {
            self.1 = Some(tree.root);
        }
        self.0.extend(update.nodes);
        let mut reachable = HashSet::new();
        let mut stack = vec![self.1.expect("a first update names the root")];
        while let Some(id) = stack.pop() {
            let node = self.0.get(&id).expect("every child is in the tree");
            reachable.insert(id);
            stack.extend(node.children().iter().copied());
        }
        self.0.retain(|id, _| reachable.contains(id));
    }
}

fn id_of(tree: &TreeUpdate, label: &str) -> NodeId {
    tree.nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("no node labelled {label}"))
}

#[gpui::test]
fn reused_view_replays_its_nodes_and_updates_carry_only_changes(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let presses = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, cx| Parent {
        child: cx.new(|cx| Child {
            renders: renders.clone(),
            presses: presses.clone(),
            focus: cx.focus_handle(),
            label: "Press me",
        }),
        tick: 0,
    });
    let any: AnyWindowHandle = window.into();
    let draw = |cx: &mut TestAppContext| {
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
            let whole = window.a11y_tree().cloned().expect("a tree");
            (whole, window.take_a11y_updates())
        })
        .unwrap()
    };
    let mut adapter = Adapter::default();

    // A client connects: the first update is the whole tree.
    cx.update_window(any, |_, window, _| window.simulate_a11y_activation())
        .unwrap();
    let (whole, updates) = draw(cx);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].nodes.len(), whole.nodes.len());
    updates.into_iter().for_each(|u| adapter.apply(u));
    let button = id_of(&whole, "Press me");

    // Focus the button; the focus change reaches the adapter.
    let focus = window
        .update(cx, |parent, _, cx| parent.child.read(cx).focus.clone())
        .unwrap();
    cx.update_window(any, |_, window, cx| window.focus(&focus, cx))
        .unwrap();
    let (whole, updates) = draw(cx);
    assert_eq!(whole.focus, button);
    updates.into_iter().for_each(|u| adapter.apply(u));
    let rendered = renders.get();

    // Parent frames: the child is reused, its nodes stay, focused, and only
    // the parent's changed label is sent.
    for tick in 1..=3 {
        window
            .update(cx, |parent, _, cx| {
                parent.tick = tick;
                cx.notify();
            })
            .unwrap();
        let (whole, updates) = draw(cx);
        assert_eq!(
            renders.get(),
            rendered,
            "an unchanged cached view renders again"
        );
        assert_eq!(id_of(&whole, "Press me"), button);
        assert_eq!(whole.focus, button, "a replayed node keeps its focus");
        assert_eq!(updates.len(), 1);
        let sent: Vec<_> = updates[0].nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            sent,
            [id_of(&whole, &format!("Tick {tick}"))],
            "only changes are sent"
        );
        updates.into_iter().for_each(|u| adapter.apply(u));
        let whole_ids: HashSet<_> = whole.nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(adapter.0.keys().copied().collect::<HashSet<_>>(), whole_ids);
        for (id, node) in &whole.nodes {
            assert_eq!(adapter.0.get(id), Some(node), "the adapter's node matches");
        }
    }

    // A frame where nothing changed sends nothing.
    cx.update_window(any, |_, window, _| window.refresh()).ok();
    window.update(cx, |_, _, cx| cx.notify()).unwrap();
    let (_, updates) = draw(cx);
    assert!(updates.iter().all(|u| u.nodes.is_empty()), "{updates:?}");

    // The replayed node still answers an assistive Press.
    cx.update_window(any, |_, window, cx| {
        window.perform_a11y_action(button, Action::Click, None, cx)
    })
    .unwrap();
    assert_eq!(presses.get(), 1, "a replayed node keeps its actions");

    // The child changes: it renders, and its node is sent.
    window
        .update(cx, |parent, _, cx| {
            parent.child.update(cx, |child, cx| {
                child.label = "Pressed";
                cx.notify();
            })
        })
        .unwrap();
    let (whole, updates) = draw(cx);
    assert!(renders.get() > rendered);
    let pressed = id_of(&whole, "Pressed");
    assert_eq!(pressed, button);
    assert!(
        updates
            .iter()
            .any(|u| u.nodes.iter().any(|(id, _)| *id == button))
    );
    updates.into_iter().for_each(|u| adapter.apply(u));
    assert_eq!(
        adapter.0.get(&button).and_then(|n| n.label()),
        Some("Pressed")
    );
}
