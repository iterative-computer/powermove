use crate::{
    contract::Panels,
    panel_document::PanelDocument,
    panel_renderers::{CanvasCache, PanelFeedback, PanelPageEvents, WarmPanels},
    panel_slot::{PanelAttachment, editor_rect},
    transport, *,
};
use block2::DynBlock;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::*;
use objc2_web_kit::*;
use pm_remote_dom::protocol::Batch;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    rc::Rc,
};

type Task = Retained<ProtocolObject<dyn WKURLSchemeTask>>;

/// How the app begins a sheet, and forgets one that has not begun. `begin`
/// is `(parent, sheet, begin, cancel)`: `begin` makes the AppKit call,
/// `cancel` answers the page if the sheet never begins (a development lane
/// holds sheets while the app is inactive); it returns whether it began now.
/// `forget` is called with a sheet its page closed before it began.
/// A deferred AppKit call: a sheet's begin, or its cancellation.
pub type SheetCall = Box<dyn FnOnce()>;
#[derive(Clone, Copy)]
pub struct SheetGate {
    pub begin: fn(&NSWindow, &NSWindow, SheetCall, SheetCall) -> bool,
    pub forget: fn(&NSWindow),
}
static SHEET_GATE: std::sync::OnceLock<SheetGate> = std::sync::OnceLock::new();
/// Sets the [`SheetGate`] every extension sheet goes through (once per app;
/// without one, a sheet begins at once).
pub fn set_sheet_gate(gate: SheetGate) {
    let _ = SHEET_GATE.set(gate);
}
/// A sheet its page closed: one still waiting to begin is cancelled.
fn forget_sheet(sheet: &NSWindow) {
    if let Some(gate) = SHEET_GATE.get() {
        (gate.forget)(sheet);
    }
}
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
fn covering_rect(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect::new(
        x,
        y,
        (a.x + a.w).max(b.x + b.w) - x,
        (a.y + a.h).max(b.y + b.h) - y,
    )
}

fn snapshot_geometry(source: Rect, target: Rect, clip: Rect, height: f64) -> (Rect, Rect) {
    let visible = target.intersection(clip);
    (
        Rect::new(
            visible.x,
            height - visible.y - visible.h,
            visible.w,
            visible.h,
        ),
        Rect::new(
            target.x - visible.x,
            visible.y + visible.h - target.y - source.h,
            source.w,
            source.h,
        ),
    )
}
#[derive(Default)]
struct PanelSnapshot {
    request_sequence: u64,
    build_done: bool,
    requested: u64,
    revision: u64,
    captured_revision: u64,
    image: Option<(Rect, Retained<NSImage>)>,
}
impl PanelSnapshot {
    fn invalidate_build(&mut self) {
        self.request_sequence = self.request_sequence.wrapping_add(1);
        self.requested = 0;
        self.build_done = false;
    }
    fn begin_capture(&mut self, generation: u64) -> u64 {
        self.request_sequence = self.request_sequence.wrapping_add(1);
        self.requested = generation;
        self.build_done = false;
        self.request_sequence
    }
    fn accepts(&self, sequence: u64) -> bool {
        self.request_sequence == sequence
    }
    fn complete(&mut self, rect: Rect, image: Option<&NSImage>) {
        self.build_done = true;
        if let Some(image) = image {
            let size = image.size();
            if size.width > 0. && size.height > 0. {
                self.image = Some((rect, image.retain()));
                self.captured_revision = self.revision;
            }
        }
    }
}
#[derive(Clone)]
struct Overscan {
    revision: u64,
    id: String,
    generation: u64,
    rect: Rect,
    ready: bool,
    capturing: bool,
}
struct Shared {
    trust: Trust,
    handler: Rc<dyn SyncHandler>,
    channel: RefCell<Channel>,
    panels: RefCell<Panels>,
    pending: RefCell<Option<(Task, u64)>>,
    ready: Cell<bool>,
    sheets: RefCell<Vec<(Retained<NSWindow>, Retained<WKWebView>)>>,
    panel_feedback: RefCell<Option<Rc<PanelFeedback>>>,
    slot_owner: RefCell<Option<String>>,
    slot_dom_ready: Cell<bool>,
    slot_snapshot_pending: Cell<bool>,
    slot_painted: Cell<bool>,
    geometry_dirty: Cell<bool>,
    closed: Cell<bool>,
    layouts: RefCell<BTreeMap<String, (u64, Rect, bool)>>,
    /// Footprints the shared page can still be painting until it acknowledges
    /// the newest placement. They must not leak through its viewer aperture.
    stale_layouts: RefCell<BTreeMap<String, Rect>>,
    retiring_layouts: RefCell<BTreeMap<String, (u64, Rect)>>,
    visibility_sequence: Cell<u64>,
    layout_sequence: Cell<u64>,
    snapshots: RefCell<BTreeMap<String, PanelSnapshot>>,
    overscan: RefCell<Option<Overscan>>,
    canvases: RefCell<crate::canvas::CanvasResources>,
    /// Faces the page may load by key (`font` route).
    fonts: RefCell<BTreeMap<String, std::sync::Arc<[u8]>>>,
}
impl Shared {
    fn panel_safe_stage(&self, stage: Rect, frozen: &BTreeMap<String, Rect>) -> Vec<Rect> {
        let stale = self.stale_layouts.borrow();
        let retiring = self.retiring_layouts.borrow();
        let layouts = self.layouts.borrow();
        stale
            .values()
            .chain(retiring.values().map(|(_, rect)| rect))
            .chain(
                frozen
                    .keys()
                    .filter_map(|id| layouts.get(id).map(|(_, r, _)| r)),
            )
            .fold(vec![stage], |regions, cut| {
                regions
                    .into_iter()
                    .flat_map(|r| subtract_build_region(r, *cut))
                    .collect()
            })
    }

