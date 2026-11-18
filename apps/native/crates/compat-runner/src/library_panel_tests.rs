// PM-509: Library's enable uses this same kernel transaction. Collect the
// production host's admitted DOM, with no AppKit windows or real profile.
#[derive(Default)]
struct LibraryDomBatches(Vec<Value>);
impl pm_compat_host::DomPanelHost for LibraryDomBatches {
    fn generation(&self) -> u64 {
        1
    }
    fn create(&mut self, _: pm_compat_host::PanelSpec) -> pm_compat_host::Result<()> {
        unreachable!()
    }
    fn attach(&mut self, _: u64, _: pm_compat_host::PanelSlot) -> pm_compat_host::Result<()> {
        unreachable!()
    }
    fn detach(&mut self, _: u64) -> pm_compat_host::Result<()> {
        unreachable!()
    }
    fn destroy(&mut self, _: u64) -> pm_compat_host::Result<()> {
        unreachable!()
    }
    fn focus(&mut self, _: u64, _: Option<u64>) -> pm_compat_host::Result<()> {
        unreachable!()
    }
    fn apply(&mut self, _: u64, _: u64, batch: Value) -> pm_compat_host::Result<()> {
        self.0.push(batch);
        Ok(())
    }
}
fn library_enable_panel_probe(store: bool) {
    let root = tempfile::tempdir().unwrap();
    let copied = (!store)
        .then(|| std::env::var("PM_LIBRARY_ANCHOR_COPY").ok())
        .flatten();
    let owner = if copied.is_some() {
        "anchor-point"
    } else {
        "synthetic-library"
    };
    let source = if store {
        "export default function(api){api.commands.register({id:'reload',label:'Reload own helper',run(){return globalThis.__storeCheck.rpc.call('reload')}});api.panels.register({id:'controls',title:'Synthetic Notes',build(body){const note=document.createElement('textarea');note.value='Synthetic library controls';note.oninput=()=>api.storage.set('note',note.value);body.append(note);const label=document.createElement('p');label.textContent='Synthetic library controls';body.append(label);}})}"
    } else {
        "export default function(api){api.panels.register({id:'controls',title:'Synthetic Anchor',build(body){const root=document.createElement('div');const style=document.createElement('style');style.textContent='.synthetic-grid{display:grid;grid-template-columns:repeat(3,1fr)}';root.className='synthetic-grid';root.append(style);for(let i=0;i<9;i++){const b=document.createElement('button');b.textContent=String(i);root.append(b);}const label=document.createElement('p');label.textContent='Synthetic library controls';root.append(label);body.append(root);}})}"
    };
    let extensions = root.path().join("extensions");
    let copied_source = copied
        .as_ref()
        .map(|path| fs::read_to_string(Path::new(path).join("index.ts")).unwrap());
    extension(
        &extensions,
        owner,
        &[("index.ts", copied_source.as_deref().unwrap_or(source))],
    );
    if store {
        fs::write(
            extensions.join("synthetic-library/.powermove-store.json"),
            r#"{"repoId":"synthetic-library","releaseId":"synthetic"}"#,
        )
        .unwrap();
    } else if let Some(path) = &copied {
        fs::copy(
            Path::new(path).join("manifest.json"),
            extensions.join(owner).join("manifest.json"),
        )
        .unwrap();
    } else {
        fs::write(extensions.join("synthetic-library/manifest.json"), json!({"id":"synthetic-library","name":"Synthetic Anchor","version":"1.0.0","apiVersion":1}).to_string()).unwrap();
    }
    let fixture = Fixture::start(root);
    let panel = if copied.is_some() {
        "anchor-point"
    } else if store {
        "synthetic-library.controls"
    } else {
        "controls"
    };
    fixture.until("Library initial registration", |host| {
        host.is_ready() && contributes(&host.publication(), "panels", panel)
    });
    let turn = |on| {
        let tx = pm_kernel::state::switch_extension(&fixture.kernel.state(), owner, on).unwrap();
        fixture
            .kernel
            .state_transaction(tx)
            .wait()
            .unwrap()
            .unwrap();
    };
    turn(false);
    fixture.until("Library off", |host| {
        host.is_ready() && !contributes(&host.publication(), "panels", panel)
    });
    fixture.host.set_panel(panel, true);
    fixture
        .host
        .place_panel(panel, Some([220., 40., 218., 300.]));
    turn(true);
    fixture.until("Library enabled", |host| {
        host.is_ready() && contributes(&host.publication(), "panels", panel)
    });
    let sentinel = if copied.is_some() {
        "Center new layers"
    } else {
        "Synthetic library controls"
    };
    let captured = std::cell::RefCell::new(LibraryDomBatches::default());
    let paints = |host: &ExtensionHost| {
        let batches = if store {
            let publication = host.publication();
            let view = publication.contributions.as_ref().unwrap()["panels"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == panel)
                .unwrap()["view"]
                .as_u64()
                .unwrap();
            host.take_store_batches("synthetic-library", view)
        } else {
            host.take_batches(host.view())
        };
        let mut captured = captured.borrow_mut();
        for batch in batches {
            batch.present(&mut *captured).unwrap();
        }
        captured.0.iter().any(|b| {
            b["ops"]
                .as_array()
                .unwrap()
                .iter()
                .any(|op| op["op"] == "text" && op["value"] == sentinel)
        })
    };
    fixture.until("Library enabled panel publishes its body", &paints);
    if store {
        let view = |host: &ExtensionHost| {
            host.publication()
                .contributions
                .as_ref()
                .and_then(|c| c["panels"].as_array())
                .and_then(|panels| panels.iter().find(|p| p["id"] == panel))
                .and_then(|p| p["view"].as_u64())
        };
        let before = view(&fixture.host).unwrap();
        captured.borrow_mut().0.clear();
        // Same owner-scoped RPC as the error view's Reload button.
        fixture
            .host
            .run_command("synthetic-library.reload", json!([]));
        fixture.until("Reload retires the helper generation", |host| {
            host.is_ready() && view(host).is_some_and(|now| now != before)
        });
        fixture.until("Reload republishes the open panel body", &paints);
    }
    captured.borrow_mut().0.clear();
    fixture.host.set_panel(panel, false);
    fixture.host.set_panel(panel, true);
    fixture
        .host
        .place_panel(panel, Some([320., 60., 218., 300.]));
    fixture.until("Dock menu reopen publishes its body", &paints);
    if let Ok(dir) = std::env::var("PM_LIBRARY_PROBE_CAPTURE") {
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            Path::new(&dir).join(if store {
                "library-store-dom.json"
            } else {
                "library-local-dom.json"
            }),
            serde_json::to_vec(&captured.borrow().0).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn library_local_enable_and_dock_reopen_publish_panel_content() {
    library_enable_panel_probe(false);
}
#[test]
fn library_store_enable_and_dock_reopen_publish_panel_content() {
    library_enable_panel_probe(true);
}
