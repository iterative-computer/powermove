//! Viewer representation binding. Originals remain the document's media; this
//! resolver is local to a viewer render and is never passed to an export.
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use super::document::MediaMap;
use pm_model::Node;
use pm_protocol::frame::{MediaPreference, Representation};
use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_render::AssetResolver;

#[derive(Default)]
pub(super) struct ProxyMedia {
    entries: HashMap<String, (Instant, Option<PathBuf>, Option<FileStamp>)>,
    playing: bool,
    intent: String,
    choices: HashMap<(String, Option<String>), Option<PathBuf>>,
    deletions: Option<(Option<(SystemTime, u64)>, serde_json::Value)>,
}

type FileStamp = (u64, u64, u64, i64, i64, i64, i64);
fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

fn text(node: Option<&Node>) -> Option<String> {
    match node?.kind() {
        pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

impl ProxyMedia {
    fn refresh_deletions(&mut self) {
        let Some(file) = std::env::var_os("POWERMOVE_PROXY_LOCATIONS") else {
            return;
        };
        let Some(parent) = std::path::Path::new(&file).parent() else {
            return;
        };
        let file = parent.join("Proxy Deletions.json");
        let stamp = std::fs::metadata(&file)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if self
            .deletions
            .as_ref()
            .is_some_and(|(old, _)| *old == stamp)
        {
            return;
        }
        let value = if stamp.is_none() {
            serde_json::json!({"version":1,"deleted":{}})
        } else {
            // Damaged machine-local state cannot revive a proxy the user
            // deleted. Main reports the named state-file error to the UI.
            std::fs::read(&file)
                .ok()
                .and_then(|v| serde_json::from_slice(&v).ok())
                .unwrap_or(serde_json::Value::Null)
        };
        self.deletions = Some((stamp, value));
        self.choices.clear();
    }

    fn roots() -> Vec<PathBuf> {
        let Some(file) = std::env::var_os("POWERMOVE_PROXY_LOCATIONS") else {
            return Vec::new();
        };
        let Ok(bytes) = std::fs::read(file) else {
            return Vec::new();
        };
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("roots")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(PathBuf::from))
            .filter(|p| p.is_absolute())
            .collect()
    }

    pub fn resolver<'a>(
        &mut self,
        doc: &pm_model::Document,
        media: &'a MediaMap,
        preference: MediaPreference,
        playing: bool,
        slow: &[PathBuf],
    ) -> ProxyResolver<'a> {
        let mut proxies = BTreeMap::new();
        let mut forced = Vec::new();
        self.refresh_deletions();
        let project = text(doc.root().and_then(|r| r.get("id"))).unwrap_or_default();
        let intent = doc
            .root()
            .and_then(|r| r.get("media"))
            .and_then(|r| r.get("proxies"));
        let format = match text(intent.and_then(|r| r.get("format"))).as_deref() {
            Some("proResLt") => ProxyFormat::ProResLt,
            Some("hevc10") => ProxyFormat::Hevc10,
            _ => ProxyFormat::ProResProxy,
        };
        let size = match text(intent.and_then(|r| r.get("size"))).as_deref() {
            Some("half") => ProxySize::Half,
            Some("quarter") => ProxySize::Quarter,
            Some("p1080") => ProxySize::P1080,
            Some("p720") => ProxySize::P720,
            _ => ProxySize::Automatic,
        };
        let intent_key = format!("{project}:{preference:?}:{format:?}:{size:?}");
        if !playing || !self.playing || self.intent != intent_key {
            self.choices.clear();
        }
        self.playing = playing;
        self.intent = intent_key;
        if let Some(pm_model::NodeKind::Object(assets)) =
            doc.root().and_then(|r| r.get("assets")).map(Node::kind)
        {
            for (id, meta) in assets {
                if !text(meta.get("kind")).is_some_and(|kind| kind == "video")
                    && !matches!(
                        meta.get("imageSequence").map(Node::kind),
                        Some(pm_model::NodeKind::Bool(true))
                    )
                {
                    continue;
                }
                let id = id.to_string_lossy();
                if matches!(
                    meta.get("alwaysPlayOriginal").map(Node::kind),
                    Some(pm_model::NodeKind::Bool(true))
                ) {
                    forced.push(id);
                    continue;
                }
                if preference == MediaPreference::Originals {
                    continue;
                }
                let original = media.path(&id);
                let hash = text(meta.get("contentHash"))
                    .or_else(|| text(meta.get("link").and_then(|r| r.get("hash"))));
                if self.deletions.as_ref().is_some_and(|(_, value)| {
                    is_deleted(value, &project, &id, hash.as_deref(), format, size)
                }) {
                    continue;
                }
                let choice_key = (id.clone(), hash.clone());
                if playing && let Some(choice) = self.choices.get(&choice_key) {
                    if let Some(path) = choice {
                        proxies.insert(id, path.clone());
                    }
                    continue;
                }
                let needed = preference == MediaPreference::Proxies
                    || original.as_ref().is_none_or(|p| !p.exists())
                    || playing && original.as_ref().is_some_and(|p| slow.contains(p));
                if !needed {
                    if playing {
                        self.choices.insert(choice_key, None);
                    }
                    continue;
                }
                let Some(hash) = hash else { continue };
                let key = format!("{hash}:{format:?}:{size:?}");
                let refresh = self.entries.get(&key).is_none_or(|(at, file, stamp)| {
                    file.is_none() && at.elapsed() > Duration::from_secs(1)
                        || file.as_ref().is_some_and(|p| file_stamp(p) != *stamp)
                });
                if refresh {
                    let mut found = None;
                    for root in Self::roots() {
                        if let Ok(store) = pm_proxy::ProxyStore::open(&root)
                            && let Ok(Some(proxy)) =
                                store.find_for_source(&hash, Some(format), Some(size))
                        {
                            found = Some(proxy.path);
                            break;
                        }
                    }
                    let stamp = found.as_deref().and_then(file_stamp);
                    self.entries
                        .insert(key.clone(), (Instant::now(), found, stamp));
                }
                let choice = self.entries.get(&key).and_then(|(_, file, _)| file.clone());
                if playing {
                    self.choices.insert(choice_key, choice.clone());
                }
                if let Some(path) = choice {
                    proxies.insert(id, path);
                }
            }
        }
        // Expired sources do not accumulate as projects are edited/relinked.
        if self.entries.len() > 4096 {
            self.entries.clear();
        }
        ProxyResolver {
            original: media,
            proxies,
            forced,
            preference,
            used: Mutex::new(Representation::Original),
            assets_used: Default::default(),
        }
    }

    pub fn finish(&mut self, resolver: &ProxyResolver<'_>) {
        let used = resolver
            .assets_used
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.choices.retain(|(asset, _), _| used.contains(asset));
    }
}

