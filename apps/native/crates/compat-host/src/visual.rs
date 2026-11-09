//! The viewer for an agent's visual reply (`publish_visual_reply`): one
//! window per reply, its page under the reply's Content Security Policy, a
//! private non-persistent data store, no bridge to the app, and no navigation
//! past the document it was given (main's sandboxed visual-reply frame).
use block2::DynBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_app_kit::*;
use objc2_foundation::*;
use objc2_web_kit::*;
use std::cell::{Cell, RefCell};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name="PMVisualReplyGuard"]
    #[ivars=Cell<bool>]
    struct Guard;
    unsafe impl NSObjectProtocol for Guard {}
    unsafe impl WKNavigationDelegate for Guard {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn policy(&self,_web:&WKWebView,action:&WKNavigationAction,done:&DynBlock<dyn Fn(WKNavigationActionPolicy)>){
            let url=unsafe{action.request()}.URL().and_then(|u|u.absoluteString()).map(|s|s.to_string()).unwrap_or_default();
            // Only the document it was given, once; fragment links stay in it.
            let first=!self.ivars().replace(true);
            let allow=url=="about:blank" && first || url.starts_with("about:blank#");
            done.call((if allow{WKNavigationActionPolicy::Allow}else{WKNavigationActionPolicy::Cancel},));
        }
    }
    unsafe impl WKUIDelegate for Guard {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        unsafe fn popout(&self,_web:&WKWebView,_cfg:&WKWebViewConfiguration,_action:&WKNavigationAction,_features:&WKWindowFeatures)->Option<Retained<WKWebView>>{None}
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
impl Guard {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(false));
        unsafe { msg_send![super(this), init] }
    }
}
/// An open viewer: its window, web view and navigation guard.
type Viewer = (Retained<NSWindow>, Retained<WKWebView>, Retained<Guard>);
thread_local! {
    /// Open viewers; closed ones are let go when the next one opens.
    static OPEN: RefCell<Vec<Viewer>> = const { RefCell::new(Vec::new()) };
}
/// Show `document` titled `title`, `height` points tall, beside `parent`.
/// Main thread only. Returns the window number (for tests and automation).
pub fn show(parent: Option<&NSWindow>, title: &str, document: &str, height: f64) -> isize {
    let mtm = MainThreadMarker::new().expect("visual replies open on the main thread");
    OPEN.with_borrow_mut(|open| open.retain(|(w, _, _)| w.isVisible()));
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    unsafe {
        config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
        let prefs = config.preferences();
        prefs.setJavaScriptCanOpenWindowsAutomatically(false);
        prefs.setValue_forKey(
            Some(&NSNumber::new_bool(false)),
            &NSString::from_str("developerExtrasEnabled"),
        );
    }
    let size = NSSize::new(720., height.clamp(160., 900.));
    let web = unsafe {
        WKWebView::initWithFrame_configuration(
            WKWebView::alloc(mtm),
            NSRect::new(NSPoint::ZERO, size),
            &config,
        )
    };
    let guard = Guard::new(mtm);
    unsafe {
        web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*guard)));
        web.setUIDelegate(Some(ProtocolObject::from_ref(&*guard)));
    }
    web.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, size),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(title));
    window.setContentMinSize(NSSize::new(320., 160.));
    window.setContentView(Some(&web));
    match parent {
        Some(parent) => {
            let frame = parent.frame();
            let origin = NSPoint::new(
                frame.origin.x + (frame.size.width - size.width) / 2.,
                frame.origin.y + (frame.size.height - size.height) / 2.,
            );
            window.setFrameOrigin(origin);
        }
        None => window.center(),
    }
    unsafe { web.loadHTMLString_baseURL(&NSString::from_str(document), None) };
    window.makeKeyAndOrderFront(None);
    let number = window.windowNumber();
    OPEN.with_borrow_mut(|open| open.push((window, web, guard)));
    number
}
