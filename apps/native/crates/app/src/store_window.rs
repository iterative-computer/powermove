//! Window ▸ Store (S075–S078): browse the storefront and its seven kinds,
//! search, read an extension's page with its versions and what it uses, and
//! install, update or uninstall. Library lists what the Store installed on
//! this Mac and which of those have updates (the shipped app's "Needs
//! attention"). Every request runs on a worker; installs go through the same
//! verified path the palette used, so a Store extension runs contained as
//! before, and every window rescans when one lands.
use gpui::{prelude::*, *};
use pm_compat_runner::{StoreAuthority, discovery, store_install};
use pm_design::Tokens;
use pm_services::{
    ServiceErrorCode,
    store::{InstalledRelease, StoreClient, StoreTransport},
    surfaces::store::{
        Catalog, CatalogQuery, CatalogState, Detail, DetailAction, Health, ItemFacts, ItemGroup,
        KINDS, Listing, detail_action, includes_text, kind_label, permission_lines, relative_date,
        requires_text,
    },
};
use serde_json::Value;
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
enum Reply {
    Shelves(u64, Result<Vec<(String, Vec<Listing>)>, String>),
    Catalog(pm_services::surfaces::store::SearchReply),
    Detail(u64, Result<Detail, String>),
    Library(Vec<Installed>, Option<String>),
    Done(String),
}

pub struct StoreWindow {
    services: Services,
    tokens: Tokens,
    pub page: Page,
    pub query: String,
    search: Option<Entity<pm_shell::text::TextField>>,
    pub focus: FocusHandle,
    pub shelves: Load<Vec<(String, Vec<Listing>)>>,
    catalog: Catalog,
    pub detail: Option<(Listing, Load<Detail>)>,
    pub library: Load<Vec<Installed>>,
    /// The extension or repository an install, update or uninstall is running for.
    pub busy: Option<String>,
    pub message: Option<String>,
    generation: u64,
    tx: async_channel::Sender<Reply>,
}

