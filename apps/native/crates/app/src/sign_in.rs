//! The Powermove account's sign-in, one experience everywhere: the email,
//! the 6-digit code sent to it (paste, the sixth digit sends it, a wrong
//! code shakes and clears, Resend after a cooldown, Change), then "You’re
//! in". The first run's account step and the sign-in sheet (Settings, Home,
//! the Store) run this flow and draw it with the Welcome view's sign-in
//! block; the registry calls are `cloud`'s (`send_code`, `verify_code`,
//! Google's return link, the handle).
use gpui::{AnyWindowHandle, App, AppContext as _, Bounds, Entity, WindowBounds, WindowKind};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use pm_shell::welcome::{Button, Frame, SignIn as SignInFrame, SignInStage};
use pm_shell::welcome_view::{OnAction, WelcomeView, sheet_size};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

/// Resend is offered again this long after a code goes out.
pub(crate) const RESEND_AFTER: Duration = Duration::from_secs(30);
/// The signed-in check stays this long before what comes next.
pub(crate) const SIGNED_IN_PAUSE: Duration = Duration::from_millis(600);

/// What the flow asks of its host.
#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    /// Email a code to this address (`cloud::send_code`).
    SendCode(String),
    /// Check the code, then save the session and read the account
    /// (`cloud::verify_code`).
    VerifyCode { email: String, code: String },
    /// The action `id` after a while (the check's pause, Resend's countdown).
    After(Duration, &'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Retry {
    Send,
    Verify,
}

/// The sign-in, in the window: an email, then the code the registry sends
/// to it, then a check. One path for a new account and an existing one: the
/// registry makes the account if the address has none.
#[derive(Debug, Default)]
pub(crate) struct SignInFlow {
    pub(crate) stage: SignInStage,
    pub(crate) email: String,
    pub(crate) code: String,
    /// The handle typed on the sheet's handle step.
    pub(crate) handle: String,
    /// Bumped when the flow sets a field's text (a wrong code cleared).
    pub(crate) revision: u64,
    /// Bumped once per wrong or expired code.
    pub(crate) shake: u64,
    /// The first code is on its way, or a code is being checked (Continue
    /// spins).
    pub(crate) busy: bool,
    /// A new code is on its way (Resend).
    pub(crate) resending: bool,
    /// When Resend is offered again.
    pub(crate) resend_at: Option<Instant>,
    /// The line under the field, and what Try again does (`None`: there
    /// is nothing to try again).
    pub(crate) error: Option<(String, Option<Retry>)>,
    /// Signed in already, and asked for another account (the first run).
    pub(crate) switching: bool,
    /// The registry asked for the browser check, and the sheet waits on it.
    pub(crate) verifying: bool,
}

/// Could be an address: something, an @, and a domain with a dot in it.
pub(crate) fn plausible_email(email: &str) -> bool {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && email.len() <= 254
        && !email.chars().any(char::is_whitespace)
        && !domain.contains('@')
        && domain
            .rsplit_once('.')
            .is_some_and(|(name, top)| !name.is_empty() && !name.ends_with('.') && top.len() >= 2)
}

