//! Portable production-crate probes. No UI, codec or live Store is fabricated.
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
type Check = Result<(), String>;
fn checked<T, E: std::fmt::Debug>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|e| format!("{e:?}"))
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = checked(std::fs::canonicalize(std::env::temp_dir()))?.join(format!(
            "pm-acceptance-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        checked(std::fs::create_dir(&path))?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub const IDS: &[&str] = &[
    "W001.service-entry",
    "W012.service-storage",
    "W019.background-tasks",
    "W022.media-archive",
    "W077.render-cache",
    "W040.native-raster",
    "W040.remote-router",
    "W032.native-surface",
    "W038.store-view",
    "W022.proxy-identity",
];
pub fn check(id: &str) -> Option<Check> {
    if !IDS.contains(&id) {
        return None;
    }
    Some(match id {
        "W001.service-entry" => entry(),
        "W012.service-storage" => storage(),
        "W019.background-tasks" => tasks(),
        "W022.media-archive" => archive(),
        "W077.render-cache" => cache(),
        "W040.native-raster" => raster(),
        "W040.remote-router" => remote(),
        "W032.native-surface" => surface(),
        "W038.store-view" => store_view(),
        "W022.proxy-identity" => proxy(),
        _ => return None,
    })
}
fn entry() -> Check {
    use pm_services::{
        entrypoints::*,
        error::{Result, ServiceError, ServiceErrorCode},
    };
    #[derive(Default)]
    struct Host {
        paths: Vec<PathBuf>,
        fail: bool,
    }
    impl EntryHook for Host {
        fn auth(&mut self, _: &AuthLink) -> Result<()> {
            Err(ServiceError::message(
                ServiceErrorCode::Unavailable,
                "synthetic auth unavailable",
            ))
        }
        fn open_project(&mut self, path: &Path) -> Result<()> {
            if self.fail {
                return Err(ServiceError::message(
                    ServiceErrorCode::Unavailable,
                    "synthetic kernel unavailable",
                ));
            }
            self.paths.push(path.into());
            Ok(())
        }
        fn register_associations(&mut self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
    }
    let mut queue = EntryQueue::default();
    checked(queue.push_file("/synthetic/First.PMV".into()))?;
    checked(queue.push_file("/synthetic/First.PMV".into()))?;
    checked(queue.push_file("/synthetic/Second.pmv".into()))?;
    let mut host = Host {
        fail: true,
        ..Default::default()
    };
    assert!(queue.drain_files(&mut host).is_err());
    assert_eq!(queue.pending(), (0, 2));
    host.fail = false;
    checked(queue.drain_files(&mut host))?;
    assert_eq!(
        host.paths,
        vec![
            PathBuf::from("/synthetic/First.PMV"),
            PathBuf::from("/synthetic/Second.pmv")
        ]
    );
    assert_eq!(queue.pending(), (0, 0));
    assert!(queue.push_file("relative.pmv".into()).is_err());
    Ok(())
}
fn storage() -> Check {
    #[cfg(test)]
    let _process_gate = crate::FIXTURE_PROCESS_GATE.lock().unwrap();
    use pm_services::storage::ServiceStorage;
    let scratch = Scratch::new()?;
    let root = scratch.0.join("services");
    let value = json!({"unknown":{"values":[null,true,1.23456789012345]},"extension":"authored"});
    let mut storage = checked(ServiceStorage::open(&root))?;
    assert!(ServiceStorage::open(&root).is_err());
    checked(storage.set("ext.synthetic", value.clone()))?;
    let first = checked(storage.backup("project", &b"synthetic PMV bytes"[..]))?;
    checked(storage.archive_setting("ext.synthetic"))?;
    assert_eq!(checked(storage.setting("ext.synthetic"))?, None);
    drop(storage);
    let mut storage = checked(ServiceStorage::open(&root))?;
    assert_eq!(checked(storage.setting("ext.synthetic"))?, None);
    checked(storage.set("ext.synthetic", value.clone()))?;
    assert_eq!(checked(storage.setting("ext.synthetic"))?, Some(value));
    assert_eq!(checked(storage.backups("project"))?.len(), 1);
    assert!(first.path.exists());
    Ok(())
}
fn tasks() -> Check {
    use pm_promedia::tasks::*;
    let mut tasks = BackgroundTasks::default();
    let spec = TaskSpec {
        title: "Synthetic footage".into(),
        kind: TaskKind::Proxy,
        unit: ProgressUnit::Frames,
        priority: 1,
        total: 10,
    };
    let id = checked(tasks.enqueue("window".into(), "project".into(), spec))?;
    let ticket = tasks.start_next().ok_or("missing worker ticket")?;
    checked(tasks.progress(ticket, 3, 10))?;
    checked(tasks.pause(id))?;
    assert!(tasks.finish(ticket, Ok(())).is_err());
    checked(tasks.settle_control(ticket))?;
    checked(tasks.resume(id))?;
    let next = tasks.start_next().ok_or("missing resumed ticket")?;
    assert!(tasks.progress(ticket, 4, 10).is_err());
    checked(tasks.progress(next, 5, 10))?;
    checked(tasks.cancel(id))?;
    assert!(tasks.finish(next, Ok(())).is_err());
    checked(tasks.settle_control(next))?;
    assert_eq!(tasks.get(id).unwrap().state, TaskState::Cancelled);
    assert_eq!(tasks.snapshot("other").len(), 0);
    let cursor = tasks.events("window", 0).cursor;
    checked(tasks.retire(id))?;
    let events = tasks.events("window", cursor);
    assert!(events.reset && events.snapshot.is_empty());
    Ok(())
}
fn archive() -> Check {
    use pm_promedia::storage::*;
    let scratch = Scratch::new()?;
    let settings = StorageSettings {
        locations: [
            StorageClass::Originals,
            StorageClass::Proxies,
            StorageClass::Optimized,
            StorageClass::Renders,
        ]
        .into_iter()
        .map(|class| {
            (
                class,
                Location {
                    directory: scratch.0.join(format!("{class:?}")),
                    quota_bytes: 1024,
                },
            )
        })
        .collect(),
        archive: scratch.0.join("archive"),
    };
    let storage = checked(Storage::open(settings.clone()))?;
    for class in [
        StorageClass::Originals,
        StorageClass::Proxies,
        StorageClass::Optimized,
    ] {
        let path = checked(storage.put(class, "source.bytes", b"synthetic retained media"))?;
        assert!(storage.purge(class, "source.bytes", true).is_err());
        assert_eq!(checked(std::fs::read(&path))?, b"synthetic retained media");
        let archived = checked(storage.purge(class, "source.bytes", false))?;
        assert!(!path.exists());
        assert_eq!(
            checked(std::fs::read(archived))?,
            b"synthetic retained media"
        );
    }
    drop(storage);
    let storage = checked(Storage::open(settings))?;
    assert_eq!(
        checked(storage.usage(StorageClass::Originals))?.active_bytes,
        0
    );
    Ok(())
}
fn proxy() -> Check {
    use pm_promedia::media::*;
    let info = MediaInfo {
        width: 1920,
        height: 1080,
        frame_rate: Rational {
            numerator: 30000,
            denominator: 1001,
        },
        time_base: Rational {
            numerator: 1,
            denominator: 30000,
        },
        start_tick: -10,
        duration_ticks: 30030,
        alpha: true,
        audio_channels: 2,
        audio_sample_rate: 48000,
        colour: "bt709/srgb/full".into(),
    };
    let mut catalog = checked(MediaCatalog::new("project".into()))?;
    checked(catalog.register("source".into(), [1; 32], info.clone()))?;
    let profile = Profile {
        width: 960,
        height: 540,
        codec: "synthetic-alpha".into(),
        container: "mov".into(),
    };
    let mut proxy_info = info.clone();
    proxy_info.width = 960;
    proxy_info.height = 540;
    let artifact = Artifact {
        path: "/synthetic/proxy.mov".into(),
        fingerprint: [2; 32],
        info: proxy_info,
    };
    checked(catalog.attach(
        "source",
        RenditionKind::Proxy,
        Rendition {
            source: [1; 32],
            profile,
            artifact: artifact.clone(),
            online: true,
        },
    ))?;
    let status = checked(catalog.status("source"))?;
    assert_eq!(status.availability, Availability::ProxyOnly);
    assert!(status.export_warning);
    assert!(catalog.resolve("source", None).is_err());
    assert_eq!(
        checked(catalog.resolve("source", Some(RenditionKind::Proxy)))?,
        &artifact
    );
    let mut original = Artifact {
        path: "/synthetic/original.mov".into(),
        fingerprint: [3; 32],
        info,
    };
    assert!(catalog.relink_original("source", original.clone()).is_err());
    assert_eq!(
        checked(catalog.status("source"))?.availability,
        Availability::ProxyOnly
    );
    original.fingerprint = [1; 32];
    checked(catalog.relink_original("source", original.clone()))?;
    assert_eq!(checked(catalog.resolve("source", None))?, &original);
    assert!(!checked(catalog.status("source"))?.export_warning);
    assert_eq!(
        checked(catalog.resolve("source", None))?.info.frame_rate,
        Rational {
            numerator: 30000,
            denominator: 1001
        }
    );
    Ok(())
}
fn cache() -> Check {
    use pm_promedia::cache::*;
    let scratch = Scratch::new()?;
    let root = scratch.0.join("renders");
    let key = RenderKey {
        graph: [1; 32],
        media: [2; 32],
        ticks: 1001,
        width: 2,
        height: 1,
        format: "rgba16f-linear-bt709-premul".into(),
        renderer_version: "synthetic".into(),
    };
    let cache = checked(RenderCache::open(&root, 1024))?;
    checked(cache.put(&key, b"synthetic frame"))?;
    checked(cache.keep_rendered(&key, true))?;
    drop(cache);
    let cache = checked(RenderCache::open(&root, 1024))?;
    assert_eq!(checked(cache.state(&key, false))?, BarState::Kept);
    assert!(cache.trim_to(0).is_err());
    let mut edited = key.clone();
    edited.graph[0] = 3;
    assert_eq!(checked(cache.state(&edited, false))?, BarState::Missing);
    assert_eq!(checked(cache.get(&key))?, Some(b"synthetic frame".to_vec()));
    assert!(cache.put(&key, b"different bytes").is_err());
    Ok(())
}
fn raster() -> Check {
    use pm_recorder::*;
    let mut canvas = checked(Canvas::new(17, 2, 1))?;
    checked(canvas.set_fill_color("#ff0000"))?;
    checked(canvas.fill_rect(0., 0., 1., 1.))?;
    assert_eq!(
        checked(canvas.get_image_data(0, 0, 2, 1))?.bytes(),
        &[255, 0, 0, 255, 0, 0, 0, 0]
    );
    assert_eq!(
        checked(canvas.get_image_data(-1, 0, 2, 1))?.bytes(),
        &[0, 0, 0, 0, 255, 0, 0, 255]
    );
    let mut sources = Sources::default();
    let source = checked(sources.insert(
        SourceKey { id: 99, version: 1 },
        checked(ImageData::from_rgba(1, 1, vec![0, 255, 0, 255]))?,
    ))?;
    let mut target = checked(Canvas::new(18, 1, 1))?;
    checked(target.draw_image(source, Rect::new(0., 0., 1., 1.), Rect::new(0., 0., 1., 1.)))?;
    sources.retire(SourceKey { id: 99, version: 1 });
    drop(sources);
    assert_eq!(
        checked(target.get_image_data(0, 0, 1, 1))?.bytes(),
        &[0, 255, 0, 255]
    );
    Ok(())
}
fn remote() -> Check {
    use pm_remote_dom::{protocol::*, *};
    struct Queue<T> {
        values: Vec<T>,
        full: bool,
    }
    impl<T> TrySink<T> for Queue<T> {
        fn try_send(&mut self, value: T) -> Result<(), Error> {
            if self.full {
                return Err(Error::Backpressure);
            }
            self.values.push(value);
            Ok(())
        }
    }
    let session = Session {
        panel: "synthetic".into(),
        generation: 1,
    };
    let mut router = checked(Router::new(session.clone(), Limits::default()))?;
    let batch = Batch {
        version: VERSION,
        session: session.clone(),
        seq: 1,
        caused_by: None,
        ops: vec![
            Op::Create {
                id: 6,
                kind: NodeKind::Element,
                name: "input".into(),
                namespace: None,
            },
            Op::Insert {
                parent: BODY,
                id: 6,
                before: None,
            },
            Op::Listen {
                id: 6,
                event: "input".into(),
                enabled: true,
                policy: EventPolicy::Observe,
            },
        ],
    };
    let mut queue = Queue {
        values: vec![],
        full: true,
    };
    assert!(router.route_batch(batch.clone(), &mut queue).is_err());
    assert_eq!(router.inspect().sent, 0);
    queue.full = false;
    checked(router.route_batch(batch, &mut queue))?;
    assert_eq!(queue.values.len(), 1);
    let event = PageMessage::Event {
        session,
        seq: 1,
        applied: 1,
        target: 6,
        event: "input".into(),
        bubbles: true,
        cancelable: false,
        default_prevented: false,
        data: Default::default(),
    };
    let mut input = Queue {
        values: vec![],
        full: true,
    };
    assert!(router.route_page(event.clone(), &mut input).is_err());
    assert_eq!(router.inspect().last_event, 0);
    input.full = false;
    checked(router.route_page(event.clone(), &mut input))?;
    assert!(router.route_page(event, &mut input).is_err());
    assert_eq!(input.values.len(), 1);
    Ok(())
}
/// Admit the actual shim's V8 output through the production Rust router. The
/// page event is synthetic; WebKit application/paint is still a separate gate.
pub fn admit_remote_evidence(
    evidence: serde_json::Value,
    deliver: impl FnOnce(serde_json::Value) -> Result<serde_json::Value, String>,
) -> Check {
    use pm_remote_dom::{protocol::*, *};
    let batches: Vec<Batch> = checked(serde_json::from_value(evidence["batches"].clone()))?;
    assert_eq!(batches.len(), 1);
    let mut router = checked(Router::new(batches[0].session.clone(), Limits::default()))?;
    let (mut page, page_receiver) = checked(bounded_channel(2))?;
    checked(router.route_batch(batches[0].clone(), &mut page))?;
    assert_eq!(checked(page_receiver.try_recv())?.seq, 1);
    let event = PageMessage::Event {
        session: batches[0].session.clone(),
        seq: 1,
        applied: 1,
        target: evidence["input"].as_u64().ok_or("input identity missing")?,
        event: "input".into(),
        bubbles: true,
        cancelable: false,
        default_prevented: false,
        data: [("state".into(), json!({"value":"Authored λ text"}))].into(),
    };
    let (mut realm, realm_receiver) = checked(bounded_channel(1))?;
    checked(router.route_page(event, &mut realm))?;
    let forwarded = checked(realm_receiver.try_recv())?;
    let response = deliver(checked(serde_json::to_value(forwarded))?)?;
    let returned: Vec<Batch> = checked(serde_json::from_value(response["batches"].clone()))?;
    assert_eq!(returned.len(), 2);
    assert_eq!(
        checked(serde_json::to_value(&returned[0]))?,
        checked(serde_json::to_value(&batches[0]))?
    );
    checked(router.route_batch(returned[1].clone(), &mut page))?;
    let output = checked(page_receiver.try_recv())?;
    assert_eq!(output.caused_by, Some(1));
    assert!(
        output
            .ops
            .iter()
            .any(|op| matches!(op,Op::Text{value,..} if value=="Authored λ text"))
    );
    assert_eq!(router.inspect().last_event, 1);
    assert_eq!(router.inspect().sent, 2);
    Ok(())
}
fn surface() -> Check {
    use pm_surface::{binding::*, *};
    #[derive(Default)]
    struct Sink {
        values: Vec<Edit>,
        full: bool,
    }
    impl BindingSink for Sink {
        fn enqueue(&mut self, edit: Edit) -> Result<(), String> {
            if self.full {
                return Err("synthetic queue full".into());
            }
            self.values.push(edit);
            Ok(())
        }
    }
    let mut surface = Surface::default();
    checked(surface.apply_json(&json!({"baseRevision":0,"ops":[{"op":"create","id":1,"type":"input"},{"op":"insertBefore","parent":0,"id":1,"before":null},{"op":"setProps","id":1,"patch":{"control":"number","binding":"opacity","value":12.3456789012345}}]}).to_string()))?;
    let mut bindings = Bindings::default();
    let mut sink = Sink::default();
    checked(bindings.begin(&surface, 1, &mut sink))?;
    checked(bindings.write(1, json!(12.3456789012345), &mut sink))?;
    let revision = surface.revision();
    assert!(surface.apply_json(&json!({"baseRevision":revision,"ops":[{"op":"setProps","id":1,"patch":{"value":99}},{"op":"remove","id":999}]}).to_string()).is_err());
    assert_eq!(surface.revision(), revision);
    assert_eq!(
        surface.node(1).unwrap().props["value"],
        json!(12.3456789012345)
    );
    checked(
        surface.apply_json(
            &json!({"baseRevision":revision,"ops":[{"op":"remove","id":1}]}).to_string(),
        ),
    )?;
    sink.full = true;
    assert!(bindings.reconcile(&surface, &mut sink).is_err());
    sink.full = false;
    checked(bindings.reconcile(&surface, &mut sink))?;
    assert_eq!(
        sink.values,
        vec![
            Edit::Begin {
                binding: "opacity".into()
            },
            Edit::Write {
                binding: "opacity".into(),
                value: json!(12.3456789012345)
            },
            Edit::Cancel {
                binding: "opacity".into()
            }
        ]
    );
    Ok(())
}
fn store_view() -> Check {
    use pm_store_host::{identity::*, view::*};
    let id = checked(ExtensionId::new("synthetic-ext"))?;
    let identity = checked(DataStoreIdentity::new("synthetic-profile", id.clone()))?;
    let token = ViewToken {
        data_store: identity.clone(),
        view: "panel".into(),
        generation: 1,
    };
    let (mut view, create) = checked(View::start(token, 0, RecoveryPolicy::default()))?;
    assert!(matches!(create, Action::Create { .. }));
    checked(view.ready(1, 1))?;
    assert!(matches!(
        checked(view.fail(1, Failure::Crash, 2))?,
        Action::Seal { .. }
    ));
    assert!(matches!(checked(view.sealed(1, 3))?, Action::Stop { .. }));
    checked(view.stopped(1, 4))?;
    checked(view.restart(2, 1000))?;
    assert!(view.ready(1, 1001).is_err());
    checked(view.ready(2, 1001))?;
    assert_eq!(view.token().data_store, identity);
    assert_eq!(
        checked(DataStoreIdentity::new("synthetic-profile", id.clone()))?,
        identity
    );
    assert_ne!(
        checked(DataStoreIdentity::new("other-profile", id))?.key,
        identity.key
    );
    assert_eq!(identity.kernel_storage_key(), "ext.synthetic-ext");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    macro_rules! cases{($($name:ident=>$id:literal),*$(,)?)=>{$(#[test]fn $name(){check($id).unwrap().unwrap();})*}}
    cases! {entry_retry=>"W001.service-entry",storage_reopen=>"W012.service-storage",task_cancellation=>"W019.background-tasks",archive_never_delete=>"W022.media-archive",cache_invalidation=>"W077.render-cache",raster_pixels=>"W040.native-raster",remote_backpressure=>"W040.remote-router",surface_cancel=>"W032.native-surface",store_recovery=>"W038.store-view",proxy_identity=>"W022.proxy-identity"}
    #[test]
    fn unknown_is_not_a_pass() {
        assert!(check("unknown").is_none());
    }
}
