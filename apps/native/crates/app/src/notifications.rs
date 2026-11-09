//! S114 agent notifications and their authorization outcome (N007), over
//! pm-services `surface_backing::authorization`. The agent panel's run loop
//! tells us a run was sent, finished or asked a question; the policy decides,
//! `UNUserNotificationCenter` presents. Authorization is asked the first time
//! a run is sent, never at launch, and a denial changes nothing in the app.
use crate::session::Controller;
use block2::RcBlock;
use gpui::*;
use objc2::{
    AnyThread, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use pm_panel_agentpanel::runtime::Notice;
use pm_services::{
    Result as ServiceResult,
    surface_backing::authorization::{
        Authorization, AuthorizationHost, AuthorizationRequest, Capability, CompletionNotice,
        Delivery, NotificationContext, NotificationHost, Notifications,
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

/// The bundled sound, played by the notification itself so Focus applies.
const SOUND: &str = "little-victory-deep.wav";

static NOTICES: OnceLock<async_channel::Sender<(u64, Notice)>> = OnceLock::new();
/// Last known authorization, written from the center's callbacks.
static STATUS: AtomicU8 = AtomicU8::new(0);
static CLICKS: OnceLock<async_channel::Sender<String>> = OnceLock::new();

fn encode(a: Authorization) -> u8 {
    match a {
        Authorization::NotDetermined => 0,
        Authorization::Authorized => 1,
        Authorization::Provisional => 2,
        Authorization::Denied => 3,
        _ => 4,
    }
}
pub fn status() -> Authorization {
    match STATUS.load(Ordering::Acquire) {
        0 => Authorization::NotDetermined,
        1 => Authorization::Authorized,
        2 => Authorization::Provisional,
        3 => Authorization::Denied,
        _ => Authorization::Unavailable,
    }
}

/// The sink one window's agent runtime reports to.
pub fn sink(window_key: u64) -> Arc<pm_builtins::NoticeSink> {
    Arc::new(move |notice| {
        if let Some(tx) = NOTICES.get() {
            let _ = tx.try_send((window_key, notice));
        }
    })
}

struct State {
    policy: Notifications,
    center: Option<Center>,
    /// Delivered notices by identifier (thread), for a click.
    delivered: HashMap<String, CompletionNotice>,
}
impl Global for State {}

/// Once per app: listens for notices and notification clicks.
pub fn install(cx: &mut App) {
    if NOTICES.get().is_some() {
        return;
    }
    let (tx, rx) = async_channel::unbounded();
    let _ = NOTICES.set(tx);
    let (clicks_tx, clicks) = async_channel::unbounded();
    let _ = CLICKS.set(clicks_tx);
    cx.set_global(State {
        policy: Notifications::default(),
        center: Center::new(),
        delivered: HashMap::new(),
    });
    if let Some(center) = &cx.global::<State>().center {
        center.refresh();
    }
    cx.spawn(async move |cx| {
        while let Ok((window, notice)) = rx.recv().await {
            cx.update(|cx| handle(window, notice, cx));
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(thread) = clicks.recv().await {
            cx.update(|cx| clicked(&thread, cx));
        }
    })
    .detach();
}

fn handle(window: u64, notice: Notice, cx: &mut App) {
    let state = cx.global_mut::<State>();
    let Some(center) = state.center.as_ref() else {
        return;
    };
    center.refresh();
    let mut host = Host(center);
    let (notice, stopped) = match notice {
        Notice::Sent { .. } => {
            let _ = state.policy.run_sent(&mut host);
            return;
        }
        Notice::Finished {
            thread,
            run,
            stopped,
        } => (
            CompletionNotice::finished(run.clone(), run, thread, window.to_string()),
            stopped,
        ),
        Notice::Question {
            thread,
            run,
            item,
            text,
        } => (
            CompletionNotice::question(item, run, thread, window.to_string(), &text),
            false,
        ),
    };
    let context = NotificationContext {
        enabled: true,
        frontmost: frontmost(),
        agent_visible: agent_visible(window, cx),
        stopped_by_user: stopped,
    };
    let state = cx.global_mut::<State>();
    let Some(center) = state.center.as_ref() else {
        return;
    };
    if let Ok(Delivery::Sent) = state.policy.complete(&mut Host(center), &notice, context) {
        state
            .delivered
            .insert(notice.identifier().to_owned(), notice);
    }
}

fn frontmost() -> bool {
    objc2::MainThreadMarker::new()
        .is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive())
}
fn controller(window: u64, cx: &App) -> Option<Entity<Controller>> {
    crate::entry::editor(window, cx)
}
fn agent_visible(window: u64, cx: &App) -> bool {
    controller(window, cx).is_some_and(|c| c.read(cx).builtins.windows.contains_key("agent"))
}

/// A click brings the thread's window forward with the Agent open.
fn clicked(thread: &str, cx: &mut App) {
    let state = cx.global::<State>();
    let Some(notice) = state.delivered.get(thread).cloned() else {
        return;
    };
    let Some(center) = state.center.as_ref() else {
        return;
    };
    let _ = state.policy.activate(&mut Host(center), &notice);
    let Some(window) = notice.window_id.parse::<u64>().ok() else {
        return;
    };
    let Some(editor) = controller(window, cx) else {
        return;
    };
    if let Some(handle) = cx
        .windows()
        .into_iter()
        .find(|w| w.window_id().as_u64() == window)
    {
        let _ = handle.update(cx, |_, w, cx| {
            w.activate();
            editor.update(cx, |v, cx| {
                if !v.builtins.windows.contains_key("agent") {
                    v.show_builtin("agent", w, cx);
                }
            });
        });
    }
}

struct Host<'a>(&'a Center);
impl AuthorizationHost for Host<'_> {
    fn status(&mut self, capability: Capability, _: &str) -> ServiceResult<Authorization> {
        Ok(match capability {
            Capability::Notifications => status(),
            _ => Authorization::Unavailable,
        })
    }
    fn request(&mut self, request: &AuthorizationRequest) -> ServiceResult<Authorization> {
        if request.capability == Capability::Notifications {
            self.0.request();
        }
        Ok(status())
    }
}
impl NotificationHost for Host<'_> {
    fn deliver(&mut self, notice: &CompletionNotice) -> ServiceResult<()> {
        self.0.deliver(notice);
        Ok(())
    }
    /// The click itself is handled in `clicked`, which has the App.
    fn activate_thread(&mut self, _: &str, _: &str) -> ServiceResult<()> {
        Ok(())
    }
}

