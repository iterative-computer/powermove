//! Window ▸ Store (S075–S078): browse the storefront and its seven kinds,
//! search, read an extension's page with its versions and what it uses, and
//! install, update or uninstall. Library lists what the Store installed on
//! this Mac and which of those have updates (the shipped app's "Needs
//! attention"). Every request runs on a worker; installs go through the same
//! verified path the palette used, so a Store extension runs contained as
//! before, and every window rescans when one lands.
use gpui::{prelude::*, *};
use pm_compat_runner::{StoreAuthority, discovery, store_install};
use pm_design::Text;
use pm_design::Tokens;
use pm_services::surfaces::publishing::{Draft, Problem};
use pm_services::{
    ServiceErrorCode,
    store::{InstalledRelease, StoreClient, StoreTransport},
    surfaces::store::{
        Catalog, CatalogQuery, CatalogState, Detail, DetailAction, Health, ItemFacts, ItemGroup,
        KINDS, Listing, detail_action, includes_text, kind_label, permission_lines, relative_date,
        requires_text,
    },
};
use pm_shell::{
    kit::{self, Button, Kit, Recipes, ink},
    motion::Motion,
    native_control::{ControlSpec, Kind, NativeControl},
    symbol::Weight,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub type Client = StoreClient<Box<dyn StoreTransport + Send>>;
/// What the window reaches outside itself; tests hand in a fixture Store.
#[derive(Clone)]
pub struct Services {
    pub client: Arc<dyn Fn() -> pm_services::Result<Client> + Send + Sync>,
    pub kernel: Arc<dyn StoreAuthority>,
    pub profile: PathBuf,
    /// Where releases come from and the keys they must be signed with;
    /// `None` while this build pins no Store key (installs are refused).
    pub source: Option<Arc<store_install::Source>>,
    /// The Powermove account's registry client, for publishing.
    pub account: Arc<dyn Fn() -> crate::cloud::Http + Send + Sync>,
}
impl Services {
    /// The app's Store over the system HTTP stack, bound to the Store origin.
    pub fn system(kernel: pm_kernel::KernelHandle, profile: PathBuf) -> Self {
        Self {
            client: Arc::new(|| {
                let origin = store_install::origin();
                let transport: Box<dyn StoreTransport + Send> = Box::new(Http { origin });
                StoreClient::new(origin, transport)
            }),
            kernel: Arc::new(kernel),
            account: {
                let profile = profile.clone();
                Arc::new(move || {
                    crate::cloud::Http::new(
                        store_install::origin(),
                        Box::new(crate::cloud::Keychain::for_app(&profile)),
                        profile.clone(),
                    )
                })
            },
            profile,
            source: store_install::source().ok().flatten().map(Arc::new),
        }
    }
}
/// `pm-services`' Store transport on the system HTTP stack: the Store origin
/// only, at the request and at every redirect.
struct Http {
    origin: &'static str,
}
impl StoreTransport for Http {
    fn send(
        &mut self,
        request: pm_services::store::Request,
        max_bytes: usize,
    ) -> pm_services::Result<pm_services::store::Response> {
        use pm_services::{ServiceError, store::Method};
        let (method, body) = match request.method {
            Method::Get => ("GET", Vec::new()),
            Method::PostVersions => (
                "POST",
                serde_json::to_vec(&request.body.unwrap_or(Value::Null)).unwrap_or_default(),
            ),
        };
        let mut headers = vec![("X-Powermove-Client".into(), client_header())];
        if method == "POST" {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        let response = pm_compat_runner::net::send(
            pm_runtime_v8::FetchRequest {
                url: request.url.into(),
                method: method.into(),
                headers,
                body,
                max_response_bytes: max_bytes,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            },
            pm_compat_runner::net::Reach::Origin(self.origin),
        )
        .map_err(|error| ServiceError::message(ServiceErrorCode::Offline, error.to_string()))?;
        Ok(pm_services::store::Response {
            status: response.status,
            body: response.body,
        })
    }
}
/// `X-Powermove-Client`, as the shipped app names itself to the registry.
fn client_header() -> String {
    format!("desktop/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Discover,
    Kind(usize),
    Library,
}
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Discover => "Discover",
            Self::Kind(i) => KINDS[i].1,
            Self::Library => "Library",
        }
    }
}
/// One storefront shelf: its id (`featured`, `new` or a kind), its title and
/// its listings.
#[derive(Clone, Debug, PartialEq)]
pub struct Shelf {
    pub id: String,
    pub title: String,
    pub listings: Vec<Listing>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}
/// A Store install on this Mac: its folder carries the Store marker.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub repo: Option<String>,
    pub release: Option<String>,
    /// (version, release id) of a newer release the Store has.
    pub update: Option<(String, String)>,
    /// The Store no longer lists it.
    pub removed: bool,
}
/// A release a person chose to install: the page's extension and version
/// for it, which its signed envelope must name.
#[derive(Clone, Debug)]
pub struct Pick {
    pub release: String,
    pub repo: String,
    pub extension: String,
    pub version: String,
    pub name: String,
}
/// An extension made on this Mac (no Store marker): it can be published.
#[derive(Clone, Debug, PartialEq)]
pub struct Yours {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The version this Mac last published.
    pub published: Option<String>,
}
enum Reply {
    Shelves(u64, Result<Vec<Shelf>, String>),
    Catalog(pm_services::surfaces::store::SearchReply),
    Detail(u64, Box<Result<Detail, String>>),
    /// What is installed, what was made here, a note, and whether the Store
    /// was asked about updates (the first, local reply was not).
    Library(Vec<Installed>, Vec<Yours>, Option<String>, bool),
    Prepared(Box<Result<crate::publish::Prepared, String>>),
    Frozen(
        Box<(
            crate::publish::Prepared,
            Result<crate::publish::Frozen, String>,
        )>,
    ),
    Done(String),
}

pub struct StoreWindow {
    motion: Motion<u64>,
    frame_kit: Option<Kit>,
    services: Services,
    tokens: Tokens,
    pub page: Page,
    pub query: String,
    search: Option<Entity<pm_shell::text::TextField>>,
    /// Discover · Library, the system's segmented control (a real window only).
    pages: Option<Entity<NativeControl>>,
    pub focus: FocusHandle,
    pub shelves: Load<Vec<Shelf>>,
    catalog: Catalog,
    pub detail: Option<(Listing, Load<Detail>)>,
    pub library: Load<Vec<Installed>>,
    /// Extensions made on this Mac (Library › Yours).
    pub yours: Vec<Yours>,
    /// The publish being prepared or confirmed, and the sheet asking for it.
    publishing: Option<(crate::publish::Prepared, Option<crate::sheets::OpenSheet>)>,
    owner: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>>,
    /// The extension or repository an install, update or uninstall is running for.
    pub busy: Option<String>,
    pub message: Option<String>,
    /// Shown as a page of the editor window, under its toolbar: the sidebar
    /// keeps no room for window buttons.
    pub embedded: bool,
    generation: u64,
    /// The storefront's own requests: a page opened meanwhile keeps it.
    shelves_generation: u64,
    tx: async_channel::Sender<Reply>,
}