    fn capture_layout_is_current(&self, id: &str, generation: u64) -> bool {
        let layouts = self.layouts.borrow();
        layouts
            .get(id)
            .is_some_and(|(current, _, _)| *current == generation)
            && layouts.values().all(|(_, _, applied)| *applied)
            && self.retiring_layouts.borrow().is_empty()
    }

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
    fn queue_document(&self) -> Result<()> {
        let slots: Vec<_> = self
            .panels
            .borrow()
            .items
            .values()
            .map(|panel| (panel.spec.clone(), panel.slot.clone()))
            .collect();
        for (spec, slot) in slots {
            self.send(json!({"kind":"create","panel":spec.id,"label":spec.label}))?;
            self.send(json!({"kind":"slot","panel":spec.id,"slot":slot}))?;
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
            self.layouts.borrow_mut().clear();
            self.stale_layouts.borrow_mut().clear();
            self.retiring_layouts.borrow_mut().clear();
            self.snapshots.borrow_mut().clear();
            self.overscan.borrow_mut().take();
            // Pane renderers queue their owner and initial document before
            // navigation, so the first poll can already paint it.
            if self.slot_owner.borrow().is_none() {
                self.queue_document()?;
            }
            self.handler.event(HostEvent::Ready { generation });
            return Ok(Value::Null);
        }
        if kind == "failure" {
            // Why, in the log: what the page was applying and its error.
            let why: String = m["payload"].to_string().chars().take(600).collect();
            eprintln!("extension page stopped: {why}");
            self.handler
                .event(HostEvent::Failure("compat_applier_stopped"));
            return Ok(Value::Null);
        }
        if kind == "overscan_dirty" {
            if let Some(id) = m["payload"]["id"].as_str() {
                if let Some(snapshot) = self.snapshots.borrow_mut().get_mut(id) {
                    snapshot.revision += 1;
                }
                self.geometry_dirty.set(true);
                self.handler.event(HostEvent::GeometryChanged);
            }
            return Ok(Value::Null);
        }
        if kind == "overscan_cleared" {
            let matches = self.overscan.borrow().as_ref().is_some_and(|p| {
                m["payload"]["id"] == p.id && m["payload"]["generation"] == p.generation
            });
            if matches {
                self.overscan.borrow_mut().take();
                self.geometry_dirty.set(true);
                self.handler.event(HostEvent::GeometryChanged);
            }
            return Ok(Value::Null);
        }
        if kind == "overscan_ready" {
            if let Some(pending) = self.overscan.borrow_mut().as_mut() {
                if m["payload"]["id"] == pending.id
                    && m["payload"]["generation"] == pending.generation
                {
                    pending.ready = true;
                    self.geometry_dirty.set(true);
                    self.handler.event(HostEvent::GeometryChanged);
                }
            }
            return Ok(Value::Null);
        }
        if kind == "dock_content" {
            let payload = &m["payload"];
            if let (Some(id), Some(height)) = (
                payload["id"].as_str().filter(|id| id.len() <= 256),
                payload["height"]
                    .as_f64()
                    .filter(|h| h.is_finite() && *h > 0.),
            ) {
                self.handler.event(HostEvent::DockContent {
                    id: id.into(),
                    height,
                });
            }
            return Ok(Value::Null);
        }
        if kind == "dock_scroll" || kind == "dock_reveal" {
            let payload = &m["payload"];
            if let Some(id) = payload["id"].as_str().filter(|id| id.len() <= 256) {
                if kind == "dock_scroll" {
                    if let Some(delta) = payload["delta"].as_f64().filter(|v| v.is_finite()) {
                        self.handler.event(HostEvent::DockScroll {
                            id: id.into(),
                            delta,
                        });
                    }
                } else if let (Some(top), Some(height)) =
                    (payload["top"].as_f64(), payload["height"].as_f64())
                {
                    if top.is_finite() && height.is_finite() {
                        self.handler.event(HostEvent::DockReveal {
                            id: id.into(),
                            top,
                            height,
                        });
                    }
                }
            }
            return Ok(Value::Null);
        }
        if kind == "panel_visibility_applied" {
            let sequence = m["payload"]["sequence"]
                .as_u64()
                .ok_or(Error::InvalidRequest)?;
            if sequence <= self.visibility_sequence.get() {
                let mut retiring = self.retiring_layouts.borrow_mut();
                let count = retiring.len();
                retiring.retain(|_, (pending, _)| *pending > sequence);
                if retiring.len() != count {
                    self.geometry_dirty.set(true);
                    self.handler.event(HostEvent::GeometryChanged);
                }
            }
            return Ok(Value::Null);
        }
        if kind == "panel_slot_ready" {
            let id = m["payload"]["id"].as_str().ok_or(Error::InvalidRequest)?;
            if self.slot_owner.borrow().as_deref() == Some(id) && !self.slot_dom_ready.replace(true)
            {
                self.geometry_dirty.set(true);
                self.handler.event(HostEvent::GeometryChanged);
            }
            return Ok(Value::Null);
        }
        if kind == "panel_layout_applied" {
            let payload = &m["payload"];
            let id = payload["id"].as_str().ok_or(Error::InvalidRequest)?;
            let generation = payload["generation"]
                .as_u64()
                .ok_or(Error::InvalidRequest)?;
            if let Some(layout) = self.layouts.borrow_mut().get_mut(id) {
                if layout.0 == generation {
                    let painted: Rect = serde_json::from_value(payload["rect"].clone())
                        .map_err(|_| Error::InvalidGeometry)?;
                    if !painted.valid()
                        || [
                            painted.x - layout.1.x,
                            painted.y - layout.1.y,
                            painted.w - layout.1.w,
                            painted.h - layout.1.h,
                        ]
                        .iter()
                        .any(|d| d.abs() > 0.5)
                    {
                        return Ok(Value::Null);
                    }
                    if layout.2 {
                        return Ok(Value::Null);
                    }
                    layout.2 = true;
                    self.stale_layouts.borrow_mut().remove(id);
                    if let Some(snapshot) = self.snapshots.borrow_mut().get_mut(id) {
                        snapshot.revision += 1;
                    }
                    self.geometry_dirty.set(true);
                    self.handler.event(HostEvent::GeometryChanged);
                }
            }
            return Ok(Value::Null);
        }
        if kind == "dock_content" {
            let payload = &m["payload"];
            if let (Some(id), Some(height)) = (
                payload["id"].as_str().filter(|id| id.len() <= 256),
                payload["height"]
                    .as_f64()
                    .filter(|h| h.is_finite() && *h > 0.),
            ) {
                self.handler.event(HostEvent::DockContent {
                    id: id.into(),
                    height,
                });
            }
            return Ok(Value::Null);
        }
        if kind == "dock_scroll" || kind == "dock_reveal" {
            let payload = &m["payload"];
            if let Some(id) = payload["id"].as_str().filter(|id| id.len() <= 256) {
                if kind == "dock_scroll" {
                    if let Some(delta) = payload["delta"].as_f64().filter(|v| v.is_finite()) {
                        self.handler.event(HostEvent::DockScroll {
                            id: id.into(),
                            delta,
                        });
                    }
                } else if let (Some(top), Some(height)) =
                    (payload["top"].as_f64(), payload["height"].as_f64())
                {
                    if top.is_finite() && height.is_finite() {
                        self.handler.event(HostEvent::DockReveal {
                            id: id.into(),
                            top,
                            height,
                        });
                    }
                }
            }
            return Ok(Value::Null);
        }
        if kind == "fonts_loaded" {
            let keys = m["payload"]["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .filter(|k| self.fonts.borrow().contains_key(k))
                .collect();
            self.handler.event(HostEvent::FontsLoaded(keys));
            return Ok(Value::Null);
        }
        if !self.panels.borrow().items.contains_key(&id) {
            return Err(Error::UnknownPanel);
        }
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
                // Hidden pages still acknowledge DOM/layout; only input needs a
                // slot. Focus, selection and scroll are the browser's own as a
                // page leaves the screen (a tab switch fires blur): they pass,
                // as a refusal stopped the page and blanked every extension
                // panel in the window (PM-509).
                if payload["kind"] == "event"
                    && !payload["event"].as_str().is_some_and(passes_detached)
                    && self.panels.borrow().items[&id].slot.is_none()
                {
                    eprintln!(
                        "extension page: input refused while hidden: {}",
                        payload["event"]
                    );
                    return Err(Error::Forbidden);
                }
                if let Some(feedback) = self.panel_feedback.borrow().as_ref() {
                    feedback.received(None, payload)?;
                } else {
                    self.handler.remote_dom(payload)?;
                }
                Ok(Value::Null)
            }
            _ => Err(Error::InvalidRequest),
        }
    }
}
/// Events a page fires by itself while it is off screen, not input.
fn passes_detached(event: &str) -> bool {
    matches!(
        event,
        "focus"
            | "blur"
            | "focusin"
            | "focusout"
            | "selectionchange"
            | "scroll"
            | "visibilitychange"
            | "pagehide"
            | "pageshow"
            | "resize"
    )
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
            let children = self.ivars().sheets.borrow();
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
            let child=self.ivars().sheets.borrow().iter().any(|(_,v)|std::ptr::eq(&**v,web));
            // Only host-created about:blank loads; every external/file/data URL,
            // frame navigation and download is denied. Sheets are trusted only.
            // A ui.modal sheet's child loads about:blank with its frame in the fragment.
            let allow=(url=="about:blank" || (child && url.starts_with("about:blank#pm-modal="))) && (!self.ivars().ready.get() || child || (!self.ivars().trust.is_store() && unsafe{action.targetFrame()}.is_none()));
            done.call((if allow{WKNavigationActionPolicy::Allow}else{WKNavigationActionPolicy::Cancel},));
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        unsafe fn terminated(&self,_web:&WKWebView){
            self.ivars().handler.event(HostEvent::ProcessTerminated);
        }
    }
    unsafe impl WKUIDelegate for Bridge {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        unsafe fn open_sheet(&self,web:&WKWebView,cfg:&WKWebViewConfiguration,action:&WKNavigationAction,_features:&WKWindowFeatures)->Option<Retained<WKWebView>>{
            let url=unsafe{action.request()}.URL().and_then(|u|u.absoluteString()).map(|s|s.to_string()).unwrap_or_default();
            url.strip_prefix("about:blank#pm-modal=").and_then(|frame|self.make_modal(web,cfg,frame))
        }
        #[unsafe(method(webViewDidClose:))]
        unsafe fn child_closed(&self,web:&WKWebView){
            let mut children=self.ivars().sheets.borrow_mut();
            if let Some(i)=children.iter().position(|(_,v)|std::ptr::eq(&**v,web)) {let (w,v)=children.remove(i);forget_sheet(&w);unsafe{v.setUIDelegate(None);v.setNavigationDelegate(None)};if let Some(p)=w.sheetParent(){p.endSheet(&w)}
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
        if s.trust.is_store() || s.sheets.borrow().len() >= 8 {
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
        s.sheets.borrow_mut().push((window.clone(), child.clone()));
        // A page with no window (headless tests) keeps the sheet unshown; its
        // body and actions still work.
        if let Some(parent) = parent {
            let (on, sheet) = (parent.clone(), window.clone());
            let begin = Box::new(move || on.beginSheet_completionHandler(&sheet, None));
            // Never begun: the page hears the modal dismissed.
            let page = child.clone();
            let cancel = Box::new(move || unsafe {
                page.evaluateJavaScript_completionHandler(
                    &NSString::from_str(
                        "document.querySelector('[data-pm-modal-dismiss]')?.click()",
                    ),
                    None,
                )
            });
            match SHEET_GATE.get() {
                Some(gate) => {
                    (gate.begin)(&parent, &window, begin, cancel);
                }
                None => begin(),
            }
        }
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

/// The shared page is never a drag destination. It spans the whole window
/// under the native panels, and AppKit gives a drag to the registered view
/// under the pointer whatever `hitTest:` answers, so a WKWebView registered
/// for files took every file dragged over the window, panels it does not
/// show included. WebKit registers when it is made, when it joins a window
/// and as its page loads, with no frame drawn after it on a window at rest:
/// the page's view ([`PageView`]) ignores the registration, and anything
/// registered before stays cleared here. As in v1.2.0 (an extension panel's
/// drop reached the window's own), files go to the native drop targets, and
/// over an extension panel to the window's drop, which imports them.
fn refuse_drags(web: &WKWebView) {
    if web.registeredDraggedTypes().count() > 0 {
        web.unregisterDraggedTypes();
    }
}

// The shared page's web view. It is never a drag destination (above), and
// it spans the whole window under the native
// panels and, once a person has clicked an extension panel, holds the
// keyboard: AppKit then sent it every plain mouse move in the window, so the
// timeline, the viewer and every native panel lost their hover (no whiter rim,
// no trim grip, no row switches) and WebKit's cursor reset to an arrow over
// them, until the next click elsewhere. A move or cursor update over the
// window's own content (outside the extension panels, where the native view
// answers the hit test) goes to that view instead; over a panel it is
// WebKit's.
define_class!(
    #[unsafe(super(WKWebView))]
    #[thread_kind=MainThreadOnly]
    #[name="PMCompatWebView"]
    #[ivars=Rc<Cell<Option<(f64, f64)>>>]
    struct PageView;
    impl PageView {
        /// Where the page was last pressed (slot points, top-left origin):
        /// the panel there is the one its caret belongs to (PM-294).
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(slot) = unsafe { self.superview() } {
                let local = slot.convertPoint_fromView(event.locationInWindow(), None);
                self.ivars().set(Some((local.x, slot.bounds().size.height - local.y)));
            }
            unsafe { msg_send![super(self), mouseDown: event] }
        }
        /// WebKit registers for files as it is made, joins a window and
        /// loads: ignored, so the page never takes a Finder drop.
        #[unsafe(method(registerForDraggedTypes:))]
        fn register_for_dragged_types(&self, _types: &NSArray<NSPasteboardType>) {}
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            match self.native_under(event) {
                Some(view) => unsafe { msg_send![&*view, mouseMoved: event] },
                None => unsafe { msg_send![super(self), mouseMoved: event] },
            }
        }
        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, event: &NSEvent) {
            if self.native_under(event).is_none() {
                unsafe { msg_send![super(self), cursorUpdate: event] }
            }
        }
    }
);
impl PageView {
    /// The native view under `event`'s pointer, when it is not this page
    /// (the pointer is outside every extension panel).
    fn native_under(&self, event: &NSEvent) -> Option<Retained<NSView>> {
        let window = self.window()?;
        let hit = window.contentView()?.hitTest(event.locationInWindow())?;
        (!hit.isDescendantOf(self)).then_some(hit)
    }
}

// Snapshot overlays never intercept the page's input.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[name="PMCompatSnapshotView"]
    #[ivars=()]
    struct SnapshotView;
    impl SnapshotView {
        #[unsafe(method_id(hitTest:))]
        fn hit(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
    }
);
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
        /// S5: a key the page leaves (WebKit hands it on up the responder
        /// chain: no field took it, no page script stopped it) goes back to
        /// the editor, as Electron's window listener runs it: the editor's
        /// view takes the keyboard and the key is delivered again, so Space
        /// plays and the timeline's keys run after a click in a DOM panel.
        #[unsafe(method(keyDown:))]
        fn key_down(&self,event:&NSEvent){
            let Some(editor)=self.editor_view() else{return};
            let Some(window)=self.window() else{return};
            if window.makeFirstResponder(Some(&editor)){
                NSApplication::sharedApplication(MainThreadMarker::from(self)).postEvent_atStart(event,true);
            }
        }
    }
);
impl SlotView {
    /// The editor's GPUI view beside this slot (the content's key handler).
    fn editor_view(&self) -> Option<Retained<NSView>> {
        let parent = unsafe { self.superview() }?;
        parent
            .subviews()
            .to_vec()
            .into_iter()
            .find(|v| v.class().name().to_bytes() == b"GPUIView")
    }
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(RefCell::new(vec![]))),initWithFrame:frame]
        }
    }
    /// Shows the page only within `rects` (window points from the top), each
    /// with continuous bottom corners of `radius` (a panel body's corners).
    fn clip(&self, rects: Vec<Rect>, radius: f64, fades: &[(Rect, [f64; 2])]) {
        use objc2_quartz_core::{CAGradientLayer, CALayer, CAShapeLayer};
        let path = CGMutablePath::new();
        let group = CALayer::new();
        group.setFrame(self.bounds());
        let h = self.bounds().size.height;
        for r in &rects {
            let frame = NSRect::new(NSPoint::new(r.x, h - r.y - r.h), NSSize::new(r.w, r.h));
            let fade = fades
                .iter()
                .find(|(rect, distances)| rect == r && *distances != [0., 0.]);
            if let Some((_, [top, bottom])) = fade {
                if r.h <= 0. || r.w <= 0. {
                    continue;
                }
                let gradient = CAGradientLayer::new();
                let stop = |alpha: f64| {
                    let color =
                        NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., alpha).CGColor();
                    unsafe { Retained::retain(&*color as *const _ as *mut AnyObject).unwrap() }
                };
                let top_edge = (*top / r.h).min(1.);
                let bottom_edge = (1. - *bottom / r.h).max(top_edge);
                let colors = [
                    if *top > 0. { 0. } else { 1. },
                    1.,
                    1.,
                    if *bottom > 0. { 0. } else { 1. },
                ]
                .map(stop);
                unsafe {
                    gradient.setColors(Some(&NSArray::from_retained_slice(&colors)));
                }
                gradient.setLocations(Some(&NSArray::from_retained_slice(
                    &[0., top_edge, bottom_edge, 1.].map(NSNumber::new_f64),
                )));
                gradient.setStartPoint(NSPoint::new(0.5, 1.));
                gradient.setEndPoint(NSPoint::new(0.5, 0.));
                gradient.setFrame(frame);
                let shape = CAShapeLayer::new();
                shape.setFrame(gradient.bounds());
                let rounded = CGMutablePath::new();
                add_bottom_rounded(&rounded, gradient.bounds(), radius);
                shape.setPath(Some(&rounded));
                unsafe {
                    gradient.setMask(Some(&shape));
                }
                group.addSublayer(&gradient);
            } else {
                add_bottom_rounded(&path, frame, radius);
            }
        }
        *self.ivars().borrow_mut() = rects;
        self.setWantsLayer(true);
        let mask = CAShapeLayer::new();
        mask.setFrame(self.bounds());
        mask.setPath(Some(&path));
        group.addSublayer(&mask);
        if let Some(layer) = self.layer() {
            unsafe { layer.setMask(Some(&group)) }
        }
    }
}