struct Center {
    center: Retained<UNUserNotificationCenter>,
    _delegate: Retained<Delegate>,
}
impl Center {
    /// None outside an app bundle, where the center aborts the process.
    fn new() -> Option<Self> {
        objc2_foundation::NSBundle::mainBundle().bundleIdentifier()?;
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = Delegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Some(Self {
            center,
            _delegate: delegate,
        })
    }
    fn refresh(&self) {
        let done = RcBlock::new(|settings: std::ptr::NonNull<UNNotificationSettings>| {
            let settings = unsafe { settings.as_ref() };
            let status = match settings.authorizationStatus() {
                UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Ephemeral => {
                    Authorization::Authorized
                }
                UNAuthorizationStatus::Provisional => Authorization::Provisional,
                UNAuthorizationStatus::Denied => Authorization::Denied,
                _ => Authorization::NotDetermined,
            };
            STATUS.store(encode(status), Ordering::Release);
            pm_builtins::set_notification_status(status);
        });
        self.center
            .getNotificationSettingsWithCompletionHandler(&done);
    }
    fn request(&self) {
        let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
            let status = if !error.is_null() {
                Authorization::Unavailable
            } else if granted.as_bool() {
                Authorization::Authorized
            } else {
                Authorization::Denied
            };
            STATUS.store(encode(status), Ordering::Release);
            pm_builtins::set_notification_status(status);
        });
        self.center
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
    }
    fn deliver(&self, notice: &CompletionNotice) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&notice.title));
        content.setBody(&NSString::from_str(&notice.body));
        let sound = UNNotificationSound::soundNamed(&NSString::from_str(SOUND));
        content.setSound(Some(&sound));
        // The thread is the identifier: a newer notice replaces the old one.
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(notice.identifier()),
            &content,
            None,
        );
        self.center
            .addNotificationRequest_withCompletionHandler(&request, None);
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PMNativeNotificationDelegate"]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let thread = response.notification().request().identifier().to_string();
            if let Some(tx) = CLICKS.get() {
                let _ = tx.try_send(thread);
            }
            completion.call(());
        }
        /// The policy already left out the frontmost-and-visible case; a
        /// notice that got here shows as a banner (the agent is hidden).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);
impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}
