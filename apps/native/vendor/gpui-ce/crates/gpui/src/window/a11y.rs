//! Accessibility support, provided by [AccessKit][accesskit].
//!
//! There are user-facing guide-level docs [here](crate::_accessibility).
//!
//! ## Architecture
//!
//! ```text
//!                              ┌────────────────────────────────┐   ┌─────────────────────┐
//!                           ┌─▶│ AccessKit Adapter (MacOS)      │◀─▶│ MacOS System APIs   │
//!                           │  └────────────────────────────────┘   └─────────────────────┘
//!                           │
//! ┌──────┐   ┌───────────┐  │  ┌────────────────────────────────┐   ┌─────────────────────┐
//! │ GPUI │◀─▶│ AccessKit │◀─┼─▶│ AccessKit Adapter (Windows)    │◀─▶│ Windows System APIs │
//! └──────┘   └───────────┘  │  └────────────────────────────────┘   └─────────────────────┘
//!                           │
//!                           │  ┌────────────────────────────────┐   ┌─────────────────────┐
//!                           └─▶│ AccessKit Adapter (Linux)      │◀─▶│ dbus                │
//!                              └────────────────────────────────┘   └─────────────────────┘
//! ```
//!
//! In order for GPUI apps to be usable for people using assistive technology,
//! we must do a few things:
//! - Inform the system when the UI changes meaningfully. This includes:
//!   - Reporting new/removed/changed UI elements
//!   - *Not* reporting irrelevant UI changes, e.g. an invisible `div()` being
//!     added.
//!   - Reporting the appearance and capabilities of each UI element. For example:
//!     - What does this piece of text say?
//!     - How far along is this progress bar?
//!     - Can this node be focused?
//!     - Can this node have a value directly assigned? (e.g. a slider)
//! - Allowing the system to interact with the UI by dispatching actions to
//!   nodes. Note that AccessKit has its own [`Action`] type, which is not the
//!   [`crate::Action`] trait.
//! - Activate and deactivate accessibility features when requested by the
//!   system.
//!
//! Activating and deactivating at the right time is trivial, so I won't go into
//! detail here. The other two are almost orthogonal in implementation.
//!
//! The state for both lives in the [`A11y`] struct in this module.
//!
//! ### Reporting UI changes
//!
//! Every frame, we build a [`TreeUpdate`] and send it to the platform-specific
//! adapter. A [`TreeUpdate`] is a representation of a subset of the UI tree.
//! When the adapter receives the update, it diffs it against the previous
//! update, and calls platform-specific APIs to inform screen readers about the
//! changes. Nodes may have been created, destroyed, or updated.
//!
//! Each node has an ID, and this ID *should* be stable across frames. If a
//! node's ID changes, then, from AccessKit's point of view, it is a different
//! node.
//!
//! We derive the node ID from the [`GlobalElementId`] in
//! [`GlobalElementId::accesskit_node_id`]. Nodes without [`GlobalElementId`]s
//! cannot produce an AccessKit [`NodeId`], and so are not included in the
//! accessibility tree. We try to warn when using accessibility APIs on
//! [`div()`] without setting an ID.
//!
//! This all happens in [`Drawable::prepaint`]. The [`A11y`] struct maintains a
//! stack of nodes during prepainting, which we can use to calculate the
//! [`NodeId`]s, and record parent-child relationships. Once all [`Element`]s in
//! a frame have been prepainted, we send the resulting [`TreeUpdate`] object to
//! the adapter and the screen reader can announce the changes.
//!
//! #### Synthetic children
//!
//! Additionally, some nodes can register "synthetic children" using
//! [`Element::a11y_synthetic_children`]. Normally, one accesskit node is pushed
//! for every [`Element`] with a role and id. However, sometimes a single
//! element may want to produce many accesskit nodes. These extra nodes are
//! referred to as "synthetic children" of the element providing a non-default
//! [`Element::a11y_synthetic_children`] implementation.
//!
//! The user is provided a builder-style API using [`A11ySubtreeBuilder`], which
//! allows them to create push nodes that are children of the current node, as
//! well as modify the current node itself.
//!
//! GPUI calls this callback *after* prepainting (and just before popping the
//! corresponding element), since this step may need prepaint information to be
//! available. In the future, we may want to add prepaint information more
//! generally to [`Element::write_a11y_info`], but for now that's not necessary.
//!
//! ### Responding to actions
//!
//! On adapter creation, we provide a callback to the adapter, which can be used
//! to dispatch actions. This callback forwards to [`A11y::action_listeners`], a
//! mapping from [`NodeId`]s to action handlers (basically just `Box<dyn
//! Fn()>`).
//!
//! This is populated in:
//! - [`Window::on_a11y_action`], which is called by:
//! - [`Interactivity::paint`], which is called by:
//! - [`StatefulInteractiveElement::on_a11y_action`], which is a public-facing API
//!
//! These are cleared at the start of a frame, and re-populated during painting.
//!
//! [`NodeId`]: accesskit::NodeId

use crate::*;

pub(crate) mod debug;