impl SignInFlow {
    /// The email and code stages' own actions, the same wherever the flow
    /// is shown; `None` for anything else (the host's).
    pub(crate) fn act(&mut self, id: &str) -> Option<Vec<Effect>> {
        let none = Some(vec![]);
        if let Some(text) = id.strip_prefix("field:email:") {
            if self.stage == SignInStage::Email && !self.busy {
                self.email = text.to_owned();
                self.error = None;
            }
            return none;
        }
        if let Some(text) = id.strip_prefix("field:code:") {
            if self.stage != SignInStage::Code || self.busy {
                return none;
            }
            self.code = text.to_owned();
            self.error = None;
            // The sixth digit sends it.
            return Some(self.verify());
        }
        if let Some(text) = id.strip_prefix("field:handle:") {
            if self.stage == SignInStage::Handle && !self.busy {
                self.handle = text.trim_start_matches('@').to_lowercase();
                self.error = None;
            }
            return none;
        }
        match (self.stage, id) {
            (SignInStage::Email, "continue" | "retry") => Some(self.send()),
            (SignInStage::Code, "continue") => Some(self.verify()),
            (SignInStage::Code, "retry")
                if self
                    .error
                    .as_ref()
                    .is_some_and(|(_, r)| *r == Some(Retry::Verify)) =>
            {
                Some(self.verify())
            }
            // A new code: Continue stays as it is (the line under the cells
            // says it went).
            (SignInStage::Code, "retry" | "resend") => {
                let ready = self.resend_at.is_none_or(|at| Instant::now() >= at);
                if self.busy || self.resending || !(ready || id == "retry") {
                    return none;
                }
                self.resending = true;
                self.error = None;
                Some(vec![Effect::SendCode(self.email.trim().to_owned())])
            }
            // Back a step: the email again, as it was typed.
            (SignInStage::Code, "change" | "escape" | "back") => {
                self.stage = SignInStage::Email;
                self.code.clear();
                self.busy = false;
                self.resending = false;
                self.error = None;
                self.revision += 1;
                none
            }
            (SignInStage::Code, "tick") => Some(match self.resend_at {
                Some(at) if Instant::now() < at => {
                    vec![Effect::After(Duration::from_secs(1), "tick")]
                }
                _ => vec![],
            }),
            _ => None,
        }
    }
    /// Continue on the email: the code goes out, Continue spinning meanwhile.
    pub(crate) fn send(&mut self) -> Vec<Effect> {
        if self.busy || !plausible_email(&self.email) {
            return vec![];
        }
        self.busy = true;
        self.error = None;
        vec![Effect::SendCode(self.email.trim().to_owned())]
    }
    /// All six digits: the code is checked.
    pub(crate) fn verify(&mut self) -> Vec<Effect> {
        if self.busy || self.code.len() != 6 {
            return vec![];
        }
        self.busy = true;
        self.error = None;
        vec![Effect::VerifyCode {
            email: self.email.trim().to_owned(),
            code: self.code.clone(),
        }]
    }
    pub(crate) fn code_sent(&mut self, result: Result<(), String>) -> Vec<Effect> {
        if !self.busy && !self.resending {
            return vec![];
        }
        self.busy = false;
        self.resending = false;
        match result {
            Ok(()) => {
                if self.stage == SignInStage::Email {
                    self.stage = SignInStage::Code;
                }
                self.code.clear();
                self.revision += 1;
                self.error = None;
                self.resend_at = Some(Instant::now() + RESEND_AFTER);
                vec![Effect::After(Duration::from_secs(1), "tick")]
            }
            Err(code) => {
                let retry = (code == "network").then_some(Retry::Send);
                self.error = Some((crate::cloud::code_error(&code, false), retry));
                vec![]
            }
        }
    }
    /// The code was checked: signed in as the account's email (`Done`), or
    /// why not. A wrong or expired code shakes and clears.
    pub(crate) fn verified(&mut self, result: Result<String, String>) -> Vec<Effect> {
        // Changed back to the email meanwhile: this code is not wanted.
        if !self.busy || self.stage != SignInStage::Code {
            return vec![];
        }
        self.busy = false;
        match result {
            Ok(email) => {
                self.stage = SignInStage::Done;
                self.error = None;
                self.email = email;
                vec![Effect::After(SIGNED_IN_PAUSE, "signed_in")]
            }
            Err(code) => {
                let message = crate::cloud::code_error(&code, true);
                if matches!(
                    code.as_str(),
                    "api:unauthorized" | "api:bad_request" | "api:gone"
                ) {
                    // Shaken once and cleared, for the code to be typed again.
                    self.code.clear();
                    self.revision += 1;
                    self.shake += 1;
                    if code == "api:gone" {
                        self.resend_at = None;
                    }
                    self.error = Some((message, None));
                } else {
                    let retry = (code == "network").then_some(Retry::Verify);
                    self.error = Some((message, retry));
                }
                vec![]
            }
        }
    }
    /// Whether the primary action can go: a plausible email, six digits, a
    /// handle.
    pub(crate) fn ready(&self) -> bool {
        match self.stage {
            SignInStage::Email => plausible_email(&self.email),
            SignInStage::Code => self.code.len() == 6,
            SignInStage::Handle => !self.handle.is_empty(),
            SignInStage::Done | SignInStage::Browser => true,
        }
    }
    /// The Welcome view's sign-in block for the flow as it stands.
    pub(crate) fn frame(&self) -> SignInFrame {
        let resend_in = self.resend_at.map(|at| {
            at.saturating_duration_since(Instant::now())
                .as_secs_f64()
                .ceil() as u32
        });
        SignInFrame {
            stage: self.stage,
            email: self.email.clone(),
            code: self.code.clone(),
            handle: self.handle.clone(),
            revision: self.revision,
            shake: self.shake,
            resend_in: resend_in.or((self.stage == SignInStage::Code).then_some(0)),
            retry: self.error.as_ref().is_some_and(|(_, r)| r.is_some()),
        }
    }
}

