use crate::{contract::Panels, transport, *};
use block2::DynBlock;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::*;
use objc2_web_kit::*;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    io::Read,
    rc::Rc,
    time::{Duration, Instant},
};

type Task = Retained<ProtocolObject<dyn WKURLSchemeTask>>;
#[derive(Default)]
pub struct StoreDataStores {
    stores: BTreeMap<String, Retained<WKWebsiteDataStore>>,
}
impl StoreDataStores {
    /// Call once per app. Keeping this registry alive preserves extension storage
    /// across hosts; distinct extensions never receive the same data-store object.
    fn get_for(
        &mut self,
        trust: &Trust,
        identity: Option<&pm_store_host::identity::DataStoreIdentity>,
    ) -> Retained<WKWebsiteDataStore> {
        match trust {
            Trust::Trusted => unsafe {
                WKWebsiteDataStore::nonPersistentDataStore(MainThreadMarker::new().unwrap())
            },
            Trust::Store { extension, .. } => self
                .stores
                .entry(
                    identity
                        .map(|id| format!("profile:{}", id.key))
                        .unwrap_or_else(|| format!("legacy:{extension}")),
                )
                .or_insert_with(|| unsafe {
                    WKWebsiteDataStore::nonPersistentDataStore(MainThreadMarker::new().unwrap())
                })
                .clone(),
        }
    }
}
struct Recovery {
    old_pid: i32,
    started: Instant,
    termination: bool,
}
struct Shared {
    trust: Trust,
    handler: Rc<dyn SyncHandler>,
    methods: BTreeMap<String, String>,
    channel: RefCell<Channel>,
    panels: RefCell<Panels>,
    pending: RefCell<Option<(Task, u64)>>,
    ready: Cell<bool>,
    recovery: RefCell<Option<Recovery>>,
    popouts: RefCell<Vec<(Retained<NSWindow>, Retained<WKWebView>)>>,
    geometry_dirty: Cell<bool>,
    closed: Cell<bool>,
    canvases: RefCell<crate::canvas::CanvasResources>,
    /// Faces the page may load by key (`font` route).
    fonts: RefCell<BTreeMap<String, std::sync::Arc<[u8]>>>,
}
impl Shared {
    fn send(&self, event: Value) -> Result<()> {
        let generation = self.channel.borrow().generation;
        self.channel.borrow_mut().send(generation, event)?;
        let pending = self.pending.borrow_mut().take();
        if let Some((task, after)) = pending {
            let response = self.channel.borrow_mut().poll(after);
            match response {
                Ok(Some(value)) => reply(&task, &value),
                Ok(None) => *self.pending.borrow_mut() = Some((task, after)),
                Err(_) => fail_task(&task),
            }
        }
        Ok(())
    }
    fn handle(&self, payload: &str) -> Result<Value> {
        let m: Value = serde_json::from_str(payload).map_err(|_| Error::InvalidRequest)?;
        let generation = m["generation"].as_u64().ok_or(Error::InvalidRequest)?;
        if generation != self.channel.borrow().generation || self.closed.get() {
            return Err(Error::StaleGeneration);
        }
        let id = m["panel"].as_u64().ok_or(Error::InvalidRequest)?;
        let kind = m["kind"].as_str().ok_or(Error::InvalidRequest)?;
        if kind == "ready" {
            if self.ready.get() {
                return Err(Error::InvalidRequest);
            }
            if self.trust.is_store() && m["payload"]["origin"] != "null" {
                self.closed.set(true);
                self.handler
                    .event(HostEvent::Failure("compat_store_origin_not_opaque"));
                return Err(Error::Forbidden);
            }
            self.ready.set(true);
            // Rebuild roots before asking the bridge for a fresh node snapshot.
            let definitions: Vec<_> = self
                .panels
                .borrow()
                .items
                .values()
                .map(|p| (p.spec.clone(), p.slot.clone()))
                .collect();
            for (p, slot) in definitions {
                self.send(json!({"kind":"create","panel":p.id,"label":p.label}))?;
                self.send(json!({"kind":"slot","panel":p.id,"slot":slot}))?;
            }
            self.handler.event(HostEvent::Ready { generation });
            return Ok(Value::Null);
        }
        if kind == "failure" {
            self.handler
                .event(HostEvent::Failure("compat_applier_stopped"));
            return Ok(Value::Null);
        }
        if kind == "fonts_loaded" {
            self.handler.event(HostEvent::FontsLoaded);
            return Ok(Value::Null);
        }
        if !self.panels.borrow().items.contains_key(&id) {
            return Err(Error::UnknownPanel);
        }
        let context = RequestContext {
            trust: self.trust.clone(),
            generation,
            panel: id,
        };
        let payload = m["payload"].clone();
        match kind {
            "canvas_applied" => {
                let resource = payload["resource"].as_u64().ok_or(Error::InvalidRequest)?;
                self.canvases.borrow_mut().remove(resource)?;
                if payload["failed"] == true {
                    self.handler
                        .event(HostEvent::Failure("compat_canvas_frame_unavailable"));
                }
                Ok(Value::Null)
            }
            "remote_dom" => {
                // Hidden pages still acknowledge DOM/layout; only input needs a slot.
                if payload["kind"] == "event" && self.panels.borrow().items[&id].slot.is_none() {
                    return Err(Error::Forbidden);
                }
                self.handler.remote_dom(&context, payload)?;
                Ok(Value::Null)
            }
            "overlays" => {
                let rects = serde_json::from_value(payload).map_err(|_| Error::InvalidGeometry)?;
                let old = self.panels.borrow().items[&id].overlays.clone();
                self.panels.borrow_mut().overlays(id, rects)?;
                if old != self.panels.borrow().items[&id].overlays {
                    self.geometry_dirty.set(true);
                    self.handler.event(HostEvent::CompositionChanged)
                }
                Ok(Value::Null)
            }
            "dom" | "key" => {
                if self.panels.borrow().items[&id].slot.is_none() {
                    return Err(Error::Forbidden);
                }
                self.handler.event(if kind == "dom" {
                    HostEvent::Dom { context, payload }
                } else {
                    HostEvent::Key { context, payload }
                });
                Ok(Value::Null)
            }
            "call" => {
                let method = m["method"].as_str().ok_or(Error::InvalidRequest)?;
                if self.trust.is_store() {
                    let permission = self.methods.get(method).ok_or(Error::Forbidden)?;
                    if !self.trust.permits(permission) {
                        return Err(Error::Forbidden);
                    }
                }
                let answer = self.handler.call(&context, method, payload)?;
                if serde_json::to_vec(&answer)
                    .map_err(|_| Error::InvalidRequest)?
                    .len()
                    > MAX_MESSAGE_BYTES
                {
                    return Err(Error::Backpressure);
                }
                Ok(answer)
            }
            _ => Err(Error::InvalidRequest),
        }
    }
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name="PMCompatBridge"]
    #[ivars=Rc<Shared>]
    struct Bridge;
    unsafe impl NSObjectProtocol for Bridge {}
    impl Bridge {
        /// A ui.modal sheet button: click its twin in the realm's DOM, so the
        /// action runs where the extension defined it.
        #[unsafe(method(modalAction:))]
        fn modal_action(&self, sender: &NSButton) {
            let Some(window) = sender.window() else { return };
            let children = self.ivars().popouts.borrow();
            let Some((_, web)) = children.iter().find(|(w, _)| std::ptr::eq(&**w, &*window)) else { return };
            let tag = sender.tag();
            let script = if tag >= 0 {
                format!("document.querySelector('[data-pm-modal-action=\"{tag}\"]')?.click()")
            } else {
                "document.querySelector('[data-pm-modal-dismiss]')?.click()".to_owned()
            };
            unsafe { web.evaluateJavaScript_completionHandler(&NSString::from_str(&script), None) };
        }
    }
    unsafe impl WKURLSchemeHandler for Bridge {
        #[unsafe(method(webView:startURLSchemeTask:))]
        unsafe fn start(&self,_web:&WKWebView,task:&ProtocolObject<dyn WKURLSchemeTask>){
            let s=self.ivars();
            let raw=unsafe{task.request()}.URL().and_then(|u|u.absoluteString()).map(|s|s.to_string()).unwrap_or_default();
            let route=s.channel.borrow().route(&raw);
            match route {
                Ok((route,Some(q))) if route=="canvas"=>{
                    let resource = q.parse::<u64>().ok().and_then(|id|s.canvases.borrow().get(id).ok());
                    match resource {Some((bytes,mime))=>reply_bytes(task,&bytes,&mime),None=>fail_task(task)}
                },
                Ok((route,Some(q))) if route=="font"=>{
                    let bytes=s.fonts.borrow().get(&q).cloned();
                    match bytes {Some(bytes)=>reply_bytes(task,&bytes,"font/sfnt"),None=>fail_task(task)}
                },
                Ok((route,Some(q))) if route=="call"=>{
                    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||s.handle(&q))).unwrap_or(Err(Error::InvalidRequest));
                    let value=match result {Ok(v)=>json!({"ok":true,"value":v}),Err(e)=>json!({"ok":false,"error":e.to_string()})};reply(task,&value);
                },
                Ok((route,Some(q))) if route=="events"=>{
                    let Some(after)=q.parse::<u64>().ok() else {fail_task(task);return};
                    let polled=s.channel.borrow_mut().poll(after);
                    match polled {
                        Ok(Some(v))=>reply(task,&v),
                        Ok(None)=>{
                            // There is exactly one long poll per rendering document.
                            if s.pending.borrow().is_some(){fail_task(task);return}
                            let retained=unsafe{Retained::retain(task as *const _ as *mut ProtocolObject<dyn WKURLSchemeTask>)}.unwrap();
                            *s.pending.borrow_mut()=Some((retained,after));
                        },Err(_)=>fail_task(task)
                    }
                },_=>fail_task(task)
            }
        }
        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn stop(&self,_web:&WKWebView,task:&ProtocolObject<dyn WKURLSchemeTask>){
            let mut pending=self.ivars().pending.borrow_mut();
            if pending.as_ref().is_some_and(|(p,_)|std::ptr::eq(&**p,task)){pending.take();}
        }
    }
    unsafe impl WKNavigationDelegate for Bridge {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn policy(&self,web:&WKWebView,action:&WKNavigationAction,done:&DynBlock<dyn Fn(WKNavigationActionPolicy)>){
            let url=unsafe{action.request()}.URL().and_then(|u|u.absoluteString()).map(|s|s.to_string()).unwrap_or_default();
            let child=self.ivars().popouts.borrow().iter().any(|(_,v)|std::ptr::eq(&**v,web));
            // Only host-created about:blank loads; every external/file/data URL,
            // frame navigation and download is denied. Popouts are trusted only.
            // A ui.modal sheet's child loads about:blank with its frame in the fragment.
            let allow=(url=="about:blank" || (child && url.starts_with("about:blank#pm-modal="))) && (!self.ivars().ready.get() || child || (!self.ivars().trust.is_store() && unsafe{action.targetFrame()}.is_none()));
            done.call((if allow{WKNavigationActionPolicy::Allow}else{WKNavigationActionPolicy::Cancel},));
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        unsafe fn terminated(&self,_web:&WKWebView){
            if let Some(r)=self.ivars().recovery.borrow_mut().as_mut(){r.termination=true;}
            self.ivars().handler.event(HostEvent::ProcessTerminated);
        }
    }
    unsafe impl WKUIDelegate for Bridge {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        unsafe fn popout(&self,web:&WKWebView,cfg:&WKWebViewConfiguration,action:&WKNavigationAction,_features:&WKWindowFeatures)->Option<Retained<WKWebView>>{
            self.make_popout(web,cfg,action)
        }
        #[unsafe(method(webViewDidClose:))]
        unsafe fn child_closed(&self,web:&WKWebView){
            let mut children=self.ivars().popouts.borrow_mut();
            if let Some(i)=children.iter().position(|(_,v)|std::ptr::eq(&**v,web)) {let (w,v)=children.remove(i);unsafe{v.setUIDelegate(None);v.setNavigationDelegate(None)};if let Some(p)=w.sheetParent(){p.endSheet(&w)}
                if let Some(p)=w.parentWindow(){p.removeChildWindow(&w)}w.close();}
        }
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        unsafe fn capture(&self,_web:&WKWebView,_origin:&WKSecurityOrigin,_frame:&WKFrameInfo,_kind:WKMediaCaptureType,done:&DynBlock<dyn Fn(WKPermissionDecision)>){done.call((WKPermissionDecision::Deny,));}
        #[unsafe(method(webView:requestDeviceOrientationAndMotionPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        unsafe fn motion(&self,_web:&WKWebView,_origin:&WKSecurityOrigin,_frame:&WKFrameInfo,done:&DynBlock<dyn Fn(WKPermissionDecision)>){done.call((WKPermissionDecision::Deny,));}
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        unsafe fn files(&self,_web:&WKWebView,_params:&WKOpenPanelParameters,_frame:&WKFrameInfo,done:&DynBlock<dyn Fn(*mut NSArray<NSURL>)>){done.call((std::ptr::null_mut(),));}
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        unsafe fn alert(&self,_web:&WKWebView,_message:&NSString,_frame:&WKFrameInfo,done:&DynBlock<dyn Fn()>){done.call(());}
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        unsafe fn confirm(&self,_web:&WKWebView,_message:&NSString,_frame:&WKFrameInfo,done:&DynBlock<dyn Fn(Bool)>){done.call((Bool::NO,));}
    }
);
impl Bridge {
    fn make_popout(
        &self,
        web: &WKWebView,
        cfg: &WKWebViewConfiguration,
        action: &WKNavigationAction,
    ) -> Option<Retained<WKWebView>> {
        let s = self.ivars();
        if s.trust.is_store() || s.popouts.borrow().len() >= 8 {
            return None;
        }
        let url = unsafe { action.request() }
            .URL()?
            .absoluteString()?
            .to_string();
        if let Some(frame) = url.strip_prefix("about:blank#pm-modal=") {
            return self.make_modal(web, cfg, frame);
        }
        if url != "about:blank" {
            return None;
        }
        let r = NSRect::new(NSPoint::new(140., 140.), NSSize::new(440., 260.));
        let child = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(self.mtm()),
                NSRect::new(NSPoint::ZERO, r.size),
                cfg,
            )
        };
        unsafe {
            child.setUIDelegate(Some(ProtocolObject::from_ref(self)));
            child.setNavigationDelegate(Some(ProtocolObject::from_ref(self)));
        }
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(self.mtm()),
                r,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Powermove · Compat popout"));
        window.setContentView(Some(&child));
        if let Some(parent) = web.window() {
            unsafe {
                parent.addChildWindow_ordered(&window, NSWindowOrderingMode::Above);
            }
        }
        window.makeKeyAndOrderFront(None);
        s.popouts.borrow_mut().push((window, child.clone()));
        s.handler.event(HostEvent::PopoutOpened);
        Some(child)
    }
}