impl StoreWindow {
    fn motion(&mut self) -> &mut Motion<u64> {
        &mut self.motion
    }
    fn kit(&self) -> &Kit {
        self.frame_kit
            .as_ref()
            .expect("Store controls are built during render")
    }
    pub fn new(services: Services, tokens: Tokens, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = async_channel::unbounded();
        cx.spawn(async move |this, cx| {
            while let Ok(reply) = rx.recv().await {
                if this.update(cx, |v, cx| v.receive(reply, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        // The storefront last seen draws at once; a fresh one replaces it.
        let shelves = cached_shelves(&services.profile).map_or(Load::Loading, Load::Ready);
        let mut window = Self {
            motion: Motion::default(),
            frame_kit: None,
            services,
            tokens,
            page: Page::Discover,
            query: String::new(),
            search: None,
            pages: None,
            focus: cx.focus_handle(),
            shelves,
            catalog: Catalog::default(),
            detail: None,
            library: Load::Loading,
            yours: Vec::new(),
            publishing: None,
            owner: None,
            busy: None,
            message: None,
            embedded: false,
            generation: 0,
            shelves_generation: 0,
            tx,
        };
        window.load_shelves();
        window.load_library();
        window
    }
    /// The native search field; only a real AppKit window can host one.
    pub fn attach_search(&mut self, window: &Window, cx: &mut Context<Self>) {
        use pm_shell::text::{TextEvent, TextField, TextKind};
        let field = cx.new(|cx| {
            let mut field = TextField::new(
                TextKind::Primary,
                "Search the Store",
                "",
                self.tokens.clone(),
                window,
                cx,
            );
            field.set_placeholder("Search");
            field
        });
        cx.subscribe(&field, |v, _, event: &TextEvent, cx| match event {
            TextEvent::Changed(text) | TextEvent::Committed(text) | TextEvent::Submitted(text) => {
                v.set_query(text, cx)
            }
            _ => {}
        })
        .detach();
        self.search = Some(field);
        let index = json!(usize::from(self.page == Page::Library));
        let tokens = self.tokens.clone();
        let pages = cx.new(|cx| {
            NativeControl::new(
                ControlSpec {
                    kind: Kind::Segmented,
                    label: "Store pages",
                    identifier: "store.pages",
                    value: &index,
                    options: vec![json!("Discover"), json!("Library")],
                    disabled: false,
                    default: false,
                    destructive: false,
                    inline: false,
                    quiet: false,
                    pull: false,
                    tint: None,
                    symbol: None,
                    glass: true,
                    well: false,
                },
                &tokens,
                window,
                cx,
            )
        });
        cx.subscribe(&pages, |v: &mut Self, _, value: &Value, cx| {
            let page = if value.as_u64() == Some(1) {
                Page::Library
            } else {
                Page::Discover
            };
            if page != v.page || v.detail.is_some() {
                v.set_page(page, cx);
            }
        })
        .detach();
        self.pages = Some(pages);
    }
    /// The segmented control follows the page however it changed.
    fn show_page(&self, cx: &mut Context<Self>) {
        if let Some(pages) = &self.pages {
            let index = json!(usize::from(self.page == Page::Library));
            pages.update(cx, |p, cx| {
                p.update(&index, &[json!("Discover"), json!("Library")], false, cx)
            });
        }
    }
    /// Runs `work` on a worker with a fresh Store client.
    fn run(&self, work: impl FnOnce(pm_services::Result<Client>) -> Reply + Send + 'static) {
        let (client, tx) = (self.services.client.clone(), self.tx.clone());
        std::thread::Builder::new()
            .name("pm-store".into())
            .spawn(move || {
                let _ = tx.send_blocking(work(client()));
            })
            .ok();
    }
    fn load_shelves(&mut self) {
        self.shelves_generation += 1;
        let generation = self.shelves_generation;
        if !matches!(self.shelves, Load::Ready(_)) {
            self.shelves = Load::Loading;
        }
        let profile = self.services.profile.clone();
        self.run(move |client| {
            let result = client
                .and_then(|mut c| c.browse())
                .map_err(|error| error.to_string())
                .map(|shelves| {
                    keep_shelves(&profile, &shelves);
                    shelves_of(
                        shelves
                            .iter()
                            .map(|s| (s.id.as_str(), s.title.as_str(), &s.items)),
                    )
                });
            Reply::Shelves(generation, result)
        });
    }
    /// A kind's shelf or search results, through `pm-services`' catalog
    /// (stale replies are dropped; earlier results stay until replaced).
    fn load_catalog(&mut self) {
        let category = match self.page {
            Page::Kind(i) => Some(KINDS[i].0.to_owned()),
            _ => None,
        };
        let text = (!self.query.trim().is_empty()).then(|| self.query.trim().to_owned());
        match self.catalog.search(CatalogQuery { text, category }) {
            Ok(work) => self.run(move |client| match client {
                Ok(mut client) => Reply::Catalog(work.run(&mut client)),
                Err(error) => Reply::Catalog(work.fail(error)),
            }),
            Err(_) => self.message = Some("Search is limited to 120 characters.".into()),
        }
    }
    fn more(&mut self, cx: &mut Context<Self>) {
        if let Some(work) = self.catalog.more() {
            self.run(move |client| match client {
                Ok(mut client) => Reply::Catalog(work.run(&mut client)),
                Err(error) => Reply::Catalog(work.fail(error)),
            });
            cx.notify();
        }
    }
    /// What the Store installed here, then which of those it has updates for.
    fn load_library(&mut self) {
        let profile = self.services.profile.clone();
        let tx = self.tx.clone();
        self.run(move |client| {
            let mut installed = scan(&profile);
            // What is on this Mac shows at once; updates fill in.
            let _ = tx.send_blocking(Reply::Library(
                installed.clone(),
                yours(&profile),
                None,
                false,
            ));
            let releases: Vec<_> = installed
                .iter()
                .filter_map(|i| {
                    Some(InstalledRelease {
                        repo_id: i.repo.clone()?,
                        release_id: i.release.clone()?,
                    })
                })
                .filter(|r| uuid_like(&r.repo_id) && uuid_like(&r.release_id))
                .collect();
            let checked = client.and_then(|mut c| c.versions(&releases));
            let note = match checked {
                Ok(versions) => {
                    for version in versions {
                        let Some(item) = installed
                            .iter_mut()
                            .find(|i| i.repo.as_deref() == Some(version.repo_id.as_str()))
                        else {
                            continue;
                        };
                        item.removed = version.state != "ok";
                        item.update = version
                            .latest
                            .filter(|l| item.release.as_deref() != Some(l.release_id.as_str()))
                            .map(|l| (l.version, l.release_id));
                    }
                    None
                }
                Err(error) => Some(load_error(error)),
            };
            Reply::Library(installed, yours(&profile), note, true)
        });
    }
    pub fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.detail = None;
        self.message = None;
        match page {
            Page::Discover if self.query.trim().is_empty() => self.load_shelves(),
            Page::Library => self.load_library(),
            _ => self.load_catalog(),
        }
        self.show_page(cx);
        cx.notify();
    }
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.query == text {
            return;
        }
        self.query = text.to_owned();
        self.detail = None;
        self.message = None;
        if self.page == Page::Library {
            self.page = Page::Discover;
            self.show_page(cx);
        }
        if self.query.trim().is_empty() && self.page == Page::Discover {
            self.catalog.close();
            self.load_shelves();
        } else {
            self.load_catalog();
        }
        cx.notify();
    }
    pub fn open_detail(&mut self, listing: Listing, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (handle, slug) = (listing.handle.clone(), listing.slug.clone());
        self.detail = Some((listing, Load::Loading));
        self.message = None;
        self.run(move |client| {
            let detail = client
                .and_then(|mut c| c.detail(&handle, &slug))
                .map_err(|error| error.to_string())
                .and_then(|value| Detail::from_value(&value).ok_or_else(|| GENERIC.to_owned()));
            Reply::Detail(generation, Box::new(detail))
        });
        cx.notify();
    }
    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.detail.take().is_some() {
            self.message = None;
            cx.notify();
        }
    }
    fn installed_repo(&self, repo: &str) -> bool {
        matches!(&self.library, Load::Ready(items) if items.iter().any(|i| i.repo.as_deref() == Some(repo)))
    }
    /// The Library row of a listing: the Store install from its repository.
    fn installed(&self, listing: &Listing) -> Option<&Installed> {
        let Load::Ready(items) = &self.library else {
            return None;
        };
        items
            .iter()
            .find(|i| i.repo.as_deref() == Some(listing.repo_id.as_str()))
    }
    /// The detail page's primary action, by the shipped precedence.
    pub fn action(&self, detail: &Detail) -> DetailAction {
        let item = self.installed(&detail.listing).map(|i| ItemFacts {
            group: ItemGroup::Store,
            health: Health::Ready,
            enabled: true,
            fork: false,
            published_repo: None,
            update: i.update.is_some(),
            update_modified: false,
            removed: i.removed,
            publish: None,
        });
        let installing = self.busy.as_deref() == Some(detail.listing.repo_id.as_str());
        detail_action(
            item.as_ref(),
            Some(&detail.listing.repo_id),
            detail.declares_values,
            installing,
            detail.listing.latest.as_ref().and_then(|l| l.2),
        )
    }
    /// Installs (or updates to) a release, then every window rescans.
    pub fn install(&mut self, pick: Pick, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(source) = self.services.source.clone() else {
            self.message = Some(SIGNING.into());
            cx.notify();
            return;
        };
        let updating = self.installed_repo(&pick.repo);
        self.busy = Some(pick.repo.clone());
        self.message = None;
        let (kernel, profile) = (self.services.kernel.clone(), self.services.profile.clone());
        self.run(move |_| {
            let request = store_install::Request {
                release: &pick.release,
                repo: &pick.repo,
                extension: &pick.extension,
                version: &pick.version,
            };
            match store_install::install_from(
                &source,
                kernel.as_ref(),
                &profile,
                "native",
                &request,
            ) {
                Ok(_) => {
                    pm_compat_runner::host::rescan_all();
                    crate::cloud::remember_install(&profile, &pick.repo, Some(&pick.release));
                    Reply::Done(if updating {
                        format!("{} is updated.", pick.name)
                    } else {
                        format!("{} is installed.", pick.name)
                    })
                }
                Err(error) => Reply::Done(refusal(error)),
            }
        });
        cx.notify();
    }
    /// Asks first (default Cancel); the folder and its values are archived.
    pub fn confirm_uninstall(
        &mut self,
        id: String,
        name: String,
        repo: Option<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        use crate::alert::{Alert, Button};
        let Some(ns_window) = crate::platform::ns_window(window) else {
            return;
        };
        let rx = Alert::new(
            format!("Uninstall {name}?"),
            "Its files move to the archive in your Powermove profile, and its settings are kept if you install it again.",
            vec![
                Button::new("Cancel", false).default().cancel(),
                Button::new("Uninstall", true).destructive(),
            ],
        )
        .sheet(&ns_window);
        cx.spawn(async move |this, cx| {
            if rx.recv().await == Ok(Some(true)) {
                let _ = this.update(cx, |v, cx| v.uninstall(id, name, repo, cx));
            }
        })
        .detach();
    }
    pub fn uninstall(
        &mut self,
        id: String,
        name: String,
        repo: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(id.clone());
        let (kernel, profile) = (self.services.kernel.clone(), self.services.profile.clone());
        self.run(
            move |_| match store_install::uninstall(kernel.as_ref(), &profile, &id) {
                Ok(()) => {
                    pm_compat_runner::host::rescan_all();
                    if let Some(repo) = &repo {
                        crate::cloud::remember_install(&profile, repo, None);
                    }
                    Reply::Done(format!("{name} is uninstalled."))
                }
                Err(error) => Reply::Done(refusal(error)),
            },
        );
        cx.notify();
    }
    fn receive(&mut self, reply: Reply, cx: &mut Context<Self>) {
        let failure = match &reply {
            Reply::Shelves(_, Err(error)) => Some(error.as_str()),
            Reply::Detail(_, result) => result.as_ref().as_ref().err().map(String::as_str),
            _ => None,
        };
        if let Some(error) = failure {
            crate::error_log::shared(cx)
                .borrow_mut()
                .push("Store", error);
        }
        match reply {
            Reply::Shelves(generation, result) if generation == self.shelves_generation => {
                self.shelves = match result {
                    Ok(shelves) => Load::Ready(shelves),
                    // Offline, the storefront last seen stays up.
                    Err(_) if matches!(self.shelves, Load::Ready(_)) => return,
                    Err(error) => Load::Failed(error),
                };
            }
            Reply::Catalog(reply) => {
                if let Some(error) = reply.error() {
                    crate::error_log::shared(cx)
                        .borrow_mut()
                        .push("Store", &error.to_string());
                }
                self.catalog.receive(reply);
            }
            Reply::Detail(generation, result) if generation == self.generation => {
                if let Some((_, load)) = &mut self.detail {
                    *load = match *result {
                        Ok(detail) => Load::Ready(detail),
                        Err(error) => Load::Failed(error),
                    };
                }
            }
            Reply::Library(mut items, yours, note, checked) => {
                // A local listing keeps what the Store last said about each
                // install, until it is asked again.
                if !checked && let Load::Ready(known) = &self.library {
                    for item in &mut items {
                        if let Some(was) = known
                            .iter()
                            .find(|k| k.id == item.id && k.release == item.release)
                        {
                            item.update = was.update.clone();
                            item.removed = was.removed;
                        }
                    }
                }
                self.library = Load::Ready(items);
                self.yours = yours;
                if note.is_some() && self.page == Page::Library {
                    self.message = note;
                }
            }
            Reply::Prepared(result) => {
                self.busy = None;
                match *result {
                    Ok(prepared) => match crate::publish::plan_problem(&prepared.plan) {
                        Some(problem) => self.message = Some(problem),
                        None => {
                            let draft = Draft::for_plan(&prepared.plan);
                            self.publishing = Some((prepared, None));
                            self.publish_sheet(draft, None, cx);
                        }
                    },
                    Err(message) => self.message = Some(message),
                }
            }
            Reply::Frozen(boxed) => match *boxed {
                (prepared, Ok(frozen)) => {
                    self.publishing = Some((prepared, None));
                    self.confirm_publish(frozen, cx)
                }
                (_, Err(message)) => {
                    self.busy = None;
                    self.publishing = None;
                    self.message = Some(message);
                }
            },
            Reply::Done(message) => {
                self.message = Some(message);
                if self.busy.take().is_some() {
                    self.load_library();
                }
            }
            _ => {}
        }
        cx.notify();
    }
    /// The kind or search page's rows once its request answered.
    #[cfg(test)]
    pub fn rows_for_tests(&self) -> Vec<String> {
        if self.catalog.state != CatalogState::Ready {
            return vec!["(loading)".into()];
        }
        self.catalog_items().into_iter().map(|l| l.slug).collect()
    }
    /// Library › Yours › Publish…: the account and the folder are checked on
    /// a worker, then the publish sheet asks for the version and listing.
    pub fn publish(&mut self, id: String, window: &Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.publishing.is_some() {
            return;
        }
        self.owner = crate::platform::ns_window(window);
        self.busy = Some(id.clone());
        self.message = None;
        let (account, profile) = (self.services.account.clone(), self.services.profile.clone());
        let healthy = crate::publish::healthy(self.services.kernel.as_ref(), &id);
        self.run(move |_| {
            let mut http = account();
            Reply::Prepared(Box::new(crate::publish::prepare(
                &mut http, &profile, &id, healthy,
            )))
        });
        cx.notify();
    }
    fn publish_sheet(&mut self, draft: Draft, problem: Option<String>, cx: &mut Context<Self>) {
        use crate::dialogs::{Button, DialogId, Field, NativePiece, Surface, WindowId};
        let (Some(owner), Some((prepared, sheet))) = (self.owner.clone(), self.publishing.as_mut())
        else {
            return;
        };
        if let Some(old) = sheet.take() {
            old.close();
        }
        let plan = &prepared.plan;
        let text = |id: &str, label: &str, value: &str| {
            Field::new(id, label, NativePiece::TextField, vec![value.into()])
        };
        let popup = |id: &str, label: &str, options: &[(&str, &str)], value: &str| Field {
            selected: options.iter().position(|(v, _)| *v == value).unwrap_or(0),
            ..Field::new(
                id,
                label,
                NativePiece::PopUpButton,
                options.iter().map(|(_, l)| (*l).into()).collect(),
            )
        };
        let mut fields = vec![
            text("version", "Version", &draft.version),
            text("notes", "Release Notes", &draft.notes),
        ];
        if plan.first_publish {
            fields.push(text("name", "Name", &draft.name));
            fields.push(text("tagline", "Tagline", &draft.tagline));
            fields.push(popup("category", "Kind", &CATEGORIES, &draft.category));
            fields.push(popup(
                "visibility",
                "Visibility",
                &VISIBILITY,
                &draft.visibility,
            ));
        }
        let mut surface = Surface::sheet(
            DialogId("publish".into()),
            WindowId("store".into()),
            &if plan.first_publish {
                format!("Publish {}", plan.coordinate)
            } else {
                format!("Publish an Update to {}", plan.coordinate)
            },
            fields,
            "Publish…",
        );
        surface.line = Some(match &plan.last_version {
            Some(last) => {
                format!("The Store has {last}. People who installed it are offered this version.")
            }
            None => "It appears on the Store under your handle. Everyone can install it.".into(),
        });
        surface.buttons = vec![Button::cancel(false), Button::action("Publish…")];
        surface.warning = problem;
        let open = crate::sheets::OpenSheet::present(
            crate::sheets::from_surface(&surface, &self.tokens),
            &owner,
            Some("version"),
        );
        let actions = open.actions.clone();
        *sheet = Some(open);
        cx.spawn(async move |this, cx| {
            // Buttons only; a field also sends its id when editing ends.
            while let Ok(action) = actions.recv().await {
                if action == "apply" || action == "cancel" {
                    let _ = this.update(cx, |v, cx| v.publish_action(&action, cx));
                }
            }
        })
        .detach();
    }
    fn publish_action(&mut self, action: &str, cx: &mut Context<Self>) {
        let Some((prepared, Some(sheet))) = self.publishing.as_mut() else {
            return;
        };
        if action == "cancel" {
            sheet.close();
            self.publishing = None;
            cx.notify();
            return;
        }
        let plan = &prepared.plan;
        let mut draft = Draft::for_plan(plan);
        draft.version = sheet.text("version");
        draft.notes = sheet.text("notes");
        if plan.first_publish {
            draft.name = sheet.text("name");
            draft.tagline = sheet.text("tagline");
            draft.category = CATEGORIES[sheet
                .popup("category")
                .unwrap_or(0)
                .min(CATEGORIES.len() - 1)]
            .0
            .into();
            draft.visibility = VISIBILITY[sheet.popup("visibility").unwrap_or(0).min(1)]
                .0
                .into();
        }
        crate::publish::draft_reasons(prepared, &mut draft);
        match draft.form(plan) {
            Err(problems) => {
                let text = problem_text(&problems[0], plan);
                self.publish_sheet(draft, Some(text), cx);
            }
            Ok(form) => {
                sheet.close();
                let (Some((prepared, _)), now) = (self.publishing.take(), now_seconds()) else {
                    return;
                };
                self.busy = Some(prepared.plan.local_id.clone());
                self.run(move |_| {
                    let frozen = crate::publish::freeze(&prepared, &form, now);
                    Reply::Frozen(Box::new((prepared, frozen)))
                });
            }
        }
        cx.notify();
    }
    /// v1's confirmation names the frozen snapshot; Publish uploads exactly it.
    fn confirm_publish(&mut self, frozen: crate::publish::Frozen, cx: &mut Context<Self>) {
        use crate::alert::{Alert, Button};
        let Some(owner) = self.owner.clone() else {
            return;
        };
        let (message, detail) = frozen.confirmation();
        let rx = Alert::new(
            message,
            detail,
            vec![
                Button::new("Publish", true).default(),
                Button::new("Cancel", false).cancel(),
            ],
        )
        .sheet(&owner);
        cx.spawn(async move |this, cx| {
            let publish = rx.recv().await == Ok(Some(true));
            let _ = this.update(cx, |v, cx| v.upload(publish.then_some(frozen), cx));
        })
        .detach();
    }
    /// The confirmed upload (or its cancellation).
    pub fn upload(&mut self, frozen: Option<crate::publish::Frozen>, cx: &mut Context<Self>) {
        let Some((prepared, _)) = self.publishing.take() else {
            return;
        };
        let Some(frozen) = frozen else {
            self.busy = None;
            cx.notify();
            return;
        };
        let (account, profile) = (self.services.account.clone(), self.services.profile.clone());
        self.run(move |_| {
            let mut http = account();
            Reply::Done(
                crate::publish::upload(&mut http, &profile, &prepared, frozen)
                    .unwrap_or_else(|message| message),
            )
        });
        cx.notify();
    }
    fn catalog_items(&self) -> Vec<Listing> {
        self.catalog
            .items
            .iter()
            .filter_map(Listing::from_value)
            .collect()
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "f" && event.keystroke.modifiers.platform {
            if let Some(field) = &self.search {
                field.update(cx, |f, cx| f.begin_editing(cx));
            }
            return;
        }
        if event.keystroke.key == "escape" {
            // Esc steps back from a listing or a kind's page; on Discover or
            // Library the Store, a page of the window, closes back to the
            // project.
            if self.detail.is_none() && matches!(self.page, Page::Kind(_)) {
                self.set_page(Page::Discover, cx);
            } else if self.detail.is_none() && self.embedded {
                pm_shell::send_menu_command("closeTab");
            } else {
                self.back(cx);
            }
        }
    }
}

const CATEGORIES: [(&str, &str); 7] = [
    ("effects", "Effect"),
    ("transitions", "Transition"),
    ("panels", "Panel"),
    ("themes", "Theme"),
    ("commands", "Commands"),
    ("layers", "Layer"),
    ("tools", "Tool"),
];
const VISIBILITY: [(&str, &str); 2] = [("public", "Public"), ("unlisted", "Unlisted")];
fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
/// An install or uninstall failure as the sentence the Store gave for it.
fn refusal(error: pm_runtime_v8::Error) -> String {
    match error {
        pm_runtime_v8::Error::Host(message) => message,
        _ => "Powermove couldn’t change this extension. Try again.".into(),
    }
}
/// A publish form's first problem, as one sentence.
fn problem_text(
    problem: &pm_services::surfaces::publishing::FieldProblem,
    plan: &pm_services::surfaces::publishing::Plan,
) -> String {
    match problem.problem {
        Problem::VersionRequired => "Enter a version, such as 1.0.0.".into(),
        Problem::VersionInvalid => "Use a version like 1.2.3.".into(),
        Problem::VersionNotHigher => format!(
            "Use a version higher than {}.",
            plan.last_version.as_deref().unwrap_or("the last one")
        ),
        Problem::NotesTooLong => "Release notes can be up to 4,000 characters.".into(),
        Problem::NameRequired => "Add a name for the Store.".into(),
        Problem::NameTooLong => "Names can be up to 80 characters.".into(),
        Problem::TaglineTooLong => "Taglines can be up to 160 characters.".into(),
        Problem::WaiverTooShort | Problem::WaiverTooLong => format!(
            "{} looks like a random key. Remove it, or add `// powermove-secret-ok: <why>` beside it.",
            problem.field
        ),
        _ => "This extension can’t be published as it is.".into(),
    }
}
const GENERIC: &str = "Something went wrong. Try again.";
const SIGNING: &str =
    "Store installs need a signed release, and the Store doesn't sign releases yet.";
/// A failed read, as one sentence for the person who asked (v1.2.0 `actionErrorText`).
fn load_error(error: pm_services::ServiceError) -> String {
    match error.code {
        ServiceErrorCode::Offline => "Can’t reach the Store. Check your connection and try again.",
        ServiceErrorCode::NotFound => "This extension isn’t on the Store.",
        _ => GENERIC,
    }
    .into()
}
fn uuid_like(s: &str) -> bool {
    s.len() == 36 && s.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
}
/// Extensions made on this Mac (no Store marker), with what was published.
pub fn yours(profile: &std::path::Path) -> Vec<Yours> {
    let provenance: Value = std::fs::read(profile.join("extensions-provenance.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    discovery::scan(&[profile.join("extensions")])
        .into_iter()
        .filter(|found| found.store.is_none())
        .map(|found| {
            let manifest = found.manifest.as_ref().ok();
            let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
            Yours {
                name: text(manifest.and_then(|m| m.get("name"))).unwrap_or(found.id.clone()),
                version: text(manifest.and_then(|m| m.get("version"))).unwrap_or_default(),
                published: text(provenance[&found.id]["published"].get("version")),
                id: found.id,
            }
        })
        .collect()
}
/// The Store installs in this profile's extensions folder.
pub fn scan(profile: &std::path::Path) -> Vec<Installed> {
    discovery::scan(&[profile.join("extensions")])
        .into_iter()
        .filter_map(|found| {
            let marker = found.store.clone()?;
            let manifest = found.manifest.as_ref().ok();
            let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
            Some(Installed {
                name: text(manifest.and_then(|m| m.get("name"))).unwrap_or(found.id.clone()),
                version: text(manifest.and_then(|m| m.get("version"))).unwrap_or_default(),
                repo: text(marker.get("repoId")),
                release: text(marker.get("releaseId")),
                id: found.id,
                update: None,
                removed: false,
            })
        })
        .collect()
}
fn today() -> (i32, u32, u32) {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    pm_services::surfaces::store::civil_from_days(days)
}

impl StoreWindow {
    fn button(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        enabled: bool,
        primary: bool,
    ) -> Stateful<Div> {
        let id: SharedString = id.into();
        self.kit()
            .button(
                id.clone(),
                label.to_owned(),
                if primary {
                    Button::Primary
                } else {
                    Button::Neutral
                },
                None,
                None,
            )
            .debug_selector(move || id.to_string())
            .when(!enabled, |d| self.kit().disabled(d))
    }
    fn loading(&self) -> Div {
        self.kit().progress(None)
    }
    fn load_failed(&self, cx: &mut Context<Self>) -> Div {
        self.kit()
            .empty_state(
                "wifi.exclamationmark",
                "Can’t Reach the Store",
                "Check your internet connection and try again.",
            )
            .child(
                self.kit()
                    .button("store-retry", "Try Again", Button::Neutral, None, None)
                    .debug_selector(|| "store-retry".into())
                    .on_click(cx.listener(|v, _, _, cx| {
                        if let Some((listing, _)) = v.detail.clone() {
                            v.open_detail(listing, cx);
                        } else if v.page == Page::Library {
                            v.load_library();
                        } else if v.page == Page::Discover && v.query.trim().is_empty() {
                            v.load_shelves();
                        } else {
                            v.load_catalog();
                        }
                        cx.notify();
                    })),
            )
    }
    /// v1.2.0's card (`st-item`): the art on a tile, then three tiers of
    /// text (the name, one line of what it is, one quiet line of facts) and
    /// one action at the trailing edge. The caller makes the card a target.
    fn frame(
        &self,
        id: &str,
        width: f32,
        face: Face,
        facts: Div,
        action: AnyElement,
    ) -> Stateful<Div> {
        let t = &self.tokens;
        let c = &t.colors;
        let Face { seed, name, line } = face;
        let (from, to) = art(seed);
        let card = div()
            .id(SharedString::from(id.to_owned()))
            .flex()
            .flex_none()
            .w(px(width))
            .items_center()
            .gap(px(t.space.lg))
            .p(px(CARD_PAD))
            .corner(t, t.radius.lg)
            .bg(rgb(c.panel));
        self.kit()
            .wash(card, kit::key(id), t.radius.lg)
            .child(
                div()
                    .flex_none()
                    .size(px(TILE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .corner(t, t.radius.md + t.space.hair)
                    .bg(rgb(c.floating))
                    .child(
                        div()
                            .size(px(ART))
                            .corner(t, t.radius.md)
                            .bg(linear_gradient(
                                135.,
                                linear_color_stop(rgb(from), 0.),
                                linear_color_stop(rgb(to), 1.),
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_role(t, Text::Title)
                            .child(name.to_owned()),
                    )
                    .children(line.map(|(text, ink)| {
                        div()
                            .truncate()
                            .text_role(t, Text::Body)
                            .text_color(rgb(ink))
                            .child(text)
                    }))
                    .child(
                        facts
                            .flex()
                            .items_center()
                            .gap(px(t.space.xs))
                            .min_w(px(0.))
                            .pt(px(t.space.hair))
                            .text_role(t, Text::Body)
                            .text_color(rgb(c.faint)),
                    ),
            )
            .child(action)
    }
    /// A listing's card: what it does, who made it, and a check when it is
    /// here or a plus that opens its page (where its access is shown before
    /// it is installed). `place` names the shelf it is on: one listing can be
    /// on several, and each card keeps its own hover.
    fn card(
        &self,
        place: &str,
        listing: &Listing,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let slug = listing.slug.clone();
        let item = listing.clone();
        let open = item.clone();
        let symbol = |name: &str, color: u32| {
            pm_shell::symbol::symbol(name, m.symbol_inline, Weight::Semibold, color, window)
        };
        let action = if self.installed(listing).is_some() {
            div()
                .id(SharedString::from(format!(
                    "store-{place}-{}-action",
                    listing.repo_id
                )))
                .role(Role::Image)
                .aria_label("Installed")
                .flex_none()
                .size(px(m.hit))
                .flex()
                .items_center()
                .justify_center()
                .child(symbol("checkmark", c.muted))
                .into_any_element()
        } else {
            self.kit()
                .icon_button(
                    format!("store-{place}-{}-action", listing.repo_id),
                    "plus",
                    format!("Get: {}", listing.name),
                    false,
                )
                .on_click(cx.listener(move |v, _, _, cx| {
                    cx.stop_propagation();
                    v.open_detail(open.clone(), cx)
                }))
                .into_any_element()
        };
        let facts = div()
            .child(div().truncate().child(format!("by {}", listing.handle)))
            .when(listing.verified, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!(
                            "store-{place}-{}-verified",
                            listing.repo_id
                        )))
                        .flex_none()
                        .role(Role::Image)
                        .aria_label("Verified")
                        .child(symbol("checkmark.seal.fill", c.accent_text)),
                )
            });
        // Keyed by repository: two makers' listings may share a slug.
        self.frame(
            &format!("store-{place}-{}", listing.repo_id),
            width,
            Face {
                seed: &listing.repo_id,
                name: &listing.name,
                line: (!listing.tagline.is_empty()).then(|| (listing.tagline.clone(), c.muted)),
            },
            facts,
            action,
        )
        .debug_selector(move || format!("store-row-{slug}"))
        .on_click(cx.listener(move |v, _, _, cx| v.open_detail(item.clone(), cx)))
        .into_any_element()
    }
    /// How wide a card is and how many share a row: two (one when the
    /// column is narrow), as v1.2.0's grid.
    fn card_width(&self, window: &Window) -> (f32, usize) {
        let t = &self.tokens;
        let column = (f32::from(window.viewport_size().width) - 2. * t.space.xxl)
            .min(t.metrics.store_column);
        let across = if column < NARROW { 1 } else { 2 };
        let width = ((column - t.space.md * (across - 1) as f32) / across as f32).floor();
        (width, across)
    }
    fn grid(&self, cards: Vec<AnyElement>, across: usize) -> AnyElement {
        let gap = self.tokens.space.md;
        let mut rows = div().flex().flex_col().gap(px(gap));
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            rows = rows.child(
                div()
                    .flex()
                    .gap(px(gap))
                    .children(cards.by_ref().take(across)),
            );
        }
        rows.into_any_element()
    }
    fn cards(
        &self,
        place: &str,
        listings: &[Listing],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (width, across) = self.card_width(window);
        let cards = listings
            .iter()
            .map(|listing| self.card(place, listing, width, window, cx))
            .collect();
        self.grid(cards, across)
    }
    fn status(&self, text: &str) -> Div {
        div()
            .px(px(CARD_PAD))
            .pb(px(self.tokens.space.xl))
            .text_color(rgb(self.tokens.colors.secondary))
            .child(text.to_string())
    }
    /// A section's head: its name on the cards' text edge, and one quiet
    /// action at the far edge.
    fn heading(&self, text: &str, trailing: Option<AnyElement>) -> Div {
        let t = &self.tokens;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.md))
            .h(px(t.metrics.row))
            .mb(px(t.space.md))
            .pl(px(CARD_PAD))
            .child(
                div()
                    .id(SharedString::from(format!("store-head-{text}")))
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .role(Role::Heading)
                    .text_role(t, Text::Heading)
                    .child(text.to_string()),
            )
            .children(trailing)
    }
    /// A quiet text action at a section head's far edge.
    fn head_action(&self, id: &str, label: &str) -> Stateful<Div> {
        let id = id.to_owned();
        self.kit()
            .button(id.clone(), label.to_owned(), Button::Quiet, None, None)
            .debug_selector(move || id.clone())
    }
    /// A section: its head, then what it holds, then the room before the next.
    fn section(&self, head: Div, body: impl IntoElement) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .pb(px(self.tokens.space.section))
            .child(head)
            .child(body)
            .into_any_element()
    }
    fn list(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let searching = !self.query.trim().is_empty();
        match self.page {
            Page::Discover if !searching => match &self.shelves {
                Load::Loading => out.push(self.loading().into_any_element()),
                Load::Failed(_) => out.push(self.load_failed(cx).into_any_element()),
                Load::Ready(shelves) if shelves.is_empty() => out.push(
                    self.kit()
                        .empty_state(
                            "shippingbox",
                            "Nothing Here Yet",
                            "Extensions people publish show up here. Make one with your agent \
                             and publish it from your Library.",
                        )
                        .into_any_element(),
                ),
                Load::Ready(shelves) => {
                    for shelf in shelves.clone() {
                        let kind = KINDS.iter().position(|(id, _)| *id == shelf.id);
                        let title = if shelf.id == "featured" {
                            "Featured"
                        } else {
                            shelf.title.as_str()
                        };
                        let all = kind.map(|i| {
                            self.head_action(&format!("store-show-all-{}", shelf.id), "Show All")
                                .on_click(
                                    cx.listener(move |v, _, _, cx| v.set_page(Page::Kind(i), cx)),
                                )
                                .into_any_element()
                        });
                        let cards = self.cards(&shelf.id, &shelf.listings, window, cx);
                        out.push(self.section(self.heading(title, all), cards));
                    }
                }
            },
            Page::Library => out.extend(self.library_rows(window, cx)),
            _ => {
                let items = self.catalog_items();
                let title = if searching {
                    format!("Results for “{}”", self.query.trim())
                } else {
                    self.page.title().to_owned()
                };
                let mut body = div().flex().flex_col().gap(px(self.tokens.space.xl));
                if !items.is_empty() {
                    body = body.child(self.cards("results", &items, window, cx));
                }
                match self.catalog.state {
                    CatalogState::Loading => body = body.child(self.loading()),
                    CatalogState::Failed(_) => body = body.child(self.load_failed(cx)),
                    CatalogState::Ready if items.is_empty() => {
                        body = body.child(if searching {
                            self.kit().empty_state(
                                "magnifyingglass",
                                &format!("No Results for “{}”", self.query.trim()),
                                "Try a shorter name, a maker’s handle, or a kind such as \
                                 “transition”.",
                            )
                        } else {
                            self.kit().empty_state(
                                "shippingbox",
                                "Nothing Here Yet",
                                &format!(
                                    "No one has published {} yet.",
                                    self.page.title().to_lowercase()
                                ),
                            )
                        })
                    }
                    _ => {}
                }
                if self.catalog.next_cursor.is_some()
                    && !matches!(self.catalog.state, CatalogState::Loading)
                {
                    body = body.child(
                        div().flex().justify_center().child(
                            self.button("store-more", "Show More", true, false)
                                .on_click(cx.listener(|v, _, _, cx| v.more(cx))),
                        ),
                    );
                }
                out.push(self.section(self.heading(&title, None), body));
            }
        }
        out
    }
    /// Home's header row (the same window): the title, the search pill in
    /// the middle, Discover · Library at the far edge. On a kind's page the
    /// title is the way back, as v1.2.0's.
    fn header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = &self.tokens;
        let (c, m, tr) = (&t.colors, &t.metrics, &t.translucency);
        let kit = self.kit();
        let back = matches!(self.page, Page::Kind(_)) && self.query.trim().is_empty();
        let title = if back {
            div()
                .id("store-title-back")
                .debug_selector(|| "store-title-back".into())
                .flex()
                .flex_none()
                .items_center()
                .gap(px(t.space.xs))
                .role(Role::Button)
                .aria_label("Back to Discover")
                .child(kit.symbol(
                    "chevron.left",
                    m.symbol_toolbar + 1.,
                    Weight::Semibold,
                    c.muted,
                ))
                .child(div().text_role(t, Text::Headline).child("Discover"))
                .on_click(cx.listener(|v, _, _, cx| v.set_page(Page::Discover, cx)))
        } else {
            div()
                .id("store-heading")
                .flex_none()
                .role(Role::Heading)
                .text_role(t, Text::Headline)
                .child("Store")
        };
        let width = f32::from(window.viewport_size().width);
        let search_key = kit::key("store-search");
        let search = kit.track_hover(
            div()
                .id("store-search")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(t.space.md))
                .w(px(SEARCH_MAX.min(width - SEARCH_ROOM).max(SEARCH_LEAST)))
                .h(px(ACTION))
                .px(px(t.space.md + t.space.hair))
                .rounded(px(t.radius.md))
                .bg(ink(
                    c.text,
                    tr.hover + (tr.selected - tr.hover) * kit.hovered(search_key),
                ))
                .child(kit.symbol(
                    "magnifyingglass",
                    m.symbol_toolbar + 1.,
                    Weight::Regular,
                    c.muted,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .children(self.search.clone())
                        .when(self.search.is_none(), |d| {
                            d.text_color(rgb(c.faint)).child("Search")
                        }),
                )
                .when(self.query.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_role(t, Text::Caption)
                            .text_color(rgb(c.faint))
                            .child("⌘F"),
                    )
                })
                .on_click(cx.listener(|v, _, _, cx| {
                    if let Some(field) = &v.search {
                        field.update(cx, |f, cx| f.begin_editing(cx));
                    }
                })),
            search_key,
        );
        let side = || div().flex().flex_1().min_w(px(0.)).items_center();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.xl))
            .h(px(HEADER))
            .mb(px(t.space.lg))
            .child(side().child(title))
            .child(search)
            .child(side().justify_end().children(self.pages.clone()))
    }
    /// Library (S077): what needs you, what the Store installed here and
    /// what was made here, each a section of cards.
    fn library_rows(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = &self.tokens;
        let c = &t.colors;
        let items = match &self.library {
            Load::Ready(items) => items.clone(),
            Load::Loading => return vec![self.loading().into_any_element()],
            Load::Failed(_) => return vec![self.load_failed(cx).into_any_element()],
        };
        let (width, across) = self.card_width(window);
        let mut out = Vec::new();
        let (attention, calm): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|i| i.removed || i.update.is_some());
        let installed_card = |item: Installed, cx: &mut Context<Self>| {
            let (line, ink) = if item.removed {
                ("No longer on the Store".to_owned(), c.warning)
            } else if let Some((version, _)) = &item.update {
                (format!("Update to {version} available"), c.warning)
            } else {
                (format!("Version {}", item.version), c.muted)
            };
            let id = item.id.clone();
            let action = match (&item.update, &item.repo, item.removed) {
                (Some((version, release)), Some(repo), false) => {
                    let pick = Pick {
                        release: release.clone(),
                        repo: repo.clone(),
                        extension: item.id.clone(),
                        version: version.clone(),
                        name: item.name.clone(),
                    };
                    let busy = self.busy.as_deref() == Some(repo.as_str());
                    self.button(
                        format!("store-update-{id}"),
                        if busy { "Updating…" } else { "Update" },
                        self.busy.is_none() && self.services.source.is_some(),
                        true,
                    )
                    .on_click(cx.listener(move |v, _, _, cx| v.install(pick.clone(), cx)))
                    .into_any_element()
                }
                _ => div().into_any_element(),
            };
            let facts = div().child(div().truncate().child(if item.update.is_some() {
                format!("Version {} installed", item.version)
            } else {
                "From the Store".to_owned()
            }));
            self.frame(
                &format!("store-installed-{id}"),
                width,
                Face {
                    seed: item.repo.as_deref().unwrap_or(&item.id),
                    name: &item.name,
                    line: Some((line, ink)),
                },
                facts,
                action,
            )
            .debug_selector(move || format!("store-installed-{id}"))
            .into_any_element()
        };
        // Check for Updates heads the first Store section.
        let mut check = (!attention.is_empty() || !calm.is_empty()).then(|| {
            self.head_action("store-check-updates", "Check for Updates")
                .when(self.busy.is_some(), |d| self.kit().disabled(d))
                .on_click(cx.listener(|v, _, _, cx| {
                    if v.busy.is_none() {
                        v.message = None;
                        v.load_library();
                        cx.notify();
                    }
                }))
                .into_any_element()
        });
        let needs_you = !attention.is_empty();
        if needs_you {
            let cards = attention
                .into_iter()
                .map(|i| installed_card(i, cx))
                .collect();
            let grid = self.grid(cards, across);
            out.push(self.section(self.heading("Needs Attention", check.take()), grid));
        }
        if !calm.is_empty() {
            let cards = calm.into_iter().map(|i| installed_card(i, cx)).collect();
            let grid = self.grid(cards, across);
            out.push(self.section(self.heading("Installed", check.take()), grid));
        } else if !needs_you {
            let empty = self
                .kit()
                .empty_state(
                    "shippingbox",
                    "Nothing Installed Yet",
                    "Extensions you install from the Store live here, and update when their \
                     makers publish.",
                )
                .child(
                    self.button("store-discover", "Discover Extensions", true, false)
                        .on_click(cx.listener(|v, _, _, cx| v.set_page(Page::Discover, cx))),
                );
            out.push(self.section(self.heading("Installed", None), empty));
        }
        // Made on this Mac: published from here (v1.2.0's Library › Made by You).
        let made = if self.yours.is_empty() {
            self.kit()
                .empty_state(
                    "hammer",
                    "Nothing Made Yet",
                    "Ask your agent for an effect, a panel or a theme. What it makes shows up \
                     here, ready to publish.",
                )
                .into_any_element()
        } else {
            let cards = self
                .yours
                .clone()
                .into_iter()
                .map(|item| {
                    let (status, label) = match &item.published {
                        Some(published) if *published == item.version => {
                            (format!("Published {published}"), "Publish Update…")
                        }
                        Some(published) => (
                            format!("Published {published} · changed since"),
                            "Publish Update…",
                        ),
                        None => ("Not on the Store".to_owned(), "Publish…"),
                    };
                    let id = item.id.clone();
                    let busy = self.busy.as_deref() == Some(id.as_str());
                    let publish = {
                        let id = id.clone();
                        self.button(
                            format!("store-publish-{id}"),
                            if busy { "Publishing…" } else { label },
                            self.busy.is_none() && self.publishing.is_none(),
                            false,
                        )
                        .on_click(
                            cx.listener(move |v, _, window, cx| v.publish(id.clone(), window, cx)),
                        )
                        .into_any_element()
                    };
                    self.frame(
                        &format!("store-yours-{id}"),
                        width,
                        Face {
                            seed: &format!("local:{id}"),
                            name: &item.name,
                            line: Some((format!("Version {}", item.version), c.muted)),
                        },
                        div().child(div().truncate().child(status)),
                        publish,
                    )
                    .into_any_element()
                })
                .collect();
            self.grid(cards, across)
        };
        out.push(self.section(self.heading("Made by You", None), made));
        out
    }
    /// An extension's page (S076): its head (art, name, what it does, who
    /// made it, and the action at the trailing edge), then About and a card
    /// of facts and one of versions, each under a small label as v1.2.0's.
    fn detail_view(
        &self,
        listing: &Listing,
        load: &Load<Detail>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let back = div()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .mb(px(t.space.xl))
            .child(
                self.kit()
                    .icon_button("store-back", "chevron.left", "Back", false)
                    .debug_selector(|| "store-back".into())
                    .on_click(cx.listener(|v, _, _, cx| v.back(cx))),
            )
            .child(
                div()
                    .text_role(t, Text::Label)
                    .text_color(rgb(c.muted))
                    .child(self.title_for_back()),
            );
        let (from, to) = art(&listing.repo_id);
        let seal = listing.verified.then(|| {
            div()
                .id("store-detail-verified")
                .flex_none()
                .role(Role::Image)
                .aria_label("Verified")
                .child(pm_shell::symbol::symbol(
                    "checkmark.seal.fill",
                    m.symbol_inline,
                    Weight::Semibold,
                    c.accent_text,
                    window,
                ))
        });
        let copy = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(px(t.space.hair))
            .child(
                div()
                    .text_role(t, Text::Display)
                    .child(listing.name.clone()),
            )
            .when(!listing.tagline.is_empty(), |d| {
                d.child(
                    div()
                        .text_role(t, Text::Reading)
                        .text_color(rgb(c.secondary))
                        .child(listing.tagline.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.xs))
                    .text_color(rgb(c.muted))
                    .child(format!("by {}", listing.handle))
                    .children(seal),
            );
        let mut head = div()
            .flex()
            .items_center()
            .gap(px(t.space.xl))
            .pb(px(t.space.xxl))
            .child(
                div()
                    .flex_none()
                    .size(px(HERO))
                    .corner(t, t.radius.xl + t.space.xs)
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(rgb(from), 0.),
                        linear_color_stop(rgb(to), 1.),
                    )),
            )
            .child(copy);
        let detail = match load {
            Load::Loading => {
                return vec![
                    back.into_any_element(),
                    head.into_any_element(),
                    self.loading().into_any_element(),
                ];
            }
            Load::Failed(_) => {
                return vec![
                    back.into_any_element(),
                    head.into_any_element(),
                    self.load_failed(cx).into_any_element(),
                ];
            }
            Load::Ready(detail) => detail,
        };
        // The one action, then Uninstall for an install.
        let action = self.action(detail);
        let installed = self.installed(listing).cloned();
        let can_act = match action {
            DetailAction::Install | DetailAction::InstallAndSetUp | DetailAction::Update => {
                self.services.source.is_some() && self.busy.is_none()
            }
            _ => false,
        };
        let primary = matches!(
            action,
            DetailAction::Install | DetailAction::InstallAndSetUp | DetailAction::Update
        );
        // (extension, version, release) the page offers.
        let offer = match action {
            DetailAction::Update => installed
                .as_ref()
                .and_then(|i| Some((i.id.clone(), i.update.clone()?))),
            _ => listing
                .latest
                .as_ref()
                .map(|l| (listing.slug.clone(), (l.1.clone(), l.0.clone()))),
        };
        let mut button = self.button("store-action", action.label(), can_act, primary);
        if can_act && let Some((extension, (version, release))) = offer {
            let pick = Pick {
                release,
                repo: listing.repo_id.clone(),
                extension,
                version,
                name: listing.name.clone(),
            };
            button = button.on_click(cx.listener(move |v, _, _, cx| v.install(pick.clone(), cx)));
        }
        let mut actions = div()
            .flex()
            .flex_none()
            .flex_col()
            .items_end()
            .gap(px(t.space.md))
            .child(button);
        if let Some(item) = &installed {
            let (id, name, repo) = (item.id.clone(), item.name.clone(), item.repo.clone());
            actions = actions.child(
                self.button("store-uninstall", "Uninstall…", self.busy.is_none(), false)
                    .on_click(cx.listener(move |v, _, window, cx| {
                        v.confirm_uninstall(id.clone(), name.clone(), repo.clone(), window, cx)
                    })),
            );
        }
        head = head.child(actions);
        let mut out = vec![back.into_any_element(), head.into_any_element()];
        out.extend(self.message_line().map(IntoElement::into_any_element));
        if primary && self.services.source.is_none() {
            out.push(
                self.status(SIGNING)
                    .debug_selector(|| "store-signing".into())
                    .into_any_element(),
            );
        }
        // A small label over its card or text, as a Settings page.
        let block = |title: &str, body: AnyElement| {
            div()
                .flex()
                .flex_col()
                .pb(px(t.space.xxl))
                .child(
                    div()
                        .px(px(CARD_PAD))
                        .pb(px(t.space.md))
                        .text_role(t, Text::Label)
                        .text_color(rgb(c.muted))
                        .child(title.to_string()),
                )
                .child(body)
                .into_any_element()
        };
        let card = || {
            div()
                .flex()
                .flex_col()
                .corner(t, t.radius.lg)
                .bg(rgb(c.panel))
        };
        let fact = |key: &str, value: AnyElement| {
            div()
                .flex()
                .gap(px(t.space.xl))
                .px(px(CARD_PAD))
                .py(px(t.space.md + t.space.hair))
                .child(
                    div()
                        .flex_none()
                        .w(px(FACT_KEY))
                        .text_color(rgb(c.secondary))
                        .child(key.to_string()),
                )
                .child(div().flex_1().min_w(px(0.)).child(value))
        };
        let text = |s: String| div().child(s).into_any_element();
        if let Some(about) = &detail.about {
            out.push(block(
                "About",
                div()
                    .px(px(CARD_PAD))
                    .text_role(t, Text::Reading)
                    .child(about.clone())
                    .into_any_element(),
            ));
        }
        let mut facts: Vec<Div> = vec![
            fact("Made by", text(listing.handle.clone())),
            fact("Kind", text(kind_label(&listing.category).to_string())),
        ];
        if let Some(latest) = &listing.latest {
            facts.push(fact(
                "Version",
                text(match &installed {
                    Some(item) if item.version != latest.1 => {
                        format!("{} installed · {} available", item.version, latest.1)
                    }
                    _ => latest.1.clone(),
                }),
            ));
        }
        let uses = permission_lines(&detail.listing.permissions);
        if !uses.is_empty() || !detail.link_hosts.is_empty() {
            let mut lines = div().flex().flex_col().gap(px(t.space.hair));
            for (label, warn) in uses {
                lines = lines.child(
                    div()
                        .text_color(rgb(if warn { c.warning } else { c.text }))
                        .child(label),
                );
            }
            if !detail.link_hosts.is_empty() {
                lines = lines.child(div().child(format!(
                    "Opens {} in your browser without asking",
                    detail.link_hosts.join(", ")
                )));
            }
            facts.push(fact("Access", lines.into_any_element()));
        }
        if !detail.contributes.is_empty() {
            facts.push(fact("Includes", text(includes_text(&detail.contributes))));
        }
        facts.push(fact(
            "Compatibility",
            text(requires_text(listing.latest.as_ref().and_then(|l| l.2)).to_string()),
        ));
        let mut details = card();
        for (i, row) in facts.into_iter().enumerate() {
            if i > 0 {
                details = details.child(self.kit().separator().mx(px(CARD_PAD)));
            }
            details = details.child(row);
        }
        out.push(block("Details", details.into_any_element()));
        let mut versions = card();
        for (i, release) in detail.releases.iter().enumerate() {
            if i > 0 {
                versions = versions.child(self.kit().separator().mx(px(CARD_PAD)));
            }
            let id = release.version.clone();
            let date = relative_date(&release.published_at, today());
            versions = versions.child(
                div()
                    .debug_selector(move || format!("store-version-{id}"))
                    .flex()
                    .flex_col()
                    .px(px(CARD_PAD))
                    .py(px(t.space.md + t.space.hair))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(t.space.md))
                            .when(release.withdrawn, |d| {
                                d.child(div().text_color(rgb(c.warning)).child("Withdrawn"))
                            })
                            .child(
                                div()
                                    .text_role(t, Text::Label)
                                    .text_color(rgb(if release.withdrawn {
                                        c.muted
                                    } else {
                                        c.text
                                    }))
                                    .child(release.version.clone()),
                            )
                            .child(
                                div()
                                    .text_role(t, Text::Caption)
                                    .text_color(rgb(c.muted))
                                    .child(date),
                            ),
                    )
                    .children(release.notes.clone().map(|note| {
                        div()
                            .pt(px(t.space.hair))
                            .text_color(rgb(c.secondary))
                            .child(note)
                    })),
            );
        }
        if !detail.releases.is_empty() {
            out.push(block("Version History", versions.into_any_element()));
        }
        out
    }
    fn message_line(&self) -> Option<Div> {
        let message = self.message.clone()?;
        Some(
            self.status(&message)
                .debug_selector(|| "store-message".into()),
        )
    }
    fn title_for_back(&self) -> &'static str {
        if self.query.trim().is_empty() {
            self.page.title()
        } else {
            "Results"
        }
    }
}