/// Adds `rect` (y up) to `path` with continuous corners of `radius` at its
/// bottom (min y) corners and square top ones, wound as `CGPathAddRect`.
fn add_bottom_rounded(path: &CGMutablePath, rect: NSRect, radius: f64) {
    use pm_design::corner::{CONTINUOUS_REACH, continuous_corner, fit};
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    let (x1, y1) = (x0 + rect.size.width, y0 + rect.size.height);
    let r = fit(radius, rect.size.width, rect.size.height);
    let p = Some(path);
    let none = std::ptr::null();
    unsafe {
        if r <= 0. {
            CGMutablePath::add_rect(p, none, rect);
            return;
        }
        let corner = |c, a, b| {
            for [c1, c2, to] in continuous_corner(c, a, b, r).1 {
                CGMutablePath::add_curve_to_point(p, none, c1.0, c1.1, c2.0, c2.1, to.0, to.1);
            }
        };
        let k = CONTINUOUS_REACH * r;
        CGMutablePath::move_to_point(p, none, x0 + k, y0);
        CGMutablePath::add_line_to_point(p, none, x1 - k, y0);
        corner((x1, y0), (-1., 0.), (0., 1.));
        CGMutablePath::add_line_to_point(p, none, x1, y1);
        CGMutablePath::add_line_to_point(p, none, x0, y1);
        CGMutablePath::add_line_to_point(p, none, x0, y0 + k);
        corner((x0, y0), (0., 1.), (1., 0.));
        CGMutablePath::close_subpath(p);
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
    renderers: Option<PanelRenderers>,
    mirror_source: bool,
    panel_attachment: Option<PanelAttachment>,
    panel_frame: Option<NSRect>,
    ground: Retained<objc2_quartz_core::CAShapeLayer>,
    /// Where the page was last pressed ([`PageView`]), until a present
    /// names the panel there.
    pressed: Rc<Cell<Option<(f64, f64)>>>,
    /// The panels shown and their bodies (`set_bodies`); `None`: unknown,
    /// a press is judged by the regions alone.
    bodies: Option<Vec<(String, Rect)>>,
    body_fades: Vec<(Rect, [f64; 2])>,
    /// The panel last pressed, whose field the caret is in.
    caret_panel: Option<String>,
    activity: Option<Activity>,
    job_count: u32,
    /// Where the page's own content may show, when narrower than the
    /// regions (`set_content_regions`); `None`: everywhere they show.
    content: Option<Vec<Rect>>,
    /// The panels' colour under the page (sRGB, 0–1), shown where the
    /// regions show and the content does not (`set_fill`).
    fill: Option<[f64; 4]>,
    overlays: BTreeMap<String, (Retained<SnapshotView>, Retained<NSImageView>)>,
    snapshot_targets: BTreeMap<String, Rect>,
    frozen_pixels: BTreeMap<String, (Rect, Retained<NSImage>)>,
    build_masks: BTreeMap<String, (u64, f32)>,
    /// The continuous radius of each region's bottom corners
    /// (`set_corner_radius`): the panels' bodies end in their corners.
    corner_radius: f64,
}
struct PanelMirror {
    page: Box<MacDomHost>,
    sequence: u64,
    theme_applied: bool,
    batches: VecDeque<(Batch, Option<u64>)>,
    canvases: BTreeMap<u64, CanvasFrame>,
}

impl PanelMirror {
    fn send_updates(
        &mut self,
        id: &str,
        feedback: &PanelFeedback,
        theme: Option<&(String, pm_design::Scheme)>,
        fonts: &[PageFace],
    ) {
        if !self.theme_applied
            && let Some((css, scheme)) = theme
        {
            self.theme_applied = self.page.set_native_style(css, *scheme).is_ok();
        }
        if !fonts.is_empty() {
            let _ = self.page.add_fonts(fonts);
        }
        while let Some((batch, source)) = self.batches.front() {
            let mut batch = batch.clone();
            batch.seq = self.sequence + 1;
            batch.caused_by = feedback.local_input(Some(id), batch.caused_by);
            let event = json!({"kind":"batch", "panel":1, "batch":batch, "source_seq":source});
            if self.page.shared.send(event).is_err() {
                break;
            }
            self.sequence += 1;
            self.batches.pop_front();
        }
    }
}

struct PanelRenderers {
    parent: Retained<NSView>,
    editor: Retained<NSView>,
    document: PanelDocument,
    feedback: Rc<PanelFeedback>,
    views: BTreeMap<String, PanelMirror>,
    warm: WarmPanels<PanelMirror>,
    requested: Vec<(String, Rect, Rect)>,
    canvases: CanvasCache,
    fonts: BTreeMap<String, PageFace>,
    theme: Option<(String, pm_design::Scheme)>,
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
        Self::new_configured(
            mtm,
            trust,
            handler,
            stores,
            page_bundle,
            Some(&session.data_store),
        )
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
        Self::new_configured(mtm, trust, handler, stores, page_bundle, None)
    }
    fn new_configured(
        mtm: MainThreadMarker,
        trust: Trust,
        handler: Rc<dyn SyncHandler>,
        stores: &mut StoreDataStores,
        page_bundle: &str,
        identity: Option<&pm_store_host::identity::DataStoreIdentity>,
    ) -> Result<Self> {
        let host = Self::with_data_store(
            mtm,
            trust.clone(),
            handler,
            stores.get_for(&trust, identity),
            format!(
                "{page_bundle}\n{}\n{REMOTE_DOM_SHELL}",
                include_str!("panel-layout.js")
            ),
            None,
        )?;
        host.load();
        Ok(host)
    }

    fn with_data_store(
        mtm: MainThreadMarker,
        trust: Trust,
        handler: Rc<dyn SyncHandler>,
        data_store: Retained<WKWebsiteDataStore>,
        applier: String,
        process_pool: Option<&WKProcessPool>,
    ) -> Result<Self> {
        if objc2_core_graphics::CGSessionCopyCurrentDictionary().is_none() {
            return Err(Error::Unsupported("compat_gui_session_unavailable"));
        }
        let token = random_token()?;
        let shared = Rc::new(Shared {
            trust: trust.clone(),
            handler,
            channel: RefCell::new(Channel::new(token, 1)),
            panels: RefCell::new(Panels::default()),
            pending: RefCell::new(None),
            ready: Cell::new(false),
            sheets: RefCell::new(vec![]),
            panel_feedback: RefCell::default(),
            slot_owner: RefCell::default(),
            slot_dom_ready: Cell::default(),
            slot_snapshot_pending: Cell::default(),
            slot_painted: Cell::default(),
            geometry_dirty: Cell::new(true),
            closed: Cell::new(false),
            layouts: RefCell::default(),
            stale_layouts: RefCell::default(),
            retiring_layouts: RefCell::default(),
            visibility_sequence: Cell::new(0),
            layout_sequence: Cell::new(0),
            snapshots: RefCell::default(),
            overscan: RefCell::default(),
            canvases: RefCell::new(crate::canvas::CanvasResources::default()),
            fonts: RefCell::default(),
        });
        let bridge: Retained<Bridge> =
            unsafe { msg_send![super(Bridge::alloc(mtm).set_ivars(shared.clone())), init] };
        let config = unsafe { WKWebViewConfiguration::new(mtm) };
        unsafe {
            config.setWebsiteDataStore(&data_store);
            if let Some(pool) = process_pool {
                config.setProcessPool(pool);
            }
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
        let pressed: Rc<Cell<Option<(f64, f64)>>> = Rc::default();
        let web: Retained<WKWebView> = unsafe {
            let page: Retained<PageView> = msg_send![
                super(PageView::alloc(mtm).set_ivars(pressed.clone())),
                initWithFrame: frame,
                configuration: &*config
            ];
            Retained::into_super(page)
        };
        unsafe {
            web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*bridge)));
            web.setUIDelegate(Some(ProtocolObject::from_ref(&*bridge)));
            // Transparent: the native dock material supplies panel grounds;
            // the viewer stage also shows through the page.
            web.setValue_forKey(
                Some(&NSNumber::new_bool(false)),
                &NSString::from_str("drawsBackground"),
            );
        }
        refuse_drags(&web);
        let slot = SlotView::new(mtm, frame);
        slot.setWantsLayer(true);
        let ground = objc2_quartz_core::CAShapeLayer::new();
        if let Some(layer) = slot.layer() {
            layer.addSublayer(&ground);
        }
        slot.addSubview(&web);
        slot.setAccessibilityElement(true);
        slot.setAccessibilityRole(Some(&NSString::from_str("AXGroup")));
        slot.setAccessibilityLabel(Some(&NSString::from_str("Compatibility panels")));
        Ok(Self {
            shared,
            applier,
            bridge,
            web,
            pressed,
            bodies: None,
            body_fades: Vec::new(),
            caret_panel: None,
            slot,
            renderers: None,
            mirror_source: false,
            panel_attachment: None,
            panel_frame: None,
            ground,
            activity: None,
            job_count: 0,
            content: None,
            fill: None,
            overlays: BTreeMap::new(),
            snapshot_targets: BTreeMap::new(),
            frozen_pixels: BTreeMap::new(),
            build_masks: BTreeMap::new(),
            corner_radius: 0.,
        })
    }
    fn load(&self) {
        let c = self.shared.channel.borrow();
        let nonce = random_token().expect("OS entropy");
        let html = transport::page(
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
    /// Resolved theme tokens, including accessibility, from the owning shell.
    pub fn update_native_style(
        &self,
        light: &pm_design::Tokens,
        dark: &pm_design::Tokens,
        scheme: pm_design::Scheme,
    ) -> Result<()> {
        self.set_native_style(&crate::native_style::stylesheet(light, dark), scheme)
    }
    pub fn update_renderer_style(&mut self, css: &str, scheme: pm_design::Scheme) {
        if let Some(renderers) = self.renderers.as_mut() {
            renderers.theme = Some((css.to_owned(), scheme));
            for mirror in renderers
                .views
                .values_mut()
                .chain(renderers.warm.panes().map(|(_, pane)| pane))
            {
                mirror.theme_applied = mirror.page.set_native_style(css, scheme).is_ok();
            }
        }
        let _ = self.set_native_style(css, scheme);
    }

    /// Reuse a generated sheet across every rendering document in a window.
    pub fn set_native_style(&self, css: &str, scheme: pm_design::Scheme) -> Result<()> {
        self.shared.send(serde_json::json!({
            "kind": "theme",
            "css": css,
            "scheme": if scheme == pm_design::Scheme::Dark { "dark" } else { "light" },
        }))
    }
    pub fn web_view(&self) -> &WKWebView {
        &self.web
    }
    /// The bodies of the page's open ui.modal sheets.
    pub fn child_web_views(&self) -> Vec<Retained<WKWebView>> {
        self.shared
            .sheets
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
        if !self.shared.ready.get() {
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
        Ok(self.evaluate_json(generation, &source))
    }
    /// One contributed panel's text and controls, as an agent reads a panel
    /// (`get_panel_state`): its `section[data-pm-contribution]` read by a fixed
    /// script (nothing an extension or caller supplies runs). Password and
    /// file inputs are left out. Invoke on AppKit and poll the ticket.
    pub fn read_contribution(&self, panel: &str) -> Result<CaptureTicket<Value>> {
        let generation = self.generation();
        if !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        if panel.len() > 512 {
            return Err(Error::InvalidRequest);
        }
        let id = serde_json::to_string(panel).map_err(|_| Error::InvalidRequest)?;
        Ok(self.evaluate_json(generation, &READ_CONTRIBUTION.replace("$ID", &id)))
    }
    /// Runs one of this crate's fixed scripts, which returns JSON text.
    fn evaluate_json(&self, generation: Generation, source: &str) -> CaptureTicket<Value> {
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
            self.web
                .evaluateJavaScript_completionHandler(&NSString::from_str(source), Some(&callback));
        }
        ticket
    }
    /// Actual WebKit snapshot completion, independent of DOM batch admission.
    /// This captures this rendering document; the app owns full-window captures.
    pub fn capture_png(&self, generation: Generation) -> Result<CaptureTicket<Vec<u8>>> {
        if generation != self.generation() {
            return Err(Error::StaleGeneration);
        }
        self.snapshot(generation, None)
    }
    /// The page's pixels inside `rect` (page points, top-left origin): one
    /// panel's body, as WebKit draws it.
    pub fn capture_rect_png(&self, rect: Rect) -> Result<CaptureTicket<Vec<u8>>> {
        if !(rect.w > 0. && rect.h > 0.) {
            return Err(Error::InvalidRequest);
        }
        let mtm = MainThreadMarker::new().ok_or(Error::InvalidRequest)?;
        let configuration = unsafe { WKSnapshotConfiguration::new(mtm) };
        unsafe {
            configuration.setRect(NSRect::new(
                NSPoint::new(rect.x, rect.y),
                NSSize::new(rect.w, rect.h),
            ));
        }
        self.snapshot(self.generation(), Some(&configuration))
    }
    fn snapshot(
        &self,
        generation: Generation,
        configuration: Option<&WKSnapshotConfiguration>,
    ) -> Result<CaptureTicket<Vec<u8>>> {
        if !self.shared.ready.get() {
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
                .takeSnapshotWithConfiguration_completionHandler(configuration, &callback);
        }
        Ok(ticket)
    }
    /// Nonblocking main-thread admission. Pixels were encoded on the realm worker.
    /// Faces for the page's text layout: each new one is served by the `font`
    /// route and loaded as a `FontFace` before any later batch applies; the
    /// page answers `HostEvent::FontsLoaded` once they are.
    pub fn add_fonts(&mut self, faces: &[PageFace]) -> Result<()> {
        if let Some(renderers) = self.renderers.as_mut() {
            renderers
                .fonts
                .extend(faces.iter().map(|face| (face.key(), face.clone())));
        }
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
        if !self.shared.ready.get() {
            return Err(Error::Backpressure);
        }
        if let Some(renderers) = self.renderers.as_mut() {
            renderers.canvases.update(frame);
            for mirror in renderers
                .views
                .values_mut()
                .chain(renderers.warm.panes().map(|(_, pane)| pane))
            {
                mirror.canvases.insert(frame.node, frame.clone());
            }
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
    /// Install below GPUI's drawing view. Do not call from the present callback.
    pub fn install_below(&self, container: &NSView, gpui: &NSView) {
        container.addSubview_positioned_relativeTo(
            &self.slot,
            NSWindowOrderingMode::Below,
            Some(gpui),
        );
        refuse_drags(&self.web);
    }
    /// Keep a stage page for window-level drawing and render visible
    /// contributions in independent, clipped pages of the same realm.
    pub fn enable_panel_renderers(&mut self, parent: &NSView, editor: &NSView) {
        let feedback = Rc::new(PanelFeedback::new(self.shared.handler.clone()));
        self.shared.panel_feedback.replace(Some(feedback.clone()));
        self.renderers = Some(PanelRenderers {
            parent: parent.retain(),
            editor: editor.retain(),
            feedback,
            document: PanelDocument::default(),
            views: BTreeMap::new(),
            warm: WarmPanels::default(),
            requested: Vec::new(),
            canvases: CanvasCache::default(),
            fonts: BTreeMap::new(),
            theme: None,
        });
    }

    pub fn present_panel_renderers(&mut self, placements: &[(String, Rect, Rect)]) -> bool {
        let Some(renderers) = self.renderers.as_mut() else {
            return false;
        };
        let changed = renderers.requested != placements;
        if changed {
            renderers.requested = placements.to_vec();
        }
        let now = std::time::Instant::now();
        let retired: Vec<_> = renderers
            .views
            .keys()
            .filter(|id| {
                !placements
                    .iter()
                    .any(|(panel, _, clip)| panel == *id && clip.w > 0. && clip.h > 0.)
            })
            .cloned()
            .collect();
        let mut presented = !retired.is_empty();
        for id in retired {
            if let Some(mut mirror) = renderers.views.remove(&id) {
                mirror.page.detach_panel_view();
                renderers.feedback.retired(&id);
                renderers.warm.retain(id, mirror, now);
            }
        }
        let mut missing = false;
        for (id, body, clip) in placements {
            if clip.w <= 0. || clip.h <= 0. {
                continue;
            }
            if !renderers.views.contains_key(id)
                && let Some(mirror) = renderers.warm.reveal(id, now)
            {
                renderers.feedback.opening(id);
                renderers.views.insert(id.clone(), mirror);
            }
            if let Some(mirror) = renderers.views.get_mut(id) {
                let fade = self.body_fades.iter().find(|(rect, _)| rect == clip);
                mirror.page.set_body_fades(
                    fade.into_iter()
                        .map(|(_, distances)| (Rect::new(0., 0., body.w, body.h), *distances))
                        .collect(),
                );
                mirror.page.set_fill(self.fill);
                mirror.page.set_corner_radius(self.corner_radius);
                if changed || mirror.page.composition_dirty() {
                    let attached = mirror.page.panel_is_attached();
                    if mirror.page.present_panel_view(id, *body, *clip).is_ok() {
                        let now_attached = mirror.page.panel_is_attached();
                        if now_attached {
                            renderers.feedback.place(id, (body.x, body.y));
                        }
                        presented |= attached != now_attached || changed && now_attached;
                    }
                }
            } else {
                missing = true;
            }
        }
        if changed && (missing || renderers.warm.next_expiry().is_some()) {
            self.shared.handler.event(HostEvent::GeometryChanged);
        }
        presented
    }

    #[cfg(feature = "macos-probes")]
    #[doc(hidden)]
    pub fn panel_probe(&self, id: &str) -> Option<&MacDomHost> {
        self.renderers
            .as_ref()?
            .views
            .get(id)
            .map(|mirror| &*mirror.page)
    }

    pub fn warm_panels(&self) -> Vec<(String, std::time::Instant)> {
        self.renderers.as_ref().map_or_else(Vec::new, |renderers| {
            renderers
                .warm
                .ids_and_deadlines()
                .map(|(id, deadline)| (id.to_owned(), deadline))
                .collect()
        })
    }

    pub fn evict_warm_panel(&mut self, id: &str) {
        if let Some(renderers) = &mut self.renderers {
            renderers.warm.remove(id);
        }
    }

    pub fn next_panel_expiry(&self) -> Option<std::time::Instant> {
        self.renderers.as_ref()?.warm.next_expiry()
    }

    /// Apply admitted deltas and seed newly visible renderers. No WebKit
    /// document runs an extension module or creates another execution realm.
    pub fn service_panel_renderers(&mut self) {
        let Some(renderers) = self.renderers.as_mut() else {
            return;
        };
        renderers.warm.expire(std::time::Instant::now());
        let mtm = MainThreadMarker::from(&*self.web);
        let mut seed = None;
        let fonts: Vec<_> = renderers.fonts.values().cloned().collect();
        for (id, body, clip) in &renderers.requested {
            if clip.w <= 0. || clip.h <= 0. || renderers.views.contains_key(id) {
                continue;
            }
            let Some(snapshot) = seed.get_or_insert_with(|| renderers.document.snapshot()) else {
                continue;
            };
            let handler = Rc::new(PanelPageEvents {
                owner: id.clone(),
                feedback: renderers.feedback.clone(),
            });
            let config = unsafe { self.web.configuration() };
            let (data_store, process_pool) =
                unsafe { (config.websiteDataStore(), config.processPool()) };
            let Ok(mut page) = Self::with_data_store(
                mtm,
                self.shared.trust.clone(),
                handler,
                data_store,
                self.applier.clone(),
                Some(&process_pool),
            ) else {
                continue;
            };
            page.mirror_source = true;
            let preferences = unsafe { page.web.configuration().preferences() };
            if preferences.respondsToSelector(sel!(setInactiveSchedulingPolicy:)) {
                unsafe {
                    preferences.setInactiveSchedulingPolicy(WKInactiveSchedulingPolicy::None)
                };
            }
            page.install_panel_below(&renderers.parent, &renderers.editor);
            if page
                .create(PanelSpec {
                    id: 1,
                    label: id.clone(),
                })
                .is_err()
            {
                continue;
            }
            // Keep the page's first delivery hidden until it paints the
            // current owner. Input routing remains distinct for every pane.
            let viewport = Rect::new(0., 0., body.w, body.h);
            let _ = page.present(viewport);
            let _ = page.attach(
                1,
                PanelSlot {
                    bounds: viewport,
                    clip: viewport,
                },
            );
            if page.shared.queue_document().is_err() || page.prepare_panel_document(id).is_err() {
                continue;
            }
            let template = Batch {
                version: snapshot.version,
                session: snapshot.session.clone(),
                seq: snapshot.seq,
                caused_by: None,
                ops: Vec::new(),
            };
            let mut batches = VecDeque::new();
            let mut operations = Vec::new();
            let mut size = 0;
            for op in &snapshot.ops {
                let bytes = serde_json::to_vec(op).map_or(0, |bytes| bytes.len() + 1);
                if (size + bytes > MAX_MESSAGE_BYTES - 4096 || operations.len() >= 8192)
                    && !operations.is_empty()
                {
                    batches.push_back((
                        Batch {
                            ops: std::mem::take(&mut operations),
                            ..template.clone()
                        },
                        None,
                    ));
                    size = 0;
                }
                operations.push(op.clone());
                size += bytes;
            }
            batches.push_back((
                Batch {
                    ops: operations,
                    ..template
                },
                Some(snapshot.seq),
            ));
            renderers.feedback.new_document(id);
            renderers.feedback.opening(id);
            let mut mirror = PanelMirror {
                page: Box::new(page),
                sequence: 0,
                theme_applied: false,
                batches,
                canvases: renderers
                    .canvases
                    .frames()
                    .map(|frame| (frame.node, frame.clone()))
                    .collect(),
            };
            mirror.send_updates(id, &renderers.feedback, renderers.theme.as_ref(), &fonts);
            mirror.page.load();
            renderers.views.insert(id.clone(), mirror);
        }
        for (id, mirror) in renderers
            .views
            .iter_mut()
            .map(|(id, pane)| (id.as_str(), pane))
            .chain(renderers.warm.panes())
        {
            if !mirror.page.shared.ready.get() || mirror.page.prepare_panel_document(id).is_err() {
                continue;
            }
            mirror.send_updates(id, &renderers.feedback, renderers.theme.as_ref(), &fonts);
            if mirror.batches.is_empty() {
                mirror.canvases.retain(|_, frame| {
                    let mut local = frame.clone();
                    local.dom_sequence = mirror.sequence;
                    mirror.page.present_canvas(1, &local).is_err()
                });
            }
        }
    }

    /// Give this rendering document a physically separate view for one
    /// contribution. The first presentation attaches it when it can paint.
    pub fn install_panel_below(&mut self, container: &NSView, editor: &NSView) {
        let mtm = MainThreadMarker::from(&*self.web);
        self.slot.removeFromSuperview();
        self.panel_attachment = Some(PanelAttachment::new(mtm, container, editor, &self.slot));
    }

    pub fn panel_is_attached(&self) -> bool {
        self.panel_frame.is_some() && self.shared.slot_painted.get()
    }

    pub fn detach_panel_view(&mut self) {
        if self.panel_frame.is_some() {
            self.release_keyboard();
        }
        if let Some(attachment) = &self.panel_attachment {
            attachment.detach();
        }
        self.panel_frame = None;
    }

    fn prepare_panel_document(&self, id: &str) -> Result<()> {
        if self.shared.slot_owner.borrow().as_deref() != Some(id) {
            if let Some(attachment) = &self.panel_attachment {
                attachment.detach();
            }
            self.shared.send(json!({
                "kind": "panel_slot", "panel": 0, "id": id, "wait_for_seed": self.mirror_source
            }))?;
            self.shared.slot_owner.replace(Some(id.to_owned()));
            self.shared.slot_dom_ready.set(false);
            self.shared.slot_snapshot_pending.set(false);
            self.shared.slot_painted.set(false);
        }
        Ok(())
    }

    fn prime_panel_pixels(&self, id: &str) {
        if !self.shared.slot_dom_ready.get() || self.shared.slot_snapshot_pending.replace(true) {
            return;
        }
        let id = id.to_owned();
        let shared = self.shared.clone();
        let painted = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            if shared.closed.get() || shared.slot_owner.borrow().as_deref() != Some(id.as_str()) {
                return;
            }
            shared.slot_snapshot_pending.set(false);
            if error.is_null()
                && unsafe { image.as_ref() }.is_some_and(|image| image.size().width > 0.)
            {
                shared.slot_painted.set(true);
                shared.geometry_dirty.set(true);
                shared.handler.event(HostEvent::GeometryChanged);
            }
        });
        unsafe {
            self.web
                .takeSnapshotWithConfiguration_completionHandler(None, &painted)
        };
    }

    /// Slot bounds and clipping come from the exact frame GPUI paints. WebKit
    /// has its own viewport and cannot spill into another panel or its header.
    pub fn present_panel_view(&mut self, id: &str, body: Rect, clip: Rect) -> Result<()> {
        if !body.valid() || !clip.valid() {
            return Err(Error::InvalidGeometry);
        }
        let Some(attachment) = &self.panel_attachment else {
            return Err(Error::InvalidGeometry);
        };
        if !self.shared.ready.get() {
            return Ok(());
        }
        self.prepare_panel_document(id)?;
        if !self.shared.slot_painted.get() {
            let frame = NSRect::new(NSPoint::ZERO, NSSize::new(body.w, body.h));
            if self.web.frame() != frame {
                self.web.setFrame(frame);
            }
            self.prime_panel_pixels(id);
            return Ok(());
        }
        self.panel_frame = attachment.place(body, clip);
        if self.panel_frame.is_none() {
            return Ok(());
        }
        let viewport = Rect::new(0., 0., body.w, body.h);
        let mut layouts = self.shared.layouts.borrow_mut();
        if layouts.get(id).is_none_or(|(_, rect, _)| *rect != viewport) {
            let sequence = self.shared.layout_sequence.get() + 1;
            self.shared.layout_sequence.set(sequence);
            layouts.clear();
            layouts.insert(id.to_owned(), (sequence, viewport, true));
        }
        drop(layouts);
        self.set_regions_with_transparent_overlays(vec![viewport], Vec::new())?;
        self.set_content_regions(None)?;
        self.present(viewport)
    }

    /// The continuous radius of every region's bottom corners: a panel's body
    /// ends in the panel's own rounded corners, so the page is clipped there
    /// as the shell clips its own content.
    pub fn set_corner_radius(&mut self, radius: f64) {
        if self.corner_radius != radius {
            self.corner_radius = radius;
            self.shared.geometry_dirty.set(true);
        }
    }
    /// Show panel bodies on their ground while keeping overlay regions transparent.
    /// The viewer stage shares the page, but must never inherit the panel fill.
    pub fn set_regions_with_transparent_overlays(
        &mut self,
        panels: Vec<Rect>,
        overlays: Vec<Rect>,
    ) -> Result<()> {
        if self
            .shared
            .panels
            .borrow_mut()
            .set_regions_with_transparent_overlays(panels, overlays)?
        {
            self.shared.geometry_dirty.set(true);
        }
        Ok(())
    }
    /// Where the page's content may show within the regions: a panel whose
    /// content the page has not moved yet shows its fill there instead of
    /// another panel's content. `None` shows it wherever the regions do.
    pub fn set_content_regions(&mut self, content: Option<Vec<Rect>>) -> Result<()> {
        if let Some(c) = &content
            && (c.len() > 256 || c.iter().any(|r| !r.valid()))
        {
            return Err(Error::InvalidGeometry);
        }
        if self.content != content {
            self.content = content;
            self.shared.geometry_dirty.set(true);
        }
        Ok(())
    }
    /// The colour shown under the page within its regions (the panels'
    /// ground), sRGB red, green, blue and alpha in 0–1.
    /// Whether the panels' ground, as last presented, covers window point
    /// (`x`, `y`), top-left origin: the slot's own background, or the ground
    /// layer's path with a colour. False where the page shows what lies
    /// under it (the viewer's stage, PM-378). For tests.
    #[doc(hidden)]
    pub fn ground_covers(&self, x: f64, y: f64) -> bool {
        let Some(layer) = self.slot.layer() else {
            return false;
        };
        if layer.backgroundColor().is_some() {
            return true;
        }
        let h = self.slot.bounds().size.height;
        let point = NSPoint::new(x, h - y);
        self.ground.fillColor().is_some()
            && self.ground.path().is_some_and(|path| unsafe {
                objc2_core_graphics::CGPath::contains_point(
                    Some(&path),
                    std::ptr::null(),
                    point,
                    false,
                )
            })
    }
    /// The extension panels shown through this page and their bodies, so a
    /// press is known by its panel: a tab beside it shows in the same body.
    pub fn set_bodies(&mut self, bodies: Vec<(String, Rect)>) {
        if self.bodies.as_ref() != Some(&bodies) {
            // Resolve a pending press against the slot that received it.
            if self.bodies.is_some() {
                self.caret_left();
            }
            self.bodies = Some(bodies);
            self.shared.geometry_dirty.set(true);
        }
    }
    pub fn set_body_fades(&mut self, fades: Vec<(Rect, [f64; 2])>) {
        if self.body_fades != fades {
            self.body_fades = fades;
            self.shared.geometry_dirty.set(true);
        }
    }
    /// Whether the panel last pressed stopped showing (PM-294).
    fn caret_left(&mut self) -> bool {
        let Some(bodies) = &self.bodies else {
            let Some((x, y)) = self.pressed.get() else {
                return false;
            };
            return !match &self.content {
                Some(content) => content.iter().any(|r| r.contains(x, y)),
                None => self
                    .shared
                    .panels
                    .borrow()
                    .visible()
                    .iter()
                    .any(|r| r.contains(x, y)),
            };
        };
        if let Some((x, y)) = self.pressed.take() {
            self.caret_panel = bodies
                .iter()
                .find(|(_, r)| r.contains(x, y))
                .map(|(id, _)| id.clone());
        }
        self.caret_panel
            .as_ref()
            .is_some_and(|id| !bodies.iter().any(|(shown, _)| shown == id))
    }
    pub fn set_fill(&mut self, fill: Option<[f64; 4]>) {
        if self.fill != fill {
            self.fill = fill;
            self.shared.geometry_dirty.set(true);
        }
    }
    /// Reuse the asynchronous WK snapshot cache. A new commit invalidates only
    /// this panel; frame changes are already keyed by layout generation.
    pub fn panel_build_snapshot(
        &mut self,
        id: &str,
        revision: u64,
        live_alpha: f32,
    ) -> (bool, Option<(Rect, Retained<NSImage>)>) {
        if let Some(renderers) = &mut self.renderers {
            return renderers.views.get_mut(id).map_or((false, None), |mirror| {
                mirror.page.panel_build_snapshot(id, revision, live_alpha)
            });
        }
        if self.build_masks.get(id).map(|v| v.0) != Some(revision) {
            let mut snapshots = self.shared.snapshots.borrow_mut();
            let snapshot = snapshots.entry(id.to_owned()).or_default();
            snapshot.invalidate_build();
        }
        if self
            .build_masks
            .insert(id.to_owned(), (revision, live_alpha))
            != Some((revision, live_alpha))
        {
            self.shared.geometry_dirty.set(true);
        }
        let snapshots = self.shared.snapshots.borrow();
        let snapshot = snapshots.get(id);
        let image = snapshot.and_then(|s| s.image.clone());
        (snapshot.is_some_and(|s| s.build_done), image)
    }
    pub fn forget_panel_snapshot(&mut self, id: &str) {
        if let Some(renderers) = &mut self.renderers {
            if let Some(mirror) = renderers.views.get_mut(id) {
                mirror.page.forget_panel_snapshot(id);
            }
            renderers.warm.remove(id);
        }
        self.shared.snapshots.borrow_mut().remove(id);
    }
    pub fn finish_panel_build(&mut self, id: &str) {
        if let Some(mirror) = self.renderers.as_mut().and_then(|r| r.views.get_mut(id)) {
            mirror.page.finish_panel_build(id);
        }
        if self.build_masks.remove(id).is_some() {
            self.shared.geometry_dirty.set(true);
        }
    }
    /// Only the matching layout generation can release its native safety mask.
    pub fn panel_layout_applied(&self, id: &str, rect: Rect) -> bool {
        if let Some(renderers) = &self.renderers {
            return renderers.views.get(id).is_some_and(|mirror| {
                mirror.page.panel_is_attached()
                    && mirror.page.web.frame().size == NSSize::new(rect.w, rect.h)
            });
        }
        self.shared
            .layouts
            .borrow()
            .get(id)
            .is_some_and(|(_, placed, applied)| *placed == rect && *applied)
    }
    /// Send only changed geometry directly to the page, once per native frame.
    pub fn place_panels(
        &mut self,
        panels: Vec<(String, Rect)>,
        freeze: &BTreeSet<String>,
    ) -> Result<()> {
        if !self.shared.ready.get() {
            return Ok(());
        }
        let mut layouts = self.shared.layouts.borrow().clone();
        let mut stale = self.shared.stale_layouts.borrow().clone();
        let snapshots = self.shared.snapshots.borrow();
        let snapshot_targets: BTreeMap<_, _> = panels
            .iter()
            .filter(|(id, _)| {
                freeze.contains(id)
                    && layouts.contains_key(id)
                    && snapshots.get(id).is_some_and(|s| s.image.is_some())
            })
            .cloned()
            .collect();
        drop(snapshots);
        let mut changed = vec![];
        let active: Vec<String> = panels.iter().map(|(id, _)| id.clone()).collect();
        let visibility = self.shared.visibility_sequence.get() + 1;
        let mut retiring = self.shared.retiring_layouts.borrow().clone();
        let mut retired = false;
        layouts.retain(|id, (_, rect, _)| {
            if active.contains(id) {
                return true;
            }
            retired = true;
            let mut footprint = stale
                .remove(id)
                .map_or(*rect, |old| covering_rect(old, *rect));
            if let Some((_, old)) = retiring.get(id) {
                footprint = covering_rect(*old, footprint);
            }
            retiring.insert(id.clone(), (visibility, footprint));
            false
        });
        for (id, rect) in panels {
            if !rect.valid() {
                return Err(Error::InvalidGeometry);
            }
            let previous = layouts.get(&id);
            if snapshot_targets.contains_key(&id) {
                continue;
            }
            if !retired && previous.is_some_and(|(_, old, _)| *old == rect) {
                continue;
            }
            if let Some((_, old, _)) = previous {
                stale
                    .entry(id.clone())
                    .and_modify(|r| *r = covering_rect(*r, *old))
                    .or_insert(*old);
            }
            let generation = self.shared.layout_sequence.get() + 1;
            self.shared.layout_sequence.set(generation);
            layouts.insert(id.clone(), (generation, rect, false));
            changed.push(json!({"id":id,"generation":generation,"rect":rect}));
        }
        if retired || !changed.is_empty() {
            self.shared.send(
                json!({"kind":"panel_layout","panel":0,"placements":changed,"active":active,"sequence":visibility}),
            )?;
            self.shared.visibility_sequence.set(visibility);
            self.shared.geometry_dirty.set(true);
        }
        *self.shared.layouts.borrow_mut() = layouts;
        *self.shared.stale_layouts.borrow_mut() = stale;
        *self.shared.retiring_layouts.borrow_mut() = retiring;
        if self.snapshot_targets != snapshot_targets {
            self.frozen_pixels.retain(|id, _| snapshot_targets.contains_key(id));
            let snapshots = self.shared.snapshots.borrow();
            for id in snapshot_targets.keys() {
                if let Some(image) = snapshots.get(id).and_then(|s| s.image.as_ref()) {
                    self.frozen_pixels.entry(id.clone()).or_insert_with(|| image.clone());
                }
            }
            self.snapshot_targets = snapshot_targets;
            self.shared.geometry_dirty.set(true);
        }
        Ok(())
    }
    pub fn panel_layouts_settled(&self) -> bool {
        self.shared.layouts.borrow().values().all(|(_, _, applied)| *applied)
    }
    pub fn panel_uses_snapshot(&self, id: &str) -> bool {
        self.snapshot_targets.contains_key(id)
    }
    /// Exercise the .19 shared-page mask in the offscreen comparison.
    #[cfg(feature = "macos-probes")]
    #[doc(hidden)]
    pub fn set_panel_content_regions(
        &mut self,
        panels: &[(String, Rect, Rect)],
        stage: Option<Rect>,
    ) -> Result<()> {
        let settled = self.panel_layouts_settled();
        let content = panels
            .iter()
            .filter(|(id, rect, _)| {
                settled && !self.panel_uses_snapshot(id) && self.panel_layout_applied(id, *rect)
            })
            .map(|(_, _, clip)| *clip)
            .chain(
                stage
                    .into_iter()
                    .flat_map(|rect| self.panel_safe_stage(rect)),
            )
            .collect();
        self.set_content_regions(Some(content))
    }

    /// The actual AppKit picture, or the live region selected by the layer mask.
    #[cfg(feature = "macos-probes")]
    #[doc(hidden)]
    pub fn presented_panel_probe(&self, id: &str, x: f64, y: f64) -> (bool, Option<Vec<u8>>) {
        if let Some((view, picture)) = self.overlays.get(id) {
            if !view.isHidden() {
                let png = picture
                    .image()
                    .and_then(|image| image.TIFFRepresentation())
                    .and_then(|data| NSBitmapImageRep::imageRepWithData(&data))
                    .and_then(|rep| unsafe {
                        rep.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    })
                    .map(|data| unsafe { data.as_bytes_unchecked() }.to_vec());
                return (false, png);
            }
        }
        let layer = self.web.layer().unwrap();
        let y = if self.web.isFlipped() {
            y
        } else {
            self.web.bounds().size.height - y
        };
        let live = layer
            .mask()
            .and_then(|mask| mask.downcast::<objc2_quartz_core::CAShapeLayer>().ok())
            .and_then(|mask| mask.path())
            .is_some_and(|path| unsafe {
                objc2_core_graphics::CGPath::contains_point(
                    Some(&path),
                    std::ptr::null(),
                    NSPoint::new(x, y),
                    false,
                )
            });
        (live, None)
    }

    /// A shared page can contain a viewer overlay and panel roots. Its viewer
    /// hole must not reveal an old root beside the panel's native snapshot.
    pub fn panel_safe_stage(&self, stage: Rect) -> Vec<Rect> {
        self.shared.panel_safe_stage(stage, &self.snapshot_targets)
    }
    /// An idle clone is painted in a full-height strip outside every visible
    /// page region. The original roots, scrolling and events stay untouched.
    fn overscan_snapshot(&mut self, viewport: Rect) {
        let pending = self.shared.overscan.borrow().clone();
        if let Some(pending) = pending {
            if !pending.ready || pending.capturing {
                return;
            }
            self.shared
                .overscan
                .borrow_mut()
                .as_mut()
                .unwrap()
                .capturing = true;
            let config =
                unsafe { WKSnapshotConfiguration::new(MainThreadMarker::from(&*self.web)) };
            unsafe {
                config.setRect(NSRect::new(
                    NSPoint::new(pending.rect.x, pending.rect.y),
                    NSSize::new(pending.rect.w, pending.rect.h),
                ));
            }
            let shared = self.shared.clone();
            let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if shared.closed.get() {
                    return;
                }
                let valid = shared.capture_layout_is_current(&pending.id, pending.generation);
                if valid && error.is_null() {
                    if let Some(image) = unsafe { image.as_ref() } {
                        let mut snapshots = shared.snapshots.borrow_mut();
                        let snapshot = snapshots.entry(pending.id.clone()).or_default();
                        if snapshot.revision == pending.revision {
                            snapshot.complete(pending.rect, Some(image));
                        }
                    }
                }
                let _ = shared.send(json!({"kind":"overscan_clear","panel":0,"id":pending.id,"generation":pending.generation}));
                shared.geometry_dirty.set(true);
                shared.handler.event(HostEvent::GeometryChanged);
            });
            unsafe {
                self.web
                    .takeSnapshotWithConfiguration_completionHandler(Some(&config), &callback);
            }
            return;
        }
        let layouts = self.shared.layouts.borrow();
        if layouts.is_empty() || layouts.values().any(|(_, _, applied)| !applied) {
            return;
        }
        let snapshots = self.shared.snapshots.borrow();
        let candidate = layouts.iter().find(|(id, (generation, rect, _))| {
            let clip = self
                .bodies
                .as_ref()
                .and_then(|bodies| bodies.iter().find(|(panel, _)| panel == *id))
                .map_or(viewport, |(_, clip)| viewport.intersection(*clip));
            rect.w > 0.
                && rect.h > 0.
                && clip.w > 0.
                && clip.h > 0.
                && snapshots.get(*id).is_none_or(|snapshot| {
                    snapshot.requested != *generation
                        && snapshot.image.as_ref().is_none_or(|(source, _)| {
                            source.w != rect.w
                                || source.h < rect.h.min(viewport.h)
                                || snapshot.captured_revision < snapshot.revision
                        })
                })
        });
        let Some((id, (generation, rect, _))) = candidate else {
            return;
        };
        let regions = self.shared.panels.borrow().visible();
        let strip = [0., (viewport.w - rect.w) / 2., viewport.w - rect.w]
            .into_iter()
            .map(|x| Rect::new(x, 0., rect.w, rect.h.min(viewport.h)))
            .find(|strip| {
                regions.iter().all(|r| {
                    let overlap = strip.intersection(*r);
                    overlap.w == 0. || overlap.h == 0.
                })
            });
        let Some(strip) = strip else {
            return;
        };
        let pending = Overscan {
            revision: snapshots.get(id).map_or(0, |s| s.revision),
            id: id.clone(),
            generation: *generation,
            rect: strip,
            ready: false,
            capturing: false,
        };
        drop(snapshots);
        if self.shared.send(json!({"kind":"overscan_prepare","panel":0,"id":id,"generation":generation,"rect":strip})).is_ok() {
            self.shared.snapshots.borrow_mut().entry(id.clone()).or_default().requested = *generation;
            *self.shared.overscan.borrow_mut() = Some(pending);
        }
    }
    /// Cache settled panel pixels. A gesture pins those pixels and can scale
    /// them across the divider, clipped to the new panel instead of reflowing.
    fn present_snapshots(&mut self, viewport: Rect) {
        let layouts = self.shared.layouts.borrow().clone();
        let settled = self.panel_layouts_settled();
        self.build_masks.retain(|id, _| layouts.contains_key(id));
        self.overlays.retain(|id, (view, _)| {
            let keep = layouts.contains_key(id);
            if !keep {
                view.removeFromSuperview();
            }
            keep
        });
        for (id, (generation, layout_rect, applied)) in layouts {
            let frozen = self.snapshot_targets.get(&id).copied();
            let rect = frozen.unwrap_or(layout_rect);
            // Capturing a rectangle in the shared page is safe only when
            // every panel is in its acknowledged slot, and the whole capture
            // is on the page. Offscreen/partial panels use an isolated copy.
            let clip = self
                .bodies
                .as_ref()
                .and_then(|bodies| bodies.iter().find(|(panel, _)| panel == &id))
                .map_or(viewport, |(_, clip)| viewport.intersection(*clip));
            if frozen.is_none()
                && settled && applied && rect.intersection(clip) == rect && rect.w > 0. && rect.h > 0.
            {
                let mut snapshots = self.shared.snapshots.borrow_mut();
                let snapshot = snapshots.entry(id.clone()).or_default();
                if let Some((view, _)) = self.overlays.get(&id) {
                    view.setHidden(true);
                }
                if snapshot.requested == generation
                    || snapshot.image.as_ref().is_some_and(|(source, _)| {
                        source.w == rect.w
                            && source.h == rect.h
                            && snapshot.captured_revision == snapshot.revision
                            && snapshot.build_done
                    })
                {
                    continue;
                }
                let request_sequence = snapshot.begin_capture(generation);
                drop(snapshots);
                let shared = self.shared.clone();
                let configuration =
                    unsafe { WKSnapshotConfiguration::new(MainThreadMarker::from(&*self.web)) };
                unsafe {
                    configuration.setRect(NSRect::new(
                        NSPoint::new(rect.x, rect.y),
                        NSSize::new(rect.w, rect.h),
                    ));
                }
                let callback =
                    block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                        if shared.closed.get() {
                            return;
                        }
                        let valid = shared.capture_layout_is_current(&id, generation)
                            && shared
                                .snapshots
                                .borrow()
                                .get(&id)
                                .is_some_and(|s| s.accepts(request_sequence));
                        if !valid {
                            return;
                        }
                        let image = if error.is_null() {
                            unsafe { image.as_ref() }
                        } else {
                            None
                        };
                        if let Some(snapshot) = shared.snapshots.borrow_mut().get_mut(&id) {
                            snapshot.complete(rect, image);
                        }
                        shared.geometry_dirty.set(true);
                        shared.handler.event(HostEvent::GeometryChanged);
                    });
                unsafe {
                    self.web.takeSnapshotWithConfiguration_completionHandler(
                        Some(&configuration),
                        &callback,
                    );
                }
            } else if frozen.is_some() || !settled {
                let snapshots = self.shared.snapshots.borrow();
                let image = if frozen.is_some() {
                    self.frozen_pixels.get(&id)
                } else {
                    snapshots.get(&id).and_then(|s| s.image.as_ref())
                };
                let Some((source, image)) = image
                else {
                    if let Some((view, _)) = self.overlays.get(&id) {
                        view.setHidden(true);
                    }
                    continue;
                };
                let mtm = MainThreadMarker::from(&*self.web);
                let (view, picture) = self.overlays.entry(id).or_insert_with(|| {
                    let view: Retained<SnapshotView> =
                        unsafe { msg_send![super(SnapshotView::alloc(mtm).set_ivars(())), init] };
                    view.setWantsLayer(true);
                    if let Some(layer) = view.layer() {
                        layer.setMasksToBounds(true);
                    }
                    let picture = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
                    picture.setImageScaling(NSImageScaling::ScaleNone);
                    picture.setImageAlignment(NSImageAlignment::AlignTopLeft);
                    view.addSubview(&picture);
                    self.slot.addSubview(&view);
                    (view, picture)
                });
                // Scale only across the divider. Overscan may have captured
                // more height than the body, for scrolling behind its clip.
                let scaled = Rect { w: rect.w, ..*source };
                let (outer, inner) = snapshot_geometry(scaled, rect, clip, viewport.h);
                picture.setImageScaling(NSImageScaling::ScaleAxesIndependently);
                view.setFrame(NSRect::new(
                    NSPoint::new(outer.x, outer.y),
                    NSSize::new(outer.w, outer.h),
                ));
                picture.setFrame(NSRect::new(
                    NSPoint::new(inner.x, inner.y),
                    NSSize::new(inner.w, inner.h),
                ));
                picture.setImage(Some(image));
                if let Some(layer) = view.layer() {
                    let [r, g, b, _] = self.fill.unwrap_or([0., 0., 0., 1.]);
                    layer.setBackgroundColor(Some(
                        &NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.).CGColor(),
                    ));
                }
                view.setHidden(outer.w <= 0. || outer.h <= 0.);
            } else if let Some((view, _)) = self.overlays.get(&id) {
                view.setHidden(true);
            }
        }
    }
    /// Called in P-LAYER's present callback, in the same CATransaction as GPUI.
    /// No JS call or wait occurs here.
    /// A panel leaving its slot gives the window back the keyboard if the
    /// shared page holds it: AppKit draws the text input indicator (the
    /// input-source badge) at a page's caret while the page is first
    /// responder, so it stayed where a hidden panel's field was (PM-294).
    /// The window's own view takes the keys again, as after a click there.
    fn release_keyboard(&self) {
        let Some(window) = self.web.window() else {
            return;
        };
        let holds = window.firstResponder().is_some_and(|r| {
            r.downcast_ref::<NSView>()
                .is_some_and(|v| v.isDescendantOf(&self.web))
        });
        if holds {
            let to = self
                .panel_attachment
                .as_ref()
                .map(PanelAttachment::editor)
                .or_else(|| self.slot.editor_view())
                .or_else(|| window.contentView());
            window.makeFirstResponder(to.as_deref().map(|v| v as &NSResponder));
        }
    }
    pub fn present(&mut self, viewport: Rect) -> Result<()> {
        if !viewport.valid() || viewport.x != 0. || viewport.y != 0. {
            return Err(Error::InvalidGeometry);
        }
        self.shared.panels.borrow_mut().viewport = Rect::new(0., 0., viewport.w, viewport.h);
        let frame = self
            .panel_frame
            .or_else(|| {
                self.renderers
                    .as_ref()
                    .map(|renderers| editor_rect(viewport, &renderers.editor, &renderers.parent))
            })
            .unwrap_or_else(|| {
                NSRect::new(
                    NSPoint::new(viewport.x, viewport.y),
                    NSSize::new(viewport.w, viewport.h),
                )
            });
        if self.slot.frame() != frame {
            self.slot.setFrame(frame);
        }
        self.web.setFrame(NSRect::new(
            NSPoint::ZERO,
            NSSize::new(viewport.w, viewport.h),
        ));
        self.slot.clip(
            self.shared.panels.borrow().visible(),
            self.corner_radius,
            &self.body_fades,
        );
        if self.renderers.is_some() {
            // Body grounds are painted here. Only the stage's live drawing
            // may receive input in this full-window page.
            *self.slot.ivars().borrow_mut() = self.content.clone().unwrap_or_default();
        }
        // The panel the page's caret is in stopped showing (another tab,
        // closed, a layout without it): the editor takes the keyboard back,
        // or keys and the input-source badge stay with a hidden field
        // (PM-294). A hidden panel is only a body gone; the shared page is
        // never detached for it, and a tab beside it can have the same body.
        if self.caret_left() {
            self.caret_panel = None;
            self.pressed.set(None);
            self.release_keyboard();
        }
        // The page can cover both panels and the viewer. Only panel bodies
        // receive a ground; the viewer's native surface must show through.
        let panels = self.shared.panels.borrow();
        let visible = panels.visible();
        let rects = panels.fill_regions.as_deref().unwrap_or(&visible);
        let path = CGMutablePath::new();
        let h = self.slot.bounds().size.height;
        let cuts = self
            .build_masks
            .keys()
            .filter_map(|id| self.shared.layouts.borrow().get(id).map(|(_, r, _)| *r))
            .collect::<Vec<_>>();
        let mut ground_regions = rects.to_vec();
        for cut in &cuts {
            ground_regions = ground_regions
                .into_iter()
                .flat_map(|r| subtract_build_region(r, *cut))
                .collect();
        }
        for r in &ground_regions {
            unsafe {
                CGMutablePath::add_rect(
                    Some(&path),
                    std::ptr::null(),
                    NSRect::new(NSPoint::new(r.x, h - r.y - r.h), NSSize::new(r.w, r.h)),
                );
            }
        }
        self.ground.setFrame(self.slot.bounds());
        self.ground.setPath(Some(&path));
        let color = self
            .fill
            .map(|[r, g, b, a]| NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a).CGColor());
        self.ground.setFillColor(color.as_deref());
        drop(panels);
        self.web.setWantsLayer(true);
        if let Some(layer) = self.web.layer() {
            let build_content = if self.content.is_none() && !self.build_masks.is_empty() {
                Some(self.shared.panels.borrow().visible())
            } else {
                self.content.clone()
            };
            match &build_content {
                Some(rects) => {
                    let path = CGMutablePath::new();
                    // WebKit's view is flipped (its layer's geometry with it):
                    // the mask's rectangles are in window points from the top.
                    let h = self.web.bounds().size.height;
                    let flipped = self.web.isFlipped();
                    let overscan = self.shared.overscan.borrow().as_ref().map(|p| p.rect);
                    let regions: Vec<Rect> = rects
                        .iter()
                        .flat_map(|r| {
                            if let Some(strip) = overscan {
                                vec![
                                    r.intersection(Rect::new(0., 0., strip.x, viewport.h)),
                                    r.intersection(Rect::new(
                                        strip.x + strip.w,
                                        0.,
                                        (viewport.w - strip.x - strip.w).max(0.),
                                        viewport.h,
                                    )),
                                ]
                            } else {
                                vec![*r]
                            }
                        })
                        .filter(|r| r.w > 0. && r.h > 0.)
                        .collect();
                    let all_regions = regions.clone();
                    let mut regions = regions;
                    for cut in &cuts {
                        regions = regions
                            .into_iter()
                            .flat_map(|r| subtract_build_region(r, *cut))
                            .collect();
                    }
                    for r in &regions {
                        let y = if flipped { r.y } else { h - r.y - r.h };
                        unsafe {
                            CGMutablePath::add_rect(
                                Some(&path),
                                std::ptr::null(),
                                NSRect::new(NSPoint::new(r.x, y), NSSize::new(r.w, r.h)),
                            );
                        }
                    }
                    let mask = objc2_quartz_core::CAShapeLayer::new();
                    mask.setFrame(self.web.bounds());
                    mask.setPath(Some(&path));
                    if self.build_masks.is_empty() {
                        unsafe { layer.setMask(Some(&mask)) }
                    } else {
                        let group = objc2_quartz_core::CALayer::new();
                        group.setFrame(self.web.bounds());
                        group.addSublayer(&mask);
                        for (id, (_, alpha)) in &self.build_masks {
                            let Some((_, cut, _)) = self.shared.layouts.borrow().get(id).cloned()
                            else {
                                continue;
                            };
                            if *alpha <= 0. {
                                continue;
                            }
                            let path = CGMutablePath::new();
                            for r in &all_regions {
                                let r = r.intersection(cut);
                                if r.w <= 0. || r.h <= 0. {
                                    continue;
                                }
                                let y = if flipped { r.y } else { h - r.y - r.h };
                                unsafe {
                                    CGMutablePath::add_rect(
                                        Some(&path),
                                        std::ptr::null(),
                                        NSRect::new(NSPoint::new(r.x, y), NSSize::new(r.w, r.h)),
                                    );
                                }
                            }
                            let reveal = objc2_quartz_core::CAShapeLayer::new();
                            reveal.setFrame(self.web.bounds());
                            reveal.setPath(Some(&path));
                            reveal.setOpacity(*alpha);
                            group.addSublayer(&reveal);
                        }
                        unsafe { layer.setMask(Some(&group)) }
                    }
                }
                None => unsafe { layer.setMask(None) },
            }
        }
        if self.renderers.is_none()
            && (self.panel_attachment.is_none() || !self.build_masks.is_empty())
        {
            self.present_snapshots(viewport);
        }
        for id in self.build_masks.keys() {
            if let Some((view, _)) = self.overlays.get(id) {
                view.setHidden(true);
            }
        }
        if self.renderers.is_none()
            && self.panel_attachment.is_none()
            && (self.snapshot_targets.is_empty() || self.shared.overscan.borrow().is_some())
        {
            self.overscan_snapshot(viewport);
        }
        refuse_drags(&self.web);
        self.shared.geometry_dirty.set(false);
        Ok(())
    }
    pub fn composition_dirty(&self) -> bool {
        self.shared.geometry_dirty.get() || self.pressed.get().is_some()
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
        // Ordered in so WebKit keeps the page running; never while the person
        // hid the app, which ordering a window in would undo (PM-401).
        if !NSApplication::sharedApplication(MainThreadMarker::new().unwrap()).isHidden() {
            a.window.orderFront(None);
        }
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
    fn close_sheets(&self) {
        let sheets: Vec<_> = self.shared.sheets.borrow_mut().drain(..).collect();
        for (w, v) in sheets {
            forget_sheet(&w);
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
        if !self.shared.panels.borrow().items.is_empty() {
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
        self.shared.geometry_dirty.set(true);
        Ok(())
    }
    fn detach(&mut self, id: PanelId) -> Result<()> {
        self.check(self.generation(), id)?;
        self.release_keyboard();
        if self.shared.ready.get() {
            self.shared
                .send(json!({"kind":"slot","panel":id,"slot":Value::Null}))?;
        }
        let mut panels = self.shared.panels.borrow_mut();
        let p = panels.get(id)?;
        p.slot = None;
        self.shared.geometry_dirty.set(true);
        Ok(())
    }
    fn destroy(&mut self, id: PanelId) -> Result<()> {
        self.check(self.generation(), id)?;
        self.release_keyboard();
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
        if let Some(renderers) = self.renderers.as_mut() {
            let admitted: Batch =
                serde_json::from_value(batch).map_err(|_| Error::InvalidRequest)?;
            let mut primary = admitted.clone();
            primary.caused_by = renderers.feedback.local_input(None, primary.caused_by);
            self.shared
                .send(json!({"kind":"batch","panel":id,"batch":primary}))?;
            renderers.document.apply(&admitted);
            for mirror in renderers
                .views
                .values_mut()
                .chain(renderers.warm.panes().map(|(_, pane)| pane))
            {
                mirror
                    .batches
                    .push_back((admitted.clone(), Some(admitted.seq)));
            }
        } else {
            self.shared
                .send(json!({"kind":"batch","panel":id,"batch":batch}))?;
        }
        Ok(())
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
}
impl Drop for MacDomHost {
    fn drop(&mut self) {
        self.shared.closed.set(true);
        self.job_count = self.job_count.min(1);
        self.end_background_job();
        self.close_sheets();
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
/// the shell's borrow of the host held, such as `Drop`) must not
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

/// `read_contribution`'s script: the panel's text (even off screen) and controls
/// (role, label, value, state, panel-relative bounds), capped.
const READ_CONTRIBUTION: &str = r#"(()=>{const root=document.querySelector('[data-pm-contribution='+JSON.stringify($ID)+']');if(!root)throw Error('no such panel');const box=root.getBoundingClientRect();const text=(root.textContent||'').replace(/\s+/g,' ').trim().slice(0,20000);const controls=[];for(const el of root.querySelectorAll('button,a[href],input,select,textarea,[role=button],[role=checkbox],[role=switch],[role=slider],[role=tab],[role=menuitem],[contenteditable=""],[contenteditable=true]')){if(controls.length>=200)break;const type=(el.getAttribute('type')||'').toLowerCase();if(type==='password'||type==='file'||type==='hidden')continue;const r=el.getBoundingClientRect();if(r.width===0&&r.height===0)continue;const tag=el.tagName;const role=el.getAttribute('role')||(tag==='A'?'link':tag==='SELECT'?'combobox':tag==='TEXTAREA'?'textbox':tag==='INPUT'?(type==='checkbox'?'checkbox':type==='radio'?'radio':type==='range'?'slider':'textbox'):el.isContentEditable?'textbox':'button');const label=(el.getAttribute('aria-label')||(el.labels&&el.labels[0]&&el.labels[0].textContent)||(tag==='INPUT'||tag==='SELECT'||tag==='TEXTAREA'?'':el.textContent)||el.getAttribute('placeholder')||el.getAttribute('title')||el.getAttribute('name')||'').trim().slice(0,200);const c={role,label};if((tag==='INPUT'||tag==='TEXTAREA'||tag==='SELECT')&&type!=='checkbox'&&type!=='radio')c.value=String(el.value).slice(0,500);if(type==='checkbox'||type==='radio')c.checked=!!el.checked;if(el.disabled||el.getAttribute('aria-disabled')==='true')c.disabled=true;if(tag==='SELECT')c.options=Array.from(el.options).slice(0,100).map(o=>o.text);c.bounds={x:Math.round(r.left-box.left),y:Math.round(r.top-box.top),width:Math.round(r.width),height:Math.round(r.height)};controls.push(c);}return JSON.stringify({text,controls});})()"#;

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[derive(Default)]
    struct Handler {
        feedback: Cell<u32>,
        events: Cell<u32>,
    }
    impl SyncHandler for Handler {
        fn event(&self, _: HostEvent) {
            self.events.set(self.events.get() + 1)
        }
        fn remote_dom(&self, _: Value) -> Result<()> {
            self.feedback.set(self.feedback.get() + 1);
            Ok(())
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
            bounds: Rect::new(0., 0., 10., 10.),
            clip: Rect::new(0., 0., 10., 10.),
        });
        (
            Shared {
                trust,
                handler: handler.clone(),
                channel: RefCell::new(Channel::new("synthetic-capability".into(), 3)),
                canvases: RefCell::new(crate::canvas::CanvasResources::default()),
                fonts: RefCell::default(),
                panels: RefCell::new(panels),
                pending: RefCell::new(None),
                ready: Cell::new(true),
                sheets: RefCell::new(vec![]),
                panel_feedback: RefCell::default(),
                slot_owner: RefCell::default(),
                slot_dom_ready: Cell::default(),
                slot_snapshot_pending: Cell::default(),
                slot_painted: Cell::default(),
                geometry_dirty: Cell::new(false),
                closed: Cell::new(false),
                layouts: RefCell::default(),
                stale_layouts: RefCell::default(),
                retiring_layouts: RefCell::default(),
                visibility_sequence: Cell::new(0),
                layout_sequence: Cell::new(0),
                snapshots: RefCell::default(),
                overscan: RefCell::default(),
            },
            handler,
        )
    }
    #[test]
    fn overscan_acknowledgements_cannot_clear_or_ready_a_newer_capture() {
        let (s, _) = shared(Trust::Trusted);
        *s.overscan.borrow_mut() = Some(Overscan {
            revision: 0,
            id: "logos".into(),
            generation: 2,
            rect: Rect::new(0., 0., 300., 900.),
            ready: false,
            capturing: false,
        });
        for kind in ["overscan_ready", "overscan_cleared"] {
            s.handle(
                &json!({"generation":3,"panel":0,"kind":kind,
                "payload":{"id":"logos","generation":1}})
                .to_string(),
            )
            .unwrap();
            assert!(!s.overscan.borrow().as_ref().unwrap().ready);
        }
        s.handle(
            &json!({"generation":3,"panel":0,"kind":"overscan_ready",
            "payload":{"id":"logos","generation":2}})
            .to_string(),
        )
        .unwrap();
        assert!(s.overscan.borrow().as_ref().unwrap().ready);
        s.handle(
            &json!({"generation":3,"panel":0,"kind":"overscan_cleared",
            "payload":{"id":"logos","generation":2}})
            .to_string(),
        )
        .unwrap();
        assert!(s.overscan.borrow().is_none());
    }
    #[test]
    fn snapshot_moves_and_resizes_pin_text_to_top_left_without_scaling() {
        let source = Rect::new(50., 100., 300., 200.);
        for target in [
            Rect::new(80., 150., 300., 200.),
            Rect::new(50., 100., 180., 90.),
            Rect::new(50., 100., 400., 300.),
        ] {
            let (outer, inner) = snapshot_geometry(source, target, target, 900.);
            assert_eq!(outer.x, target.x);
            assert_eq!(outer.y + outer.h, 900. - target.y);
            assert_eq!((outer.w, outer.h), (target.w, target.h));
            assert_eq!((inner.w, inner.h), (source.w, source.h));
            assert_eq!(inner.y + inner.h, target.h);
        }
    }
    #[test]
    fn dock_panel_retirement_keeps_its_native_mask_until_the_matching_page_ack() {
        let (state, _) = shared(Trust::Trusted);
        let stage = Rect::new(0., 0., 800., 500.);
        let old = Rect::new(480., 60., 280., 140.);
        state.visibility_sequence.set(3);
        state
            .retiring_layouts
            .borrow_mut()
            .insert("mirage".into(), (3, old));
        let frozen = BTreeMap::new();
        let covered = |state: &Shared| {
            state
                .panel_safe_stage(stage, &frozen)
                .iter()
                .any(|r| r.contains(500., 100.))
        };
        assert!(!covered(&state));
        for sequence in [2, 4] {
            state
                .handle(
                    &json!({"generation":3,"panel":0,"kind":"panel_visibility_applied",
                "payload":{"sequence":sequence}})
                    .to_string(),
                )
                .unwrap();
            assert!(!covered(&state));
        }
        state
            .handle(
                &json!({"generation":3,"panel":0,"kind":"panel_visibility_applied",
            "payload":{"sequence":3}})
                .to_string(),
            )
            .unwrap();
        assert!(covered(&state));
    }

    #[test]
    fn a_pending_neighbour_invalidates_a_shared_page_capture() {
        let (state, _) = shared(Trust::Trusted);
        let rect = Rect::new(0., 0., 300., 100.);
        state.layouts.borrow_mut().extend([
            ("pixel-comparison".into(), (5, rect, true)),
            ("pexels".into(), (6, Rect::new(0., 100., 300., 100.), false)),
        ]);

        assert!(!state.capture_layout_is_current("pixel-comparison", 5));
        state.layouts.borrow_mut().get_mut("pexels").unwrap().2 = true;
        assert!(!state.capture_layout_is_current("pixel-comparison", 4));
        assert!(state.capture_layout_is_current("pixel-comparison", 5));
    }

    #[test]
    fn snapshot_clips_to_its_body_and_keeps_the_same_content_origin() {
        let source = Rect::new(0., 0., 300., 200.);
        let target = Rect::new(700., 15., 300., 200.);
        let clip = Rect::new(700., 70., 300., 110.);
        let (outer, inner) = snapshot_geometry(source, target, clip, 600.);
        assert_eq!(outer, Rect::new(700., 420., 300., 110.));
        assert_eq!(outer.y + inner.y + inner.h, 600. - target.y);
    }

    #[test]
    fn failed_snapshot_keeps_the_previous_pixels_for_that_panel() {
        let mut snapshot = PanelSnapshot::default();
        let rect = Rect::new(0., 0., 300., 100.);
        let good = NSImage::initWithSize(NSImage::alloc(), NSSize::new(300., 100.));
        snapshot.complete(rect, Some(&good));
        snapshot.begin_capture(2);
        snapshot.complete(Rect::new(0., -10., 300., 100.), None);
        let (kept_rect, kept) = snapshot.image.as_ref().unwrap();
        assert_eq!(*kept_rect, rect);
        assert!(std::ptr::eq(&**kept, &*good));
    }

    #[test]
    fn panel_layout_acknowledgements_ignore_stale_generations() {
        let (s, _) = shared(Trust::Trusted);
        s.layouts
            .borrow_mut()
            .insert("logos".into(), (2, Rect::new(0., 0., 100., 80.), false));
        let stage = Rect::new(100., 0., 400., 80.);
        s.stale_layouts
            .borrow_mut()
            .insert("logos".into(), Rect::new(0., 0., 200., 80.));
        let frozen = BTreeMap::new();
        let masked = vec![Rect::new(200., 0., 300., 80.)];
        for generation in [1, 3] {
            s.handle(
                &json!({"generation":3,"panel":0,"kind":"panel_layout_applied",
                "payload":{"id":"logos","generation":generation}})
                .to_string(),
            )
            .unwrap();
            assert!(!s.layouts.borrow()["logos"].2);
            assert_eq!(s.panel_safe_stage(stage, &frozen), masked);
        }
        s.handle(
            &json!({"generation":3,"panel":0,"kind":"panel_layout_applied",
            "payload":{"id":"logos","generation":2,"rect":{"x":0,"y":0,"w":200,"h":80}}})
            .to_string(),
        )
        .unwrap();
        assert!(!s.layouts.borrow()["logos"].2);
        assert_eq!(s.panel_safe_stage(stage, &frozen), masked);
        s.handle(
            &json!({"generation":3,"panel":0,"kind":"panel_layout_applied",
            "payload":{"id":"logos","generation":2,"rect":{"x":0,"y":0,"w":100,"h":80}}})
            .to_string(),
        )
        .unwrap();
        assert!(s.layouts.borrow()["logos"].2);
        assert_eq!(s.panel_safe_stage(stage, &frozen), [stage]);
    }
    const INPUT: &str =
        r#"{"generation":3,"panel":1,"kind":"remote_dom","payload":{"kind":"event"}}"#;
    #[test]
    fn stale_and_unknown_panel_requests_do_not_reach_handler() {
        let (s, h) = shared(Trust::Trusted);
        assert_eq!(
            s.handle(&INPUT.replace(r#""generation":3"#, r#""generation":2"#)),
            Err(Error::StaleGeneration)
        );
        assert_eq!(
            s.handle(&INPUT.replace(r#""panel":1"#, r#""panel":2"#)),
            Err(Error::UnknownPanel)
        );
        assert_eq!(h.feedback.get(), 0);
    }
    #[test]
    fn detached_panels_cannot_forward_input() {
        let (s, h) = shared(Trust::Trusted);
        assert_eq!(s.handle(INPUT), Ok(Value::Null));
        s.panels.borrow_mut().get(1).unwrap().slot = None;
        assert_eq!(s.handle(INPUT), Err(Error::Forbidden));
        assert_eq!(h.feedback.get(), 1);
        let click = INPUT.replace(r#"{"kind":"event"}"#, r#"{"kind":"event","event":"click"}"#);
        assert_eq!(s.handle(&click), Err(Error::Forbidden));
        assert_eq!(h.feedback.get(), 1);
    }
    /// PM-509: a tab switch detaches the window's page and the browser fires
    /// blur and focus in it. Refused, they stopped the page's applier and
    /// every extension panel in the window stayed blank. They reach the
    /// realm; input stays refused.
    #[test]
    fn a_detached_page_still_hears_its_own_focus_and_blur() {
        let (s, h) = shared(Trust::Trusted);
        s.panels.borrow_mut().get(1).unwrap().slot = None;
        // visibilitychange: the 17-tab run's page stopped on it (PM-509/513).
        let own = [
            "blur",
            "focusout",
            "focus",
            "focusin",
            "selectionchange",
            "scroll",
            "visibilitychange",
            "pagehide",
            "pageshow",
            "resize",
        ];
        for event in own {
            let message = INPUT.replace(
                r#"{"kind":"event"}"#,
                &format!(r#"{{"kind":"event","event":"{event}"}}"#),
            );
            assert_eq!(s.handle(&message), Ok(Value::Null), "{event}");
        }
        assert_eq!(h.feedback.get() as usize, own.len());
    }
    #[test]
    fn stopped_hosts_reject_requests() {
        let (s, h) = shared(Trust::Trusted);
        s.closed.set(true);
        assert_eq!(s.handle(INPUT), Err(Error::StaleGeneration));
        assert_eq!(h.feedback.get(), 0);
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

fn subtract_build_region(r: Rect, cut: Rect) -> Vec<Rect> {
    let i = r.intersection(cut);
    if i.w <= 0. || i.h <= 0. {
        return vec![r];
    }
    [
        Rect::new(r.x, r.y, r.w, i.y - r.y),
        Rect::new(r.x, i.y + i.h, r.w, r.y + r.h - i.y - i.h),
        Rect::new(r.x, i.y, i.x - r.x, i.h),
        Rect::new(i.x + i.w, i.y, r.x + r.w - i.x - i.w, i.h),
    ]
    .into_iter()
    .filter(|r| r.w > 0. && r.h > 0.)
    .collect()
}
#[cfg(test)]
mod build_mask_tests {
    use super::*;
    #[test]
    fn stale_commit_snapshots_cannot_release_a_newer_control() {
        let mut snapshot = PanelSnapshot::default();
        let first = snapshot.begin_capture(10);
        snapshot.invalidate_build();
        assert!(!snapshot.accepts(first));
        let second = snapshot.begin_capture(10);
        assert!(!snapshot.accepts(first));
        assert!(snapshot.accepts(second));
    }
    #[test]
    fn building_panel_mask_does_not_hide_its_neighbour() {
        let a = Rect::new(0., 0., 100., 100.);
        let b = Rect::new(100., 0., 100., 100.);
        assert_eq!(subtract_build_region(b, a), vec![b]);
        assert!(subtract_build_region(a, a).is_empty());
        let pieces = subtract_build_region(Rect::new(0., 0., 200., 100.), a);
        assert_eq!(pieces, vec![b]);
    }
}