impl Bridge {
    /// ui.modal: a sheet on the extension's window. Its title and buttons are
    /// native; its body is the child web view the page moves the modal's DOM
    /// into. Buttons right-aligned, the primary one last and on Return.
    fn make_modal(
        &self,
        web: &WKWebView,
        cfg: &WKWebViewConfiguration,
        frame: &str,
    ) -> Option<Retained<WKWebView>> {
        let s = self.ivars();
        if s.trust.is_store() || s.popouts.borrow().len() >= 8 {
            return None;
        }
        let parent = web.window();
        let mtm = self.mtm();
        let frame: Value = NSString::from_str(frame)
            .stringByRemovingPercentEncoding()
            .and_then(|text| serde_json::from_str(&text.to_string()).ok())
            .unwrap_or_default();
        let width = frame["width"].as_f64().unwrap_or(420.).clamp(240., 1200.);
        let (pad, title_h, body_h, buttons_h) = (20., 20., 280., 32.);
        let height = pad + title_h + 12. + body_h + 16. + buttons_h + pad;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::ZERO, NSSize::new(width, height)),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let content = window.contentView()?;
        let title = NSTextField::labelWithString(
            &NSString::from_str(frame["title"].as_str().unwrap_or_default()),
            mtm,
        );
        title.setFont(Some(&NSFont::boldSystemFontOfSize(13.)));
        title.setFrame(NSRect::new(
            NSPoint::new(pad, height - pad - title_h),
            NSSize::new(width - 2. * pad, title_h),
        ));
        title.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        content.addSubview(&title);
        let child = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0., pad + buttons_h + 16.),
                    NSSize::new(width, body_h),
                ),
                cfg,
            )
        };
        unsafe {
            child.setUIDelegate(Some(ProtocolObject::from_ref(self)));
            child.setNavigationDelegate(Some(ProtocolObject::from_ref(self)));
            child.setValue_forKey(
                Some(&NSNumber::new_bool(false)),
                &NSString::from_str("drawsBackground"),
            );
        }
        child.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        content.addSubview(&child);
        let mut actions: Vec<(String, bool, isize)> = frame["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, a)| {
                (
                    a["label"].as_str().unwrap_or("OK").to_owned(),
                    a["pri"] == true,
                    i as isize,
                )
            })
            .collect();
        if actions.is_empty() {
            actions.push(("Close".into(), true, -1));
        }
        let mut right = width - pad;
        for (label, primary, tag) in actions.iter().rev() {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(label),
                    Some(self),
                    Some(sel!(modalAction:)),
                    mtm,
                )
            };
            button.setTag(*tag);
            if *primary {
                button.setKeyEquivalent(&NSString::from_str("\r"));
            }
            button.sizeToFit();
            let size = button.frame().size;
            let w = size.width.max(80.);
            right -= w;
            button.setFrame(NSRect::new(
                NSPoint::new(right, pad),
                NSSize::new(w, size.height),
            ));
            button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            content.addSubview(&button);
            right -= 12.;
        }
        s.popouts.borrow_mut().push((window.clone(), child.clone()));
        // A page with no window (headless tests) keeps the sheet unshown; its
        // body and actions still work.
        if let Some(parent) = parent {
            parent.beginSheet_completionHandler(&window, None);
        }
        s.handler.event(HostEvent::PopoutOpened);
        Some(child)
    }
}
fn reply(task: &ProtocolObject<dyn WKURLSchemeTask>, value: &Value) {
    let bytes = serde_json::to_vec(value).expect("JSON value");
    reply_bytes(task, &bytes, "application/json");
}
fn reply_bytes(task: &ProtocolObject<dyn WKURLSchemeTask>, bytes: &[u8], mime: &str) {
    let keys = [
        NSString::from_str("Content-Type"),
        NSString::from_str("Access-Control-Allow-Origin"),
        NSString::from_str("Cache-Control"),
    ];
    let values = [
        NSString::from_str(mime),
        NSString::from_str("*"),
        NSString::from_str("no-store"),
    ];
    let headers = NSDictionary::from_slices(
        &keys.iter().map(|v| &**v).collect::<Vec<_>>(),
        &values.iter().map(|v| &**v).collect::<Vec<_>>(),
    );
    unsafe {
        let response = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
            NSHTTPURLResponse::alloc(),
            &task.request().URL().unwrap(),
            200,
            None,
            Some(&headers),
        )
        .unwrap();
        task.didReceiveResponse(&response);
        task.didReceiveData(&NSData::with_bytes(bytes));
        task.didFinish();
    }
}
fn fail_task(task: &ProtocolObject<dyn WKURLSchemeTask>) {
    unsafe {
        task.didFailWithError(&NSError::errorWithDomain_code_userInfo(
            &NSString::from_str("PMCompatChannel"),
            1,
            None,
        ));
    }
}