// ---- The sheet ----

/// The open sign-in sheet: its flow, its view and window.
struct Sheet {
    flow: SignInFlow,
    view: Option<Entity<WelcomeView>>,
    window: Option<AnyWindowHandle>,
}
thread_local! {
    static OPEN: RefCell<Option<Rc<RefCell<Sheet>>>> = const { RefCell::new(None) };
}

/// The sheet's words and actions for the flow as it stands.
fn sheet_frame(flow: &SignInFlow) -> Frame {
    // The registry's browser check is open: a quiet wait with Cancel.
    if flow.verifying {
        return Frame {
            surface: "sign-in".into(),
            title: "Verify in your browser…".into(),
            line: "Finish the quick check in your browser.".into(),
            busy: true,
            buttons: vec![Button::new("cancel_verify", "Cancel")],
            default: None,
            sign_in: Some(flow.frame()),
            ..Frame::default()
        };
    }
    let (title, line) = match flow.stage {
        SignInStage::Email => (
            "Sign in to Powermove".to_owned(),
            "Use your email or Google account.".to_owned(),
        ),
        SignInStage::Code => (
            "Check your email".to_owned(),
            // The address stands under it, with Change.
            "Enter the 6-digit code we emailed you.".to_owned(),
        ),
        SignInStage::Handle => (
            "Choose your handle".to_owned(),
            "Your name on the Powermove Store.".to_owned(),
        ),
        SignInStage::Browser => (
            "Continue in your browser".to_owned(),
            "Finish with Google, then come back here.".to_owned(),
        ),
        SignInStage::Done => (
            "You’re in".to_owned(),
            format!("Signed in as {}.", flow.email),
        ),
    };
    let ready = flow.ready();
    let primary = |id: &str, label: &str| Button {
        // Clickable while it spins: a second press does nothing.
        enabled: ready || flow.busy,
        ..Button::new(id, label)
    };
    let (buttons, default) = match flow.stage {
        SignInStage::Email => (
            vec![
                primary("continue", "Continue"),
                Button::new("google", "Continue with Google"),
                Button::new("cancel", "Cancel"),
            ],
            Some("continue"),
        ),
        SignInStage::Code => (
            vec![
                primary("continue", "Continue"),
                Button::new("cancel", "Cancel"),
            ],
            Some("continue"),
        ),
        SignInStage::Handle => (
            vec![
                primary("choose", "Choose Handle"),
                Button::new("not_now", "Not Now"),
            ],
            Some("choose"),
        ),
        SignInStage::Browser => (
            vec![
                Button::new("back", "Use Email Instead"),
                Button::new("cancel", "Cancel"),
            ],
            None,
        ),
        // The check, then the sheet closes by itself.
        SignInStage::Done => (vec![], None),
    };
    Frame {
        surface: "sign-in".into(),
        title,
        line,
        error: flow.error.as_ref().map(|(message, _)| message.clone()),
        busy: flow.busy,
        buttons,
        default: default.map(str::to_owned),
        sign_in: Some(flow.frame()),
        ..Frame::default()
    }
}