fn is_deleted(
    value: &serde_json::Value,
    project: &str,
    asset: &str,
    hash: Option<&str>,
    format: ProxyFormat,
    size: ProxySize,
) -> bool {
    if value.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return true;
    }
    let Some(entry) = value
        .get("deleted")
        .and_then(|v| v.get(project))
        .and_then(|v| v.get(asset))
    else {
        return false;
    };
    entry.get("format") == serde_json::to_value(format).ok().as_ref()
        && entry.get("size") == serde_json::to_value(size).ok().as_ref()
        && (entry.get("hash").is_some_and(|v| v.is_null())
            || entry.get("hash").and_then(|v| v.as_str()) == hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_is_scoped_to_project_source_and_recipe() {
        let value = serde_json::json!({"version":1,"deleted":{"project":{"asset":{"hash":"source","format":"proResProxy","size":"half","at":0}}}});
        assert!(is_deleted(
            &value,
            "project",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
        assert!(!is_deleted(
            &value,
            "other",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
        assert!(!is_deleted(
            &value,
            "project",
            "asset",
            Some("relinked"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
        assert!(!is_deleted(
            &value,
            "project",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Quarter
        ));
        assert!(is_deleted(
            &serde_json::Value::Null,
            "project",
            "asset",
            Some("source"),
            ProxyFormat::ProResProxy,
            ProxySize::Half
        ));
    }
}

pub(super) struct ProxyResolver<'a> {
    original: &'a MediaMap,
    proxies: BTreeMap<String, PathBuf>,
    forced: Vec<String>,
    preference: MediaPreference,
    used: Mutex<Representation>,
    assets_used: Mutex<Vec<String>>,
}
impl ProxyResolver<'_> {
    pub fn representation(&self) -> Representation {
        *self.used.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn signature(&self) -> String {
        let sources: Vec<_> = self
            .proxies
            .iter()
            .map(|(id, path)| (id, path, file_stamp(path)))
            .collect();
        format!("{:?}:{:?}:{sources:?}", self.preference, self.forced)
    }
}
impl AssetResolver for ProxyResolver<'_> {
    fn path(&self, asset: &str, record: Option<&Node>) -> Option<PathBuf> {
        self.assets_used
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(asset.to_owned());
        if let Some(path) = self.proxies.get(asset) {
            *self.used.lock().unwrap_or_else(|e| e.into_inner()) = Representation::Proxy;
            return Some(path.clone());
        }
        if self.preference == MediaPreference::Proxies
            && !self.forced.iter().any(|id| id == asset)
            && record.is_some_and(|r| {
                text(r.get("kind")).is_some_and(|kind| kind == "video" || kind == "imageSequence")
                    || matches!(
                        r.get("imageSequence").map(Node::kind),
                        Some(pm_model::NodeKind::Bool(true))
                    )
            })
        {
            // Proxy Only is a named missing-media layer, never an original.
            return None;
        }
        AssetResolver::path(self.original, asset, record)
    }
}