// A shared full-window page with an exact mask and identical native hit geometry.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[name="PMCompatSlotView"]
    #[ivars=RefCell<Vec<Rect>>]
    struct SlotView;
    impl SlotView {
        #[unsafe(method_id(hitTest:))]
        fn hit(&self,point:NSPoint)->Option<Retained<NSView>>{
            let local=self.convertPoint_fromView(point,unsafe{self.superview()}.as_deref());
            let y=self.bounds().size.height-local.y;
            if self.ivars().borrow().iter().any(|r|r.contains(local.x,y)){unsafe{msg_send![super(self),hitTest:point]}}else{None}
        }
    }
);
impl SlotView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(RefCell::new(vec![]))),initWithFrame:frame]
        }
    }
    fn clip(&self, rects: Vec<Rect>) {
        let path = CGMutablePath::new();
        let h = self.bounds().size.height;
        for r in &rects {
            unsafe {
                CGMutablePath::add_rect(
                    Some(&path),
                    std::ptr::null(),
                    NSRect::new(NSPoint::new(r.x, h - r.y - r.h), NSSize::new(r.w, r.h)),
                );
            }
        }
        *self.ivars().borrow_mut() = rects;
        self.setWantsLayer(true);
        let mask = objc2_quartz_core::CAShapeLayer::new();
        // A mask is drawn within its own bounds; without a frame it hides everything.
        mask.setFrame(self.bounds());
        mask.setPath(Some(&path));
        if let Some(layer) = self.layer() {
            unsafe { layer.setMask(Some(&mask)) }
        }
    }
}