/// The sign-in sheet on `parent`: the email first. One sheet at a time.
pub fn open(parent: Retained<NSWindow>, cx: &mut App) {
    if OPEN.with(|open| open.borrow().is_some()) {
        return;
    }
    // The GPUI window `parent` is, for the sheet to hang from.
    let Some(owner) = cx.windows().into_iter().find(|handle| {
        handle
            .update(cx, |_, window, _| {
                crate::platform::ns_window(window).is_some_and(|w| std::ptr::eq(&*w, &*parent))
            })
            .unwrap_or(false)
    }) else {
        return;
    };
    let sheet = Rc::new(RefCell::new(Sheet {
        flow: SignInFlow::default(),
        view: None,
        window: None,
    }));
    OPEN.with(|open| *open.borrow_mut() = Some(sheet.clone()));
    let on_action: OnAction = {
        let sheet = sheet.clone();
        Rc::new(move |id, cx| act(&sheet, id, cx))
    };
    let frame = sheet_frame(&sheet.borrow().flow);
    let tokens = pm_shell::tokens();
    let (w, h) = sheet_size();
    let mut options = gpui::WindowOptions::new().kind(WindowKind::Dialog);
    options.parent = Some(owner);
    options.is_resizable = false;
    options.is_minimizable = false;
    options.is_movable = false;
    let made = sheet.clone();
    let opened = cx.open_window(
        options.window_bounds(Some(WindowBounds::Windowed(Bounds::new(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(w), gpui::px(h)),
        )))),
        move |window, cx| {
            let view = cx.new(|cx| {
                let mut view = WelcomeView::new(tokens, frame, on_action, cx);
                view.sheet = true;
                view.hold_focus(window, cx);
                view
            });
            made.borrow_mut().view = Some(view.clone());
            view
        },
    );
    match opened {
        Ok(handle) => sheet.borrow_mut().window = Some(handle.into()),
        Err(error) => {
            eprintln!("The sign-in sheet could not open: {error:#}");
            OPEN.with(|open| *open.borrow_mut() = None);
        }
    }
}

/// Shows the flow as it stands.
fn refresh(sheet: &Rc<RefCell<Sheet>>, cx: &mut App) {
    let (view, frame) = {
        let s = sheet.borrow();
        (s.view.clone(), sheet_frame(&s.flow))
    };
    if let Some(view) = view {
        view.update(cx, |v, cx| v.set_frame(frame, cx));
    }
}

/// Closes the sheet; a Google sign-in still out is let go.
fn close(sheet: &Rc<RefCell<Sheet>>, cx: &mut App) {
    crate::cloud::cancel_google(cx);
    crate::cloud::cancel_human(cx);
    let window = sheet.borrow_mut().window.take();
    OPEN.with(|open| *open.borrow_mut() = None);
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

/// One of the sheet's actions, or its fields' typing and keys.
fn act(sheet: &Rc<RefCell<Sheet>>, id: &str, cx: &mut App) {
    let stage = sheet.borrow().flow.stage;
    if id == "cancel_verify" || (id == "escape" && sheet.borrow().flow.verifying) {
        // The worker stops waiting; the request fails and the step says so.
        return crate::cloud::cancel_human(cx);
    }
    match (stage, id) {
        (SignInStage::Email, "cancel" | "escape") | (_, "cancel") => return close(sheet, cx),
        (SignInStage::Done, "done" | "signed_in" | "escape") => return close(sheet, cx),
        (SignInStage::Handle, "not_now" | "escape") => {
            let mut s = sheet.borrow_mut();
            s.flow.stage = SignInStage::Done;
            s.flow.error = None;
            drop(s);
            refresh(sheet, cx);
            return after(sheet, SIGNED_IN_PAUSE * 3, "signed_in", cx);
        }
        (SignInStage::Handle, "choose" | "continue") => return claim(sheet, cx),
        (SignInStage::Email, "google") => return google(sheet, cx),
        (SignInStage::Browser, "back" | "escape") => {
            crate::cloud::cancel_google(cx);
            sheet.borrow_mut().flow.stage = SignInStage::Email;
            return refresh(sheet, cx);
        }
        _ => {}
    }
    let effects = sheet.borrow_mut().flow.act(id);
    if let Some(effects) = effects {
        apply(sheet, effects, cx);
    }
    refresh(sheet, cx);
}

/// While a code is on its way or being checked, the sheet follows the
/// registry's browser check: "Verify in your browser…" while the worker
/// waits on it, the step again once it is done.
fn watch_human(sheet: &Rc<RefCell<Sheet>>, cx: &mut App) {
    let sheet = sheet.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            let done = cx.update(|cx| {
                let verifying = crate::cloud::verifying_human(cx);
                let (changed, waiting) = {
                    let mut s = sheet.borrow_mut();
                    let changed = s.flow.verifying != verifying;
                    s.flow.verifying = verifying;
                    (changed, s.flow.busy || s.flow.resending)
                };
                if changed {
                    refresh(&sheet, cx);
                }
                !waiting && !verifying
            });
            if done {
                break;
            }
        }
    })
    .detach();
}

