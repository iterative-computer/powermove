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
    pub renaming: Option<ItemId>,
    pub drag: Option<Drag>,
    // Distinguish no target from an active root target in JSON (Option<Option>
    // loses that distinction because both serialize as null).
    pub drop_target: Option<DropTarget>,
    pub external_drag: bool,
    pub composition_count: usize,
    /// Only missing files are listed (the status line's offline count).
    pub missing_only: bool,
    /// The search the model applies, so the field shows what the host set.
    pub query: String,
    /// Moves each time the host, not the panel, set the filter (the status
    /// line's missing files): the panel's search field takes `query` then,
    /// and only then, even while the person is typing in it.
    pub host_filter: u64,
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
    host_filter: u64,
}
impl ProjectBridge {
    /// The host's own filter (the status line's missing files): from the top
    /// folder, marked so the panel's search field follows it.
    pub fn apply_host_filter(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        filter: crate::Filter,
    ) -> Result<(), Error> {
        for input in [Input::OpenFolder(None), Input::Filter(filter)] {
            self.request(
                c,
                r,
                Request {
                    owner: c.stamp.clone(),
                    action: Action::Input(input),
                },
            )?;
        }
        self.host_filter += 1;
        Ok(())
    }
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
            missing_only: self.panel.filter.missing_only,
            query: self.panel.filter.query.clone(),
            host_filter: self.host_filter,
        })
    }
    pub fn request(
        &mut self,
        c: &Catalog,
        r: &dyn MediaRuntime,
        request: Request,
    ) -> Result<Reply, Error> {
        // A page is a read of the latest catalog. Ordinary document edits
        // may advance its revision while the request is queued. Other
        // actions still require the exact state they were made against.
        let current_page = matches!(&request.action, Action::Page { .. })
            && request.owner.project == c.stamp.project
            && request.owner.document == c.stamp.document
            && request.owner.revision <= c.stamp.revision;
        if request.owner != c.stamp && !current_page {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UnknownRuntime;

    fn setup() -> (Catalog, ProjectBridge) {
        let project = pm_model::json::parse(
            r#"{"compId":"C","compName":"Main","w":64,"h":64,"dur":10,"fps":30,"layers":[],"assets":{},"comps":{}}"#,
        ).unwrap();
        let catalog = Catalog::new(
            "P".into(),
            pm_kernel::DocumentStore::new_untitled(project).snapshot(),
        )
        .unwrap();
        let mut bridge = ProjectBridge::default();
        bridge.refresh(&catalog, &UnknownRuntime);
        (catalog, bridge)
    }

    #[test]
    fn trim_revision_does_not_fail_a_read_only_project_page() {
        let (mut c, mut b) = setup();
        let owner = c.stamp.clone();
        c.stamp.revision += 1;
        b.refresh(&c, &UnknownRuntime);
        let reply = b
            .request(
                &c,
                &UnknownRuntime,
                Request {
                    owner: owner.clone(),
                    action: Action::Page {
                        offset: 0,
                        limit: 96,
                    },
                },
            )
            .expect("ordinary document edits must not fail a media page read");
        assert_eq!(reply.view.page.stamp, c.stamp);
        assert!(
            matches!(
                b.request(
                    &c,
                    &UnknownRuntime,
                    Request {
                        owner,
                        action: Action::Input(Input::FilesEnter),
                    }
                ),
                Err(Error::Stale)
            ),
            "edits still require their exact owner"
        );
    }

    #[test]
    fn page_reads_still_refuse_other_documents_projects_and_future_revisions() {
        let (c, mut bridge) = setup();
        for owner in [
            Stamp {
                document: c.stamp.document + 1,
                ..c.stamp.clone()
            },
            Stamp {
                project: "other".into(),
                ..c.stamp.clone()
            },
            Stamp {
                revision: c.stamp.revision + 1,
                ..c.stamp.clone()
            },
        ] {
            assert!(matches!(
                bridge.request(
                    &c,
                    &UnknownRuntime,
                    Request {
                        owner,
                        action: Action::Page {
                            offset: 0,
                            limit: 96
                        },
                    }
                ),
                Err(Error::Stale)
            ));
        }
    }
}