/// Owns one trusted window's rendering document, or one Store extension's view.
/// Construct/use/drop on AppKit's main thread. No timers or global app settings.
pub struct MacDomHost {
    shared: Rc<Shared>,
    applier: String,
    bridge: Retained<Bridge>,
    web: Retained<WKWebView>,
    slot: Retained<SlotView>,
    activity: Option<Activity>,
    job_count: u32,
    dedicated_document: bool,
}
struct Activity {
    policy: WKInactiveSchedulingPolicy,
    window: Retained<NSWindow>,
    parent: Option<Retained<NSView>>,
    frame: NSRect,
    parked: bool,
    next_sibling: Option<Retained<NSView>>,
}
impl MacDomHost {
    pub fn new_store_remote_dom(
        mtm: MainThreadMarker,
        session: &pm_store_host::adapter::StoreSession,
        handler: Rc<dyn SyncHandler>,
        stores: &mut StoreDataStores,
        page_bundle: &str,
    ) -> Result<Self> {
        let permissions = session
            .grants
            .iter()
            .map(|p| {
                serde_json::to_value(p)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let trust = Trust::Store {
            extension: session.data_store.extension.as_str().into(),
            permissions,
        };
        let applier = format!("{page_bundle}\n{REMOTE_DOM_SHELL}");
        let mut host = Self::new_configured(
            mtm,
            trust,
            handler,
            stores,
            BTreeMap::new(),
            &applier,
            Some(&session.data_store),
        )?;
        host.dedicated_document = true;
        Ok(host)
    }
    /// A dedicated WebKit document using the production protocol applier bundle.
    /// `page_bundle` is host-built code exposing PMRemoteDOM.PageApplier.
    pub fn new_remote_dom(
        mtm: MainThreadMarker,
        trust: Trust,
        handler: Rc<dyn SyncHandler>,
        stores: &mut StoreDataStores,
        page_bundle: &str,
    ) -> Result<Self> {
        let applier = format!("{page_bundle}\n{REMOTE_DOM_SHELL}");
        let mut host =
            Self::new_with_applier(mtm, trust, handler, stores, BTreeMap::new(), &applier)?;
        host.dedicated_document = true;
        Ok(host)
    }
    /// `methods` maps Store method names to manifest permission names. Unknown
    /// Store methods are denied; the kernel must independently recheck admission.
    pub fn new(
        mtm: MainThreadMarker,
        trust: Trust,
        handler: Rc<dyn SyncHandler>,
        stores: &mut StoreDataStores,
        methods: BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_with_applier(mtm, trust, handler, stores, methods, PAGE_APPLIER)
    }
    /// A trusted, bundled renderer adapter can replace the fixture applier. It
    /// must implement the envelope/event contract, never execute extension code.
    pub fn new_with_applier(
        mtm: MainThreadMarker,
        trust: Trust,
        handler: Rc<dyn SyncHandler>,
        stores: &mut StoreDataStores,
        methods: BTreeMap<String, String>,
        applier: &str,
    ) -> Result<Self> {
        Self::new_configured(mtm, trust, handler, stores, methods, applier, None)
    }
    fn new_configured(
        mtm: MainThreadMarker,
        trust: Trust,
        handler: Rc<dyn SyncHandler>,
        stores: &mut StoreDataStores,
        methods: BTreeMap<String, String>,
        applier: &str,
        identity: Option<&pm_store_host::identity::DataStoreIdentity>,
    ) -> Result<Self> {
        if objc2_core_graphics::CGSessionCopyCurrentDictionary().is_none() {
            return Err(Error::Unsupported("compat_gui_session_unavailable"));
        }
        let token = random_token()?;
        let shared = Rc::new(Shared {
            trust: trust.clone(),
            handler,
            methods,
            channel: RefCell::new(Channel::new(token, 1)),
            panels: RefCell::new(Panels::default()),
            pending: RefCell::new(None),
            ready: Cell::new(false),
            recovery: RefCell::new(None),
            popouts: RefCell::new(vec![]),
            geometry_dirty: Cell::new(true),
            closed: Cell::new(false),
            canvases: RefCell::new(crate::canvas::CanvasResources::default()),
            fonts: RefCell::default(),
        });
        let bridge: Retained<Bridge> =
            unsafe { msg_send![super(Bridge::alloc(mtm).set_ivars(shared.clone())), init] };
        let config = unsafe { WKWebViewConfiguration::new(mtm) };
        unsafe {
            config.setWebsiteDataStore(&stores.get_for(&trust, identity));
            config.setURLSchemeHandler_forURLScheme(
                Some(ProtocolObject::from_ref(&*bridge)),
                &NSString::from_str("pmcompat"),
            );
            // A trusted page opens ui.modal sheets from code, not a click
            // (children stay about:blank only); a Store page never opens one.
            config
                .preferences()
                .setJavaScriptCanOpenWindowsAutomatically(!trust.is_store());
        }
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(1., 1.));
        let web = unsafe {
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config)
        };
        unsafe {
            web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*bridge)));
            web.setUIDelegate(Some(ProtocolObject::from_ref(&*bridge)));
        }
        let slot = SlotView::new(mtm, frame);
        slot.addSubview(&web);
        slot.setAccessibilityElement(true);
        slot.setAccessibilityRole(Some(&NSString::from_str("AXGroup")));
        slot.setAccessibilityLabel(Some(&NSString::from_str("Compatibility panels")));
        let host = Self {
            shared,
            applier: applier.to_owned(),
            bridge,
            web,
            slot,
            activity: None,
            job_count: 0,
            dedicated_document: false,
        };
        host.load();
        Ok(host)
    }
    fn load(&self) {
        let c = self.shared.channel.borrow();
        let nonce = random_token().expect("OS entropy");
        let html = transport::page_with_applier(
            &c.base(),
            &nonce,
            c.generation,
            &self.applier,
            &self.shared.trust,
        );
        unsafe {
            self.web
                .loadHTMLString_baseURL(&NSString::from_str(&html), None);
        }
    }
    pub fn web_view(&self) -> &WKWebView {
        &self.web
    }
    /// The page's open child web views (popouts and ui.modal sheet bodies).
    pub fn child_web_views(&self) -> Vec<Retained<WKWebView>> {
        self.shared
            .popouts
            .borrow()
            .iter()
            .map(|(_, web)| web.clone())
            .collect()
    }
    /// Read exactly one rendered page node; no expression supplied by extension
    /// or caller executes in WebKit. Invoke on AppKit and poll on the worker.
    pub fn inspect_capture(
        &self,
        generation: Generation,
        selector: &str,
        value: bool,
    ) -> Result<CaptureTicket<Value>> {
        if generation != self.generation() {
            return Err(Error::StaleGeneration);
        }
        if !self.dedicated_document || !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        if selector.len() > 4096 {
            return Err(Error::InvalidRequest);
        }
        let selector = serde_json::to_string(selector).map_err(|_| Error::InvalidRequest)?;
        let property = if value { "value" } else { "textContent" };
        let source = format!(
            "(()=>{{const nodes=document.querySelectorAll({selector});if(nodes.length!==1)throw Error('capture requires exactly one node');return JSON.stringify(nodes[0].{property})}})()"
        );
        let (sender, ticket) = crate::capture::ticket();
        let shared = self.shared.clone();
        let callback = block2::RcBlock::new(
            move |object: *mut objc2::runtime::AnyObject, error: *mut NSError| {
                let result =
                    if shared.closed.get() || shared.channel.borrow().generation != generation {
                        Err(Error::StaleGeneration)
                    } else if !error.is_null() {
                        Err(Error::InvalidRequest)
                    } else {
                        // SAFETY: WebKit owns the callback arguments for this invocation.
                        unsafe { object.as_ref() }
                            .and_then(|object| object.downcast_ref::<NSString>())
                            .ok_or(Error::InvalidRequest)
                            .and_then(|text| {
                                serde_json::from_str(&text.to_string())
                                    .map_err(|_| Error::InvalidRequest)
                            })
                    };
                let _ = sender.try_send(result);
            },
        );
        unsafe {
            self.web.evaluateJavaScript_completionHandler(
                &NSString::from_str(&source),
                Some(&callback),
            );
        }
        Ok(ticket)
    }
    /// Actual WebKit snapshot completion, independent of DOM batch admission.
    /// This captures this rendering document; the app owns full-window captures.
    pub fn capture_png(&self, generation: Generation) -> Result<CaptureTicket<Vec<u8>>> {
        if generation != self.generation() {
            return Err(Error::StaleGeneration);
        }
        if !self.dedicated_document || !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        let (sender, ticket) = crate::capture::ticket();
        let shared = self.shared.clone();
        let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let result = if shared.closed.get() || shared.channel.borrow().generation != generation
            {
                Err(Error::StaleGeneration)
            } else if !error.is_null() {
                Err(Error::Unsupported("WebKit snapshot failed"))
            } else {
                // SAFETY: WebKit owns the image for this callback. The returned
                // immutable NSData is copied before returning to WebKit.
                unsafe { image.as_ref() }
                    .and_then(|image| image.TIFFRepresentation())
                    .and_then(|data| NSBitmapImageRep::imageRepWithData(&data))
                    .and_then(|rep| unsafe {
                        rep.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    })
                    .map(|data| unsafe { data.as_bytes_unchecked() }.to_vec())
                    .ok_or(Error::Unsupported("WebKit PNG encoding failed"))
            };
            let _ = sender.try_send(result);
        });
        unsafe {
            self.web
                .takeSnapshotWithConfiguration_completionHandler(None, &callback);
        }
        Ok(ticket)
    }
    /// Nonblocking main-thread admission. Pixels were encoded on the realm worker.
    /// Faces for the page's text layout: each new one is served by the `font`
    /// route and loaded as a `FontFace` before any later batch applies; the
    /// page answers `HostEvent::FontsLoaded` once they are.
    pub fn add_fonts(&mut self, faces: &[PageFace]) -> Result<()> {
        let mut added = vec![];
        for face in faces {
            let key = face.key();
            if self.shared.fonts.borrow().contains_key(&key) {
                continue;
            }
            self.shared
                .fonts
                .borrow_mut()
                .insert(key.clone(), face.bytes.clone());
            added.push(json!({"family":face.family,"weight":face.weight,
                "style":if face.italic {"italic"} else {"normal"},"key":key}));
        }
        if added.is_empty() {
            return Ok(());
        }
        self.shared.send(json!({"kind":"fonts","faces":added}))
    }
    /// Keep the caller's frame on Backpressure; the page releases resources only
    /// after decoding and replacing its bitmap. No V8 call occurs here.
    pub fn present_canvas(&mut self, panel: PanelId, frame: &CanvasFrame) -> Result<()> {
        self.check(frame.generation, panel)?;
        if !self.dedicated_document || !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        let resource = self.shared.canvases.borrow_mut().insert(frame)?;
        let result = self.shared.send(json!({"kind":"canvas","panel":panel,
            "node":frame.node,"dom_sequence":frame.dom_sequence,"resource":resource,
            "width":frame.width,"height":frame.height}));
        if result.is_err() {
            self.shared.canvases.borrow_mut().remove(resource)?;
        }
        result
    }
    /// P-A11Y: pass this retained root once in the committed reading order,
    /// interleaved with GPUI root anchors. Preserve the real remote WebKit tree.
    pub fn accessibility_view(&self) -> Retained<NSView> {
        Retained::into_super(self.slot.clone())
    }
    #[cfg(feature = "gpui")]
    pub fn accessibility_item(&self) -> gpui_macos::composition::AccessibilityItem {
        gpui_macos::composition::AccessibilityItem::Native(self.accessibility_view())
    }
    /// Install below GPUI's drawing view. Do not call from the present callback.
    pub fn install_below(&self, container: &NSView, gpui: &NSView) {
        container.addSubview_positioned_relativeTo(
            &self.slot,
            NSWindowOrderingMode::Below,
            Some(gpui),
        );
    }
    /// Shell rectangles the shared document shows through this frame.
    pub fn set_regions(&mut self, regions: Vec<Rect>) -> Result<()> {
        if regions.len() > 256 || regions.iter().any(|r| !r.valid()) {
            return Err(Error::InvalidGeometry);
        }
        let mut panels = self.shared.panels.borrow_mut();
        if panels.regions.as_ref() != Some(&regions) {
            panels.regions = Some(regions);
            self.shared.geometry_dirty.set(true);
        }
        Ok(())
    }
    /// Called in P-LAYER's present callback, in the same CATransaction as GPUI.
    /// No JS call or wait occurs here. Returns the committed page geometry.
    pub fn present(&mut self, viewport: Rect) -> Result<CompositionSnapshot> {
        if !viewport.valid() || viewport.x != 0. || viewport.y != 0. {
            return Err(Error::InvalidGeometry);
        }
        self.shared.panels.borrow_mut().viewport = Rect::new(0., 0., viewport.w, viewport.h);
        self.slot.setFrame(NSRect::new(
            NSPoint::new(viewport.x, viewport.y),
            NSSize::new(viewport.w, viewport.h),
        ));
        self.web.setFrame(NSRect::new(
            NSPoint::ZERO,
            NSSize::new(viewport.w, viewport.h),
        ));
        let snapshot = self.composition();
        self.slot.clip(snapshot.visible_union.clone());
        self.shared.geometry_dirty.set(false);
        Ok(snapshot)
    }
    pub fn composition_dirty(&self) -> bool {
        self.shared.geometry_dirty.get()
    }
    /// Shell key monitors/menu interceptors must yield whenever this is true.
    /// Forward HostEvent::Key into shell resolution with compat=false; never replay
    /// that event into WebKit a second time. WebKit owns IME and text defaults.
    pub fn owns_first_responder(&self) -> bool {
        let children = self.shared.popouts.borrow();
        for web in std::iter::once(&*self.web).chain(children.iter().map(|(_, v)| &**v)) {
            let Some(window) = web.window() else { continue };
            if !window.isKeyWindow() {
                continue;
            }
            let mut responder = window.firstResponder();
            for _ in 0..64 {
                let Some(current) = responder else { break };
                if std::ptr::eq(
                    &*current as *const NSResponder,
                    web as *const WKWebView as *const NSResponder,
                ) {
                    return true;
                }
                responder = unsafe { current.nextResponder() };
            }
        }
        false
    }

    /// Job-scoped, nestable activity. Park only when the editor window is closed
    /// or unavailable; a visible editor keeps its slot and page dimensions.
    pub fn begin_background_job(&mut self, park: bool) -> Result<()> {
        if self.job_count > 0 {
            self.job_count = self.job_count.checked_add(1).ok_or(Error::InvalidRequest)?;
            if park {
                self.park_for_job();
            }
            return Ok(());
        }
        let prefs = unsafe { self.web.configuration().preferences() };
        if !prefs.respondsToSelector(sel!(setInactiveSchedulingPolicy:)) {
            return Err(Error::Unsupported("compat_background_policy_unavailable"));
        }
        let policy = unsafe { prefs.inactiveSchedulingPolicy() };
        unsafe { prefs.setInactiveSchedulingPolicy(WKInactiveSchedulingPolicy::None) };
        let mtm = MainThreadMarker::new().unwrap();
        let frame = self.slot.frame();
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(1., 1.)),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setCanHide(false);
        window.setIgnoresMouseEvents(true);
        self.activity = Some(Activity {
            policy,
            window,
            parent: None,
            frame,
            parked: false,
            next_sibling: None,
        });
        if park {
            self.park_for_job();
        }
        self.job_count = 1;
        Ok(())
    }
    fn park_for_job(&mut self) {
        let Some(a) = self.activity.as_mut() else {
            return;
        };
        if a.parked {
            return;
        }
        a.frame = self.slot.frame();
        a.parent = unsafe { self.slot.superview() };
        if let Some(parent) = &a.parent {
            let children = parent.subviews().to_vec();
            if let Some(index) = children
                .iter()
                .position(|v| std::ptr::eq(&**v, &*self.slot as &NSView))
            {
                a.next_sibling = children.get(index + 1).cloned();
            }
        }
        self.slot.removeFromSuperview();
        a.window.setContentView(Some(&self.slot));
        a.window.orderFront(None);
        a.parked = true;
    }
    pub fn end_background_job(&mut self) {
        if self.job_count == 0 {
            return;
        }
        self.job_count -= 1;
        if self.job_count > 0 {
            return;
        }
        if let Some(a) = self.activity.take() {
            unsafe {
                self.web
                    .configuration()
                    .preferences()
                    .setInactiveSchedulingPolicy(a.policy)
            };
            if a.parked {
                self.slot.removeFromSuperview();
                a.window.setContentView(None);
                if let Some(p) = a.parent {
                    if let Some(sibling) = a.next_sibling {
                        p.addSubview_positioned_relativeTo(
                            &self.slot,
                            NSWindowOrderingMode::Below,
                            Some(&sibling),
                        );
                    } else {
                        p.addSubview(&self.slot);
                    }
                    self.slot.setFrame(a.frame);
                }
            }
            a.window.close();
        }
    }
    /// Call only after trusted Stop approval (Store watchdog may call directly).
    /// Restart is observed by tick(); selector presence alone is never success.
    pub fn stop_and_recover(&mut self) -> Result<Generation> {
        if self.shared.recovery.borrow().is_some() {
            return Err(Error::Forbidden);
        }
        if NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            < 14
            || !self
                .web
                .respondsToSelector(sel!(_killWebContentProcessAndResetState))
            || !self.web.respondsToSelector(sel!(_webProcessIdentifier))
        {
            return Err(Error::Unsupported("compat_hang_stop_unavailable"));
        }
        let pid = self.content_pid();
        if pid <= 0 {
            return Err(Error::Unsupported("compat_content_pid_unavailable"));
        }
        let token = random_token()?;
        self.shared.handler.prepare_reset(self.generation())?;
        self.close_popouts();
        self.shared.pending.borrow_mut().take();
        self.shared.ready.set(false);
        let generation = self.generation() + 1;
        *self.shared.channel.borrow_mut() = Channel::new(token, generation);
        *self.shared.canvases.borrow_mut() = crate::canvas::CanvasResources::default();
        for p in self.shared.panels.borrow_mut().items.values_mut() {
            p.overlays.clear()
        }
        self.shared.geometry_dirty.set(true);
        *self.shared.recovery.borrow_mut() = Some(Recovery {
            old_pid: pid,
            started: Instant::now(),
            termination: false,
        });
        unsafe {
            let _: () = msg_send![&*self.web, _killWebContentProcessAndResetState];
        }
        self.load();
        Ok(generation)
    }
    pub fn content_pid(&self) -> i32 {
        if self.web.respondsToSelector(sel!(_webProcessIdentifier)) {
            unsafe { msg_send![&*self.web, _webProcessIdentifier] }
        } else {
            0
        }
    }
    /// Call from the shell's event loop (outside GPUI's draw lock). No page eval.
    pub fn tick(&mut self) {
        let mut recovery = self.shared.recovery.borrow_mut();
        if let Some(r) = recovery.as_ref() {
            let pid = self.content_pid();
            if self.shared.ready.get() && pid > 0 && pid != r.old_pid && process_gone(r.old_pid) {
                let old_pid = r.old_pid;
                recovery.take();
                drop(recovery);
                self.shared.handler.event(HostEvent::Recovered {
                    generation: self.generation(),
                    old_pid,
                    new_pid: pid,
                });
            } else if r.started.elapsed() > Duration::from_secs(20) {
                recovery.take();
                drop(recovery);
                self.shared
                    .handler
                    .event(HostEvent::Failure("compat_recovery_timeout"));
            }
        }
        // User-closed child windows are removed without retaining hidden pages.
        // A ui.modal sheet has no close box: it goes when its page closes it.
        self.shared.popouts.borrow_mut().retain(|(w, v)| {
            if w.isVisible() || !w.styleMask().contains(NSWindowStyleMask::Closable) {
                true
            } else {
                unsafe {
                    v.setNavigationDelegate(None);
                    v.setUIDelegate(None)
                };
                if let Some(p) = w.parentWindow() {
                    p.removeChildWindow(w)
                }
                false
            }
        });
    }
    pub fn popout_count(&self) -> usize {
        self.shared.popouts.borrow().len()
    }
    fn close_popouts(&self) {
        for (w, v) in self.shared.popouts.borrow_mut().drain(..) {
            unsafe {
                v.setNavigationDelegate(None);
                v.setUIDelegate(None)
            }
            unsafe { v.stopLoading() };
            if let Some(p) = w.parentWindow() {
                p.removeChildWindow(&w)
            }
            close_later(&w);
        }
    }
    fn check(&self, g: Generation, id: PanelId) -> Result<()> {
        if g != self.generation() {
            return Err(Error::StaleGeneration);
        }
        if !self.shared.panels.borrow().items.contains_key(&id) {
            return Err(Error::UnknownPanel);
        }
        Ok(())
    }
}
impl DomPanelHost for MacDomHost {
    fn generation(&self) -> Generation {
        self.shared.channel.borrow().generation
    }
    fn create(&mut self, panel: PanelSpec) -> Result<()> {
        if self.dedicated_document && !self.shared.panels.borrow().items.is_empty() {
            return Err(Error::DuplicatePanel);
        }
        if self.shared.panels.borrow().items.contains_key(&panel.id) {
            return Err(Error::DuplicatePanel);
        }
        if self.shared.ready.get() {
            self.shared
                .send(json!({"kind":"create","panel":panel.id,"label":panel.label}))?;
        }
        self.shared.panels.borrow_mut().create(panel)
    }
    fn attach(&mut self, id: PanelId, slot: PanelSlot) -> Result<()> {
        self.check(self.generation(), id)?;
        if !slot.bounds.valid() || !slot.clip.valid() {
            return Err(Error::InvalidGeometry);
        }
        if self.shared.panels.borrow().items[&id].slot.as_ref() == Some(&slot) {
            return Ok(());
        }
        if self.shared.ready.get() {
            self.shared
                .send(json!({"kind":"slot","panel":id,"slot":slot}))?;
        }
        let mut panels = self.shared.panels.borrow_mut();
        let p = panels.get(id)?;
        p.slot = Some(slot);
        p.overlays.clear();
        self.shared.geometry_dirty.set(true);
        Ok(())
    }
    fn detach(&mut self, id: PanelId) -> Result<()> {
        self.check(self.generation(), id)?;
        if self.shared.ready.get() {
            self.shared
                .send(json!({"kind":"slot","panel":id,"slot":Value::Null}))?;
        }
        let mut panels = self.shared.panels.borrow_mut();
        let p = panels.get(id)?;
        p.slot = None;
        p.overlays.clear();
        self.shared.geometry_dirty.set(true);
        Ok(())
    }
    fn destroy(&mut self, id: PanelId) -> Result<()> {
        self.check(self.generation(), id)?;
        if self.shared.ready.get() {
            self.shared.send(json!({"kind":"destroy","panel":id}))?;
        }
        self.shared.panels.borrow_mut().items.remove(&id);
        self.shared.geometry_dirty.set(true);
        Ok(())
    }
    fn apply(&mut self, g: Generation, id: PanelId, batch: Value) -> Result<()> {
        self.check(g, id)?;
        if !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        self.shared
            .send(json!({"kind":"batch","panel":id,"batch":batch}))
    }
    fn send_event(&mut self, g: Generation, id: PanelId, event: Value) -> Result<()> {
        if self.dedicated_document {
            return Err(Error::Unsupported(
                "remote DOM input must enter through the page or owner queue",
            ));
        }
        self.check(g, id)?;
        if !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        self.shared
            .send(json!({"kind":"event","panel":id,"event":event}))
    }
    fn focus(&mut self, id: PanelId, node: Option<u64>) -> Result<()> {
        self.check(self.generation(), id)?;
        if !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        if self.shared.panels.borrow().items[&id].slot.is_none() {
            return Err(Error::Forbidden);
        }
        if let Some(w) = self.web.window() {
            w.makeFirstResponder(Some(&self.web));
        }
        self.shared
            .send(json!({"kind":"focus","panel":id,"node":node}))
    }
    fn composition(&self) -> CompositionSnapshot {
        self.shared.panels.borrow().snapshot()
    }
}
impl Drop for MacDomHost {
    fn drop(&mut self) {
        self.shared.closed.set(true);
        self.job_count = self.job_count.min(1);
        self.end_background_job();
        self.close_popouts();
        self.shared.pending.borrow_mut().take();
        unsafe { self.web.stopLoading() };
        unsafe {
            self.web.setUIDelegate(None);
            self.web.setNavigationDelegate(None);
        }
        self.slot.removeFromSuperview();
        let _ = &self.bridge;
    }
}
/// Closing a key child window makes its parent key synchronously, and the
/// shell then renders inside that AppKit callback. A host method (called with
/// the shell's borrow of the host held, e.g. `stop_and_recover`) must not
/// re-enter the shell that way, so the close runs on the next run-loop turn in
/// any common mode. The pending perform request retains the window.
fn close_later(window: &NSWindow) {
    let object: &NSObject = window;
    unsafe {
        object.performSelector_withObject_afterDelay_inModes(
            sel!(close),
            None,
            0.,
            &NSArray::from_slice(&[NSRunLoopCommonModes]),
        );
    }
}
fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| Error::Unsupported("compat_entropy_unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn process_gone(pid: i32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
        fn __error() -> *mut i32;
    }
    pid > 0 && unsafe { kill(pid, 0) == -1 && *__error() == 3 }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[derive(Default)]
    struct Handler {
        calls: Cell<u32>,
        events: Cell<u32>,
    }
    impl SyncHandler for Handler {
        fn call(&self, c: &RequestContext, _: &str, args: Value) -> Result<Value> {
            assert_eq!(c.panel, 1);
            assert_eq!(c.generation, 3);
            self.calls.set(self.calls.get() + 1);
            Ok(args)
        }
        fn event(&self, _: HostEvent) {
            self.events.set(self.events.get() + 1)
        }
    }
    fn shared(trust: Trust) -> (Shared, Rc<Handler>) {
        let handler = Rc::new(Handler::default());
        let mut panels = Panels::default();
        panels
            .create(PanelSpec {
                id: 1,
                label: "Fixture".into(),
            })
            .unwrap();
        panels.get(1).unwrap().slot = Some(PanelSlot {
            id: 1,
            bounds: Rect::new(0., 0., 10., 10.),
            clip: Rect::new(0., 0., 10., 10.),
            reading_order: 0,
        });
        (
            Shared {
                trust,
                handler: handler.clone(),
                methods: BTreeMap::from([
                    ("read".into(), "project:read".into()),
                    ("write".into(), "project:write".into()),
                ]),
                channel: RefCell::new(Channel::new("synthetic-capability".into(), 3)),
                canvases: RefCell::new(crate::canvas::CanvasResources::default()),
                fonts: RefCell::default(),
                panels: RefCell::new(panels),
                pending: RefCell::new(None),
                ready: Cell::new(true),
                recovery: RefCell::new(None),
                popouts: RefCell::new(vec![]),
                geometry_dirty: Cell::new(false),
                closed: Cell::new(false),
            },
            handler,
        )
    }
    #[test]
    fn store_admits_only_mapped_and_granted_methods() {
        let (s, h) = shared(Trust::Store {
            extension: "fixture".into(),
            permissions: ["project:read".into()].into(),
        });
        assert_eq!(
            s.handle(r#"{"generation":3,"panel":1,"kind":"call","method":"read","payload":7}"#),
            Ok(json!(7))
        );
        for method in ["write", "unknown"] {
            assert_eq!(
                s.handle(
                    &json!({"generation":3,"panel":1,"kind":"call","method":method}).to_string()
                ),
                Err(Error::Forbidden)
            );
        }
        assert_eq!(h.calls.get(), 1);
    }
    #[test]
    fn stale_and_unknown_panel_requests_do_not_reach_handler() {
        let (s, h) = shared(Trust::Trusted);
        assert_eq!(
            s.handle(r#"{"generation":2,"panel":1,"kind":"call","method":"read"}"#),
            Err(Error::StaleGeneration)
        );
        assert_eq!(
            s.handle(r#"{"generation":3,"panel":2,"kind":"call","method":"read"}"#),
            Err(Error::UnknownPanel)
        );
        assert_eq!(h.calls.get(), 0);
    }
    #[test]
    fn detached_panels_cannot_forward_input() {
        let (s, h) = shared(Trust::Trusted);
        s.panels.borrow_mut().get(1).unwrap().slot = None;
        assert_eq!(
            s.handle(r#"{"generation":3,"panel":1,"kind":"key","payload":{}}"#),
            Err(Error::Forbidden)
        );
        assert_eq!(h.events.get(), 0);
    }
    #[test]
    fn stopped_hosts_reject_requests() {
        let (s, h) = shared(Trust::Trusted);
        s.closed.set(true);
        assert_eq!(
            s.handle(r#"{"generation":3,"panel":1,"kind":"call","method":"read"}"#),
            Err(Error::StaleGeneration)
        );
        assert_eq!(h.calls.get(), 0);
    }
    #[test]
    fn store_bootstrap_rejects_nonopaque_origin() {
        let (s, h) = shared(Trust::Store {
            extension: "fixture".into(),
            permissions: Default::default(),
        });
        s.ready.set(false);
        assert_eq!(s.handle(r#"{"generation":3,"panel":0,"kind":"ready","payload":{"origin":"https://example.invalid"}}"#),Err(Error::Forbidden));
        assert_eq!(h.events.get(), 1);
    }
}
