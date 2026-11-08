use crate::{
    Availability, Catalog, Edit, Error, Input, ItemId, MediaRuntime, ProxyStatus, RuntimeState,
    Stamp,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    pub query: String,
    pub kind: Option<String>,
    pub missing_only: bool,
    pub proxy: Option<ProxyStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RowData {
    Asset(Box<crate::Asset>, RuntimeState),
    Folder(crate::Folder),
    Composition(crate::Composition),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub id: ItemId,
    pub data: RowData,
    pub selected: bool,
    pub focused: bool,
    pub tab_stop: bool,
    /// Descendant-search breadcrumb, empty outside search.
    #[serde(default)]
    pub folder_path: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub stamp: Stamp,
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<Row>,
    pub next: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MenuCommand {
    AddAsset(String),
    NewCompFromAsset(String),
    Replace(String),
    Reveal(String),
    Download(String),
    DeleteAssets,
    NewComposition,
    ImportFiles,
    ImportFolder(Option<String>),
    NewFolder(Option<String>),
    OpenFolder(String),
    RenameFolder(String),
    DeleteFolder(String),
    MoveOut(ItemId),
    OpenComposition(String),
    NestComposition(String),
    CompositionSettings(String),
    DuplicateComposition(String),
    DeleteComposition(String),
    /// Proxy and optimized media for the selected videos.
    Rendition(RenditionAction),
}
/// What the selected videos' renditions get (Project panel, plan section 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenditionAction {
    CreateProxy,
    RebuildProxy,
    DeleteProxy,
    /// Use a proxy made elsewhere (camera, DIT); one video at a time.
    AttachProxy,
    CreateOptimized,
    DeleteOptimized,
    /// Always Play Original on (`true`) or off.
    PlayOriginal(bool),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub command: MenuCommand,
    pub enabled: bool,
    /// A toggle that is on (Always Play Original).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
}

/// Effects are semantic host requests. `submit` must enqueue work and return;
/// pickers, Finder, decoding and the viewer are owned by the macOS host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextMenu {
    pub owner: Stamp,
    pub entries: Vec<MenuEntry>,
    pub effects: Vec<Effect>,
}
impl ContextMenu {
    pub fn submit_effects(&self, host: &mut dyn MediaHost) {
        for effect in &self.effects {
            host.submit(HostRequest {
                owner: self.owner.clone(),
                effect: effect.clone(),
            });
        }
    }
}

