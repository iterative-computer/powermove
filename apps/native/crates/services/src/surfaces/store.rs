//! S075–S078. Metadata projections and worker search adapter, based on
//! v1.2.0 renderer/store/data.ts and StoreScreen.svelte. No install/trust IO.
use super::{Flight, Ticket};
use crate::{
    Result, ServiceErrorCode,
    store::{Search, SearchPage, StoreClient, StoreTransport},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogState {
    Idle,
    Loading,
    Ready,
    Failed(ServiceErrorCode),
}
/// S075 has no user sort control or remembered-install list. Low-level Store
/// API sort remains in its existing client for other consumers.
#[derive(Debug, Clone, Default)]
pub struct CatalogQuery {
    pub text: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug)]
pub struct SearchWork {
    pub ticket: Ticket,
    search: Search,
    append: bool,
}
impl SearchWork {
    /// Submit this method through ServiceWorker, never call it on main.
    pub fn run<T: StoreTransport>(self, client: &mut StoreClient<T>) -> SearchReply {
        let result = client.search(self.search);
        SearchReply {
            ticket: self.ticket,
            append: self.append,
            result,
        }
    }
}
pub struct SearchReply {
    ticket: Ticket,
    append: bool,
    result: Result<SearchPage>,
}

#[derive(Debug)]
pub struct Catalog {
    pub state: CatalogState,
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
    search: Search,
    flight: Flight,
    seen_cursors: BTreeSet<String>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            state: CatalogState::Idle,
            items: Vec::new(),
            next_cursor: None,
            search: Search::default(),
            flight: Flight::default(),
            seen_cursors: BTreeSet::new(),
        }
    }
}
impl Catalog {
    pub fn search(&mut self, query: CatalogQuery) -> Result<SearchWork> {
        if query
            .text
            .as_ref()
            .is_some_and(|s| s.encode_utf16().count() > 120)
        {
            return Err(crate::ServiceError::message(
                ServiceErrorCode::InvalidInput,
                "Store search exceeds 120 characters",
            ));
        }
        let search = Search {
            query: query.text.map(|s| s.trim().into()),
            category: query.category,
            ..Search::default()
        };
        self.search = search.clone();
        // S075/README §4: retain results until replacement arrives.
        self.next_cursor = None;
        self.seen_cursors.clear();
        self.state = CatalogState::Loading;
        Ok(SearchWork {
            ticket: self.flight.start(),
            search,
            append: false,
        })
    }
    pub fn more(&mut self) -> Option<SearchWork> {
        if self.flight.current().is_some() {
            return None;
        }
        let cursor = self.next_cursor.clone()?;
        if self.seen_cursors.contains(&cursor) {
            return None;
        }
        let mut search = self.search.clone();
        search.cursor = Some(cursor);
        self.state = CatalogState::Loading;
        Some(SearchWork {
            ticket: self.flight.start(),
            search,
            append: true,
        })
    }
    /// Failed later pages retain the visible items/cursor so Retry is real.
    pub fn receive(&mut self, reply: SearchReply) -> bool {
        if !self.flight.finish(reply.ticket) {
            return false;
        }
        match reply.result {
            Ok(page) => {
                if reply.append {
                    if let Some(cursor) = self.next_cursor.take() {
                        self.seen_cursors.insert(cursor);
                    }
                    self.items.extend(page.items);
                } else {
                    self.items = page.items;
                }
                self.next_cursor = page.next_cursor;
                if self
                    .next_cursor
                    .as_ref()
                    .is_some_and(|c| self.seen_cursors.contains(c))
                {
                    // A looping server cursor is visible as invalid data, never
                    // an apparent successful/complete catalog (N1 honesty).
                    self.state = CatalogState::Failed(ServiceErrorCode::InvalidData);
                } else {
                    self.state = CatalogState::Ready;
                }
            }
            Err(error) => self.state = CatalogState::Failed(error.code),
        }
        true
    }
    pub fn close(&mut self) {
        self.flight.cancel();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub local_id: String,
    pub builtin: bool,
    pub published_repo: Option<String>,
    pub origin_repo: Option<String>,
    /// Opaque provenance/contributions/variables are never discarded.
    pub metadata: Value,
}
pub fn library_item<'a>(
    repo: &str,
    publisher: &str,
    slug: &str,
    library: &'a [LibraryItem],
) -> Option<&'a LibraryItem> {
    library
        .iter()
        .find(|i| i.published_repo.as_deref() == Some(repo))
        .or_else(|| {
            library
                .iter()
                .find(|i| i.origin_repo.as_deref() == Some(repo))
        })
        .or_else(|| {
            (publisher == "powermove")
                .then(|| library.iter().find(|i| i.builtin && i.local_id == slug))
                .flatten()
        })
}
pub fn owns_listing(publisher: &str, account_handle: Option<&str>) -> bool {
    account_handle.is_some_and(|h| !h.is_empty() && h == publisher)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compatibility {
    SupportedLegacy,
    NativeV4,
    NewerHostRequired,
    InvalidVersion,
    Unavailable,
}
pub fn compatibility(api: Option<u32>) -> Compatibility {
    match api {
        None => Compatibility::Unavailable,
        Some(1..=3) => Compatibility::SupportedLegacy,
        Some(4) => Compatibility::NativeV4,
        Some(0) => Compatibility::InvalidVersion,
        Some(_) => Compatibility::NewerHostRequired,
    }
}
/// Disclosure order only, never an admission/permission grant.
pub fn ordered_permissions(permissions: &[String]) -> Vec<&str> {
    let order = [
        "full-access",
        "project:write",
        "assets",
        "network",
        "clipboard",
    ];
    let mut result: Vec<_> = order
        .into_iter()
        .filter(|p| permissions.iter().any(|s| s == p))
        .collect();
    for permission in permissions {
        if !result.contains(&permission.as_str()) {
            result.push(permission);
        }
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemGroup {
    Builtin,
    Store,
    Yours,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ready,
    NeedsTrust,
    NeedsSetup,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishKind {
    First,
    Update,
}
#[derive(Debug)]
pub struct ItemFacts {
    pub group: ItemGroup,
    pub health: Health,
    pub enabled: bool,
    pub fork: bool,
    pub published_repo: Option<String>,
    pub update: bool,
    pub update_modified: bool,
    pub removed: bool,
    pub publish: Option<PublishKind>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailAction {
    Install,
    InstallAndSetUp,
    Installing,
    BuiltIn,
    Trust,
    SetUp,
    Forked,
    Update,
    UpdateModified,
    Publish,
    PublishUpdate,
    Installed,
    TurnOn,
    Open,
    RequiresNewerHost,
}
impl DetailAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Install | Self::RequiresNewerHost => "Install",
            Self::InstallAndSetUp => "Install and Set Up",
            Self::Installing => "Installing…",
            Self::BuiltIn => "Built In",
            Self::Trust => "Trust…",
            Self::SetUp => "Set Up…",
            Self::Forked => "Forked",
            Self::Update => "Update",
            Self::UpdateModified => "Update…",
            Self::Publish => "Publish…",
            Self::PublishUpdate => "Publish Update…",
            Self::Installed => "Installed",
            Self::TurnOn => "Turn On",
            Self::Open => "Open",
        }
    }
    pub fn enabled(self) -> bool {
        !matches!(
            self,
            Self::Installing
                | Self::RequiresNewerHost
                | Self::BuiltIn
                | Self::Installed
                | Self::Forked
        )
    }
}
/// S076's single action and exact precedence; authorship never grants trust.
pub fn detail_action(
    item: Option<&ItemFacts>,
    repo_id: Option<&str>,
    declares_values: bool,
    installing: bool,
    api: Option<u32>,
) -> DetailAction {
    if installing {
        return DetailAction::Installing;
    }
    let Some(item) = item else {
        if !matches!(
            compatibility(api),
            Compatibility::SupportedLegacy | Compatibility::NativeV4
        ) {
            return DetailAction::RequiresNewerHost;
        }
        return if declares_values {
            DetailAction::InstallAndSetUp
        } else {
            DetailAction::Install
        };
    };
    if item.group == ItemGroup::Builtin {
        return DetailAction::BuiltIn;
    }
    if item.health == Health::NeedsTrust {
        return DetailAction::Trust;
    }
    if item.health == Health::NeedsSetup {
        return DetailAction::SetUp;
    }
    if item.fork && repo_id.is_some() && repo_id != item.published_repo.as_deref() {
        return DetailAction::Forked;
    }
    if item.update && !item.removed {
        return if item.update_modified {
            DetailAction::UpdateModified
        } else {
            DetailAction::Update
        };
    }
    if item.group == ItemGroup::Yours
        && let Some(kind) = item.publish
    {
        return if kind == PublishKind::First {
            DetailAction::Publish
        } else {
            DetailAction::PublishUpdate
        };
    }
    if item.group == ItemGroup::Store {
        return DetailAction::Installed;
    }
    if !item.enabled {
        return DetailAction::TurnOn;
    }
    DetailAction::Open
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ServiceError,
        store::{Request, Response},
    };
    use serde_json::json;
    struct Transport {
        status: u16,
        body: Value,
    }
    impl StoreTransport for Transport {
        fn send(&mut self, request: Request, _: usize) -> Result<Response> {
            assert_eq!(request.url.host_str(), Some("example.com"));
            Ok(Response {
                status: self.status,
                body: serde_json::to_vec(&self.body).unwrap(),
            })
        }
    }
    fn reply(work: SearchWork, items: Value, cursor: Option<&str>) -> SearchReply {
        let mut client = StoreClient::new(
            "https://example.com",
            Transport {
                status: 200,
                body: json!({"items":items,"nextCursor":cursor}),
            },
        )
        .unwrap();
        work.run(&mut client)
    }
    fn listing(name: &str) -> Value {
        json!({"repoId":"11111111-1111-4111-8111-111111111111","slug":"synthetic-extension","name":name,"iconUrl":null,"future":1})
    }
    #[test]
    fn stale_search_and_close_cannot_publish() {
        let mut c = Catalog::default();
        let old = c.search(CatalogQuery::default()).unwrap();
        let new = c
            .search(CatalogQuery {
                text: Some("new".into()),
                ..CatalogQuery::default()
            })
            .unwrap();
        assert!(!c.receive(reply(old, json!([listing("old")]), None)));
        assert!(c.receive(reply(new, json!([listing("new")]), None)));
        assert_eq!(c.items, vec![listing("new")]);
        let old = c.search(CatalogQuery::default()).unwrap();
        c.close();
        assert!(!c.receive(reply(old, json!([listing("closed")]), None)));
    }
    #[test]
    fn pagination_failure_retains_items_and_allows_retry() {
        let mut c = Catalog::default();
        let first = c.search(CatalogQuery::default()).unwrap();
        c.receive(reply(first, json!([listing("one")]), Some("page2")));
        let next = c.more().unwrap();
        assert!(c.more().is_none());
        c.receive(SearchReply {
            ticket: next.ticket,
            append: true,
            result: Err(ServiceError::message(ServiceErrorCode::Offline, "offline")),
        });
        assert_eq!(c.items.len(), 1);
        assert_eq!(c.state, CatalogState::Failed(ServiceErrorCode::Offline));
        let next = c.more().unwrap();
        c.receive(reply(next, json!([listing("two")]), None));
        assert_eq!(c.items.len(), 2);
        assert!(c.more().is_none());
    }
    #[test]
    fn looping_cursor_is_an_explicit_error() {
        let mut c = Catalog::default();
        let first = c.search(CatalogQuery::default()).unwrap();
        c.receive(reply(first, json!([]), Some("same")));
        let next = c.more().unwrap();
        c.receive(reply(next, json!([]), Some("same")));
        assert_eq!(c.state, CatalogState::Failed(ServiceErrorCode::InvalidData));
        assert!(c.more().is_none());
    }
    #[test]
    fn real_worker_adapter_maps_transport_failure() {
        let worker = crate::worker::ServiceWorker::start(|| {
            StoreClient::new(
                "https://example.com",
                Transport {
                    status: 503,
                    body: json!({}),
                },
            )
        })
        .unwrap();
        let mut c = Catalog::default();
        let work = c.search(CatalogQuery::default()).unwrap();
        let pending = worker.submit(move |client| Ok(work.run(client))).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(reply) = pending.try_take().unwrap() {
                assert!(c.receive(reply));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(c.state, CatalogState::Failed(ServiceErrorCode::Offline));
    }
    #[test]
    fn local_publish_precedes_origin_and_builtin_fallback_is_exact() {
        let item = |id: &str, published, origin, builtin| LibraryItem {
            local_id: id.into(),
            published_repo: published,
            origin_repo: origin,
            builtin,
            metadata: json!({"future":1}),
        };
        let library = vec![
            item("fork", None, Some("repo".into()), false),
            item("published", Some("repo".into()), None, false),
            item("timeline", None, None, true),
        ];
        assert_eq!(
            library_item("repo", "p", "slug", &library)
                .unwrap()
                .local_id,
            "published"
        );
        assert!(library_item("missing", "other", "timeline", &library).is_none());
        assert_eq!(
            library_item("missing", "powermove", "timeline", &library)
                .unwrap()
                .local_id,
            "timeline"
        );
        assert!(!owns_listing("p", Some("other")));
        assert!(owns_listing("p", Some("p")));
    }
    #[test]
    fn all_legacy_versions_and_unknown_permissions_remain_visible() {
        for api in 1..=3 {
            assert_eq!(compatibility(Some(api)), Compatibility::SupportedLegacy);
        }
        assert_eq!(compatibility(Some(4)), Compatibility::NativeV4);
        assert_eq!(compatibility(None), Compatibility::Unavailable);
        let p = vec![
            "network".into(),
            "future".into(),
            "full-access".into(),
            "network".into(),
        ];
        assert_eq!(
            ordered_permissions(&p),
            vec!["full-access", "network", "future"]
        );
    }
    #[test]
    fn search_retains_results_until_replaced_and_limits_utf16_input() {
        let mut c = Catalog::default();
        let first = c.search(CatalogQuery::default()).unwrap();
        c.receive(reply(first, json!([listing("previous")]), None));
        let next = c
            .search(CatalogQuery {
                text: Some("new".into()),
                ..CatalogQuery::default()
            })
            .unwrap();
        assert_eq!(c.items, vec![listing("previous")]);
        assert!(
            c.search(CatalogQuery {
                text: Some("😀".repeat(61)),
                ..CatalogQuery::default()
            })
            .is_err()
        );
        c.receive(reply(next, json!([listing("replacement")]), None));
        assert_eq!(c.items, vec![listing("replacement")]);
    }
    #[test]
    fn detail_action_precedence_matches_spec_and_never_silently_trusts_a_fork() {
        let mut item = ItemFacts {
            group: ItemGroup::Yours,
            health: Health::NeedsTrust,
            enabled: false,
            fork: true,
            published_repo: Some("own".into()),
            update: true,
            update_modified: true,
            removed: false,
            publish: Some(PublishKind::First),
        };
        assert_eq!(
            detail_action(Some(&item), Some("original"), false, false, Some(3)),
            DetailAction::Trust
        );
        item.health = Health::NeedsSetup;
        assert_eq!(
            detail_action(Some(&item), Some("original"), false, false, Some(3)),
            DetailAction::SetUp
        );
        item.health = Health::Ready;
        assert_eq!(
            detail_action(Some(&item), Some("original"), false, false, Some(3)),
            DetailAction::Forked
        );
        item.fork = false;
        assert_eq!(
            detail_action(Some(&item), None, false, false, Some(3)),
            DetailAction::UpdateModified
        );
        item.update = false;
        assert_eq!(
            detail_action(Some(&item), None, false, false, Some(3)),
            DetailAction::Publish
        );
        item.publish = None;
        assert_eq!(
            detail_action(Some(&item), None, false, false, Some(3)),
            DetailAction::TurnOn
        );
        assert_eq!(
            detail_action(None, None, true, false, Some(3)).label(),
            "Install and Set Up"
        );
        assert!(!detail_action(None, None, false, false, Some(5)).enabled());
    }
}
