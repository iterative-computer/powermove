//! V8 to WebKit, through the production host and page. Runs on the process main
//! thread with no window, activation or Dock icon, so it never takes focus.
#[cfg(target_os = "macos")]
mod mac {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSString};
    use pm_compat_host::*;
    use pm_compat_runner::{
        PAGE_BUNDLE,
        host::{ExtensionHost, HostConfig},
    };
    use pm_kernel::{KernelActor, KernelConfig, OpenSource};
    use serde_json::{Value, json};
    use std::{
        cell::RefCell,
        fs,
        rc::Rc,
        sync::{Arc, mpsc},
        time::{Duration, Instant},
    };
    struct Page(mpsc::Sender<Value>, RefCell<Vec<HostEvent>>);
    impl SyncHandler for Page {
        fn call(&self, _: &RequestContext, _: &str, _: Value) -> Result<Value> {
            Err(Error::Forbidden)
        }
        fn event(&self, event: HostEvent) {
            self.1.borrow_mut().push(event);
        }
        fn remote_dom(&self, _: &RequestContext, payload: Value) -> Result<()> {
            self.0.send(payload).map_err(|_| Error::StaleGeneration)
        }
    }
    const EXTENSION: &str = "
        export default function(api){
          api.panels.register({id:'synthetic.panel',title:'Synthetic',build(root){
            const input=document.createElement('input');input.id='name';
            const out=document.createElement('output');out.id='echo';out.textContent=api.project.get().name;
            const canvas=document.createElement('canvas');canvas.id='art';canvas.width=4;canvas.height=4;
            const x=canvas.getContext('2d');x.fillStyle='#ff0000';x.fillRect(0,0,4,4);
            input.addEventListener('input',()=>{out.textContent=input.value});
            root.append(input,out,canvas);
          }});
        }";
    const STORE_EXTENSION: &str = "
        export default function(api){
          api.panels.register({id:'view',title:'Confined',build(root){
            const input=document.createElement('input');input.id='store-name';
            const out=document.createElement('output');out.id='store-echo';out.textContent='confined '+(typeof PM);
            input.addEventListener('input',()=>{out.textContent=input.value});
            root.append(input,out);
          }});
        }";
    /// A Store helper's view: its closures in the confined process, its document
    /// in a WebKit page of its own.
    fn store_view(
        app: &NSApplication,
        mtm: MainThreadMarker,
        host: &ExtensionHost,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        const ID: &str = "synthetic-view";
        const PANEL_ID: &str = "synthetic-view.view";
        let deadline = Instant::now() + Duration::from_secs(30);
        let session = loop {
            if let Some(session) = host.store_session(ID) {
                break session;
            }
            if Instant::now() > deadline {
                return Err(format!("no Store session: {:?}", host.records()).into());
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let (feedback, messages) = mpsc::channel();
        let page = Rc::new(Page(feedback, RefCell::new(vec![])));
        let mut web = MacDomHost::new_store_remote_dom(
            mtm,
            &session,
            page.clone(),
            &mut StoreDataStores::default(),
            PAGE_BUNDLE,
        )?;
        web.create(PanelSpec {
            id: 1,
            label: "Confined".into(),
        })?;
        let viewport = Rect::new(0., 0., 640., 480.);
        web.attach(
            1,
            PanelSlot {
                id: 1,
                bounds: viewport,
                clip: viewport,
                reading_order: 0,
            },
        )?;
        web.present(viewport)?;
        // The helper run this page draws, as the app reads it from contributions.
        let view = loop {
            let view = host.contributions().and_then(|c| {
                c["panels"]
                    .as_array()?
                    .iter()
                    .find(|p| p["id"] == PANEL_ID)?["view"]
                    .as_u64()
            });
            if let Some(view) = view {
                break view;
            }
            if Instant::now() > deadline {
                return Err("no Store panel contribution".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let sink = host.store_page_sink(ID, view);
        let (mut ready, mut opened, mut stage, mut ticket) = (false, false, 0, None);
        while Instant::now() < deadline {
            if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::dateWithTimeIntervalSinceNow(0.005)),
                unsafe { NSDefaultRunLoopMode },
                true,
            ) {
                app.sendEvent(&event);
            }
            web.tick();
            for event in page.1.borrow_mut().drain(..) {
                match event {
                    HostEvent::Ready { .. } => ready = true,
                    HostEvent::Failure(reason) => return Err(reason.into()),
                    _ => (),
                }
            }
            while let Ok(message) = messages.try_recv() {
                sink(message);
            }
            if ready && !opened {
                host.set_panel(PANEL_ID, true);
                host.place_panel(PANEL_ID, Some([10., 20., 300., 200.]));
                opened = true;
            }
            if ready {
                let mut batches = host.take_store_batches(ID, view);
                while let Some(batch) = batches.front() {
                    match batch.present(&mut web) {
                        Ok(()) => {
                            batches.pop_front();
                        }
                        Err(Error::Backpressure) => break,
                        Err(error) => return Err(error.into()),
                    }
                }
                host.return_store_batches(ID, view, batches);
            }
            if let Some(reply) = ticket.as_ref().and_then(CaptureTicket::try_take) {
                ticket = None;
                let text = reply.unwrap_or(Value::Null);
                match stage {
                    0 if text == "confined undefined" => {
                        println!("store_panel_rendered_in_its_own_webkit_page=1");
                        stage = 1;
                        unsafe {
                            web.web_view().evaluateJavaScript_completionHandler(
                                &NSString::from_str("const i=document.querySelector('#store-name');i.value='typed in Store';i.dispatchEvent(new InputEvent('input',{bubbles:true}));0"),
                                None,
                            );
                        }
                    }
                    1 if text == "typed in Store" => {
                        println!("store_event_ran_confined_closure=1");
                        web.destroy(1)?;
                        return Ok(());
                    }
                    _ => (),
                }
            }
            if ready && opened && ticket.is_none() {
                ticket = web.inspect_capture(1, "#store-echo", false).ok();
            }
        }
        Err(format!("Store view timed out at stage {stage}, ready={ready}").into())
    }
    pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mtm = MainThreadMarker::new().ok_or("main thread required")?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        app.finishLaunching();
        let root = tempfile::tempdir()?;
        let directory = root.path().join("extensions/synthetic-dom");
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join("manifest.json"),
            json!({"id":"synthetic-dom","name":"Synthetic","version":"1.0.0","apiVersion":3})
                .to_string(),
        )?;
        fs::write(directory.join("index.js"), EXTENSION)?;
        fs::write(
            directory.join("manifest.json"),
            json!({"id":"synthetic-dom","name":"Synthetic","version":"1.0.0","apiVersion":3,"entry":"index.js"})
                .to_string(),
        )?;
        let store = root.path().join("extensions/synthetic-view");
        fs::create_dir_all(&store)?;
        fs::write(
            store.join("manifest.json"),
            json!({"id":"synthetic-view","name":"Confined","version":"1.0.0","apiVersion":3,"entry":"index.js","contributes":["panels"]})
                .to_string(),
        )?;
        fs::write(store.join("index.js"), STORE_EXTENSION)?;
        fs::write(
            store.join(".powermove-store.json"),
            json!({"repoId":"r","releaseId":"synthetic"}).to_string(),
        )?;
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let initial = pm_compat_runner::initial();
        let document = pm_model::json::parse(&initial["document"].to_string())
            .map_err(|_| "synthetic document")?;
        std::thread::scope(|scope| {
            let k = kernel.clone();
            scope
                .spawn(move || k.open("window", OpenSource::Untitled(document)).wait())
                .join()
        })
        .map_err(|_| "open panicked")?
        .ok_or("kernel stopped")?
        .map_err(|e| e.to_string())?;
        let host = ExtensionHost::start(HostConfig {
            kernel: kernel.clone(),
            project: "window".into(),
            roots: vec![root.path().join("extensions")],
            cache: root.path().join("cache"),
            owner: None,
            text: None,
            wake: Arc::new(|| ()),
            frames: None,
        });
        let (feedback, messages) = mpsc::channel();
        let page = Rc::new(Page(feedback, RefCell::new(vec![])));
        let mut web = MacDomHost::new_remote_dom(
            mtm,
            Trust::Trusted,
            page.clone(),
            &mut StoreDataStores::default(),
            PAGE_BUNDLE,
        )?;
        web.create(PanelSpec {
            id: 1,
            label: "Extension panels".into(),
        })?;
        let viewport = Rect::new(0., 0., 640., 480.);
        web.attach(
            1,
            PanelSlot {
                id: 1,
                bounds: viewport,
                clip: viewport,
                reading_order: 0,
            },
        )?;
        web.present(viewport)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        // The page belongs to one realm generation, as in the app.
        while host.view() == 0 {
            if Instant::now() > deadline {
                return Err("no realm".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let view = host.view();
        let sink = host.page_sink(view);
        let mut ready = false;
        let mut opened = false;
        let mut stage = 0;
        let mut ticket = None;
        let script = |web: &MacDomHost, source: &str| unsafe {
            web.web_view()
                .evaluateJavaScript_completionHandler(&NSString::from_str(source), None);
        };
        while Instant::now() < deadline {
            if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::dateWithTimeIntervalSinceNow(0.005)),
                unsafe { NSDefaultRunLoopMode },
                true,
            ) {
                app.sendEvent(&event);
            }
            web.tick();
            for event in page.1.borrow_mut().drain(..) {
                match event {
                    HostEvent::Ready { .. } => ready = true,
                    HostEvent::Failure(reason) => return Err(reason.into()),
                    _ => (),
                }
            }
            while let Ok(message) = messages.try_recv() {
                sink(message);
            }
            if host.is_ready() && !opened {
                host.set_panel("synthetic.panel", true);
                host.place_panel("synthetic.panel", Some([10., 20., 300., 200.]));
                opened = true;
            }
            if ready {
                let mut batches = host.take_batches(view);
                while let Some(batch) = batches.front() {
                    match batch.present(&mut web) {
                        Ok(()) => {
                            batches.pop_front();
                        }
                        Err(Error::Backpressure) => break,
                        Err(error) => return Err(error.into()),
                    }
                }
                host.return_batches(view, batches);
                for frame in host.take_canvases() {
                    web.present_canvas(1, &frame)?;
                }
            }
            if let Some(reply) = ticket.as_ref().and_then(CaptureTicket::try_take) {
                ticket = None;
                // The node appears only once its batch applied: keep polling.
                let text = reply.unwrap_or(Value::Null);
                match stage {
                    0 if text == "Synthetic extension check" => {
                        println!("v8_panel_rendered_in_webkit=1");
                        stage = 1;
                        script(
                            &web,
                            "const i=document.querySelector('#name');i.value='typed in WebKit';i.dispatchEvent(new InputEvent('input',{bubbles:true}));0",
                        );
                    }
                    1 if text == "typed in WebKit" => {
                        println!("webkit_event_ran_v8_closure=1");
                        stage = 2;
                    }
                    2 if text == "[255,0,0,255]" => {
                        println!("recorder_canvas_painted_in_webkit=1");
                        web.destroy(1)?;
                        store_view(&app, mtm, &host)?;
                        drop(host);
                        std::thread::spawn(move || actor.shutdown());
                        return Ok(());
                    }
                    _ => (),
                }
            }
            if ready && opened && ticket.is_none() {
                let selector = match stage {
                    0 | 1 => Some(("#echo", false)),
                    _ => None,
                };
                if let Some((selector, value)) = selector {
                    ticket = web.inspect_capture(1, selector, value).ok();
                } else {
                    ticket = web.inspect_capture(1, "#art-pixel", false).ok();
                    script(
                        &web,
                        "{const c=document.querySelector('#art');let o=document.querySelector('#art-pixel');if(!o){o=document.createElement('output');o.id='art-pixel';document.documentElement.append(o)}o.textContent=JSON.stringify([...c.getContext('2d').getImageData(0,0,1,1).data]);0}",
                    );
                }
            }
        }
        Err(format!("timed out at stage {stage}, ready={ready}, opened={opened}").into())
    }
}
fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = objc2::rc::autoreleasepool(|_| mac::run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