fn apply(sheet: &Rc<RefCell<Sheet>>, effects: Vec<Effect>, cx: &mut App) {
    for effect in effects {
        match effect {
            Effect::SendCode(email) => {
                watch_human(sheet, cx);
                let sheet = sheet.clone();
                crate::cloud::send_code(email, cx, move |result, cx| {
                    let effects = sheet.borrow_mut().flow.code_sent(result);
                    apply(&sheet, effects, cx);
                    refresh(&sheet, cx);
                });
            }
            Effect::VerifyCode { email, code } => {
                watch_human(sheet, cx);
                let sheet = sheet.clone();
                crate::cloud::verify_code(email, code, cx, move |result, cx| {
                    let effects = sheet.borrow_mut().flow.verified(result);
                    arrived(&sheet, effects, cx);
                });
            }
            Effect::After(wait, id) => after(sheet, wait, id, cx),
        }
    }
}

/// Signed in (or not): a new account chooses its handle first; else
/// "You’re in", and the sheet closes after a moment.
fn arrived(sheet: &Rc<RefCell<Sheet>>, effects: Vec<Effect>, cx: &mut App) {
    let done = sheet.borrow().flow.stage == SignInStage::Done;
    if done && crate::cloud::needs_handle(cx) {
        let mut s = sheet.borrow_mut();
        s.flow.stage = SignInStage::Handle;
        s.flow.revision += 1;
        drop(s);
        return refresh(sheet, cx);
    }
    refresh(sheet, cx);
    if done {
        // The check, then the sheet goes.
        after(sheet, SIGNED_IN_PAUSE * 3, "signed_in", cx);
    } else {
        apply(sheet, effects, cx);
    }
}

fn after(sheet: &Rc<RefCell<Sheet>>, wait: Duration, id: &'static str, cx: &mut App) {
    let sheet = sheet.clone();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        let _ = cx.update(|cx| {
            // Only the sheet still open hears it.
            if OPEN.with(|open| {
                open.borrow()
                    .as_ref()
                    .is_some_and(|o| Rc::ptr_eq(o, &sheet))
            }) {
                act(&sheet, id, cx);
            }
        });
    })
    .detach();
}

/// Choose Handle: the registry claims it for the account.
fn claim(sheet: &Rc<RefCell<Sheet>>, cx: &mut App) {
    let handle = {
        let mut s = sheet.borrow_mut();
        if s.flow.busy {
            return;
        }
        let handle = s.flow.handle.clone();
        if let Some(message) = crate::cloud::invalid_handle(&handle) {
            s.flow.error = Some((message, None));
            s.flow.shake += 1;
            drop(s);
            return refresh(sheet, cx);
        }
        s.flow.busy = true;
        s.flow.error = None;
        handle
    };
    refresh(sheet, cx);
    let done = sheet.clone();
    crate::cloud::claim_handle(handle.clone(), cx, move |result, cx| {
        let mut s = done.borrow_mut();
        s.flow.busy = false;
        match result {
            Ok(()) => {
                s.flow.stage = SignInStage::Done;
                drop(s);
                refresh(&done, cx);
                after(&done, SIGNED_IN_PAUSE * 3, "signed_in", cx);
            }
            Err(code) => {
                s.flow.error = Some((crate::cloud::handle_error(&code, &handle), None));
                s.flow.shake += 1;
                drop(s);
                refresh(&done, cx);
            }
        }
    });
}

