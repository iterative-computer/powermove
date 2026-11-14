//! Newer work in the original app, after the first launch brought its library
//! in (`pm_first_launch::later` holds the rule). A later launch never waits on
//! the original app: once the library is open, a background check (metadata
//! only) looks for newer work there, and Home shows one quiet line when there
//! is some. Bringing it in is the person's choice, from that line or from
//! Settings › Storage: Powermove quits, brings it in before the library opens
//! with its progress on screen, and opens again.
use pm_first_launch::{Config, later};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

/// The background check found work the library hasn't brought in.
static NEWER: AtomicBool = AtomicBool::new(false);
/// The person chose Restart and Bring In for this profile: a clean quit opens
/// Powermove again.
static RELAUNCH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Home's line: the original app has work this library hasn't brought in.
pub fn newer() -> bool {
    NEWER.load(Ordering::Acquire)
}
pub const NOTICE: &str = "Your original app has newer work.";
pub const NOTICE_ACTION: &str = "Bring It In…";

/// After the library opened: tidies stages an earlier attempt left empty,
/// then compares the original app's stamp. Off the main thread; Home redraws.
pub fn check(config: Config, cx: &mut gpui::App) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = later::tidy(&config.native_profile);
        let newer = later::newer_work(&config).unwrap_or_else(|error| {
            eprintln!("Looking for newer work in the original app stopped: {error:#}");
            false
        });
        let _ = tx.send_blocking(newer);
    });
    cx.spawn(async move |cx| {
        if let Ok(newer) = rx.recv().await {
            NEWER.store(newer, Ordering::Release);
            cx.update(redraw_home);
        }
    })
    .detach();
}
fn redraw_home(cx: &mut gpui::App) {
    for editor in crate::entry::editors(cx) {
        if let Some(library) = editor.read(cx).library.clone() {
            library.update(cx, |_, cx| cx.notify());
        }
    }
}
/// The question before the restart. True: Restart and Bring In.
fn confirm_alert(original_running: bool) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button};
    let mut informative = "Powermove quits, brings in what changed in the original app since it last came in, and opens again. The original app stays unchanged.".to_owned();
    if original_running {
        informative.push_str(" Quit the original app first; Powermove waits for it.");
    }
    Alert::new(
        "Bring in your recent work?",
        informative,
        vec![
            Button::new("Restart and Bring In", true).default(),
            Button::new("Cancel", false).cancel(),
        ],
    )
}
/// A clean quit after Restart and Bring In: opens this same Powermove again
/// (same executable, arguments and environment, so a test profile stays a
/// test profile) once this process has ended.
pub fn relaunch_if_requested() {
    if RELAUNCH.lock().unwrap().take().is_none() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::os::unix::process::CommandExt;
    let spawned = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"while /bin/kill -0 "$0" 2>/dev/null; do /bin/sleep 0.2; done; exec "$@""#)
        .arg(std::process::id().to_string())
        .arg(exe)
        .args(std::env::args_os().skip(1))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(error) = spawned {
        eprintln!("Powermove couldn’t open again after quitting: {error}");
    }
}
/// The quit stopped (a Save question answered Cancel): no restart, and the
/// next launch opens as usual.
pub fn quit_cancelled() {
    if let Some(profile) = RELAUNCH.lock().unwrap().take() {
        later::cancel_request(&profile);
    }
}

impl crate::session::Controller {
    /// Bring In Recent Work…, from Home's line or Settings › Storage.
    pub(crate) fn bring_in_recent_work(
        &mut self,
        window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(original) = self.electron.clone() else {
            return;
        };
        let running = pm_kernel::locks::older_electron_running(&original);
        let answer = confirm_alert(running).sheet(&window);
        let profile: PathBuf = self.profile.clone();
        cx.spawn(async move |_, cx| {
            if answer.recv().await != Ok(Some(true)) {
                return;
            }
            if let Err(error) = later::request(&profile) {
                let _ = cx.update(|cx| {
                    crate::error_log::shared(cx).borrow_mut().tell(
                        "Library",
                        &format!("Recent work couldn’t be scheduled: {error:#}"),
                    )
                });
                return;
            }
            *RELAUNCH.lock().unwrap() = Some(profile);
            let _ = cx.update(crate::lifecycle::begin_quit);
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_question_names_the_restart_and_the_original_app() {
        let alert = confirm_alert(false);
        assert_eq!(alert.buttons[0].label, "Restart and Bring In");
        assert!(alert.buttons[0].default);
        assert!(alert.informative.contains("stays unchanged"));
        assert!(confirm_alert(true).informative.contains("Quit the original app first"));
    }
    #[test]
    fn a_cancelled_quit_withdraws_the_request() {
        let temp = tempfile::tempdir().unwrap();
        later::request(temp.path()).unwrap();
        *RELAUNCH.lock().unwrap() = Some(temp.path().to_owned());
        quit_cancelled();
        assert!(!later::requested(temp.path()));
        assert!(RELAUNCH.lock().unwrap().is_none());
    }
}
