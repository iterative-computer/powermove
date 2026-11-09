//! The viewer for an agent's visual reply (`publish_visual_reply`): one
//! window per reply, its page under the reply's Content Security Policy, a
//! private non-persistent data store, no bridge to the app, and no navigation
//! past the document it was given (main's sandboxed visual-reply frame).
use block2::DynBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, ProtocolObject},
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
    unsafe impl NSWindowDelegate for Guard {
        /// A closed viewer gives its web view (and its WebKit process) back now.
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, notification: &NSNotification) {
            let Some(window) = notification.object() else { return };
            OPEN.with_borrow_mut(|open| {
                if let Some(viewer) = open.iter_mut().find(|v| {
                    std::ptr::eq(&*v.window as *const NSWindow as *const AnyObject, &*window)
                }) && let Some(web) = viewer.web.take()
                {
                    unsafe {
                        web.setNavigationDelegate(None);
                        web.setUIDelegate(None);
                        web.stopLoading();
                    }
                    viewer.window.setContentView(None);
                }
            });
        }
    }
);
impl Guard {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(false));
        unsafe { msg_send![super(this), init] }
    }
}
/// A viewer, by reply: its window, its web view until closed, and its guard.
struct Viewer {
    reply: String,
    window: Retained<NSWindow>,
    web: Option<Retained<WKWebView>>,
    /// The window's and web view's delegate; they hold it weakly.
    _guard: Retained<Guard>,
}
thread_local! {
    static OPEN: RefCell<Vec<Viewer>> = const { RefCell::new(Vec::new()) };
}
/// The viewer for reply `reply` (`document`, titled `title`, `height` points
/// tall) beside `parent`; a reply already showing is returned instead of
/// opening twice. It isn't ordered on screen: the caller presents it through
/// the app's window policy (P-CHILD), so it never lands above other apps
/// while Powermove is inactive. Main thread only.
pub fn show(
    parent: Option<&NSWindow>,
    reply: &str,
    title: &str,
    document: &str,
    height: f64,
) -> Retained<NSWindow> {
    let mtm = MainThreadMarker::new().expect("visual replies open on the main thread");
    let shown = OPEN.with_borrow_mut(|open| {
        // Closed viewers already gave back their web views; let them go.
        open.retain(|v| {
            let live = v.web.is_some() && v.window.isVisible();
            if !live {
                v.window.setDelegate(None);
            }
            live
        });
        open.iter()
            .find(|v| v.reply == reply)
            .map(|v| v.window.clone())
    });
    if let Some(window) = shown {
        return window;
    }
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
    window.setDelegate(Some(ProtocolObject::from_ref(&*guard)));
    unsafe { web.loadHTMLString_baseURL(&NSString::from_str(document), None) };
    OPEN.with_borrow_mut(|open| {
        open.push(Viewer {
            reply: reply.into(),
            window: window.clone(),
            web: Some(web),
            _guard: guard,
        })
    });
    window
}