impl Render for StoreWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion.frame(window);
        self.frame_kit = Some(Kit::new(
            &self.tokens,
            &self.motion,
            Self::motion,
            window,
            cx,
        ));
        let t = self.tokens.clone();
        let content = match self.detail.clone() {
            Some((listing, load)) => self.detail_view(&listing, &load, window, cx),
            None => self.list(window, cx),
        };
        // v1.2.0's Store column: a readable width centred however wide the
        // window grows. A list page says what happened under its header; a
        // detail page says it under its action (`detail_view`).
        let mut column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .max_w(px(t.metrics.store_column));
        if self.detail.is_none() {
            column = column
                .child(self.header(window, cx))
                .children(self.message_line());
        }
        let body = div()
            .id("store-body")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            // A window of its own keeps the toolbar's height for its buttons.
            .pt(px(if self.embedded {
                t.space.md
            } else {
                t.metrics.toolbar
            }))
            .pb(px(t.space.section * 2.))
            .px(px(t.space.xxl))
            .child(
                div()
                    .flex()
                    .justify_center()
                    .child(column.children(content)),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(t.colors.window))
            .text_color(rgb(t.colors.text))
            .text_role(&t, Text::Body)
            .font_family(pm_design::TEXT_FONT)
            .child(body)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
    }
}
/// A card's art seed, name and the line under it (text and ink).
struct Face<'a> {
    seed: &'a str,
    name: &'a str,
    line: Option<(String, u32)>,
}
/// Home's header measures, so the two pages of the window line up.
const HEADER: f32 = 48.;
const ACTION: f32 = 32.;
const SEARCH_MAX: f32 = 360.;
const SEARCH_ROOM: f32 = 320.;
const SEARCH_LEAST: f32 = 160.;
/// v1.2.0's card: 14 pt around a 44 pt tile holding 30 pt of art; one card
/// to a row below 640 pt of column.
const CARD_PAD: f32 = 14.;
const TILE: f32 = 44.;
const ART: f32 = 30.;
const NARROW: f32 = 640.;
/// An extension page's art, and the key column of its facts.
const HERO: f32 = 80.;
const FACT_KEY: f32 = 120.;