/// Continue with Google: the browser, then the return link.
fn google(sheet: &Rc<RefCell<Sheet>>, cx: &mut App) {
    let done = sheet.clone();
    let started = crate::cloud::sign_in_with_google(cx, move |result, cx| {
        let mut s = done.borrow_mut();
        match result {
            Ok(email) => {
                s.flow.stage = SignInStage::Code;
                s.flow.busy = true;
                drop(s);
                // As a checked code arrives: the handle, or "You’re in".
                let effects = done.borrow_mut().flow.verified(Ok(email));
                arrived(&done, effects, cx);
            }
            Err(message) => {
                s.flow.stage = SignInStage::Email;
                s.flow.error = Some((message, None));
                drop(s);
                refresh(&done, cx);
            }
        }
    });
    let mut s = sheet.borrow_mut();
    match started {
        Ok(()) => {
            s.flow.stage = SignInStage::Browser;
            s.flow.error = None;
        }
        Err(message) => s.flow.error = Some((message, None)),
    }
    drop(s);
    refresh(sheet, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow() -> SignInFlow {
        SignInFlow::default()
    }

    /// The sheet's steps say what they are and offer what each needs.
    #[test]
    fn the_sheet_names_each_step_and_its_actions() {
        let mut f = flow();
        let frame = sheet_frame(&f);
        assert_eq!(frame.title, "Sign in to Powermove");
        let ids: Vec<_> = frame.buttons.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["continue", "google", "cancel"]);
        assert!(!frame.buttons[0].enabled, "Continue waits for an address");
        f.act("field:email:ana@example.com");
        assert!(sheet_frame(&f).buttons[0].enabled);
        f.stage = SignInStage::Code;
        assert_eq!(sheet_frame(&f).title, "Check your email");
        f.stage = SignInStage::Handle;
        assert_eq!(sheet_frame(&f).default.as_deref(), Some("choose"));
        f.stage = SignInStage::Done;
        assert_eq!(sheet_frame(&f).title, "You’re in");
        assert!(sheet_frame(&f).buttons.is_empty(), "it closes by itself");
    }

    /// Email, code, a wrong code (shake, cleared), the right one: signed in.
    #[test]
    fn email_then_code_with_a_wrong_code_first() {
        let mut f = flow();
        f.act("field:email:ana@example.com");
        assert_eq!(
            f.act("continue"),
            Some(vec![Effect::SendCode("ana@example.com".into())])
        );
        assert!(f.busy);
        f.code_sent(Ok(()));
        assert_eq!(f.stage, SignInStage::Code);
        assert!(f.resend_at.is_some());
        // The sixth digit sends it.
        assert_eq!(f.act("field:code:00000"), Some(vec![]));
        assert!(matches!(
            f.act("field:code:000000").as_deref(),
            Some([Effect::VerifyCode { .. }])
        ));
        f.verified(Err("api:unauthorized".into()));
        assert_eq!((f.shake, f.code.as_str()), (1, ""));
        assert!(f.error.is_some());
        f.act("field:code:123456");
        let effects = f.verified(Ok("ana@example.com".into()));
        assert_eq!(f.stage, SignInStage::Done);
        assert_eq!(effects, vec![Effect::After(SIGNED_IN_PAUSE, "signed_in")]);
    }

    /// Change goes back to the email as typed; a late answer to the code
    /// is ignored.
    #[test]
    fn change_returns_to_the_email_and_drops_a_late_answer() {
        let mut f = flow();
        f.act("field:email:ana@example.com");
        f.act("continue");
        f.code_sent(Ok(()));
        f.act("field:code:123456");
        f.act("change");
        assert_eq!(
            (f.stage, f.email.as_str()),
            (SignInStage::Email, "ana@example.com")
        );
        assert!(f.verified(Ok("ana@example.com".into())).is_empty());
        assert_eq!(f.stage, SignInStage::Email);
    }

    /// Resend waits out its cooldown.
    #[test]
    fn resend_waits_for_its_cooldown() {
        let mut f = flow();
        f.act("field:email:ana@example.com");
        f.act("continue");
        f.code_sent(Ok(()));
        assert_eq!(f.act("resend"), Some(vec![]));
        f.resend_at = Some(Instant::now());
        assert_eq!(
            f.act("resend"),
            Some(vec![Effect::SendCode("ana@example.com".into())])
        );
    }

    /// The handle step reads a typed @name as the name.
    #[test]
    fn the_handle_is_typed_without_its_at() {
        let mut f = flow();
        f.stage = SignInStage::Handle;
        f.act("field:handle:@Ana");
        assert_eq!(f.handle, "ana");
        assert!(f.ready());
    }
}