use crate::{App, Bounds, FocusId, Pixels, SharedString, Window};
use accesskit::{Action, NodeId, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The fixed AccessKit node ID used for the root of every window's a11y tree.
pub(crate) const ROOT_NODE_ID: NodeId = NodeId(0);

/// A listener for an accessibility action on a specific node.
pub(crate) type A11yActionListener =
    Box<dyn FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static>;

/// Per-window accessibility state.
///
/// Manages the AccessKit tree that is built each frame and the mappings
/// needed to dispatch incoming action requests back to the right elements.
pub(crate) struct A11y {
    /// Whether accessibility has been [forcibly disabled] for this window.
    ///
    /// [forcibly disabled]: crate::Application::new_inaccessible
    force_disabled: bool,
    /// Whether the tree is built every frame even with no assistive
    /// technology connected. Set through [`crate::Application::with_accessibility_forced`]
    /// or [`crate::Window::set_a11y_forced`]; for automation and tree inspection.
    /// [`Self::force_disabled`] still wins.
    forced: bool,
    /// Whether a11y features have been requested by the system.
    ///
    /// Updated by AccessKit using callbacks provided to the adapter. Can change
    /// halfway through a frame.
    active_flag: Arc<AtomicBool>,
    /// The newest forced tree built while no assistive technology was
    /// connected; handed to the adapter when it activates.
    before_activation: Arc<parking_lot::Mutex<Option<TreeUpdate>>>,
    /// Whether a11y features are active for *this specific frame*.
    ///
    /// At the start of each frame, we load [`Self::active_flag`] (using
    /// [`Self::sync_active_flag`]) and use this to determine whether we
    /// should construct a [`TreeUpdate`] for this frame. It's important that
    /// this value is stable within a frame, because the builder API exposed by
    /// this type maintains a stack of nodes and each must be pushed and popped
    /// exactly once.
    ///
    /// At the end of the frame, we re-call [`Self::sync_active_flag`] to
    /// determine whether we should actually send the finished [`TreeUpdate`].
    active_this_frame: bool,
    /// Powermove P-A11Y-CACHE: whether the last drawn frame built a tree. Its
    /// nodes are what a reused cached view replays; without them (the first
    /// frame after activation) every cached view renders.
    built_last_draw: bool,
    /// Powermove P-A11Y-CACHE: whether the adapter holds the last tree built,
    /// so the next update can carry only changed nodes. Cleared when assistive
    /// technology (re)activates, since the adapter then starts from its
    /// initial tree.
    adapter_synced: Arc<AtomicBool>,
    pub(crate) nodes: A11yNodeBuilder,
    pub(crate) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(crate) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    pub(crate) action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
    /// The last frame's side tables, from which a reused view's entries move
    /// back when its nodes are replayed.
    rendered_focus_ids: FxHashMap<NodeId, FocusId>,
    rendered_node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    rendered_action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
    /// The window's title, used to label the root node so assistive
    /// technology can tell windows apart.
    window_title: Option<SharedString>,
    /// The focus id we most recently reported as having no accessibility node,
    /// used to log at most once per focus change rather than every frame.
    last_focus_without_node: Option<FocusId>,
    /// Retains the last tree update (and, in debug builds, per-node provenance)
    /// so it can be dumped via [`crate::Window::debug_a11y_tree_json`].
    debug: debug::A11yDebug,
    /// Maps a view's [`EntityId`] to its `Render` type name
    #[cfg(debug_assertions)]
    pub(crate) view_type_names: FxHashMap<EntityId, &'static str>,
}

impl A11y {
    pub(crate) fn new(
        active_flag: Arc<AtomicBool>,
        adapter_synced: Arc<AtomicBool>,
        before_activation: Arc<parking_lot::Mutex<Option<TreeUpdate>>>,
        force_disabled: bool,
        forced: bool,
        window_title: Option<SharedString>,
    ) -> Self {
        Self {
            force_disabled,
            forced,
            active_flag,
            before_activation,
            active_this_frame: false,
            built_last_draw: false,
            adapter_synced,
            nodes: A11yNodeBuilder::new(),
            focus_ids: FxHashMap::default(),
            node_bounds: FxHashMap::default(),
            action_listeners: FxHashMap::default(),
            rendered_focus_ids: FxHashMap::default(),
            rendered_node_bounds: FxHashMap::default(),
            rendered_action_listeners: FxHashMap::default(),
            window_title,
            last_focus_without_node: None,
            debug: debug::A11yDebug::default(),
            #[cfg(debug_assertions)]
            view_type_names: FxHashMap::default(),
        }
    }

    /// Logs (once per focus change) that the focused element is not exposed to
    /// assistive technology because it has no accessibility node. When this
    /// happens, screen readers fall back to announcing the whole window instead
    /// of the focused element. The fix is to give the element both an
    /// `.id(...)` and a `.role(...)`.
    pub(crate) fn note_focus_without_node(&mut self, focus_id: FocusId, reason: &str) {
        if self.last_focus_without_node != Some(focus_id) {
            self.last_focus_without_node = Some(focus_id);
            log::info!(
                "a11y: focused element ({focus_id:?}) has no accessibility node \
                 ({reason}); assistive technology will announce the whole window \
                 instead. Give it both an `.id(...)` and a `.role(...)` to expose it."
            );
        }
    }

    pub(crate) fn set_window_title(&mut self, title: impl Into<SharedString>) {
        self.window_title = Some(title.into());
    }

    /// Ensures that [`Self::is_active`] returns up to date information.
    ///
    /// See the docs for [`Self::active_flag`] and [`Self::active_this_frame`]
    /// for more commentary.
    pub(crate) fn sync_active_flag(&mut self) {
        self.active_this_frame =
            !self.force_disabled && (self.forced || self.active_flag.load(Ordering::SeqCst));
    }

    /// Build the tree every frame regardless of whether assistive technology
    /// is connected. Takes effect on the next frame.
    pub(crate) fn set_forced(&mut self, forced: bool) {
        self.forced = forced;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_this_frame
    }

    /// Whether this frame's tree can't come from the last frame's nodes, so
    /// cached views must render rather than replay (Powermove P-A11Y-CACHE).
    pub(crate) fn rebuilding(&self) -> bool {
        self.active_this_frame && !self.built_last_draw
    }

    /// Where the next node lands in this frame's node log; a prepaint range
    /// records it so a reused view can replay its nodes.
    pub(crate) fn node_index(&self) -> usize {
        self.nodes.all_nodes.len()
    }

    /// Powermove P-A11Y-CACHE: a cached view (or deferred draw) whose prepaint
    /// is reused replays the nodes it built last frame, unchanged, under the
    /// current parent, with their bounds, focus registrations and action
    /// listeners, instead of rendering again to rebuild them.
    pub(crate) fn replay(&mut self, range: std::ops::Range<usize>, focused: Option<FocusId>) {
        if !self.active_this_frame || range.is_empty() {
            return;
        }
        let Some(rendered) = self.debug.rendered_tree_mut() else {
            return;
        };
        let rendered_meta = std::mem::take(&mut self.nodes.rendered_meta);
        if range.end > rendered.nodes.len() || range.end > rendered_meta.len() {
            debug_assert!(false, "a11y replay range outside the last frame's tree");
            self.nodes.rendered_meta = rendered_meta;
            return;
        }
        let meta = &rendered_meta[range.clone()];
        let top = meta.iter().map(|meta| meta.depth).min().unwrap_or(0);
        let base = self.nodes.ids_stack.len() as u32;
        let replayed_descendant = self.nodes.rendered_active_descendant.filter(|descendant| {
            rendered.nodes[range.clone()]
                .iter()
                .any(|(id, _)| id == descendant)
        });
        for (index, meta) in range.zip(meta) {
            let id = rendered.nodes[index].0;
            if !self.nodes.can_push(id) {
                continue;
            }
            let node = std::mem::take(&mut rendered.nodes[index].1);
            if meta.depth == top
                && let Some(parent) = self.nodes.nodes_stack.last_mut()
            {
                parent.push_child(id);
            }
            self.nodes.all_nodes.push((id, node));
            self.nodes.meta.push(EntryMeta {
                depth: base + (meta.depth - top),
                fresh: false,
            });
            #[cfg(debug_assertions)]
            if let Some(info) = self.nodes.rendered_node_info.remove(&id) {
                self.nodes.node_info.insert(id, info);
            }
            if let Some(bounds) = self.rendered_node_bounds.remove(&id) {
                self.node_bounds.insert(id, bounds);
            }
            if let Some(listeners) = self.rendered_action_listeners.remove(&id) {
                self.action_listeners.insert(id, listeners);
            }
            if let Some(focus_id) = self.rendered_focus_ids.remove(&id) {
                self.focus_ids.insert(id, focus_id);
                if focused == Some(focus_id) {
                    self.last_focus_without_node = None;
                    self.nodes.set_focus(id);
                }
            }
        }
        self.nodes.rendered_meta = rendered_meta;
        // The focused container of an active descendant may sit inside the
        // replayed range, after the descendant (nodes are logged post-order).
        if let Some(id) = replayed_descendant
            && let Some(focus) = self.nodes.focus
            && focus != id
            && self.nodes.has_node(focus)
        {
            self.nodes.set_active_descendant(id);
        }
    }

    pub(crate) fn set_focusable(&mut self, node_id: NodeId, focus_id: FocusId) {
        self.focus_ids.insert(node_id, focus_id);
    }

    /// Report `node_id` as the currently-focused node, if it is present in the
    /// tree.
    ///
    /// Must only be called once per frame.
    pub(crate) fn set_focus(&mut self, node_id: NodeId) {
        // A focused node must have been registered as focusable this frame.
        if !self.focus_ids.contains_key(&node_id) {
            if cfg!(debug_assertions) {
                panic!("set_focus called for a node that was not registered with set_focusable");
            } else {
                log::warn!(
                    "a11y: set_focus called for a node that was not registered with \
                     set_focusable ({node_id:?})"
                );
            }
        }
        if self.nodes.has_node(node_id) {
            // The focused element is properly exposed; reset the dedup so a
            // later focus on a node-less element logs again.
            self.last_focus_without_node = None;
            self.nodes.set_focus(node_id);
        } else {
            // The element registered a focus handle and an id, but never got a
            // node because it has no role.
            if let Some(focus_id) = self.focus_ids.get(&node_id).copied() {
                self.note_focus_without_node(focus_id, "it has an id but no role");
            }
        }
    }

    pub(crate) fn set_active_descendant(&mut self, node_id: NodeId) {
        // The active descendant must be a descendant of the focused container,
        // not the focused node itself.
        if self.nodes.node_is_focused(node_id) {
            if cfg!(debug_assertions) {
                panic!("set_active_descendant called on the focused node");
            } else {
                log::warn!("a11y: set_active_descendant called on the focused node ({node_id:?})");
            }
            return;
        }
        if self.nodes.has_node(node_id) && self.nodes.focus_is_ancestor_of_current() {
            self.nodes.set_active_descendant(node_id);
        }
    }

    /// Start a frame: the last frame's side tables become what reused views
    /// replay from, and the root node is pushed. A frame that builds no tree
    /// makes the next one rebuild every view.
    pub(crate) fn begin_frame(&mut self) {
        if !self.active_this_frame {
            self.built_last_draw = false;
            return;
        }
        self.rendered_focus_ids = std::mem::take(&mut self.focus_ids);
        self.rendered_node_bounds = std::mem::take(&mut self.node_bounds);
        self.rendered_action_listeners = std::mem::take(&mut self.action_listeners);
        if !self.built_last_draw {
            self.rendered_focus_ids.clear();
            self.rendered_node_bounds.clear();
            self.rendered_action_listeners.clear();
        }
        self.nodes.begin_frame(self.window_title.as_ref());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn simulate_activation(&mut self) {
        self.adapter_synced.store(false, Ordering::SeqCst);
        self.active_flag.store(true, Ordering::SeqCst);
    }

    /// Whether this frame's update goes to the platform adapter now, rather
    /// than being kept for activation.
    pub(crate) fn adapter_listening(&self) -> bool {
        !(self.forced && !self.active_flag.load(Ordering::SeqCst))
    }

    /// A forced tree the adapter would drop (nothing connected yet) is kept
    /// for the activation, which otherwise returns only the root. Returns the
    /// update when it is for the adapter instead.
    pub(crate) fn keep_until_activation(&self, update: TreeUpdate) -> Option<TreeUpdate> {
        if self.forced && !self.active_flag.load(Ordering::SeqCst) {
            *self.before_activation.lock() = Some(update);
            return None;
        }
        Some(update)
    }

    /// Finalize the frame's tree. Returns the update for the platform: only the
    /// nodes that changed when `to_adapter` and the adapter holds the last
    /// tree (`None` when nothing changed), otherwise the whole tree. The whole
    /// tree is kept as the last frame's, for replay and inspection.
    pub(crate) fn end_frame(
        &mut self,
        frame: debug::FrameDebugInfo,
        to_adapter: bool,
    ) -> Option<TreeUpdate> {
        self.built_last_draw = true;
        let tree = self.nodes.finalize();
        let incremental = to_adapter && self.adapter_synced.load(Ordering::SeqCst);
        let update = match self.debug.last_tree_update() {
            Some(last) if incremental => {
                let changed = self.nodes.changed_since(&tree, last);
                (!changed.is_empty() || tree.focus != last.focus).then(|| TreeUpdate {
                    nodes: changed,
                    tree: None,
                    tree_id: tree.tree_id,
                    focus: tree.focus,
                })
            }
            _ => Some(tree.clone()),
        };
        self.nodes.retire_frame(&tree);
        self.debug.capture(
            tree,
            self.nodes.focus,
            self.nodes.active_descendant,
            self.window_title.as_ref(),
            frame,
        );
        self.adapter_synced.store(to_adapter, Ordering::SeqCst);
        update
    }

    pub(crate) fn debug_tree_json(&self) -> Option<String> {
        #[cfg(debug_assertions)]
        return self.debug.to_json(&self.node_bounds, &self.nodes.node_info);
        #[cfg(not(debug_assertions))]
        self.debug.to_json(&self.node_bounds)
    }

    pub(crate) fn last_tree_update(&self) -> Option<&TreeUpdate> {
        self.debug.last_tree_update()
    }

    pub(crate) fn last_node_bounds(&self, id: NodeId) -> Option<Bounds<Pixels>> {
        self.node_bounds.get(&id).copied()
    }

    pub(crate) fn frame_number(&self) -> u64 {
        self.debug.frame_number()
    }
}

/// Builder API for synthetic children. See the docs for
/// [`Element::a11y_synthetic_children`].
pub struct A11ySubtreeBuilder<'a> {
    parent_id: NodeId,
    nodes: &'a mut A11yNodeBuilder,
    // Powermove P-A11Y-SYNTHETIC: window bounds of synthetic children, so the
    // built-in Click lands on them as it does on elements.
    node_bounds: &'a mut FxHashMap<NodeId, Bounds<Pixels>>,
    scale: f32,
    /// Provenance of the real element whose `a11y_synthetic_children` is
    /// running.
    #[cfg(debug_assertions)]
    creator: debug::NodeCreator,
}

impl<'a> A11ySubtreeBuilder<'a> {
    pub(crate) fn new(
        parent_id: NodeId,
        nodes: &'a mut A11yNodeBuilder,
        node_bounds: &'a mut FxHashMap<NodeId, Bounds<Pixels>>,
        scale: f32,
    ) -> Self {
        Self {
            parent_id,
            nodes,
            node_bounds,
            scale,
            #[cfg(debug_assertions)]
            creator: debug::NodeCreator::default(),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn with_creator(mut self, creator: debug::NodeCreator) -> Self {
        self.creator = creator;
        self
    }

    /// Derive a [`NodeId`] for a synthetic child.
    ///
    /// The generated ID is based on the hash of `key`, as well as the parent's
    /// ID. This means that `key`s must be unique within the same
    /// [`Element::a11y_synthetic_children`] call, but may be duplicated across
    /// different calls.
    pub fn synthetic_node_id(&self, key: impl Hash) -> NodeId {
        let mut hasher = std::hash::DefaultHasher::default();
        self.parent_id.0.hash(&mut hasher);
        key.hash(&mut hasher);
        NodeId(hasher.finish())
    }

    /// Append a synthetic leaf node as a child of this element's node.
    ///
    /// Returns `false` if a node with this id is already present in the tree,
    /// in which case the node is discarded.
    pub fn push_child(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        let pushed = self.nodes.push_leaf(id, node);
        #[cfg(debug_assertions)]
        if pushed {
            self.nodes.record_node_info(
                id,
                debug::NodeDebugInfo {
                    synthetic: true,
                    view: self.creator.view,
                    element_id: self.creator.element_id.clone(),
                    source_location: self.creator.source_location,
                },
            );
        }
        pushed
    }

    /// Append a synthetic leaf that occupies `bounds` in the window, in logical
    /// pixels. Its accessibility frame is set from them, and an assistive Press
    /// with no listener clicks its centre, as for an element.
    pub fn push_child_at(
        &mut self,
        id: NodeId,
        mut node: accesskit::Node,
        bounds: Bounds<Pixels>,
    ) -> bool {
        let scale = self.scale as f64;
        node.set_bounds(accesskit::Rect {
            x0: bounds.origin.x.0 as f64 * scale,
            y0: bounds.origin.y.0 as f64 * scale,
            x1: (bounds.origin.x.0 + bounds.size.width.0) as f64 * scale,
            y1: (bounds.origin.y.0 + bounds.size.height.0) as f64 * scale,
        });
        let pushed = self.push_child(id, node);
        if pushed {
            self.node_bounds.insert(id, bounds);
        }
        pushed
    }

    /// A mutable reference to the parent node.
    pub fn parent_node(&mut self) -> &mut accesskit::Node {
        self.nodes
            .current_node_mut()
            .expect("A11ySubtreeBuilder exists only while its element's node is on the stack")
    }
}

pub(crate) struct A11yNodeBuilder {
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[accesskit::Node; 16]>,
    /// This is the exact type required by accesskit, so we can't just make it a
    /// `HashMap<NodeId, Node>` to remove the need for `seen_ids`
    all_nodes: Vec<(NodeId, accesskit::Node)>,
    /// Per `all_nodes` entry (Powermove P-A11Y-CACHE): its depth and whether
    /// this frame built it. A replayed range's shallowest entries are the
    /// children of the node it sits under.
    meta: Vec<EntryMeta>,
    /// `meta` of the last frame's tree, and where each of its nodes sits in it.
    rendered_meta: Vec<EntryMeta>,
    rendered_index: FxHashMap<NodeId, usize>,
    rendered_active_descendant: Option<NodeId>,
    seen_ids: FxHashSet<NodeId>,
    /// The node that GPUI considers focused. Note that this may be different to
    /// what is reported to accesskit - see [`Self::active_descendant`]
    focus: Option<NodeId>,
    /// If a node calls `.aria_active_descendant()`, AND an ancestor is focused,
    /// override it as the focused node. This supports the "active descendant"
    /// pattern, which allows a focused container to act as if a descendant is
    /// focused.
    active_descendant: Option<NodeId>,
    #[cfg(debug_assertions)]
    pub(crate) node_info: FxHashMap<NodeId, debug::NodeDebugInfo>,
    #[cfg(debug_assertions)]
    rendered_node_info: FxHashMap<NodeId, debug::NodeDebugInfo>,
}

#[derive(Clone, Copy)]
pub(crate) struct EntryMeta {
    depth: u32,
    fresh: bool,
}

impl A11yNodeBuilder {
    fn new() -> Self {
        Self {
            ids_stack: SmallVec::new(),
            nodes_stack: SmallVec::new(),
            all_nodes: Vec::new(),
            meta: Vec::new(),
            rendered_meta: Vec::new(),
            rendered_index: FxHashMap::default(),
            rendered_active_descendant: None,
            seen_ids: FxHashSet::default(),
            focus: None,
            active_descendant: None,
            #[cfg(debug_assertions)]
            node_info: FxHashMap::default(),
            #[cfg(debug_assertions)]
            rendered_node_info: FxHashMap::default(),
        }
    }

    /// Clones of this frame's built nodes that differ from `last`'s.
    fn changed_since(
        &self,
        tree: &TreeUpdate,
        last: &TreeUpdate,
    ) -> Vec<(NodeId, accesskit::Node)> {
        tree.nodes
            .iter()
            .zip(&self.meta)
            .filter(|((id, node), meta)| {
                meta.fresh
                    && self
                        .rendered_index
                        .get(id)
                        .and_then(|&index| last.nodes.get(index))
                        .is_none_or(|(last_id, last_node)| last_id != id || last_node != node)
            })
            .map(|(entry, _)| entry.clone())
            .collect()
    }

    /// This frame's tree becomes the one the next frame replays from.
    fn retire_frame(&mut self, tree: &TreeUpdate) {
        self.rendered_meta = std::mem::take(&mut self.meta);
        self.rendered_index.clear();
        self.rendered_index.extend(
            tree.nodes
                .iter()
                .enumerate()
                .map(|(index, (id, _))| (*id, index)),
        );
        self.rendered_active_descendant = self.active_descendant;
    }

    /// Records provenance for a node already pushed this frame. Debug builds only.
    #[cfg(debug_assertions)]
    pub(crate) fn record_node_info(&mut self, id: NodeId, info: debug::NodeDebugInfo) {
        self.node_info.insert(id, info);
    }

    #[must_use]
    fn can_push(&mut self, id: NodeId) -> bool {
        debug_assert!(!self.ids_stack.is_empty(), "node pushed before push_root");

        if !self.seen_ids.insert(id) {
            debug_assert!(
                false,
                "Duplicate a11y node id: {id:?}. In a release build, this node would be silently discarded from the a11y tree."
            );
            return false;
        }

        true
    }

    /// Push a new node onto the stack. It becomes a child of the current
    /// top-of-stack node.
    ///
    /// Returns `true` if the node was successfully pushed.
    pub(crate) fn push(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        if let Some(parent) = self.nodes_stack.last_mut() {
            parent.push_child(id);
        }
        self.ids_stack.push(id);
        self.nodes_stack.push(node);
        true
    }

    /// Add a leaf node as a child of the current top-of-stack node, without
    /// pushing it onto the stack. Semantically equivalent to a [`Self::push`]
    /// followed by a [`Self::pop`].
    ///
    /// Returns `true` if the node was successfully pushed.
    pub(crate) fn push_leaf(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        if let Some(parent) = self.nodes_stack.last_mut() {
            parent.push_child(id);
        }
        self.all_nodes.push((id, node));
        self.meta.push(EntryMeta {
            depth: self.ids_stack.len() as u32,
            fresh: true,
        });
        true
    }

    pub(crate) fn current_node_mut(&mut self) -> Option<&mut accesskit::Node> {
        self.nodes_stack.last_mut()
    }

    /// Pop the current node off the stack and finalize it into the all_nodes
    /// list.
    pub(crate) fn pop(&mut self) {
        debug_assert!(self.ids_stack.len() > 1, "pop would remove the root node");

        let depth = self.ids_stack.len() as u32 - 1;
        if let (Some(id), Some(node)) = (self.ids_stack.pop(), self.nodes_stack.pop()) {
            self.all_nodes.push((id, node));
            self.meta.push(EntryMeta { depth, fresh: true });
        }
    }

    /// Push the root node to start a new frame.
    fn begin_frame(&mut self, window_title: Option<&SharedString>) {
        self.all_nodes.clear();
        self.meta.clear();
        self.ids_stack.clear();
        self.nodes_stack.clear();
        self.seen_ids.clear();
        #[cfg(debug_assertions)]
        {
            self.rendered_node_info = std::mem::take(&mut self.node_info);
        }
        let mut root_node = accesskit::Node::new(accesskit::Role::Window);
        if let Some(title) = window_title {
            root_node.set_label(title.to_string());
        }

        self.ids_stack.push(ROOT_NODE_ID);
        self.nodes_stack.push(root_node);
        self.focus = None;
        self.active_descendant = None;
    }

    /// Returns whether a node with the given ID has been pushed in this frame.
    pub(crate) fn has_node(&self, id: NodeId) -> bool {
        id == ROOT_NODE_ID || self.seen_ids.contains(&id)
    }

    /// Returns whether `id` is the node currently reported as focused.
    pub(crate) fn node_is_focused(&self, id: NodeId) -> bool {
        self.focus == Some(id)
    }

    pub(crate) fn focus_is_ancestor_of_current(&self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };

        // The current node is on top of the stack; everything below it is an
        // ancestor.
        let ancestor_count = self.ids_stack.len().saturating_sub(1);
        self.ids_stack[..ancestor_count].contains(&focus)
    }

    pub(crate) fn set_active_descendant(&mut self, id: NodeId) {
        if self
            .active_descendant
            .is_some_and(|existing| existing != id)
        {
            if cfg!(debug_assertions) {
                panic!("active descendant claimed by multiple nodes in one frame");
            } else {
                log::warn!(
                    "a11y: multiple nodes claimed the active descendant this frame; \
                     using last-wins ({id:?})"
                );
            }
        }
        self.active_descendant = Some(id);
    }

    pub(crate) fn set_focus(&mut self, id: NodeId) {
        if self.focus.is_some() {
            if cfg!(debug_assertions) {
                panic!("set_focus called more than once in a single frame");
            } else {
                log::warn!(
                    "a11y: set_focus called more than once in a single frame; \
                     using last-wins ({id:?})"
                );
            }
        }
        self.focus = Some(id);
    }

    fn finalize(&mut self) -> TreeUpdate {
        // Stack should contain only the root node
        debug_assert_eq!(self.ids_stack.len(), 1);
        debug_assert_eq!(self.ids_stack[0], ROOT_NODE_ID);

        if self.ids_stack.len() != 1 {
            log::error!(
                "a11y: Stack imbalance at end of frame: expected 1 (root), got {}. \
                 Some elements may have pushed without popping.",
                self.ids_stack.len()
            );
        }

        // Pop remaining nodes (should just be the root).
        while !self.ids_stack.is_empty() {
            let depth = self.ids_stack.len() as u32 - 1;
            if let (Some(id), Some(node)) = (self.ids_stack.pop(), self.nodes_stack.pop()) {
                self.all_nodes.push((id, node));
                self.meta.push(EntryMeta { depth, fresh: true });
            }
        }

        let focus = match self.active_descendant {
            Some(id) if self.has_node(id) => id,
            Some(id) => {
                if cfg!(debug_assertions) {
                    panic!("active_descendant set to {id:?}, which is not in the tree");
                } else {
                    log::warn!("active_descendant set to {id:?}, which is not in the tree");
                    self.focus.unwrap_or(ROOT_NODE_ID)
                }
            }

            _ => self.focus.unwrap_or(ROOT_NODE_ID),
        };

        let nodes = std::mem::take(&mut self.all_nodes);
        let update = TreeUpdate {
            nodes,
            tree: Some(accesskit::Tree::new(ROOT_NODE_ID)),
            tree_id: accesskit::TreeId::ROOT,
            focus,
        };

        Self::repair_tree_update(update)
    }

    /// Accesskit panics on invalid [`TreeUpdate`]s. This function defensively
    /// checks invariants that accesskit panics on, and tries to fix them.
    fn repair_tree_update(mut update: TreeUpdate) -> TreeUpdate {
        let node_ids: FxHashSet<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();

        // Focus must point to a node in the tree.
        if !node_ids.contains(&update.focus) {
            log::error!(
                "a11y: Focused node {:?} is not in the tree ({} nodes). \
                 Falling back to root. This is a bug in the a11y tree builder.",
                update.focus,
                update.nodes.len()
            );
            update.focus = ROOT_NODE_ID;
        }

        // Every child reference must point to a node in the update.
        for (id, node) in &mut update.nodes {
            let has_invalid_child = node
                .children()
                .iter()
                .any(|child_id| !node_ids.contains(child_id));
            if has_invalid_child {
                let children = node.children();
                let invalid_count = children
                    .iter()
                    .filter(|child_id| !node_ids.contains(child_id))
                    .count();
                log::error!(
                    "a11y: Node {:?} references {} children not present in the tree. \
                     Stripping invalid child references.",
                    id,
                    invalid_count
                );
                let valid: Vec<NodeId> = children
                    .iter()
                    .copied()
                    .filter(|child_id| node_ids.contains(child_id))
                    .collect();
                node.set_children(valid);
            }
        }

        update
    }
}

#[cfg(test)]
mod tests {
    // Import specific items rather than glob-importing `super`, which would pull
    // in gpui's own `test` attribute macro and shadow the standard one.
    use super::{A11y, A11yNodeBuilder, ROOT_NODE_ID};
    use crate::FocusId;
    use accesskit::{NodeId, Role};
    use std::sync::{Arc, atomic::AtomicBool};

    fn test_node() -> accesskit::Node {
        accesskit::Node::new(Role::GenericContainer)
    }

    fn new_builder() -> A11yNodeBuilder {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame(None);
        builder
    }

    fn new_a11y() -> A11y {
        let mut a11y = A11y::new(
            Arc::new(AtomicBool::new(true)),
            Default::default(),
            false,
            false,
            None,
        );
        a11y.begin_frame();
        a11y
    }

    #[test]
    fn forced_builds_the_tree_without_an_assistive_client() {
        // No assistive technology has activated the platform adapter.
        let mut a11y = A11y::new(
            Arc::new(AtomicBool::new(false)),
            Default::default(),
            false,
            true,
            None,
        );
        a11y.sync_active_flag();
        assert!(a11y.is_active());

        a11y.set_forced(false);
        a11y.sync_active_flag();
        assert!(!a11y.is_active());
    }

    #[test]
    fn force_disabled_wins_over_forced() {
        let mut a11y = A11y::new(
            Arc::new(AtomicBool::new(true)),
            Default::default(),
            true,
            true,
            None,
        );
        a11y.sync_active_flag();
        assert!(!a11y.is_active());
    }

    #[test]
    fn active_descendant_honored_when_container_focused() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);
        assert!(builder.push(item, test_node()));

        // The item is on top of the stack; the focused container is its
        // ancestor, so the claim is honored.
        assert!(builder.focus_is_ancestor_of_current());
        builder.set_active_descendant(item);

        builder.pop(); // item
        builder.pop(); // container
        let update = builder.finalize();
        assert_eq!(update.focus, item);
    }

    #[test]
    fn active_descendant_honored_for_deep_descendant() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let group = NodeId(2);
        let item = NodeId(3);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);
        assert!(builder.push(group, test_node()));
        assert!(builder.push(item, test_node()));

        // The item is a grandchild of the focused container; depth doesn't
        // matter, the focused ancestor is still on the stack.
        assert!(builder.focus_is_ancestor_of_current());
        builder.set_active_descendant(item);

        builder.pop(); // item
        builder.pop(); // group
        builder.pop(); // container
        let update = builder.finalize();
        assert_eq!(update.focus, item);
    }

    #[test]
    fn active_descendant_ignored_when_focus_in_other_subtree() {
        let mut builder = new_builder();
        let focused_container = NodeId(1);
        let focused_leaf = NodeId(2);
        let other_container = NodeId(3);
        let other_item = NodeId(4);

        // First subtree holds real focus.
        assert!(builder.push(focused_container, test_node()));
        assert!(builder.push(focused_leaf, test_node()));
        builder.set_focus(focused_leaf);
        builder.pop(); // focused_leaf
        builder.pop(); // focused_container

        // Second subtree: its item would claim the active descendant, but the
        // focus is not on any of its ancestors, so the gate rejects it.
        assert!(builder.push(other_container, test_node()));
        assert!(builder.push(other_item, test_node()));
        assert!(!builder.focus_is_ancestor_of_current());
        builder.pop(); // other_item
        builder.pop(); // other_container

        let update = builder.finalize();
        assert_eq!(update.focus, focused_leaf);
    }

    #[test]
    fn active_descendant_ignored_when_nothing_focused() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        assert!(builder.push(item, test_node()));

        // Nothing is focused (focus defaults to the root window node), so the
        // gate rejects the claim.
        assert!(!builder.focus_is_ancestor_of_current());
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, ROOT_NODE_ID);
    }

    #[test]
    fn regular_focus_used_when_no_active_descendant() {
        let mut builder = new_builder();
        let focused = NodeId(1);

        assert!(builder.push(focused, test_node()));
        builder.set_focus(focused);
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, focused);
    }

    #[test]
    fn focus_is_ancestor_excludes_self_and_non_ancestors() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);

        // With the focused container itself on top, it is not its own (strict)
        // ancestor, so the gate is false.
        assert!(!builder.focus_is_ancestor_of_current());

        assert!(builder.push(item, test_node()));
        // Now the focused container is a strict ancestor of the item on top.
        assert!(builder.focus_is_ancestor_of_current());

        builder.pop();
        builder.pop();
    }

    // The double-claim guard panics only in debug builds; in release it falls
    // back to last-wins with a warning.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn multiple_active_descendant_claims_panic_in_debug() {
        let mut builder = new_builder();
        builder.set_active_descendant(NodeId(1));
        builder.set_active_descendant(NodeId(2));
    }

    // Setting focus twice in one frame means two elements both claimed window
    // focus; that panics in debug and falls back to last-wins in release.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "set_focus called more than once")
    )]
    fn setting_focus_twice_panics_in_debug() {
        let mut builder = new_builder();
        builder.set_focus(NodeId(1));
        builder.set_focus(NodeId(2));
    }

    // Focusing a node that was never registered as focusable is a bug: panic in
    // debug, warn in release.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "was not registered with set_focusable")
    )]
    fn set_focus_without_set_focusable() {
        let mut a11y = new_a11y();
        let node = NodeId(1);
        assert!(a11y.nodes.push(node, test_node()));
        // set_focusable was never called for `node`.
        a11y.set_focus(node);
    }

    // The focused node cannot also be its own active descendant: panic in
    // debug, warn in release.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "on the focused node"))]
    fn set_active_descendant_on_focused_node() {
        let mut a11y = new_a11y();
        let node = NodeId(1);
        assert!(a11y.nodes.push(node, test_node()));
        a11y.set_focusable(node, FocusId::default());
        a11y.set_focus(node);
        a11y.set_active_descendant(node);
    }

    // Two sibling children of a focused container both claim the active
    // descendant (both pass the focus gate). The second claim is a bug: panic
    // in debug, last-wins + warn in release.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn two_siblings_claiming_active_descendant() {
        let mut a11y = new_a11y();
        let container = NodeId(1);
        let first = NodeId(2);
        let second = NodeId(3);

        assert!(a11y.nodes.push(container, test_node()));
        a11y.set_focusable(container, FocusId::default());
        a11y.set_focus(container);

        assert!(a11y.nodes.push(first, test_node()));
        a11y.set_active_descendant(first);
        a11y.nodes.pop(); // first

        assert!(a11y.nodes.push(second, test_node()));
        a11y.set_active_descendant(second);
        a11y.nodes.pop(); // second

        a11y.nodes.pop(); // container
    }

    // Node A is focused; node C (a child of the unfocused node B) claims the
    // active descendant. The final tree must still report A as focused.
    #[test]
    fn active_descendant_in_unfocused_subtree_keeps_real_focus() {
        let mut a11y = new_a11y();
        let a = NodeId(1);
        let b = NodeId(2);
        let c = NodeId(3);

        assert!(a11y.nodes.push(a, test_node()));
        a11y.set_focusable(a, FocusId::default());
        a11y.set_focus(a);
        a11y.nodes.pop(); // a

        assert!(a11y.nodes.push(b, test_node()));
        assert!(a11y.nodes.push(c, test_node()));
        a11y.set_active_descendant(c);
        a11y.nodes.pop(); // c
        a11y.nodes.pop(); // b

        let update = a11y.end_frame(Default::default());
        assert_eq!(update.focus, a);
    }
}
