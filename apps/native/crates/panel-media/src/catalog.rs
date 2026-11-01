use crate::{Error, number, object, text};
use pm_kernel::{Published, Snapshot};
use pm_model::Value;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemId {
    Asset(String),
    Folder(String),
    Composition(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub project: String,
    pub document: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub folder: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub duration: Option<f64>,
    pub bytes: Option<f64>,
    pub format: Option<String>,
    pub source_path: Option<String>,
    pub triangles: Option<f64>,
    pub vertices: Option<f64>,
    pub references: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub items: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub id: String,
    pub name: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub duration: Option<f64>,
    pub fps: Option<f64>,
    pub open: bool,
    pub can_nest: bool,
    pub uses: usize,
}

/// Decoding/restoration is observed, never inferred from persisted metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    #[default]
    Unknown,
    Loading,
    Ready,
    Offline,
    Failed(String),
    Cloud {
        provider: String,
        download: Download,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Download {
    Stored,
    Downloading,
    Failed(String),
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyStatus {
    #[default]
    Unknown,
    Absent,
    Queued,
    Generating,
    Ready {
        version: String,
    },
    Failed(String),
    Stale,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub availability: Availability,
    pub proxy: ProxyStatus,
    /// Opaque host key; it may resolve even when original media is offline.
    pub poster: Option<String>,
}

/// Host adapter must classify the original and proxy separately. No IO in state().
pub trait MediaRuntime {
    fn state(&self, stamp: &Stamp, asset: &str) -> RuntimeState;
}
#[derive(Default)]
pub struct UnknownRuntime;
impl MediaRuntime for UnknownRuntime {
    fn state(&self, _: &Stamp, _: &str) -> RuntimeState {
        RuntimeState::default()
    }
}

/// Worker-built immutable index. Paging never serializes the whole document or
/// traverses all layers; reference counts are accumulated once on construction.
#[derive(Clone, Debug)]
pub struct Catalog {
    pub stamp: Stamp,
    pub assets: BTreeMap<String, Asset>,
    pub folders: BTreeMap<String, Folder>,
    pub compositions: BTreeMap<String, Composition>,
    pub(crate) root: Arc<Value>,
    children: HashMap<Option<String>, Vec<String>>,
    pub(crate) composition_order: Vec<String>,
}

impl Catalog {
    pub fn published(p: &Published) -> Result<Self, Error> {
        Self::new(p.project.clone(), p.snapshot.clone())
    }
    pub fn new(project: String, snapshot: Snapshot) -> Result<Self, Error> {
        Self::with_collation(project, snapshot, natural)
    }
    /// The macOS host supplies its locale-aware numeric/base comparator.
    /// `new` uses the deterministic Linux comparator, without inventing a sort UI.
    pub fn with_collation(
        project: String,
        snapshot: Snapshot,
        compare: impl Fn(&str, &str) -> Ordering,
    ) -> Result<Self, Error> {
        let root = snapshot.document.to_value().ok_or(Error::InvalidMedia)?;
        let stamp = Stamp {
            project,
            document: snapshot.doc_id,
            revision: snapshot.revision,
        };
        let mut folders = BTreeMap::new();
        for (id, f) in object(&root, "assetFolders")
            .into_iter()
            .filter(|(_, f)| f.as_object().is_some())
        {
            folders.insert(
                id.to_owned(),
                Folder {
                    id: id.into(),
                    name: text(f, "name")
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("Untitled Folder")
                        .into(),
                    parent: text(f, "parent").filter(|s| *s != id).map(str::to_owned),
                    items: 0,
                },
            );
        }
        let known: HashSet<_> = folders.keys().cloned().collect();
        for f in folders.values_mut() {
            if !f.parent.as_ref().is_some_and(|id| known.contains(id)) {
                f.parent = None;
            }
        }
        // Flatten damaged cycles and chains feeding them for navigation only;
        // the untouched source remains in the snapshot (N4).
        let mut cycles = HashSet::new();
        let mut resolved = HashSet::new();
        for id in folders.keys() {
            let mut chain = vec![];
            let mut seen = HashSet::new();
            let mut at = Some(id.as_str());
            while let Some(id) = at {
                if cycles.contains(id) || !seen.insert(id.to_owned()) {
                    cycles.extend(chain.iter().cloned());
                    break;
                }
                if resolved.contains(id) {
                    break;
                }
                chain.push(id.to_owned());
                at = folders[id].parent.as_deref();
            }
            resolved.extend(chain);
        }
        for id in cycles {
            folders.get_mut(&id).unwrap().parent = None;
        }
        let refs = reference_counts(&root);
        let assets: BTreeMap<_, _> = object(&root, "assets")
            .into_iter()
            .filter(|(_, a)| a.as_object().is_some())
            .map(|(id, a)| {
                let asset = Asset {
                    id: id.into(),
                    name: text(a, "name").unwrap_or("").into(),
                    kind: text(a, "kind").unwrap_or("").into(),
                    folder: text(a, "folder")
                        .filter(|s| folders.contains_key(*s))
                        .map(str::to_owned),
                    width: number(a, "w"),
                    height: number(a, "h"),
                    duration: number(a, "dur"),
                    bytes: number(a, "size"),
                    format: text(a, "format").map(str::to_owned),
                    source_path: text(a, "sourcePath")
                        .filter(|s| !s.is_empty())
                        .or_else(|| text(a, "path"))
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    triangles: number(a, "triangles"),
                    vertices: number(a, "vertices"),
                    references: refs.get(id).copied().unwrap_or(0),
                };
                (id.to_owned(), asset)
            })
            .collect();
        let mut children: HashMap<_, Vec<_>> = HashMap::new();
        for f in folders.values() {
            children
                .entry(f.parent.clone())
                .or_default()
                .push(f.id.clone());
        }
        for ids in children.values_mut() {
            ids.sort_by(|a, b| compare(&folders[a].name, &folders[b].name).then(a.cmp(b)));
        }
        // Bottom-up subtree sums, O(folders + assets), including very deep bins.
        let mut direct: HashMap<String, usize> = HashMap::new();
        for a in assets.values() {
            if let Some(f) = &a.folder {
                *direct.entry(f.clone()).or_default() += 1;
            }
        }
        let mut stack: Vec<_> = children
            .get(&None)
            .into_iter()
            .flatten()
            .map(|id| (id.clone(), false))
            .collect();
        while let Some((id, done)) = stack.pop() {
            if !done {
                stack.push((id.clone(), true));
                stack.extend(
                    children
                        .get(&Some(id))
                        .into_iter()
                        .flatten()
                        .map(|id| (id.clone(), false)),
                );
            } else {
                let n = direct.get(&id).copied().unwrap_or(0)
                    + children
                        .get(&Some(id.clone()))
                        .into_iter()
                        .flatten()
                        .map(|c| 1 + folders[c].items)
                        .sum::<usize>();
                folders.get_mut(&id).unwrap().items = n;
            }
        }
        let open = text(&root, "compId");
        let mut comp_values: Vec<_> = object(&root, "comps");
        // Canonical documents store the open composition in comps too.
        if let Some(id) = open
            && !comp_values.iter().any(|(k, _)| *k == id)
        {
            comp_values.push((id, &root));
        }
        let mut uses: HashMap<String, usize> = HashMap::new();
        let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
        for (parent, comp) in &comp_values {
            for l in comp
                .get("layers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if text(l, "type") == Some("precomp")
                    && let Some(target) = l.get("d").and_then(|d| text(d, "comp"))
                {
                    *uses.entry(target.into()).or_default() += 1;
                    reverse
                        .entry(target.into())
                        .or_default()
                        .push(parent.to_string());
                }
            }
        }
        let mut blocked = HashSet::new();
        let mut stack: Vec<_> = open.into_iter().map(str::to_owned).collect();
        while let Some(id) = stack.pop() {
            if !blocked.insert(id.clone()) {
                continue;
            }
            stack.extend(reverse.get(&id).into_iter().flatten().cloned());
        }
        let compositions: BTreeMap<_, _> = comp_values
            .iter()
            .filter(|(_, c)| c.as_object().is_some())
            .map(|(id, c)| {
                (
                    id.to_string(),
                    Composition {
                        id: id.to_string(),
                        name: text(c, "name")
                            .filter(|s| !s.is_empty())
                            .unwrap_or("Comp")
                            .into(),
                        width: number(c, "w"),
                        height: number(c, "h"),
                        duration: number(c, "dur"),
                        fps: number(c, "fps"),
                        open: open == Some(*id),
                        can_nest: open.is_some() && !blocked.contains(*id),
                        uses: uses.get(*id).copied().unwrap_or(0),
                    },
                )
            })
            .collect();
        let mut composition_order: Vec<_> = compositions.keys().cloned().collect();
        composition_order
            .sort_by(|a, b| compare(&compositions[a].name, &compositions[b].name).then(a.cmp(b)));
        Ok(Self {
            stamp,
            assets,
            folders,
            compositions,
            composition_order,
            root: Arc::new(root),
            children,
        })
    }
    pub fn valid_folder(&self, id: Option<&str>) -> Option<String> {
        id.filter(|id| self.folders.contains_key(*id))
            .map(str::to_owned)
    }
    pub fn path(&self, id: Option<&str>) -> Vec<&Folder> {
        let mut out = vec![];
        let mut at = self.valid_folder(id);
        while let Some(id) = at {
            let f = &self.folders[&id];
            at = f.parent.clone();
            out.push(f);
        }
        out.reverse();
        out
    }
    pub fn within(&self, at: Option<&str>, ancestor: &str) -> bool {
        self.path(at).iter().any(|f| f.id == ancestor)
    }
    pub fn contains(&self, id: &ItemId) -> bool {
        match id {
            ItemId::Asset(id) => self.assets.contains_key(id),
            ItemId::Folder(id) => self.folders.contains_key(id),
            ItemId::Composition(id) => self.compositions.contains_key(id),
        }
    }
    pub(crate) fn child_ids(&self, at: Option<&str>) -> &[String] {
        self.children
            .get(&at.map(str::to_owned))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Case-insensitive numeric ordering; locale-specific collation belongs to the
/// macOS host through `with_collation`; no locale/sort affordance is chosen here.
fn natural(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (s, t) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let aa = &a[s..i];
            let bb = &b[t..j];
            let aa = aa
                .iter()
                .position(|n| *n != b'0')
                .map(|n| &aa[n..])
                .unwrap_or(&[]);
            let bb = bb
                .iter()
                .position(|n| *n != b'0')
                .map(|n| &bb[n..])
                .unwrap_or(&[]);
            let c = aa.len().cmp(&bb.len()).then(aa.cmp(bb));
            if c != Ordering::Equal {
                return c;
            }
        } else {
            let c = a[i].cmp(&b[j]);
            if c != Ordering::Equal {
                return c;
            }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len())
}

pub(crate) fn layers(root: &Value) -> impl Iterator<Item = &Value> {
    root.get("comps")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|c| c.values())
        .chain(std::iter::once(root))
        .flat_map(|c| {
            c.get("layers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
}
pub(crate) fn layer_asset(l: &Value) -> Option<&str> {
    l.get("d").and_then(|d| text(d, "asset")).or_else(|| {
        if text(l, "type") != Some("extension") {
            return None;
        }
        let d = l.get("d")?.get("data")?;
        d.get("object")
            .and_then(|o| o.get("source"))
            .and_then(|s| text(s, "assetId"))
            .or_else(|| text(d, "assetId"))
    })
}
fn reference_counts(root: &Value) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for l in layers(root) {
        if let Some(a) = layer_asset(l) {
            *out.entry(a.into()).or_default() += 1;
        }
        if let Some(d) = l.get("d").and_then(|d| d.get("data")) {
            let objects = d.get("object").into_iter().map(|o| (o, true)).chain(
                d.get("scene")
                    .and_then(|s| s.get("objects"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|o| (o, false)),
            );
            for (o, single) in objects {
                if let Some(a) = o.get("source").and_then(|s| text(s, "assetId")) {
                    // The single extension object's source was counted above.
                    if !single {
                        *out.entry(a.into()).or_default() += 1;
                    }
                }
                for (_, a) in o
                    .get("material")
                    .map(|m| object(m, "maps"))
                    .unwrap_or_default()
                {
                    if let Some(a) = a.as_str() {
                        *out.entry(a.into()).or_default() += 1;
                    }
                }
            }
        }
    }
    out
}