impl StoreWindow {
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
        let mut window = Self {
            services,
            tokens,
            page: Page::Discover,
            query: String::new(),
            search: None,
            focus: cx.focus_handle(),
            shelves: Load::Loading,
            catalog: Catalog::default(),
            detail: None,
            library: Load::Loading,
            busy: None,
            message: None,
            generation: 0,
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
        self.generation += 1;
        let generation = self.generation;
        if !matches!(self.shelves, Load::Ready(_)) {
            self.shelves = Load::Loading;
        }
        self.run(move |client| {
            let result = client
                .and_then(|mut c| c.browse())
                .map_err(load_error)
                .map(|shelves| {
                    shelves
                        .into_iter()
                        .map(|shelf| {
                            let items =
                                shelf.items.iter().filter_map(Listing::from_value).collect();
                            (shelf.title, items)
                        })
                        .filter(|(_, items): &(String, Vec<Listing>)| !items.is_empty())
                        .collect()
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
                Err(error) => Reply::Done(load_error(error)),
            }),
            Err(_) => self.message = Some("Search is limited to 120 characters.".into()),
        }
    }
    fn more(&mut self, cx: &mut Context<Self>) {
        if let Some(work) = self.catalog.more() {
            self.run(move |client| match client {
                Ok(mut client) => Reply::Catalog(work.run(&mut client)),
                Err(error) => Reply::Done(load_error(error)),
            });
            cx.notify();
        }
    }
    /// What the Store installed here, then which of those it has updates for.
    fn load_library(&mut self) {
        let profile = self.services.profile.clone();
        self.run(move |client| {
            let mut installed = scan(&profile);
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
            Reply::Library(installed, note)
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
                .map_err(load_error)
                .and_then(|value| Detail::from_value(&value).ok_or_else(|| GENERIC.to_owned()));
            Reply::Detail(generation, detail)
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
    pub fn install(&mut self, release: String, repo: String, name: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(source) = self.services.source.clone() else {
            self.message = Some(SIGNING.into());
            cx.notify();
            return;
        };
        let updating = self.installed_repo(&repo);
        self.busy = Some(repo.clone());
        self.message = None;
        let (kernel, profile) = (self.services.kernel.clone(), self.services.profile.clone());
        self.run(move |_| {
            match store_install::install_from(
                &source,
                kernel.as_ref(),
                &profile,
                "native",
                &release,
                &repo,
            ) {
                Ok(_) => {
                    pm_compat_runner::host::rescan_all();
                    crate::cloud::remember_install(&profile, &repo, Some(&release));
                    Reply::Done(if updating {
                        format!("{name} is updated.")
                    } else {
                        format!("{name} is installed.")
                    })
                }
                Err(error) => Reply::Done(error.to_string()),
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
            &format!("Uninstall {name}?"),
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
                Err(error) => Reply::Done(error.to_string()),
            },
        );
        cx.notify();
    }
    fn receive(&mut self, reply: Reply, cx: &mut Context<Self>) {
        match reply {
            Reply::Shelves(generation, result) if generation == self.generation => {
                self.shelves = match result {
                    Ok(shelves) => Load::Ready(shelves),
                    Err(error) => Load::Failed(error),
                };
            }
            Reply::Catalog(reply) => {
                self.catalog.receive(reply);
            }
            Reply::Detail(generation, result) if generation == self.generation => {
                if let Some((_, load)) = &mut self.detail {
                    *load = match result {
                        Ok(detail) => Load::Ready(detail),
                        Err(error) => Load::Failed(error),
                    };
                }
            }
            Reply::Library(items, note) => {
                self.library = Load::Ready(items);
                if note.is_some() && self.page == Page::Library {
                    self.message = note;
                }
            }
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
    fn catalog_items(&self) -> Vec<Listing> {
        self.catalog
            .items
            .iter()
            .filter_map(Listing::from_value)
            .collect()
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            self.back(cx);
        }
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

/// Opens the Store, or brings it forward.
pub fn open(services: Services, cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find(|w| w.downcast::<StoreWindow>().is_some())
    {
        let _ = window.update(cx, |_, w, _| w.activate());
        return;
    }
    let t = pm_shell::tokens();
    let (m, tokens) = (t.metrics, t.clone());
    // As the Project Library: full-size content under a transparent title
    // bar, the sidebar on the behind-window blur.
    let background = if t.translucent() {
        gpui::MacosWindowBackground::Transparent
    } else {
        gpui::MacosWindowBackground::Opaque
    };
    let _ = cx.open_window(
        WindowOptions::new()
            .macos_window_background(background)
            .titlebar(Some(TitlebarOptions {
                title: Some("Store".into()),
                appears_transparent: true,
                ..Default::default()
            }))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(m.toolbar), px(m.toolbar)),
                size(
                    px(m.settings_width + m.settings_sidebar),
                    px(m.settings_height),
                ),
            )))),
        |window, cx| {
            if tokens.translucent() {
                pm_shell::window_backdrop(window, &tokens);
            }
            cx.new(|cx| {
                let mut store = StoreWindow::new(services, tokens, cx);
                store.attach_search(window, cx);
                store
            })
        },
    );
}

impl StoreWindow {
    fn button(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        enabled: bool,
        primary: bool,
    ) -> Stateful<Div> {
        let (c, m) = (&self.tokens.colors, &self.tokens.metrics);
        let id: SharedString = id.into();
        div()
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .flex_none()
            .px(px(m.inset))
            .h(px(m.row))
            .flex()
            .items_center()
            .rounded(px(m.control_radius))
            .when(primary && enabled, |d| {
                d.bg(rgb(c.accent)).text_color(rgb(c.selected_text))
            })
            .when(!primary || !enabled, |d| d.bg(rgb(c.panel_secondary)))
            .when(!enabled, |d| d.text_color(rgb(c.muted)))
            .child(label.to_string())
    }
    fn row(&self, listing: &Listing, cx: &mut Context<Self>) -> AnyElement {
        let (c, m, ty) = (
            &self.tokens.colors,
            &self.tokens.metrics,
            self.tokens.typography,
        );
        let slug = listing.slug.clone();
        let mut meta = vec![
            kind_label(&listing.category).to_owned(),
            format!("by {}", listing.handle),
        ];
        if let Some((.., version, _, published)) = &listing.latest {
            meta.push(version.clone());
            let date = relative_date(published, today());
            if !date.is_empty() {
                meta.push(date);
            }
        }
        let item = listing.clone();
        div()
            .id(SharedString::from(format!("store-row-{slug}")))
            .debug_selector(move || format!("store-row-{slug}"))
            .flex()
            .flex_col()
            .flex_none()
            .justify_center()
            .gap(px(m.border))
            .py(px(m.gap * 0.75))
            .px(px(m.gap))
            .rounded(px(m.control_radius))
            .child(
                div()
                    .flex()
                    .gap(px(m.gap))
                    .child(div().truncate().child(listing.name.clone()))
                    .when(self.installed(listing).is_some(), |d| {
                        d.child(div().text_color(rgb(c.muted)).child("Installed"))
                    }),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(ty.minimum))
                    .text_color(rgb(c.secondary))
                    .child(if listing.tagline.is_empty() {
                        meta.join(" · ")
                    } else {
                        format!("{} — {}", meta.join(" · "), listing.tagline)
                    }),
            )
            .on_click(cx.listener(move |v, _, _, cx| v.open_detail(item.clone(), cx)))
            .into_any_element()
    }
    fn status(&self, text: &str) -> Div {
        div()
            .p(px(self.tokens.metrics.gap))
            .text_color(rgb(self.tokens.colors.secondary))
            .child(text.to_string())
    }
    fn heading(&self, text: &str) -> Div {
        let (m, ty) = (&self.tokens.metrics, self.tokens.typography);
        div()
            .pt(px(m.inset))
            .pb(px(m.border * 2.))
            .px(px(m.gap))
            .text_size(px(ty.minimum))
            .font_weight(FontWeight(ty.semibold_weight))
            .text_color(rgb(self.tokens.colors.secondary))
            .child(text.to_string())
    }
    fn list(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let searching = !self.query.trim().is_empty();
        match self.page {
            Page::Discover if !searching => match &self.shelves {
                Load::Loading => out.push(self.status("Loading…").into_any_element()),
                Load::Failed(error) => out.push(self.status(error).into_any_element()),
                Load::Ready(shelves) if shelves.is_empty() => out.push(
                    pm_shell::view::empty_state(
                        &self.tokens,
                        "Nothing Here Yet",
                        "Extensions people publish to the Store appear here.",
                    )
                    .into_any_element(),
                ),
                Load::Ready(shelves) => {
                    // A list shows each extension once, under its first shelf.
                    let mut seen = std::collections::BTreeSet::new();
                    for (title, items) in shelves.clone() {
                        let items: Vec<_> = items
                            .into_iter()
                            .filter(|l| seen.insert(l.repo_id.clone()))
                            .collect();
                        if items.is_empty() {
                            continue;
                        }
                        out.push(self.heading(&title).into_any_element());
                        for listing in &items {
                            out.push(self.row(listing, cx));
                        }
                    }
                }
            },
            Page::Library => out.extend(self.library_rows(cx)),
            _ => {
                let items = self.catalog_items();
                for listing in &items {
                    out.push(self.row(listing, cx));
                }
                match self.catalog.state {
                    CatalogState::Loading => out.push(self.status("Loading…").into_any_element()),
                    CatalogState::Failed(code) => out.push(
                        self.status(&load_error(pm_services::ServiceError::message(code, "")))
                            .into_any_element(),
                    ),
                    CatalogState::Ready if items.is_empty() => out.push(
                        pm_shell::view::empty_state(
                            &self.tokens,
                            "No Results",
                            if searching {
                                "Try a different search."
                            } else {
                                "Nothing of this kind is on the Store yet."
                            },
                        )
                        .into_any_element(),
                    ),
                    _ => {}
                }
                if self.catalog.next_cursor.is_some()
                    && !matches!(self.catalog.state, CatalogState::Loading)
                {
                    out.push(
                        div()
                            .p(px(self.tokens.metrics.gap))
                            .child(
                                self.button("store-more", "Show More", true, false)
                                    .on_click(cx.listener(|v, _, _, cx| v.more(cx))),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        out
    }
    fn library_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (c, m, ty) = (
            &self.tokens.colors,
            &self.tokens.metrics,
            self.tokens.typography,
        );
        let mut out = Vec::new();
        let items = match &self.library {
            Load::Ready(items) => items.clone(),
            Load::Loading => return vec![self.status("Loading…").into_any_element()],
            Load::Failed(error) => return vec![self.status(error).into_any_element()],
        };
        if items.is_empty() {
            return vec![
                pm_shell::view::empty_state(
                    &self.tokens,
                    "No Store Extensions",
                    "Extensions you install from the Store appear here.",
                )
                .into_any_element(),
            ];
        }
        out.push(
            div()
                .flex()
                .px(px(m.gap))
                .pt(px(m.gap))
                .child(
                    self.button(
                        "store-check-updates",
                        "Check for Updates",
                        self.busy.is_none(),
                        false,
                    )
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.message = None;
                        v.load_library();
                        cx.notify();
                    })),
                )
                .into_any_element(),
        );
        let (attention, calm): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|i| i.removed || i.update.is_some());
        for (title, group) in [("Needs Attention", attention), ("Installed", calm)] {
            if group.is_empty() {
                continue;
            }
            out.push(self.heading(title).into_any_element());
            for item in group {
                let status = if item.removed {
                    "No longer on the Store".to_owned()
                } else if let Some((version, _)) = &item.update {
                    format!("Installed {} · Update to {version}", item.version)
                } else {
                    format!("Installed {}", item.version)
                };
                let id = item.id.clone();
                let mut row = div()
                    .id(SharedString::from(format!("store-installed-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("store-installed-{id}")
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(m.inset))
                    .py(px(m.gap * 0.75))
                    .px(px(m.gap))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.))
                            .gap(px(m.border))
                            .child(div().truncate().child(item.name.clone()))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(ty.minimum))
                                    .text_color(rgb(if item.removed {
                                        c.danger
                                    } else {
                                        c.secondary
                                    }))
                                    .child(status),
                            ),
                    );
                if let (Some((_, release)), Some(repo), false) =
                    (&item.update, &item.repo, item.removed)
                {
                    let (release, repo, name) = (release.clone(), repo.clone(), item.name.clone());
                    let busy = self.busy.as_deref() == Some(repo.as_str());
                    row = row.child(
                        self.button(
                            format!("store-update-{id}"),
                            if busy { "Updating…" } else { "Update" },
                            self.busy.is_none() && self.services.source.is_some(),
                            true,
                        )
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.install(release.clone(), repo.clone(), name.clone(), cx)
                        })),
                    );
                }
                out.push(row.into_any_element());
            }
        }
        out
    }
    fn detail_view(
        &self,
        listing: &Listing,
        load: &Load<Detail>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (c, m, ty) = (
            &self.tokens.colors,
            &self.tokens.metrics,
            self.tokens.typography,
        );
        let mut out = vec![
            div()
                .id("store-back")
                .debug_selector(|| "store-back".into())
                .flex_none()
                .px(px(m.gap))
                .pt(px(m.gap))
                .text_color(rgb(c.accent_text))
                .child(format!("‹ {}", self.title_for_back()))
                .on_click(cx.listener(|v, _, _, cx| v.back(cx)))
                .into_any_element(),
            div()
                .px(px(m.gap))
                .pt(px(m.gap))
                .text_size(px(ty.title))
                .font_weight(FontWeight(ty.semibold_weight))
                .child(listing.name.clone())
                .into_any_element(),
            div()
                .px(px(m.gap))
                .text_color(rgb(c.secondary))
                .child(format!(
                    "{} · by {}{}",
                    kind_label(&listing.category),
                    listing.handle,
                    if listing.verified {
                        " · Verified publisher"
                    } else {
                        ""
                    }
                ))
                .into_any_element(),
        ];
        let detail = match load {
            Load::Loading => {
                out.push(self.status("Loading…").into_any_element());
                return out;
            }
            Load::Failed(error) => {
                out.push(self.status(error).into_any_element());
                return out;
            }
            Load::Ready(detail) => detail,
        };
        // The one action, then Uninstall for an install.
        let action = self.action(detail);
        let installed = self.installed(listing).cloned();
        let mut actions = div().flex().gap(px(m.gap)).px(px(m.gap)).pt(px(m.gap));
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
        let release = match action {
            DetailAction::Update => installed
                .as_ref()
                .and_then(|i| i.update.clone())
                .map(|u| u.1),
            _ => listing.latest.as_ref().map(|l| l.0.clone()),
        };
        let mut button = self.button("store-action", action.label(), can_act, primary);
        if can_act && let Some(release) = release {
            let (repo, name) = (listing.repo_id.clone(), listing.name.clone());
            button = button.on_click(cx.listener(move |v, _, _, cx| {
                v.install(release.clone(), repo.clone(), name.clone(), cx)
            }));
        }
        actions = actions.child(button);
        if let Some(item) = installed {
            let (id, name, repo) = (item.id.clone(), item.name.clone(), item.repo.clone());
            actions = actions.child(
                self.button("store-uninstall", "Uninstall…", self.busy.is_none(), false)
                    .on_click(cx.listener(move |v, _, window, cx| {
                        v.confirm_uninstall(id.clone(), name.clone(), repo.clone(), window, cx)
                    })),
            );
        }
        out.push(actions.into_any_element());
        if let Some(message) = self.message_line() {
            out.push(message.into_any_element());
        }
        if primary && self.services.source.is_none() {
            out.push(
                self.status(SIGNING)
                    .debug_selector(|| "store-signing".into())
                    .into_any_element(),
            );
        }
        let block = |title: &str, body: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(m.border * 2.))
                .px(px(m.gap))
                .pt(px(m.inset))
                .child(
                    div()
                        .font_weight(FontWeight(ty.semibold_weight))
                        .child(title.to_string()),
                )
                .child(body)
                .into_any_element()
        };
        if !listing.tagline.is_empty() {
            out.push(
                div()
                    .px(px(m.gap))
                    .pt(px(m.gap))
                    .child(listing.tagline.clone())
                    .into_any_element(),
            );
        }
        if let Some(about) = &detail.about {
            out.push(block(
                "About",
                div().text_color(rgb(c.secondary)).child(about.clone()),
            ));
        }
        let uses = permission_lines(&detail.listing.permissions);
        if !uses.is_empty() || !detail.link_hosts.is_empty() {
            let mut body = div().flex().flex_col();
            for (label, warn) in uses {
                body = body.child(
                    div()
                        .text_color(rgb(if warn { c.danger } else { c.secondary }))
                        .child(label),
                );
            }
            if !detail.link_hosts.is_empty() {
                body = body.child(div().text_color(rgb(c.secondary)).child(format!(
                    "Opens {} in your browser without asking",
                    detail.link_hosts.join(", ")
                )));
            }
            out.push(block("Uses", body));
        }
        if !detail.contributes.is_empty() {
            out.push(block(
                "Includes",
                div()
                    .text_color(rgb(c.secondary))
                    .child(includes_text(&detail.contributes)),
            ));
        }
        out.push(block(
            "Requires",
            div()
                .text_color(rgb(c.secondary))
                .child(requires_text(listing.latest.as_ref().and_then(|l| l.2))),
        ));
        let mut versions = div().flex().flex_col().gap(px(m.border * 2.));
        for release in &detail.releases {
            let mut line = vec![release.version.clone()];
            let date = relative_date(&release.published_at, today());
            if !date.is_empty() {
                line.push(date);
            }
            if release.withdrawn {
                line.push("Withdrawn".into());
            }
            let id = release.version.clone();
            versions = versions.child(
                div()
                    .debug_selector(move || format!("store-version-{id}"))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_color(rgb(if release.withdrawn { c.muted } else { c.text }))
                            .child(line.join(" · ")),
                    )
                    .children(release.notes.clone().map(|note| {
                        div()
                            .text_size(px(ty.minimum))
                            .text_color(rgb(c.secondary))
                            .child(note)
                    })),
            );
        }
        out.push(block("Versions", versions));
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.tokens.clone();
        let (c, m) = (&t.colors, &t.metrics);
        let (ground, ground_alpha) = t.ground();
        let (line, line_alpha) = t.line();
        // Source list, as the Project Library's: Discover, the kinds, Library.
        let mut sidebar = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(m.border))
            .w(px(m.settings_sidebar))
            .h_full()
            .pt(px(m.toolbar))
            .px(px(m.gap))
            .bg(pm_shell::view::tint(ground, ground_alpha))
            .border_r(px(m.border))
            .border_color(pm_shell::view::tint(line, line_alpha));
        let pages = std::iter::once(Page::Discover)
            .chain((0..KINDS.len()).map(Page::Kind))
            .chain([Page::Library]);
        for page in pages {
            let label = page.title();
            let selected = self.page == page;
            if page == Page::Library || page == Page::Kind(0) {
                sidebar = sidebar.child(div().h(px(m.gap)));
            }
            sidebar = sidebar.child(
                div()
                    .id(label)
                    .debug_selector(move || format!("store-section-{label}"))
                    .flex()
                    .items_center()
                    .h(px(m.row))
                    .px(px(m.gap))
                    .rounded(px(m.control_radius))
                    .when(selected, |d| d.bg(rgb(c.selected_fill)))
                    .child(label)
                    .on_click(cx.listener(move |v, _, _, cx| v.set_page(page, cx))),
            );
        }
        let heading = match &self.detail {
            Some(_) => "",
            None if !self.query.trim().is_empty() => "Results",
            None => self.page.title(),
        };
        let mut body = div()
            .id("store-body")
            .flex_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .px(px(m.gap))
            .pb(px(m.inset));
        let content = match self.detail.clone() {
            Some((listing, load)) => self.detail_view(&listing, &load, cx),
            None => self.list(cx),
        };
        // A list page says what happened above its rows; a detail page
        // says it under its action (`detail_view`).
        if self.detail.is_none()
            && let Some(message) = self.message_line()
        {
            body = body.child(message);
        }
        body = body.children(content);
        let content = div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(c.panel))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(m.inset))
                    .h(px(m.toolbar))
                    .px(px(m.inset + m.gap))
                    .child(
                        div()
                            .text_size(px(t.typography.heading))
                            .font_weight(FontWeight(t.typography.semibold_weight))
                            .child(heading),
                    )
                    .children(
                        self.search
                            .clone()
                            .map(|field| div().w(px(m.settings_sidebar)).child(field)),
                    ),
            )
            .child(body);
        div()
            .size_full()
            .flex()
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(sidebar)
            .child(content)
    }
}