/// A listing's artwork when it has none: two hues fixed by its repository
/// (v1.2.0's `artFor`), as a diagonal ramp.
pub(crate) fn art(seed: &str) -> (u32, u32) {
    let mut hash: u32 = 0x811c_9dc5;
    for unit in seed.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let first = hash % 360;
    let second = (first + 20 + ((hash >> 9) % 60)) % 360;
    let lightness = 45 + ((hash >> 17) % 16);
    (
        pm_design::color::oklch(lightness as f32 / 100., 0.11, first as f32),
        pm_design::color::oklch((lightness + 22) as f32 / 100., 0.08, second as f32),
    )
}

#[cfg(test)]
impl StoreWindow {
    pub(crate) fn catalog_state_for_tests(&self) -> &CatalogState {
        &self.catalog.state
    }
}
/// Where the storefront last seen is kept (the admitted listings as served).
fn shelves_file(profile: &std::path::Path) -> PathBuf {
    profile.join("caches").join("store-shelves.json")
}
/// Titled shelves of listings, empty shelves dropped.
fn shelves_of<'a>(shelves: impl Iterator<Item = (&'a str, &'a str, &'a Vec<Value>)>) -> Vec<Shelf> {
    shelves
        .map(|(id, title, items)| Shelf {
            id: id.to_owned(),
            title: title.to_owned(),
            listings: items.iter().filter_map(Listing::from_value).collect(),
        })
        .filter(|shelf| !shelf.listings.is_empty())
        .collect()
}
fn keep_shelves(profile: &std::path::Path, shelves: &[pm_services::store::Shelf]) {
    let kept: Vec<Value> = shelves
        .iter()
        .map(|shelf| json!({ "id": shelf.id, "title": shelf.title, "items": shelf.items }))
        .collect();
    let file = shelves_file(profile);
    let partial = file.with_extension("json.partial");
    let written = file
        .parent()
        .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
        && serde_json::to_vec(&kept).is_ok_and(|bytes| std::fs::write(&partial, bytes).is_ok());
    if written {
        let _ = std::fs::rename(&partial, &file);
    }
}
/// The storefront last seen, if there is one worth drawing.
fn cached_shelves(profile: &std::path::Path) -> Option<Vec<Shelf>> {
    let file = shelves_file(profile);
    // The Store's own page limits keep it far below this.
    if std::fs::metadata(&file).ok()?.len() > 4 << 20 {
        return None;
    }
    let kept: Vec<Value> = serde_json::from_slice(&std::fs::read(&file).ok()?).ok()?;
    let shelves = shelves_of(kept.iter().filter_map(|shelf| {
        Some((
            shelf.get("id").and_then(Value::as_str).unwrap_or_default(),
            shelf.get("title")?.as_str()?,
            shelf.get("items")?.as_array()?,
        ))
    }));
    (!shelves.is_empty()).then_some(shelves)
}