pub trait MediaHost {
    fn submit(&mut self, request: HostRequest);
}
/// The host checks this owner before acting and before publishing completion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostRequest {
    pub owner: Stamp,
    pub effect: Effect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    ClearPreview,
    ShowPreview(String),
    TogglePreview(String),
    SeekSource {
        id: String,
        seconds: f64,
    },
    PickFiles {
        replace: Option<String>,
    },
    PickFolder {
        folder: Option<String>,
    },
    Import {
        files: Vec<String>,
        folder: Option<String>,
        as_folder: bool,
    },
    Reveal {
        id: String,
        path: String,
    },
    Download(String),
    AddAsset(String),
    NewCompFromAsset(String),
    NewComposition,
    OpenComposition(String),
    NestComposition(String),
    CompositionSettings(String),
    ConfirmDelete {
        token: u64,
        assets: Vec<String>,
        references: usize,
    },
    Focus(Option<ItemId>),
    ScrollMarquee(ScrollDirection),
    /// Proxy or optimized media work for these videos.
    Renditions {
        action: RenditionAction,
        assets: Vec<String>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollDirection {
    Before,
    After,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Outcome {
    pub owner: Option<Stamp>,
    pub handled: bool,
    pub effects: Vec<Effect>,
    pub edit: Option<Edit>,
}
impl Outcome {
    /// Enqueue effects with their captured project/document identity.
    pub fn submit_effects(&self, host: &mut dyn MediaHost) {
        let Some(owner) = &self.owner else {
            return;
        };
        for effect in &self.effects {
            host.submit(HostRequest {
                owner: owner.clone(),
                effect: effect.clone(),
            });
        }
    }
    pub(crate) fn handled() -> Self {
        Self {
            handled: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub asset: Option<String>,
    pub playing: bool,
    pub seconds: f64,
    pub duration: Option<f64>,
    pub transport: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drag {
    pub primary: ItemId,
    pub assets: Vec<String>,
    pub name: String,
    pub kind: String,
    pub duration: Option<f64>,
    pub copy_to_timeline: bool,
    pub move_to_bin: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Confirmation {
    pub token: u64,
    pub stamp: Stamp,
    pub assets: Vec<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct Marquee {
    pub pointer: u64,
    pub base: Vec<String>,
}

/// Instance-local transient state, keyed by project document lifetime. No
/// document mutation, global selection or Undo history lives in this object.
#[derive(Clone, Debug, Default)]
pub struct MediaPanel {
    pub folder: Option<String>,
    pub filter: Filter,
    pub preview: Preview,
    pub renaming: Option<String>,
    pub drag: Option<Drag>,
    pub drop_target: Option<Option<String>>,
    pub(crate) stamp: Option<Stamp>,
    pub(crate) grid_columns: usize,
    pub(crate) row_positions: HashMap<ItemId, usize>,
    /// All visible assets except these IDs; never enumerate on Select All.
    pub(crate) all_except: Option<HashSet<String>>,
    pub(crate) selection_cache: OnceLock<Vec<String>>,
    pub(crate) selection: Vec<String>,
    pub(crate) selected: HashSet<String>,
    pub(crate) active: Option<String>,
    pub(crate) anchor: Option<String>,
    pub(crate) exclusive: Option<ItemId>,
    pub(crate) focused: Option<ItemId>,
    pub(crate) rows: Vec<ItemId>,
    pub(crate) asset_order: Vec<String>,
    pub(crate) positions: HashMap<String, usize>,
    pub(crate) confirmation: Option<Confirmation>,
    pub(crate) next_token: u64,
    pub(crate) marquee: Option<Marquee>,
    pub(crate) drop_depth: usize,
}

impl MediaPanel {
    /// Apply post-commit selection only after a successful kernel ack. A failed
    /// or stale edit cannot create a phantom selected/renaming folder.
    pub fn accept_edit(
        &mut self,
        plan: &crate::PreparedEdit,
        ack: &pm_kernel::Ack,
        c: &Catalog,
        r: &dyn MediaRuntime,
    ) -> Result<Vec<Effect>, Error> {
        if plan.stamp.project != c.stamp.project
            || plan.stamp.document != c.stamp.document
            || ack.revision != c.stamp.revision
        {
            return Err(Error::Stale);
        }
        if !matches!(ack.reply, pm_kernel::Reply::Applied { .. }) {
            return Err(Error::Kernel(format!("{:?}", ack.reply)));
        }
        let mut effects = self.refresh(c, r);
        if let Some(id) = &plan.created {
            if let Some(folder) = c.folders.get(id) {
                self.folder = folder.parent.clone();
                effects.extend(self.refresh(c, r));
                self.select_exclusive(ItemId::Folder(id.clone()), &mut effects);
                self.renaming = Some(id.clone());
            } else if c.compositions.contains_key(id) {
                self.select_exclusive(ItemId::Composition(id.clone()), &mut effects);
            }
        }
        Ok(effects)
    }
    pub fn selected_assets(&self) -> &[String] {
        if let Some(excluded) = &self.all_except {
            self.selection_cache.get_or_init(|| {
                self.asset_order
                    .iter()
                    .filter(|id| !excluded.contains(*id))
                    .cloned()
                    .collect()
            })
        } else {
            &self.selection
        }
    }
    pub fn selected_count(&self) -> usize {
        self.all_except.as_ref().map_or(self.selection.len(), |e| {
            self.asset_order.len().saturating_sub(e.len())
        })
    }
    pub fn is_selected(&self, id: &str) -> bool {
        self.all_except.as_ref().map_or_else(
            || self.selected.contains(id),
            |e| self.positions.contains_key(id) && !e.contains(id),
        )
    }
    pub(crate) fn materialize_selection(&mut self) {
        if self.all_except.is_some() {
            self.selection = self.selected_assets().to_vec();
            self.selected = self.selection.iter().cloned().collect();
            self.all_except = None;
            self.selection_cache.take();
        }
    }
    pub fn selected_item(&self) -> Option<&ItemId> {
        self.exclusive.as_ref()
    }
    pub fn focused(&self) -> Option<&ItemId> {
        self.focused.as_ref()
    }
    pub fn delete_pending(&self) -> bool {
        self.confirmation.is_some()
    }
    pub fn file_drop_active(&self) -> bool {
        self.drop_depth > 0
    }
    pub fn total(&self) -> usize {
        self.rows.len()
    }
    /// Refresh on catalog/filter/runtime changes. Run on a worker for large
    /// projects. A Page request itself only touches its bounded row window.
    pub fn refresh(&mut self, c: &Catalog, runtime: &dyn MediaRuntime) -> Vec<Effect> {
        let mut effects = vec![];
        if self
            .stamp
            .as_ref()
            .is_some_and(|s| s.project != c.stamp.project || s.document != c.stamp.document)
        {
            self.folder = None;
            self.filter = Filter::default();
            self.clear(&mut effects);
            self.confirmation = None;
            self.renaming = None;
            self.drag = None;
            self.drop_target = None;
            self.drop_depth = 0;
        }
        self.stamp = Some(c.stamp.clone());
        self.folder = c.valid_folder(self.folder.as_deref());
        let needle = self.filter.query.to_lowercase();
        let mut rows = vec![];
        self.selection_cache.take();
        // N2: search descendants, without changing authored folders or ordering.
        let descendants = !needle.is_empty();
        if self.filter.kind.is_none() && !self.filter.missing_only && self.filter.proxy.is_none() {
            rows.extend(
                c.search_folders(self.folder.as_deref(), descendants)
                    .into_iter()
                    .filter(|id| c.folders[*id].name.to_lowercase().contains(&needle))
                    .cloned()
                    .map(ItemId::Folder),
            );
            if self.folder.is_none() {
                rows.extend(
                    c.composition_order
                        .iter()
                        .map(|id| &c.compositions[id])
                        .filter(|v| v.name.to_lowercase().contains(&needle))
                        .map(|v| ItemId::Composition(v.id.clone())),
                );
            }
        }
        self.asset_order = c
            .asset_order
            .iter()
            .map(|id| &c.assets[id])
            .filter(|a| {
                if a.folder != self.folder
                    && !(descendants
                        && self
                            .folder
                            .as_ref()
                            .is_none_or(|root| c.within(a.folder.as_deref(), root)))
                {
                    return false;
                }
                if self.filter.kind.as_ref().is_some_and(|k| k != &a.kind) {
                    return false;
                }
                if !needle.is_empty()
                    && !a.name.to_lowercase().contains(&needle)
                    && !a.id.to_lowercase().contains(&needle)
                    && !a
                        .source_path
                        .as_ref()
                        .is_some_and(|p| p.to_lowercase().contains(&needle))
                {
                    return false;
                }
                if self.filter.missing_only || self.filter.proxy.is_some() {
                    let s = runtime.state(&c.stamp, &a.id);
                    if self.filter.missing_only
                        && !matches!(
                            s.availability,
                            Availability::Offline
                                | Availability::Changed { .. }
                                | Availability::Failed(_)
                                | Availability::Cloud { .. }
                        )
                    {
                        return false;
                    }
                    if self.filter.proxy.as_ref().is_some_and(|p| p != &s.proxy) {
                        return false;
                    }
                }
                true
            })
            .map(|a| a.id.clone())
            .collect();
        self.positions = self
            .asset_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        rows.extend(self.asset_order.iter().cloned().map(ItemId::Asset));
        self.row_positions = rows
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        self.rows = rows;
        self.selection.retain(|id| self.positions.contains_key(id));
        self.selected = self.selection.iter().cloned().collect();
        if let Some(excluded) = &mut self.all_except {
            excluded.retain(|id| self.positions.contains_key(id));
        }
        if !self.active.as_ref().is_some_and(|id| self.is_selected(id)) {
            self.active = if self.all_except.is_some() {
                self.asset_order
                    .iter()
                    .rev()
                    .find(|id| self.is_selected(id))
                    .cloned()
            } else {
                self.selection.last().cloned()
            };
        }
        if !self
            .anchor
            .as_ref()
            .is_some_and(|id| self.positions.contains_key(id))
        {
            self.anchor = None;
        }
        if self
            .exclusive
            .as_ref()
            .is_some_and(|id| !self.rows.contains(id))
        {
            self.exclusive = None;
        }
        if self
            .focused
            .as_ref()
            .is_some_and(|id| !self.rows.contains(id))
        {
            self.focused = None;
        }
        if self
            .preview
            .asset
            .as_ref()
            .is_some_and(|id| !self.is_selected(id))
        {
            self.clear_preview(&mut effects);
        }
        if self
            .renaming
            .as_ref()
            .is_some_and(|id| !c.folders.contains_key(id))
        {
            self.renaming = None;
        }
        effects
    }
    pub fn page(
        &self,
        c: &Catalog,
        runtime: &dyn MediaRuntime,
        offset: usize,
        limit: usize,
    ) -> Result<Page, Error> {
        if self.stamp.as_ref() != Some(&c.stamp) {
            return Err(Error::Stale);
        }
        let limit = limit.min(4096); // output resource bound, not a visual size
        let rows = self
            .rows
            .iter()
            .skip(offset)
            .take(limit)
            .map(|id| {
                let (data, selected, tab_stop) = match id {
                    ItemId::Asset(id) => (
                        RowData::Asset(Box::new(c.assets[id].clone()), runtime.state(&c.stamp, id)),
                        self.is_selected(id),
                        self.active.as_ref().or(self.asset_order.first()) == Some(id),
                    ),
                    ItemId::Folder(id) => (
                        RowData::Folder(c.folders[id].clone()),
                        self.exclusive.as_ref() == Some(&ItemId::Folder(id.clone())),
                        false,
                    ),
                    ItemId::Composition(id) => (
                        RowData::Composition({
                            let mut comp = c.compositions[id].clone();
                            comp.poster = runtime.composition_poster(&c.stamp, id);
                            comp
                        }),
                        self.exclusive.as_ref() == Some(&ItemId::Composition(id.clone())),
                        false,
                    ),
                };
                Row {
                    id: id.clone(),
                    data,
                    selected,
                    focused: self.focused.as_ref() == Some(id),
                    tab_stop,
                    folder_path: if self.filter.query.is_empty() {
                        vec![]
                    } else {
                        let folder = match id {
                            ItemId::Asset(id) => c.assets[id].folder.as_deref(),
                            ItemId::Folder(id) => c.folders[id].parent.as_deref(),
                            ItemId::Composition(_) => None,
                        };
                        c.path(folder).into_iter().map(|f| f.name.clone()).collect()
                    },
                }
            })
            .collect::<Vec<_>>();
        let end = offset.saturating_add(rows.len());
        Ok(Page {
            stamp: c.stamp.clone(),
            total: self.rows.len(),
            offset,
            rows,
            next: if end < self.rows.len() && limit > 0 {
                Some(end)
            } else {
                None
            },
        })
    }
    pub(crate) fn clear_preview(&mut self, effects: &mut Vec<Effect>) {
        self.preview = Preview::default();
        effects.push(Effect::ClearPreview);
    }
    pub(crate) fn clear(&mut self, effects: &mut Vec<Effect>) {
        self.all_except = None;
        self.selection_cache.take();
        self.selection.clear();
        self.selected.clear();
        self.active = None;
        self.anchor = None;
        self.exclusive = None;
        self.focused = None;
        self.marquee = None;
        self.clear_preview(effects);
    }
    pub(crate) fn select(&mut self, id: &str, m: crate::Modifiers, effects: &mut Vec<Effect>) {
        self.exclusive = None;
        self.selection_cache.take();
        let previous = self.active.clone();
        if m.shift
            && self
                .anchor
                .as_ref()
                .is_some_and(|id| self.positions.contains_key(id))
        {
            self.materialize_selection();
            let a = self.positions[self.anchor.as_ref().unwrap()];
            let b = self.positions[id];
            if !m.primary() {
                self.selection.clear();
                self.selected.clear();
            }
            for id in &self.asset_order[a.min(b)..=a.max(b)] {
                if self.selected.insert(id.clone()) {
                    self.selection.push(id.clone());
                }
            }
        } else if m.primary() {
            if let Some(excluded) = &mut self.all_except {
                if !excluded.remove(id) {
                    excluded.insert(id.into());
                }
            } else if self.selected.remove(id) {
                self.selection.retain(|s| s != id);
            } else {
                self.selected.insert(id.into());
                self.selection.push(id.into());
            }
            self.anchor = Some(id.into());
        } else {
            self.all_except = None;
            self.selection = vec![id.into()];
            self.selected = HashSet::from([id.into()]);
            self.anchor = Some(id.into());
        }
        self.active = if self.is_selected(id) {
            Some(id.into())
        } else if self.all_except.is_some() {
            self.asset_order
                .iter()
                .rev()
                .find(|id| self.is_selected(id))
                .cloned()
        } else {
            self.selection.last().cloned()
        };
        if self.selected_count() != 1
            || previous
                .as_ref()
                .is_some_and(|p| Some(p) != self.active.as_ref())
        {
            self.clear_preview(effects);
        }
        self.focused = Some(ItemId::Asset(id.into()));
        effects.push(Effect::Focus(self.focused.clone()));
    }
    pub(crate) fn select_exclusive(&mut self, id: ItemId, effects: &mut Vec<Effect>) {
        self.all_except = None;
        self.selection_cache.take();
        self.selection.clear();
        self.selected.clear();
        self.active = None;
        self.exclusive = Some(id.clone());
        self.focused = Some(id);
        self.clear_preview(effects);
        effects.push(Effect::Focus(self.focused.clone()));
    }
    pub fn menu(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        id: Option<ItemId>,
    ) -> Result<ContextMenu, Error> {
        use MenuCommand::*;
        let mut commands = vec![];
        let mut effects = vec![];
        match id {
            None => commands.extend(
                [
                    NewComposition,
                    NewFolder(self.folder.clone()),
                    ImportFiles,
                    ImportFolder(self.folder.clone()),
                ]
                .map(|c| (c, true)),
            ),
            Some(ItemId::Asset(id)) => {
                if !self.positions.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                if !self.is_selected(&id) {
                    self.select(&id, crate::Modifiers::default(), &mut effects);
                }
                if self.selected_count() > 1 {
                    renditions(c, r, &self.rendition_targets(c, r), false, &mut commands);
                    commands.push((DeleteAssets, true));
                } else {
                    let a = &c.assets[&id];
                    let state = r.state(&c.stamp, &id);
                    if matches!(
                        state.availability,
                        Availability::Loading | Availability::Unknown
                    ) {
                        return Ok(ContextMenu {
                            owner: c.stamp.clone(),
                            entries: vec![MenuEntry {
                                command: DeleteAssets,
                                enabled: true,
                                checked: false,
                            }],
                            effects,
                        });
                    }
                    if matches!(state.availability, Availability::Ready) {
                        commands.push((AddAsset(id.clone()), true));
                        if !matches!(a.kind.as_str(), "audio" | "model") {
                            commands.push((NewCompFromAsset(id.clone()), true));
                        }
                    }
                    if let Availability::Cloud { download, .. } = &state.availability {
                        commands.push((
                            Download(id.clone()),
                            !matches!(download, crate::Download::Downloading),
                        ));
                    }
                    commands.push((Replace(id.clone()), true));
                    if self.folder.is_some() {
                        commands.push((MoveOut(ItemId::Asset(id.clone())), true));
                    }
                    if a.source_path.is_some() {
                        commands.push((Reveal(id.clone()), true));
                    }
                    if a.kind == "video" {
                        renditions(c, r, std::slice::from_ref(&id), true, &mut commands);
                    }
                    commands.push((DeleteAssets, true));
                }
            }
            Some(ItemId::Folder(id)) => {
                if !c.folders.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                self.select_exclusive(ItemId::Folder(id.clone()), &mut effects);
                commands.extend(
                    [
                        OpenFolder(id.clone()),
                        RenameFolder(id.clone()),
                        NewFolder(Some(id.clone())),
                        ImportFolder(Some(id.clone())),
                    ]
                    .map(|c| (c, true)),
                );
                if self.folder.is_some() {
                    commands.push((MoveOut(ItemId::Folder(id.clone())), true));
                }
                commands.push((DeleteFolder(id), true));
            }
            Some(ItemId::Composition(id)) => {
                let comp = c.compositions.get(&id).ok_or(Error::MissingItem)?;
                self.select_exclusive(ItemId::Composition(id.clone()), &mut effects);
                commands.extend([
                    (NewComposition, true),
                    (OpenComposition(id.clone()), !comp.open),
                    (NestComposition(id.clone()), comp.can_nest),
                    (CompositionSettings(id.clone()), true),
                    (DuplicateComposition(id.clone()), true),
                    (DeleteComposition(id), c.compositions.len() > 1),
                ]);
            }
        }
        Ok(ContextMenu {
            owner: c.stamp.clone(),
            effects,
            entries: commands
                .into_iter()
                .map(|(command, enabled)| MenuEntry {
                    checked: command == Rendition(RenditionAction::PlayOriginal(false)),
                    command,
                    enabled,
                })
                .collect(),
        })
    }
    /// The selected videos rendition commands act on.
    pub(crate) fn rendition_targets(&self, c: &Catalog, _r: &dyn MediaRuntime) -> Vec<String> {
        self.selected_assets()
            .iter()
            .filter(|id| c.assets.get(*id).is_some_and(|a| a.kind == "video"))
            .cloned()
            .collect()
    }
    /// Agent/UI entry point. Hosts dispatch exactly these inputs by stable id.
    pub fn dispatch(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        input: Input,
    ) -> Result<Outcome, Error> {
        if self.stamp.as_ref() != Some(&c.stamp) {
            return Err(Error::Stale);
        }
        let mut outcome = self.reduce(c, r, input)?;
        outcome.owner = Some(c.stamp.clone());
        Ok(outcome)
    }
}

/// The rendition commands for `videos` (none: nothing to add). Create needs
/// an online original; Rebuild and Delete need the rendition; Attach is for
/// one video. Always Play Original shows checked (as `PlayOriginal(false)`,
/// which turns it off) when every video has it.
fn renditions(
    c: &Catalog,
    r: &dyn MediaRuntime,
    videos: &[String],
    single: bool,
    commands: &mut Vec<(MenuCommand, bool)>,
) {
    use RenditionAction::*;
    if videos.is_empty() {
        return;
    }
    let states: Vec<_> = videos.iter().map(|id| r.state(&c.stamp, id)).collect();
    let online = states
        .iter()
        .any(|s| matches!(s.availability, Availability::Ready));
    let ready = |p: &ProxyStatus| matches!(p, ProxyStatus::Ready { .. });
    let proxied = states.iter().any(|s| ready(&s.proxy));
    let optimized = states.iter().any(|s| ready(&s.optimized));
    let all_original = states.iter().all(|s| s.play_original);
    commands.extend([
        (MenuCommand::Rendition(CreateProxy), online),
        (MenuCommand::Rendition(RebuildProxy), online && proxied),
        (MenuCommand::Rendition(DeleteProxy), proxied),
    ]);
    if single {
        commands.push((MenuCommand::Rendition(AttachProxy), online));
    }
    commands.extend([
        (MenuCommand::Rendition(CreateOptimized), online),
        (MenuCommand::Rendition(DeleteOptimized), optimized),
        (MenuCommand::Rendition(PlayOriginal(!all_original)), true),
    ]);
}
