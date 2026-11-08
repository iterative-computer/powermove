use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub meta: bool,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}
impl Modifiers {
    pub fn primary(self) -> bool {
        self.meta || self.control
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    A,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Enter,
    Space,
    Delete,
    Backspace,
    Escape,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyTarget {
    List,
    Row(ItemId),
    NestedControl,
    TextField,
    Source,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerPhase {
    Down,
    Click,
    DoubleClick,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Input {
    /// Native layout facts; no visual geometry is guessed in the model.
    GridColumns(usize),
    GridKey {
        target: KeyTarget,
        key: Key,
        modifiers: Modifiers,
        columns: usize,
    },
    Pointer {
        id: ItemId,
        phase: PointerPhase,
        button: u8,
        nested_control: bool,
        modifiers: Modifiers,
    },
    Key {
        target: KeyTarget,
        key: Key,
        modifiers: Modifiers,
    },
    /// The source monitor remains inside selection ownership.
    OutsidePress {
        source_monitor: bool,
        same_item: Option<ItemId>,
        inside_panel: bool,
    },
    Clear,
    /// Nested row delete button: preserve an unrelated batch selection while
    /// choosing only this asset for deletion, exactly as the tagged panel does.
    DeleteAsset(String),
    OpenFolder(Option<String>),
    Filter(Filter),
    Invoke(MenuCommand),
    BeginRename(String),
    CommitRename {
        folder: String,
        value: String,
    },
    CancelRename,
    ConfirmDelete {
        token: u64,
        approved: bool,
    },
    DragStart {
        id: ItemId,
        nested_control: bool,
    },
    DragEnd,
    FilesEnter,
    FilesLeave,
    DropLeave,
    DragOver {
        target: Option<String>,
        files: bool,
    },
    Drop {
        target: Option<String>,
        files: Vec<String>,
        modifiers: Modifiers,
    },
    /// Geometry/hit testing belongs to the future view. Pass *all* hit ids,
    /// including unmounted rows, in document order. No visual constants here.
    BeginMarquee {
        pointer: u64,
        modifiers: Modifiers,
    },
    MarqueeHits {
        pointer: u64,
        activated: bool,
        ids: Vec<String>,
    },
    EndMarquee {
        pointer: Option<u64>,
    },
    /// The view computes edge entry using its approved geometry and schedules
    /// ticks; the model owns whether the active selection may autoscroll.
    MarqueeEdge {
        pointer: u64,
        activated: bool,
        direction: ScrollDirection,
    },
    SeekSource(f64),
    /// Real measured host feedback. Stamp and id prevent late decode callbacks
    /// from reviving a preview after project/selection changes.
    PreviewFeedback {
        stamp: Stamp,
        asset: String,
        playing: bool,
        seconds: f64,
        duration: Option<f64>,
    },
}

impl MediaPanel {
    pub(crate) fn reduce(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        input: Input,
    ) -> Result<Outcome, Error> {
        let mut out = Outcome::handled();
        match input {
            Input::GridKey {
                target,
                key,
                modifiers,
                columns,
            } => {
                if columns == 0 || columns > 4096 {
                    return Err(Error::InvalidTarget);
                }
                self.grid_columns = columns;
                return self.key(c, r, target, key, modifiers);
            }
            Input::GridColumns(columns) => {
                if columns == 0 || columns > 4096 {
                    return Err(Error::InvalidTarget);
                }
                self.grid_columns = columns;
            }
            Input::Pointer {
                id,
                phase,
                button,
                nested_control,
                modifiers: m,
            } => {
                if button != 0 || nested_control {
                    return Ok(Outcome::default());
                }
                if !self.rows.contains(&id) {
                    return Err(Error::MissingItem);
                }
                match id {
                    ItemId::Asset(id) => match phase {
                        PointerPhase::Down => {
                            if self.selected_count() <= 1
                                || !self.is_selected(&id)
                                || m.primary()
                                || m.shift
                            {
                                self.select(&id, m, &mut out.effects);
                            } else {
                                self.focused = Some(ItemId::Asset(id));
                                out.effects.push(Effect::Focus(self.focused.clone()));
                            }
                        }
                        PointerPhase::Click => {
                            if !m.primary()
                                && !m.shift
                                && self.selected_count() > 1
                                && self.is_selected(&id)
                            {
                                self.select(&id, m, &mut out.effects);
                            }
                        }
                        PointerPhase::DoubleClick => {
                            if !matches!(
                                r.state(&c.stamp, &id).availability,
                                Availability::Loading | Availability::Unknown
                            ) {
                                self.select(&id, Modifiers::default(), &mut out.effects);
                                self.activate(c, r, &id, true, &mut out)?;
                            }
                        }
                    },
                    ItemId::Folder(id) => match phase {
                        PointerPhase::Down => {
                            self.select_exclusive(ItemId::Folder(id), &mut out.effects)
                        }
                        PointerPhase::DoubleClick => {
                            self.open_folder(c, r, Some(id), &mut out.effects)
                        }
                        _ => {}
                    },
                    ItemId::Composition(id) => match phase {
                        PointerPhase::Down => {
                            self.select_exclusive(ItemId::Composition(id), &mut out.effects)
                        }
                        PointerPhase::DoubleClick => out.effects.push(Effect::OpenComposition(id)),
                        _ => {}
                    },
                }
            }
            Input::Key {
                target,
                key,
                modifiers: m,
            } => return self.key(c, r, target, key, m),
            Input::OutsidePress {
                source_monitor,
                same_item,
                inside_panel,
            } => {
                if let Some(selected) = &self.exclusive {
                    if same_item.as_ref() != Some(selected) {
                        self.exclusive = None;
                    }
                } else if !inside_panel
                    && !source_monitor
                    && same_item.as_ref()
                        != self
                            .active
                            .as_ref()
                            .map(|id| ItemId::Asset(id.clone()))
                            .as_ref()
                {
                    self.clear(&mut out.effects);
                }
            }
            Input::Clear => self.clear(&mut out.effects),
            Input::DeleteAsset(id) => {
                self.request_delete(c, Some(id), &mut out)?;
            }
            Input::OpenFolder(id) => self.open_folder(c, r, id, &mut out.effects),
            Input::Filter(filter) => {
                self.filter = filter;
                out.effects.extend(self.refresh(c, r));
            }
            Input::Invoke(command) => self.invoke(c, r, command, &mut out)?,
            Input::BeginRename(id) => {
                if !c.folders.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                self.renaming = Some(id);
            }
            Input::CommitRename { folder, value } => {
                if self.renaming.as_ref() == Some(&folder) {
                    self.renaming = None;
                    let value = pm_model::json::js_trim(&value).to_owned();
                    if !value.is_empty() && c.folders.get(&folder).is_some_and(|f| f.name != value)
                    {
                        out.edit = Some(Edit::RenameFolder {
                            id: folder,
                            name: value,
                        });
                    }
                }
            }
            Input::CancelRename => self.renaming = None,
            Input::ConfirmDelete { token, approved } => {
                let pending = self
                    .confirmation
                    .as_ref()
                    .filter(|p| p.token == token)
                    .ok_or(Error::Stale)?;
                let pending = pending.clone();
                self.confirmation = None;
                if approved {
                    if pending.stamp != c.stamp {
                        return Err(Error::Stale);
                    }
                    out.edit = Some(Edit::DeleteAssets {
                        ids: pending.assets,
                        confirmed: true,
                    });
                    self.clear(&mut out.effects);
                }
            }
            Input::DragStart { id, nested_control } => {
                if nested_control {
                    return Ok(Outcome::default());
                }
                if !self.rows.contains(&id) {
                    return Err(Error::MissingItem);
                }
                let (name, kind, duration, assets, copy, bin) = match &id {
                    ItemId::Asset(id) => {
                        let a = &c.assets[id];
                        if !matches!(r.state(&c.stamp, id).availability, Availability::Ready) {
                            return Ok(Outcome::default());
                        }
                        if !self.is_selected(id) {
                            self.select(id, Modifiers::default(), &mut out.effects);
                        }
                        (
                            a.name.clone(),
                            a.kind.clone(),
                            a.duration,
                            self.asset_order
                                .iter()
                                .filter(|id| self.is_selected(id))
                                .cloned()
                                .collect(),
                            true,
                            true,
                        )
                    }
                    ItemId::Folder(id) => {
                        if self.renaming.as_ref() == Some(id) {
                            return Ok(Outcome::default());
                        }
                        self.select_exclusive(ItemId::Folder(id.clone()), &mut out.effects);
                        (
                            c.folders[id].name.clone(),
                            "folder".into(),
                            None,
                            vec![],
                            false,
                            true,
                        )
                    }
                    ItemId::Composition(id) => {
                        self.select_exclusive(ItemId::Composition(id.clone()), &mut out.effects);
                        let a = &c.compositions[id];
                        (
                            a.name.clone(),
                            "comp".into(),
                            a.duration,
                            vec![],
                            true,
                            false,
                        )
                    }
                };
                self.drag = Some(Drag {
                    primary: id,
                    assets,
                    name,
                    kind,
                    duration,
                    copy_to_timeline: copy,
                    move_to_bin: bin,
                });
            }
            Input::DragEnd => {
                self.drag = None;
                self.drop_target = None;
            }
            Input::FilesEnter => self.drop_depth = self.drop_depth.saturating_add(1),
            Input::FilesLeave => self.drop_depth = self.drop_depth.saturating_sub(1),
            Input::DropLeave => self.drop_target = None,
            Input::DragOver { target, files } => {
                if !self.can_drop(c, target.as_deref(), files) {
                    return Ok(Outcome::default());
                }
                self.drop_target = Some(target);
            }
            Input::Drop {
                target,
                files,
                modifiers,
            } => {
                let external = !files.is_empty() || self.drop_depth > 0;
                self.drop_target = None;
                self.drop_depth = 0;
                if external && files.is_empty() {
                    return Ok(out);
                }
                if !self.can_drop(c, target.as_deref(), external) {
                    return Err(Error::InvalidTarget);
                }
                if external {
                    out.effects.push(Effect::Import {
                        files,
                        folder: target,
                        as_folder: modifiers.alt,
                    });
                } else if let Some(drag) = &self.drag {
                    out.edit = Some(Edit::Move {
                        assets: drag.assets.clone(),
                        folders: if let ItemId::Folder(id) = &drag.primary {
                            vec![id.clone()]
                        } else {
                            vec![]
                        },
                        target,
                    });
                }
            }
            Input::BeginMarquee { pointer, modifiers } => {
                let base = if modifiers.primary() || modifiers.shift {
                    self.selected_assets().to_vec()
                } else {
                    vec![]
                };
                self.clear(&mut out.effects);
                self.all_except = None;
                self.selection_cache.take();
                self.selection = base.clone();
                self.selected = base.iter().cloned().collect();
                self.active = base.last().cloned();
                self.marquee = Some(crate::model::Marquee { pointer, base });
                out.effects.push(Effect::Focus(None));
            }
            Input::MarqueeHits {
                pointer,
                activated,
                ids,
            } => {
                let Some(m) = self.marquee.as_ref().filter(|m| m.pointer == pointer) else {
                    return Ok(Outcome::default());
                };
                self.all_except = None;
                self.selection_cache.take();
                self.selection = m.base.clone();
                self.selected = self.selection.iter().cloned().collect();
                if activated {
                    for id in ids {
                        if self.positions.contains_key(&id) && self.selected.insert(id.clone()) {
                            self.selection.push(id);
                        }
                    }
                    self.anchor = self.selection.first().cloned();
                }
                self.active = self.selection.last().cloned();
            }
            Input::EndMarquee { pointer } => {
                if pointer.is_none()
                    || self
                        .marquee
                        .as_ref()
                        .is_some_and(|m| Some(m.pointer) == pointer)
                {
                    self.marquee = None;
                }
            }
            Input::MarqueeEdge {
                pointer,
                activated,
                direction,
            } => {
                if activated && self.marquee.as_ref().is_some_and(|m| m.pointer == pointer) {
                    out.effects.push(Effect::ScrollMarquee(direction));
                }
            }
            Input::SeekSource(seconds) => self.seek(seconds, &mut out)?,
            Input::PreviewFeedback {
                stamp,
                asset,
                playing,
                seconds,
                duration,
            } => {
                if stamp != c.stamp || self.preview.asset.as_ref() != Some(&asset) {
                    return Err(Error::Stale);
                }
                if !seconds.is_finite()
                    || seconds < 0.0
                    || duration.is_some_and(|d| !d.is_finite() || d < 0.0)
                {
                    return Err(Error::InvalidMedia);
                }
                self.preview.playing = playing;
                self.preview.seconds = seconds;
                self.preview.duration = duration;
            }
        }
        Ok(out)
    }
    fn open_folder(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        id: Option<String>,
        effects: &mut Vec<Effect>,
    ) {
        self.clear(effects);
        self.folder = c.valid_folder(id.as_deref());
        effects.extend(self.refresh(c, r));
    }
    fn activate(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        id: &str,
        preview: bool,
        out: &mut Outcome,
    ) -> Result<(), Error> {
        let a = c.assets.get(id).ok_or(Error::MissingItem)?;
        match r.state(&c.stamp, id).availability {
            Availability::Unknown | Availability::Loading => {}
            Availability::Cloud {
                download: Download::Downloading,
                ..
            } => {}
            Availability::Cloud { .. } => {
                self.select(id, Modifiers::default(), &mut out.effects);
                out.effects.push(Effect::Download(id.into()));
            }
            Availability::Offline | Availability::Changed { .. } | Availability::Failed(_) => {
                self.select(id, Modifiers::default(), &mut out.effects);
                out.effects.push(Effect::PickFiles {
                    replace: Some(id.into()),
                });
            }
            Availability::Ready => {
                if preview {
                    if a.kind != "model" {
                        self.preview = Preview {
                            asset: Some(id.into()),
                            transport: matches!(a.kind.as_str(), "video" | "audio"),
                            duration: a.duration,
                            ..Preview::default()
                        };
                        out.effects.push(Effect::ShowPreview(id.into()));
                    }
                } else {
                    out.effects.push(Effect::AddAsset(id.into()));
                }
            }
        }
        Ok(())
    }
    fn toggle_preview(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        id: &str,
        out: &mut Outcome,
    ) -> Result<(), Error> {
        let a = c.assets.get(id).ok_or(Error::MissingItem)?;
        if a.kind == "model" || !matches!(r.state(&c.stamp, id).availability, Availability::Ready) {
            return Ok(());
        }
        if self.preview.asset.as_deref() != Some(id) {
            self.preview = Preview {
                asset: Some(id.into()),
                transport: matches!(a.kind.as_str(), "video" | "audio"),
                duration: a.duration,
                ..Preview::default()
            };
        }
        out.effects.push(Effect::TogglePreview(id.into()));
        Ok(())
    }
    fn seek(&mut self, seconds: f64, out: &mut Outcome) -> Result<(), Error> {
        if !seconds.is_finite() {
            return Err(Error::InvalidMedia);
        }
        if !self.preview.transport {
            return Ok(());
        }
        if let (Some(id), Some(duration)) = (
            &self.preview.asset,
            self.preview.duration.filter(|d| d.is_finite() && *d > 0.0),
        ) {
            self.preview.seconds = seconds.clamp(0.0, duration);
            out.effects.push(Effect::SeekSource {
                id: id.clone(),
                seconds: self.preview.seconds,
            });
        }
        Ok(())
    }
    fn request_delete(
        &mut self,
        c: &Catalog,
        asset: Option<String>,
        out: &mut Outcome,
    ) -> Result<(), Error> {
        if self.confirmation.is_some() {
            return Ok(());
        }
        let ids = if let Some(id) = asset.filter(|id| !self.is_selected(id)) {
            vec![id]
        } else if self.selected_count() > 0 {
            self.asset_order
                .iter()
                .filter(|id| self.is_selected(id))
                .cloned()
                .collect()
        } else {
            vec![]
        };
        if ids.is_empty() {
            return Ok(());
        }
        let references = ids
            .iter()
            .map(|id| {
                c.assets
                    .get(id)
                    .map(|a| a.references)
                    .ok_or(Error::MissingItem)
            })
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum();
        if references == 0 {
            out.edit = Some(Edit::DeleteAssets {
                ids,
                confirmed: false,
            });
            self.clear(&mut out.effects);
        } else {
            self.next_token = self.next_token.checked_add(1).ok_or(Error::InvalidTarget)?;
            let token = self.next_token;
            self.confirmation = Some(crate::model::Confirmation {
                token,
                stamp: c.stamp.clone(),
                assets: ids.clone(),
            });
            out.effects.push(Effect::ConfirmDelete {
                token,
                assets: ids,
                references,
            });
        }
        Ok(())
    }
    pub fn can_drop(&self, c: &Catalog, target: Option<&str>, files: bool) -> bool {
        if target.is_some_and(|id| !c.folders.contains_key(id)) {
            return false;
        }
        if files {
            return true;
        }
        self.drag.as_ref().is_some_and(|d| {
            d.move_to_bin
                && match &d.primary {
                    ItemId::Folder(id) => !c.within(target, id),
                    _ => true,
                }
        })
    }
    fn key(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        target: KeyTarget,
        key: Key,
        m: Modifiers,
    ) -> Result<Outcome, Error> {
        let mut out = Outcome::handled();
        if matches!(target, KeyTarget::TextField) {
            return Ok(Outcome::default());
        }
        if matches!(target, KeyTarget::NestedControl) {
            out.handled = !m.primary() && !m.alt && key != Key::A;
            return Ok(out);
        }
        if matches!(target, KeyTarget::Source) {
            match key {
                Key::Escape => self.clear_preview(&mut out.effects),
                Key::Space => {
                    if let Some(id) = self.preview.asset.clone() {
                        self.toggle_preview(c, r, &id, &mut out)?;
                    }
                }
                Key::Left => self.seek(self.preview.seconds - 5.0, &mut out)?,
                Key::Right => self.seek(self.preview.seconds + 5.0, &mut out)?,
                Key::Home => self.seek(0.0, &mut out)?,
                Key::End => self.seek(self.preview.duration.unwrap_or(0.0), &mut out)?,
                _ => out.handled = false,
            }
            return Ok(out);
        }
        if key == Key::A && m.primary() {
            self.exclusive = None;
            self.all_except = Some(Default::default());
            self.selection_cache.take();
            // Retain any explicit buffers until the next ordinary selection or
            // worker refresh; entering All must not visit the previous batch.
            self.active = self.asset_order.last().cloned();
            self.anchor = self.asset_order.first().cloned();
            self.clear_preview(&mut out.effects);
            return Ok(out);
        }
        if matches!(
            key,
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End
        ) {
            let n = self.rows.len();
            if n == 0 {
                return Ok(out);
            }
            let index = match &target {
                KeyTarget::Row(id) => *self.row_positions.get(id).ok_or(Error::MissingItem)?,
                _ => 0,
            };
            let columns = self.grid_columns.max(1).min(n);
            let next = match key {
                Key::Right => (index + 1) % n,
                Key::Left => (index + n - 1) % n,
                Key::Down => (((index / columns + 1) % n.div_ceil(columns)) * columns
                    + index % columns)
                    .min(n - 1),
                Key::Up => (((index / columns + n.div_ceil(columns) - 1) % n.div_ceil(columns))
                    * columns
                    + index % columns)
                    .min(n - 1),
                Key::Home => 0,
                Key::End => n - 1,
                _ => unreachable!(),
            };
            match self.rows[next].clone() {
                ItemId::Asset(id) => self.select(&id, m, &mut out.effects),
                id => self.select_exclusive(id, &mut out.effects),
            }
            return Ok(out);
        }
        match target {
            KeyTarget::List => match key {
                Key::Escape => self.clear(&mut out.effects),
                Key::Delete | Key::Backspace => self.request_delete(c, None, &mut out)?,
                _ => out.handled = false,
            },
            KeyTarget::Row(ItemId::Asset(id)) => {
                let index = *self.positions.get(&id).ok_or(Error::MissingItem)?;
                let n = self.asset_order.len();
                let next = match key {
                    Key::Down | Key::Right => Some((index + 1) % n),
                    Key::Up | Key::Left => Some((index + n - 1) % n),
                    Key::Home => Some(0),
                    Key::End => Some(n - 1),
                    _ => None,
                };
                if let Some(next) = next {
                    let id = self.asset_order[next].clone();
                    self.select(&id, m, &mut out.effects);
                } else {
                    match key {
                        Key::Enter => self.activate(c, r, &id, false, &mut out)?,
                        Key::Space => {
                            self.select(&id, Modifiers::default(), &mut out.effects);
                            self.toggle_preview(c, r, &id, &mut out)?;
                        }
                        Key::Delete | Key::Backspace => {
                            self.request_delete(c, Some(id), &mut out)?
                        }
                        Key::Escape => self.clear(&mut out.effects),
                        _ => out.handled = false,
                    }
                }
            }
            KeyTarget::Row(ItemId::Folder(id)) => {
                if !c.folders.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                match key {
                    Key::Enter => self.open_folder(c, r, Some(id), &mut out.effects),
                    Key::Delete | Key::Backspace => out.edit = Some(Edit::DeleteFolder { id }),
                    Key::Escape => self.clear(&mut out.effects),
                    _ => out.handled = false,
                }
            }
            KeyTarget::Row(ItemId::Composition(id)) => {
                if !c.compositions.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                match key {
                    Key::Enter => out.effects.push(Effect::OpenComposition(id)),
                    Key::Delete | Key::Backspace => out.edit = Some(Edit::DeleteComposition { id }),
                    Key::Escape => self.clear(&mut out.effects),
                    _ => out.handled = false,
                }
            }
            _ => out.handled = false,
        }
        Ok(out)
    }
    fn invoke(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        cmd: MenuCommand,
        out: &mut Outcome,
    ) -> Result<(), Error> {
        use MenuCommand::*;
        let open_composition = matches!(cmd, OpenComposition(_));
        match cmd {
            AddAsset(id) => self.activate(c, r, &id, false, out)?,
            NewCompFromAsset(id) => {
                let a = c.assets.get(&id).ok_or(Error::MissingItem)?;
                if matches!(a.kind.as_str(), "audio" | "model")
                    || !matches!(r.state(&c.stamp, &id).availability, Availability::Ready)
                {
                    return Err(Error::InvalidTarget);
                }
                out.effects.push(Effect::NewCompFromAsset(id));
            }
            Replace(id) => {
                if !c.assets.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                out.effects.push(Effect::PickFiles { replace: Some(id) });
            }
            Reveal(id) => {
                let path = c
                    .assets
                    .get(&id)
                    .and_then(|a| a.source_path.clone())
                    .ok_or(Error::MissingItem)?;
                self.select(&id, Modifiers::default(), &mut out.effects);
                out.effects.push(Effect::Reveal { id, path });
            }
            Download(id) => {
                if matches!(
                    r.state(&c.stamp, &id).availability,
                    Availability::Cloud {
                        download: crate::Download::Stored | crate::Download::Failed(_),
                        ..
                    }
                ) {
                    out.effects.push(Effect::Download(id));
                } else {
                    return Err(Error::InvalidTarget);
                }
            }
            DeleteAssets => self.request_delete(c, None, out)?,
            Rendition(action) => {
                let assets = self.rendition_targets(c, r);
                if assets.is_empty()
                    || (action == crate::RenditionAction::AttachProxy && assets.len() != 1)
                {
                    return Err(Error::InvalidTarget);
                }
                out.effects.push(Effect::Renditions { action, assets });
            }
            NewComposition => out.effects.push(Effect::NewComposition),
            ImportFiles => out.effects.push(Effect::PickFiles { replace: None }),
            ImportFolder(folder) => {
                if folder.is_some() && c.valid_folder(folder.as_deref()) != folder {
                    return Err(Error::MissingItem);
                }
                out.effects.push(Effect::PickFolder { folder });
            }
            NewFolder(parent) => {
                if parent.is_some() && c.valid_folder(parent.as_deref()) != parent {
                    return Err(Error::MissingItem);
                }
                out.edit = Some(Edit::CreateFolder {
                    id: pm_kernel::new_id("f"),
                    name: "Untitled Folder".into(),
                    parent,
                });
            }
            OpenFolder(id) => self.open_folder(c, r, Some(id), &mut out.effects),
            RenameFolder(id) => {
                if !c.folders.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                self.renaming = Some(id);
            }
            DeleteFolder(id) => out.edit = Some(Edit::DeleteFolder { id }),
            MoveOut(item) => {
                let target = self
                    .folder
                    .as_ref()
                    .and_then(|id| c.folders.get(id))
                    .and_then(|f| f.parent.clone());
                out.edit = Some(match item {
                    ItemId::Asset(id) => Edit::Move {
                        assets: vec![id],
                        folders: vec![],
                        target,
                    },
                    ItemId::Folder(id) => Edit::Move {
                        assets: vec![],
                        folders: vec![id],
                        target,
                    },
                    _ => return Err(Error::InvalidTarget),
                });
            }
            OpenComposition(id) | CompositionSettings(id) => {
                if !c.compositions.contains_key(&id) {
                    return Err(Error::MissingItem);
                }
                out.effects.push(if open_composition {
                    Effect::OpenComposition(id)
                } else {
                    Effect::CompositionSettings(id)
                });
            }
            NestComposition(id) => {
                if !c.compositions.get(&id).is_some_and(|c| c.can_nest) {
                    return Err(Error::InvalidTarget);
                }
                out.effects.push(Effect::NestComposition(id));
            }
            DuplicateComposition(id) => {
                out.edit = Some(Edit::DuplicateComposition {
                    id,
                    new_id: pm_kernel::new_id("C"),
                })
            }
            DeleteComposition(id) => out.edit = Some(Edit::DeleteComposition { id }),
        }
        Ok(())
    }
}
