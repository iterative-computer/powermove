//! Worker adapter for the project v4 view. No IO or main-thread JS entry.
//! Dispatch returns plans/effects, NOT a successful document edit acknowledgement.
//! The application must apply each plan through the kernel before acknowledging.
use crate::{
    Catalog, ContextMenu, Drag, Error, Folder, Input, ItemId, MediaPanel, MediaRuntime, Outcome,
    Page, Stamp,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Page { offset: usize, limit: usize },
    Input(Input),
    Menu(Option<ItemId>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub owner: Stamp,
    pub action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Projection {
    pub page: Page,
    pub folder: Option<String>,
    pub crumbs: Vec<Folder>,
    pub selected_count: usize,
    pub focused: Option<ItemId>,
    pub renaming: Option<String>,
    pub drag: Option<Drag>,
    // Distinguish no target from an active root target in JSON (Option<Option>
    // loses that distinction because both serialize as null).
    pub drop_target: Option<DropTarget>,
    pub external_drag: bool,
    pub composition_count: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DropTarget {
    pub folder: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reply {
    pub view: Projection,
    pub outcome: Option<Outcome>,
    pub menu: Option<ContextMenu>,
}
#[derive(Default)]
pub struct ProjectBridge {
    pub panel: MediaPanel,
    offset: usize,
    limit: usize,
    document: Option<(String, u64)>,
}
impl ProjectBridge {
    pub fn refresh(&mut self, catalog: &Catalog, runtime: &dyn MediaRuntime) -> Vec<crate::Effect> {
        let old_folder = self.panel.folder.clone();
        let document = (catalog.stamp.project.clone(), catalog.stamp.document);
        let changed_document = self.document.as_ref().is_some_and(|old| old != &document);
        let effects = self.panel.refresh(catalog, runtime);
        if changed_document || old_folder != self.panel.folder {
            self.offset = 0;
        }
        self.document = Some(document);
        effects
    }
    pub fn view(&self, c: &Catalog, r: &dyn MediaRuntime) -> Result<Projection, Error> {
        let offset = self.offset.min(self.panel.total());
        Ok(Projection {
            page: self.panel.page(c, r, offset, self.limit)?,
            folder: self.panel.folder.clone(),
            crumbs: c
                .path(self.panel.folder.as_deref())
                .into_iter()
                .cloned()
                .collect(),
            selected_count: self.panel.selected_count(),
            focused: self.panel.focused().cloned(),
            renaming: self.panel.renaming.clone(),
            drag: self.panel.drag.clone(),
            drop_target: self
                .panel
                .drop_target
                .clone()
                .map(|folder| DropTarget { folder }),
            external_drag: self.panel.file_drop_active(),
            composition_count: c.compositions.len(),
        })
    }
    pub fn request(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        request: Request,
    ) -> Result<Reply, Error> {
        if request.owner != c.stamp {
            return Err(Error::Stale);
        }
        let mut outcome = None;
        let mut menu = None;
        match request.action {
            Action::Page { offset, limit } => {
                // Bound the output before visiting runtime/poster state.
                if limit > 4096 {
                    return Err(Error::InvalidTarget);
                }
                self.offset = offset;
                self.limit = limit;
            }
            Action::Input(input) => {
                let folder = self.panel.folder.clone();
                let filter = self.panel.filter.clone();
                outcome = Some(self.panel.dispatch(c, r, input)?);
                if self.panel.folder != folder || self.panel.filter != filter {
                    self.offset = 0;
                }
            }
            Action::Menu(id) => {
                menu = Some(self.panel.menu(c, r, id)?);
            }
        }
        Ok(Reply {
            view: self.view(c, r)?,
            outcome,
            menu,
        })
    }
}
