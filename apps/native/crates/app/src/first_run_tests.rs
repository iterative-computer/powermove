//! Every first-launch path on synthetic profiles, through the real coordinator.
use super::*;
use crate::sign_in::SIGNED_IN_PAUSE;
use std::time::Instant;
use pm_shell::welcome::Frame;
use serde_json::json;
use std::{fs, sync::Arc};

fn write(path: &Path, value: &serde_json::Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
/// The original app's profile, with one project. No IndexedDB: media comes from
/// nothing here, and the coordinator's own tests cover media transfer.
fn source(support: &Path) {
    let root = support.join("Powermove");
    write(
        &root.join("store/projects.json"),
        &json!([{"id":"p","name":"Synthetic project"}]),
    );
    write(
        &root.join("store/project.p.json"),
        &json!({"v":"1.2.0","proj":{"id":"p","name":"Synthetic project","w":100,"h":100,"fps":30,"dur":1,"layers":[],"comps":{}}}),
    );
    write(&root.join("store/theme.json"), &json!("dark"));
}
/// Default layout under a synthetic support folder, as `Plan::resolve` builds it.
fn plan(home: &Path) -> Plan {
    Plan::resolve(None, None, Some(home.as_os_str().to_owned())).unwrap()
}
fn home() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let support = home.join(APPLICATION_SUPPORT);
    fs::create_dir_all(&support).unwrap();
    (temp, home)
}
/// Runs the real worker to its end, feeding the flow as the window would.
fn drive(flow: &mut Flow, effects: Vec<Effect>) -> Vec<Effect> {
    let mut pending = effects;
    let mut left = vec![];
    while let Some(effect) = pending.pop() {
        let Effect::Run(run) = effect else {
            left.push(effect);
            continue;
        };
        let (tx, rx) = async_channel::unbounded();
        let config = flow.plan.config.clone().unwrap();
        spawn_run(
            config,
            run,
            Arc::new(AtomicBool::new(false)),
            Arc::default(),
            tx,
        );
        while let Ok(message) = rx.recv_blocking() {
            let end = matches!(message, Message::Finished(_));
            pending.extend(flow.message(message));
            if end {
                break;
            }
        }
    }
    left
}
fn start(flow: &mut Flow) -> Vec<Effect> {
    let first = flow.start();
    drive(flow, first)
}
/// The title as it reads (a title set on two lines is one sentence).
fn titled(frame: &Frame, title: &str) {
    assert_eq!(frame.title.replace('\n', " "), title, "{frame:?}");
}

#[test]
fn the_default_profile_is_the_one_the_coordinator_writes() {
    let (_temp, home) = home();
    let plan = plan(&home);
    let support = home.join(APPLICATION_SUPPORT);
    assert_eq!(plan.profile, support.join(pm_first_launch::BETA_BUNDLE_ID));
    assert_eq!(plan.original, plan.config);
    let config = plan.config.unwrap();
    assert_eq!(config.native_profile, plan.profile);
    assert_eq!(config.electron_profile, support.join("Powermove"));
}
#[test]
fn an_override_keeps_full_migration_disabled() {
    let (_temp, home) = home();
    let isolated = home.join("isolated");
    source(&home.join(APPLICATION_SUPPORT));
    write(&isolated.join("store/projects.json"), &json!([]));
    let plan = Plan::resolve(
        Some(isolated.clone()),
        None,
        Some(home.as_os_str().to_owned()),
    )
    .unwrap();
    assert_eq!(plan.config, None);
    assert_eq!(plan.profile, isolated);
    assert_eq!(plan.launch, Launch::Fresh);
    assert!(!plan.onboard);
    assert_eq!(
        plan.electron(),
        Some(home.join(APPLICATION_SUPPORT).join("Powermove"))
    );
    assert!(Plan::resolve(None, None, None).is_none());
    assert!(Plan::resolve(None, None, Some("".into())).is_none());
}
#[test]
fn a_new_user_gets_onboarding_and_no_migration() {
    let (_temp, home) = home();
    let plan = plan(&home);
    assert_eq!(plan.launch, Launch::Fresh);
    let profile = plan.profile.clone();
    let mut flow = Flow::new(plan);
    assert!(start(&mut flow).is_empty());
    assert!(flow.visible());
    let welcome = flow.frame();
    assert!(welcome.animate);
    titled(&welcome, "Welcome to Powermove");
    // Begin is enabled from the first frame, and skips the animation.
    assert_eq!(welcome.default_action(), Some("begin"));
    flow.action("begin");
    titled(&flow.frame(), "Choose your timeline");
    flow.action("choose:tracks");
    assert_eq!(flow.frame().chosen.as_deref(), Some("tracks"));
    flow.action("continue");
    // The Powermove account, before the agents: an email first, Continue
    // waiting for one that could be an address, and Not now.
    let account = flow.frame();
    titled(&account, "Create your account");
    assert_eq!(account.default_action(), None);
    assert!(account.buttons.iter().any(|b| b.id == "not_now"));
    assert_eq!(
        account.note.as_deref(),
        Some("Already have an account? Use the same email.")
    );
    // Signed in: the step says who, and carries on.
    flow.message(Message::Powermove(Some("ana@example.com".into())));
    let signed = flow.frame();
    titled(&signed, "Your Powermove account");
    assert!(signed.line.contains("ana@example.com"));
    assert_eq!(signed.default_action(), Some("continue"));
    assert!(signed.buttons.iter().any(|b| b.id == "use_other"));
    flow.message(Message::Powermove(None));
    flow.action("not_now");
    titled(&flow.frame(), "Choose your agent");
    flow.action("back");
    titled(&flow.frame(), "Create your account");
    flow.action("back");
    titled(&flow.frame(), "Choose your timeline");
    flow.action("continue");
    flow.action("not_now");
    flow.action("choose:claude");
    flow.action("continue");
    titled(&flow.frame(), "Speech to text");
    let effects = flow.action("continue");
    let [Effect::Enter(Some(choices))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(choices["timelineMode"], "tracks");
    assert_eq!(choices["agent"]["provider"], "claude");
    assert_eq!(choices["workspaceImport"], serde_json::Value::Null);
    // Nothing is saved until the end: a closed window runs onboarding again.
    assert!(!onboarding_completed(&profile));
    fs::create_dir_all(&profile).unwrap();
    write_marker(&profile, choices).unwrap();
    assert!(onboarding_completed(&profile));
}
#[test]
fn once_the_app_has_opened_no_first_run_window_shows_again() {
    // The owner signed in, finished the steps, and Home was up; Get Aerolite
    // opened Checkout in the browser, the app lost focus, and the billing and
    // identity changes that followed brought the stage back on the last
    // step's frame. Entering ends the first run for good.
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    start(&mut flow);
    for id in ["begin", "continue", "not_now", "choose:claude", "continue", "continue"] {
        flow.action(id);
    }
    assert!(
        flow.visible(),
        "the last step shows while the library opens"
    );
    titled(&flow.frame(), "Opening Powermove");
    flow.entered();
    assert!(!flow.visible());
    for message in [
        Message::Powermove(Some("ana@example.com".into())),
        Message::AeroPlan(AeroPlan::default()),
        Message::Powermove(None),
        Message::Account(pm_agents::Provider::Claude, Some("Signed in".into())),
    ] {
        flow.message(message);
        assert!(!flow.visible());
    }
    flow.action("continue");
    flow.action("back");
    assert!(!flow.visible());
}
#[test]
fn bringing_a_workspace_records_the_opt_in_and_back_reaches_welcome() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    start(&mut flow);
    flow.action("begin");
    flow.action("back");
    titled(&flow.frame(), "Welcome to Powermove");
    for action in ["begin", "continue", "not_now", "continue"] {
        flow.action(action);
    }
    titled(&flow.frame(), "Speech to text");
    let effects = flow.action("continue");
    let [Effect::Enter(Some(choices))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(choices["workspaceImport"], serde_json::Value::Null);
    assert!(!crate::first_run::requests_workspace(choices));
}
#[test]
fn an_existing_user_gets_the_copy_then_the_intro_and_every_step_then_home() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let plan = plan(&home);
    assert_eq!(plan.launch, Launch::Migrate);
    // The original app finished its own onboarding: its marker comes across
    // with the library, and this first run here still plans the steps.
    assert!(plan.onboard);
    let profile = plan.profile.clone();
    let source_files = home.join(APPLICATION_SUPPORT).join("Powermove");
    let before = fs::read(source_files.join("store/project.p.json")).unwrap();
    write(
        &source_files.join("onboarding-v1.json"),
        &json!({"version":1}),
    );
    let mut flow = Flow::new(plan);
    // The copy runs first (the window the intro draws holds it)…
    let first = flow.start();
    assert_eq!(first, vec![Effect::Run(Run::Ordinary)]);
    assert!(flow.visible());
    assert!(flow.onboarding.is_none());
    assert!(
        drive(&mut flow, first).is_empty(),
        "no Enter before the steps"
    );
    assert_eq!(
        fs::read(source_files.join("store/project.p.json")).unwrap(),
        before
    );
    let copies = pm_kernel::LibraryStore::open(&profile).projects().unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].name, "Synthetic project");
    // …then the Welcome and the steps, then Home.
    let welcome = flow.frame();
    assert!(welcome.animate);
    titled(&welcome, "Welcome to Powermove");
    flow.action("begin");
    titled(&flow.frame(), "Choose your timeline");
    flow.action("continue");
    titled(&flow.frame(), "Create your account");
    flow.action("not_now");
    titled(&flow.frame(), "Choose your agent");
    flow.action("continue");
    titled(&flow.frame(), "Speech to text");
    let effects = flow.action("continue");
    assert!(
        matches!(&effects[..], [Effect::Enter(Some(_))]),
        "{effects:?}"
    );
}
#[test]
fn a_quit_in_the_steps_after_the_copy_brings_them_back_until_they_finish() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    write(
        &home
            .join(APPLICATION_SUPPORT)
            .join("Powermove/onboarding-v1.json"),
        &json!({"version":1}),
    );
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    titled(&flow.frame(), "Welcome to Powermove");
    flow.action("begin");
    titled(&flow.frame(), "Choose your timeline");
    // Quit here. The copy is done and the original app's marker came
    // across; the next launch still leads to the steps.
    drop(flow);
    let again = plan(&home);
    assert_eq!(again.launch, Launch::Later);
    assert!(again.onboard);
    let profile = again.profile.clone();
    let mut flow = Flow::new(again);
    assert!(flow.start().is_empty());
    titled(&flow.frame(), "Welcome to Powermove");
    flow.action("begin");
    flow.action("continue");
    flow.action("not_now");
    flow.action("continue");
    let effects = flow.action("continue");
    let [Effect::Enter(Some(choices))] = &effects[..] else {
        panic!("{effects:?}");
    };
    // Finishing the last step is what ends them.
    write_marker(&profile, choices).unwrap();
    assert!(onboarding_completed(&profile));
    assert!(!plan(&home).onboard);
}
#[test]
fn a_migrated_user_who_never_finished_onboarding_sees_it_after_the_copy() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    titled(&flow.frame(), "Welcome to Powermove");
}
#[test]
fn outcomes_that_need_review_stop_on_the_summary() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    write(
        &support.join("Powermove/onboarding-v1.json"),
        &json!({"version":1}),
    );
    // A slot the index lists with nothing saved behind it: nothing was lost,
    // so it reads as a note, not a warning.
    write(
        &support.join("Powermove/store/projects.json"),
        &json!([{"id":"p","name":"Synthetic project"},{"id":"gone","name":"Unreadable"}]),
    );
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    let frame = flow.frame();
    titled(&frame, "Your library is ready");
    assert_eq!(frame.rows.len(), 1);
    assert_eq!(frame.rows[0].severity, Severity::Note);
    assert!(
        frame.rows[0]
            .text
            .starts_with("1 project in the original app’s list had nothing saved")
    );
    assert_eq!(flow.action("show_report"), vec![Effect::ShowReport]);
    // A first run here: the steps follow the summary.
    assert!(flow.action("open_native").is_empty());
    titled(&flow.frame(), "Welcome to Powermove");
}
#[test]
fn keychain_consent_is_asked_once_and_not_now_finishes_without_touching_it() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    write(
        &support.join("Powermove/onboarding-v1.json"),
        &json!({"version":1}),
    );
    fs::create_dir_all(support.join("Powermove/env")).unwrap();
    fs::write(
        support.join("Powermove/env/synthetic.env"),
        "TOKEN=enc:AAAA\n",
    )
    .unwrap();
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    let consent = flow.frame();
    titled(&consent, "Keep your sign-ins");
    assert_eq!(consent.surface, "N002");
    assert_eq!(consent.default_action(), Some("resume_with_secret_consent"));
    assert!(consent.buttons.iter().any(|b| b.label == "Not Now"));
    let effects = flow.action("skip_secret_consent");
    assert_eq!(effects, vec![Effect::Run(Run::SkipConsent)]);
    assert!(drive(&mut flow, effects).is_empty());
    let summary = flow.frame();
    titled(&summary, "Your library is ready");
    // Sign-ins left behind go to the report, never as a count on the gate.
    assert!(summary.rows.is_empty());
    assert!(summary.buttons.iter().any(|b| b.id == "show_report"));
}
#[test]
fn a_denied_keychain_offers_a_fresh_click_and_carrying_on_without() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    flow.action("resume_with_secret_consent");
    assert_eq!(flow.frame().status.as_deref(), Some("Waiting for macOS…"));
    assert!(flow.frame().buttons.iter().all(|b| !b.enabled));
    assert!(
        flow.message(Message::Finished(Finished::Denied(
            pm_first_launch::secrets::KeychainError(-25300)
                .message()
                .into()
        )))
        .is_empty()
    );
    let denied = flow.frame();
    assert!(denied.line.contains("wasn’t found"));
    titled(&denied, "Keychain access wasn’t allowed");
    assert_eq!(denied.default_action(), Some("skip_secret_consent"));
    assert_eq!(
        flow.action("resume_with_secret_consent"),
        vec![Effect::Run(Run::WithConsent)]
    );
    assert!(flow.model.keychain_error.is_none());
    assert!(!flow.frame().line.contains("wasn’t found"));
}
#[test]
fn a_running_original_app_waits_and_the_copy_starts_when_it_quits() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    let host = String::from_utf8(
        std::process::Command::new("hostname")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    std::os::unix::fs::symlink(
        format!("{}-{}", host.trim(), std::process::id()),
        support.join("Powermove/SingletonLock"),
    )
    .unwrap();
    let plan = plan(&home);
    let profile = plan.profile.clone();
    let mut flow = Flow::new(plan);
    assert_eq!(start(&mut flow), vec![Effect::Watch]);
    titled(&flow.frame(), "Quit the original app to continue");
    assert!(flow.frame().spinner);
    assert!(!profile.join("migration.json").exists());
    assert_eq!(
        flow.message(Message::ElectronClosed),
        vec![Effect::Run(Run::Ordinary)]
    );
}
#[test]
fn an_owner_on_another_mac_asks_for_a_check() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    std::os::unix::fs::symlink("another-mac-1", support.join("Powermove/SingletonLock")).unwrap();
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    let frame = flow.frame();
    titled(&frame, "The original app might still be open");
    assert_eq!(frame.default_action(), Some("check_again"));
}
#[test]
fn pausing_stops_at_a_durable_boundary_and_resume_picks_up() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let plan = plan(&home);
    let config = plan.config.clone().unwrap();
    let mut flow = Flow::new(plan);
    flow.start();
    let (tx, rx) = async_channel::unbounded();
    spawn_run(
        config,
        Run::Ordinary,
        Arc::new(AtomicBool::new(true)),
        Arc::default(),
        tx,
    );
    let mut paused = false;
    while let Ok(message) = rx.recv_blocking() {
        paused = matches!(message, Message::Finished(Finished::Paused));
        let end = matches!(message, Message::Finished(_));
        flow.message(message);
        if end {
            break;
        }
    }
    assert!(paused);
    let frame = flow.frame();
    titled(&frame, "Copying is paused");
    assert_eq!(frame.default_action(), Some("resume"));
    let effects = flow.action("resume");
    assert_eq!(effects, vec![Effect::Run(Run::Ordinary)]);
    assert_eq!(
        flow.frame().status.as_deref(),
        Some("Picking up where it stopped…")
    );
    assert!(drive(&mut flow, effects).is_empty());
    titled(&flow.frame(), "Welcome to Powermove");
}
#[test]
fn low_disk_and_refused_profiles_say_so_with_real_numbers() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    flow.message(Message::Finished(Finished::LowDisk(LowDisk {
        needed: 7_500_000_000,
        available: 1_200_000_000,
    })));
    let frame = flow.frame();
    titled(&frame, "Not enough space");
    assert!(frame.line.contains("7.5 GB"));
    assert!(frame.line.contains("1.2 GB"));
    assert_eq!(frame.default_action(), Some("check_again"));
    let error = anyhow::Error::new(ProfileRefused);
    assert!(matches!(classify(Err(error)), Finished::ProfileRefused(_)));
}
#[test]
fn a_foreign_destination_is_refused_without_touching_either_side() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    let plan = plan(&home);
    fs::create_dir_all(&plan.profile).unwrap();
    fs::write(plan.profile.join("unexpected"), b"synthetic").unwrap();
    let mut flow = Flow::new(plan);
    assert!(start(&mut flow).is_empty());
    let frame = flow.frame();
    titled(&frame, "Powermove can’t start");
    assert!(frame.details.is_some());
}
#[test]
fn failure_details_and_classes_stay_generic() {
    let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    assert!(matches!(
        classify(Err(anyhow::Error::new(denied))),
        Finished::Failed(Failure::Permission, _)
    ));
    assert!(matches!(
        classify(Err(anyhow::anyhow!("synthetic"))),
        Finished::Failed(Failure::Unknown, _)
    ));
}
/// A first launch that imported the original library, then onboarding, then
/// work in the original app: the state Chike's Mac was in.
fn migrated_then_original_changed(home: &Path) -> Plan {
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    write(
        &support.join("Powermove/onboarding-v1.json"),
        &json!({"version":1}),
    );
    // The first launch: the full migration, once.
    let first = plan(home);
    assert_eq!(first.launch, Launch::Migrate);
    let profile = first.profile.clone();
    let mut flow = Flow::new(first);
    assert!(flow.visible());
    start(&mut flow);
    // The person went through the steps after the copy.
    write_marker(&profile, &json!({"timelineMode":"layers"})).unwrap();
    // The original app is used afterwards: its stamp changes.
    write(&support.join("Powermove/store/theme.json"), &json!("light"));
    let later = plan(home);
    assert_eq!(later.launch, Launch::Later);
    assert!(!later.onboard);
    later
}
fn theme(config: &Config) -> Option<String> {
    pm_kernel::LibraryStore::open(&config.native_profile)
        .read("theme")
        .unwrap()
        .and_then(|v| v.as_str().map(str::to_owned))
}
#[test]
fn a_later_launch_with_newer_original_work_shows_no_screen_and_takes_no_capture() {
    let (_temp, home) = home();
    let later = migrated_then_original_changed(&home);
    let config = later.config.clone().unwrap();
    assert!(later::newer_work(&config).unwrap());
    let mut flow = Flow::new(later.clone());
    // The original app is there, so startup looks for extensions first:
    // none, so it opens without a window.
    assert_eq!(flow.start(), vec![Effect::ScanExtensions]);
    assert!(!flow.visible());
    let scanned = crate::extension_transfer::scan(&config);
    assert_eq!(
        flow.message(Message::Extensions(Update::Scanned(scanned))),
        vec![Effect::Enter(None)]
    );
    assert!(!flow.visible());
    // Before the library attaches: no capture, no events, nothing merged.
    assert!(!later_store(&later, |_| panic!("a launch must not capture")).unwrap());
    let deltas = config.native_profile.join("first-launch-deltas");
    assert!(!deltas.join("pending").exists());
    assert!(!deltas.join("pending.capture").exists());
    assert_eq!(theme(&config).as_deref(), Some("dark"));
    // Still there to offer on Home and in Settings › Storage.
    assert!(later::newer_work(&config).unwrap());
}
#[test]
fn asked_for_recent_work_runs_on_screen_then_opens_the_library() {
    let (_temp, home) = home();
    let later = migrated_then_original_changed(&home);
    let config = later.config.clone().unwrap();
    later::request(&config.native_profile).unwrap();
    let mut flow = Flow::new(later);
    assert_eq!(flow.start(), vec![Effect::BringIn]);
    assert!(flow.visible());
    let frame = flow.frame();
    titled(&frame, "Bringing in recent work");
    assert_eq!(frame.line, "Your original app stays unchanged.");
    assert!(frame.spinner);
    assert_eq!(frame.buttons.len(), 1);
    assert_eq!(frame.buttons[0].id, "quit");
    // The worker, as `spawn_bring_in` runs it; its steps show as they start.
    let mut events = vec![];
    let outcome = later::bring_in(&config, |event| {
        events.push(event);
        Ok(())
    })
    .unwrap();
    assert!(matches!(outcome, Outcome::Complete { .. }));
    for event in events {
        if matches!(event, Event::Started(Step::Library)) {
            flow.message(Message::Event(event));
            assert_eq!(
                flow.frame().status.as_deref(),
                Some("Bringing in projects…")
            );
        } else {
            flow.message(Message::Event(event));
        }
    }
    assert_eq!(
        flow.message(Message::Recent(RecentEnd::Done)),
        vec![Effect::Enter(None)]
    );
    assert_eq!(theme(&config).as_deref(), Some("light"));
    assert!(!later::requested(&config.native_profile));
    assert!(!later::newer_work(&config).unwrap());
    // The launch after: nothing asked, nothing on screen.
    let mut again = Flow::new(plan(&home));
    assert_ne!(again.start(), vec![Effect::BringIn]);
    assert!(!again.visible());
}
#[test]
fn a_failed_bring_in_is_recorded_and_no_launch_tries_it_again() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, home) = home();
    let later = migrated_then_original_changed(&home);
    let config = later.config.clone().unwrap();
    // A file the capture can't read.
    let locked = config.electron_profile.join("store/project.q.json");
    write(&locked, &json!({"v":"1.2.0"}));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    later::request(&config.native_profile).unwrap();
    let error = later::bring_in(&config, |_| Ok(())).unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
    let failure = later::last_failure(&config.native_profile).unwrap();
    assert_eq!(
        failure.reason,
        "Some of the original app’s files couldn’t be read."
    );
    assert!(!later::requested(&config.native_profile));
    // On screen: why, and the way on.
    let mut flow = Flow::new(later.clone());
    flow.recent = Some(Recent::Running {
        step: None,
        progress: None,
    });
    flow.message(Message::Recent(RecentEnd::Failed {
        reason: failure.reason.clone(),
        low_disk: None,
        details: format!("{error:#}"),
    }));
    let frame = flow.frame();
    titled(&frame, "Recent work couldn’t come in");
    assert!(
        frame
            .line
            .starts_with("Some of the original app’s files couldn’t be read.")
    );
    assert_eq!(frame.default_action(), Some("open_native"));
    assert_eq!(flow.action("open_native"), vec![Effect::Enter(None)]);
    // The next launch: no screen, no new capture, the record stays.
    let mut next = Flow::new(plan(&home));
    assert_ne!(next.start(), vec![Effect::BringIn]);
    assert!(!next.visible());
    assert!(
        !later_store(&later, |_| panic!(
            "a failed capture is not retried at launch"
        ))
        .unwrap()
    );
    assert_eq!(later::last_failure(&config.native_profile), Some(failure));
}
#[test]
fn a_taken_capture_finishes_at_launch_unless_its_last_attempt_failed() {
    let (_temp, home) = home();
    let later = migrated_then_original_changed(&home);
    let config = later.config.clone().unwrap();
    // An earlier attempt took the capture and merged it; its media is left.
    assert!(matches!(
        pm_first_launch::delta_store(&config, |_| Ok(())).unwrap(),
        DeltaPrepared::MediaPending
    ));
    assert!(later_store(&later, |_| Ok(())).unwrap());
    // Had it failed, the launch leaves it for the person to ask again.
    later::record_failure(
        &config.native_profile,
        &anyhow::Error::new(LowDisk {
            needed: 8,
            available: 6,
        }),
    )
    .unwrap();
    assert!(!later_store(&later, |_| panic!("failed capture")).unwrap());
    pm_first_launch::delta_media(&config, |_| Ok(())).unwrap();
    assert_eq!(theme(&config).as_deref(), Some("light"));
}
#[test]
fn a_low_disk_stop_and_a_running_original_app_say_so() {
    let (_temp, home) = home();
    let mut plan = plan(&home);
    plan.onboard = false;
    plan.launch = Launch::Later;
    let mut flow = Flow::new(plan);
    flow.recent = Some(Recent::Running {
        step: None,
        progress: None,
    });
    assert!(flow.visible());
    flow.message(Message::Recent(RecentEnd::WaitingForElectron));
    let frame = flow.frame();
    titled(&frame, "Quit the original app");
    assert!(frame.spinner);
    assert_eq!(
        flow.action("open_native"),
        vec![Effect::WithdrawRequest, Effect::Enter(None)]
    );
    flow.entry = Entry::Waiting;
    flow.recent = Some(Recent::Running {
        step: None,
        progress: None,
    });
    flow.message(Message::Recent(RecentEnd::Failed {
        reason: "There wasn’t enough free space on this Mac.".into(),
        low_disk: Some((8_000_000_000, 6_000_000_000)),
        details: "migration paused".into(),
    }));
    assert!(
        flow.frame()
            .line
            .starts_with("It needs 8.0 GB free on this Mac, and there’s 6.0 GB.")
    );
}
#[test]
fn the_kernel_can_start_before_the_copy_without_spoiling_the_destination() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let plan = plan(&home);
    fs::create_dir_all(&plan.profile).unwrap();
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
        journal_root: Some(plan.profile.join("journals")),
        ..Default::default()
    });
    let mut flow = Flow::new(plan.clone());
    start(&mut flow);
    assert_eq!(flow.model.gate, Gate::Complete);
    crate::mcp::attach(&actor.handle(), &plan.profile).unwrap();
    actor.shutdown();
}
#[test]
fn a_later_launch_never_shows_a_screen_while_the_library_opens() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    flow.plan.onboard = false;
    flow.plan.launch = Launch::Later;
    assert_eq!(flow.start(), vec![Effect::Enter(None)]);
    assert!(!flow.visible());
    // A second press while it opens does nothing; there is nothing to press.
    assert!(flow.action("quit").contains(&Effect::Quit));
    assert!(!flow.visible());
}
/// A finished copy with something to review, as the worker sends it.
fn review(flow: &mut Flow, report: Result<pm_first_launch::surfaces::migration::Report, String>) {
    flow.message(Message::Finished(Finished::Ready(
        Outcome::Complete {
            projects: 1,
            history_refusals: 1,
            missing_projects: 0,
            blocked_owners: 0,
            proxy_only: 0,
            already_offline: 0,
            not_reached: 0,
        },
        report,
    )));
}
fn finished_onboarding(home: &Path) -> Flow {
    let mut plan = plan(home);
    plan.launch = Launch::Migrate;
    plan.onboard = false;
    Flow::new(plan)
}
#[test]
fn opening_from_the_summary_happens_once_however_often_it_is_asked() {
    let (_temp, home) = home();
    let mut flow = finished_onboarding(&home);
    review(&mut flow, Ok(Default::default()));
    titled(&flow.frame(), "Your library is ready");
    assert_eq!(flow.action("open_native"), vec![Effect::Enter(None)]);
    // A double click, or Return held down.
    assert!(flow.action("open_native").is_empty());
    assert!(flow.action("show_report").is_empty());
    let frame = flow.frame();
    assert!(frame.spinner);
    assert!(frame.buttons.is_empty());
    assert_eq!(frame.default_action(), None);
}
#[test]
fn an_unread_report_stops_on_the_summary_and_says_so() {
    let (_temp, home) = home();
    let mut flow = finished_onboarding(&home);
    flow.message(Message::Finished(Finished::Ready(
        Outcome::Complete {
            projects: 1,
            history_refusals: 0,
            missing_projects: 0,
            blocked_owners: 0,
            proxy_only: 0,
            already_offline: 0,
            not_reached: 0,
        },
        Err("unreadable".into()),
    )));
    let frame = flow.frame();
    titled(&frame, "Your library is ready");
    assert!(frame.rows[0].text.contains("couldn’t be read"));
    assert!(frame.buttons.iter().any(|b| b.id == "show_report"));
}
#[test]
fn a_failed_library_stays_on_a_gate_and_retries_with_the_same_choices() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    flow.start();
    flow.action("begin");
    flow.action("continue");
    flow.action("not_now");
    flow.action("choose:claude");
    flow.action("continue");
    let effects = flow.action("continue");
    let [Effect::Enter(Some(chosen))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert!(flow.enter_failed("locked".into()).is_empty());
    assert!(flow.visible());
    let frame = flow.frame();
    titled(&frame, "Your library couldn’t open");
    assert_eq!(frame.details.as_deref(), Some("locked"));
    assert_eq!(frame.default_action(), Some("try_again"));
    assert!(flow.action("begin").is_empty());
    assert_eq!(
        flow.action("try_again"),
        vec![Effect::Enter(Some(chosen.clone()))]
    );
    assert!(flow.frame().spinner);
}
#[test]
fn a_later_launch_that_fails_to_attach_gets_a_window() {
    let (_temp, home) = home();
    let mut plan = plan(&home);
    plan.onboard = false;
    plan.launch = Launch::Later;
    let mut flow = Flow::new(plan);
    assert_eq!(flow.start(), vec![Effect::Enter(None)]);
    assert!(!flow.visible());
    flow.enter_failed("locked".into());
    assert!(flow.visible());
    assert_eq!(flow.action("quit"), vec![Effect::Quit]);
}
#[test]
fn pause_waits_for_the_worker_and_resume_cannot_start_a_second_one() {
    let (_temp, home) = home();
    let mut flow = finished_onboarding(&home);
    flow.start();
    flow.message(Message::Event(Event::Started(
        pm_first_launch::Step::Library,
    )));
    assert_eq!(flow.action("pause"), vec![Effect::Pause]);
    let frame = flow.frame();
    titled(&frame, "Copying your library");
    assert_eq!(frame.status.as_deref(), Some("Pausing…"));
    // The worker's next event must not read as running again.
    flow.message(Message::Event(Event::Started(pm_first_launch::Step::Media)));
    assert_eq!(flow.model.gate, Gate::Pausing);
    assert!(flow.action("resume").is_empty());
    flow.message(Message::Finished(Finished::Paused));
    titled(&flow.frame(), "Copying is paused");
    assert_eq!(flow.action("resume"), vec![Effect::Run(Run::Ordinary)]);
}
#[test]
fn protected_folders_are_explained_then_a_waiting_prompt_can_be_skipped() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let mut flow = Flow::new(plan(&home));
    assert_eq!(flow.start(), vec![Effect::Run(Run::Ordinary)]);
    // Copying: the card shows each step; the running one has live bytes.
    flow.message(Message::Event(Event::Started(
        pm_first_launch::Step::Checkpoint,
    )));
    assert!(flow.activity((250, 1000), None));
    let frame = flow.frame();
    titled(&frame, "Copying your library");
    assert_eq!(frame.steps[0].title, "Library snapshot");
    assert_eq!(frame.steps[0].value, "250 bytes of 1.0 KB");
    assert_eq!(frame.steps[0].progress, Some(Progress::Determinate(0.25)));
    assert_eq!(frame.steps[1].progress, None);
    // Unchanged activity doesn't repaint.
    assert!(!flow.activity((250, 1000), None));
    flow.message(Message::Finished(Finished::Outcome(
        Outcome::NeedsFolderAccess {
            folders: vec![("Downloads".into(), 3)],
        },
    )));
    let frame = flow.frame();
    titled(&frame, "Allow access to your media");
    assert_eq!(frame.default.as_deref(), Some("allow_folders"));
    assert_eq!(
        (frame.steps[0].title.as_str(), frame.steps[0].value.as_str()),
        ("Downloads", "3 files")
    );
    assert_eq!(
        flow.action("allow_folders"),
        vec![Effect::Run(Run::Folders(FolderChoice::Allow))]
    );
    // macOS's prompt is up and unanswered.
    flow.message(Message::Event(Event::Started(
        pm_first_launch::Step::Inventory,
    )));
    assert!(flow.activity((0, 0), Some("Downloads".into())));
    let frame = flow.frame();
    titled(&frame, "Waiting for macOS");
    assert!(frame.spinner);
    assert_eq!(frame.default, None);
    assert_eq!(flow.action("skip_folder"), vec![Effect::SkipFolder]);
    flow.activity((0, 0), None);
    titled(&flow.frame(), "Copying your library");
    // Skip These Files before macOS is ever asked.
    flow.message(Message::Finished(Finished::Outcome(
        Outcome::NeedsFolderAccess {
            folders: vec![("Downloads".into(), 3)],
        },
    )));
    assert_eq!(
        flow.action("skip_folders"),
        vec![Effect::Run(Run::Folders(FolderChoice::Skip))]
    );
}

/// An existing library (made here, not copied) beside an original app that has
/// extensions it doesn't: Chike's case. `pong` loads, `figma-layer-import` is
/// an empty folder, `broken` is copied but can't load.
fn existing_library_with_original_extensions(home: &Path) -> (Plan, PathBuf) {
    let support = home.join(APPLICATION_SUPPORT);
    let original = support.join("Powermove");
    write(&original.join("store/projects.json"), &json!([]));
    let extension = |id: &str, name: &str, source: &str| {
        write(
            &original.join(format!("extensions/{id}/manifest.json")),
            &json!({"id":id,"name":name,"version":"1.0.0","apiVersion":3,"entry":"index.ts"}),
        );
        fs::write(original.join(format!("extensions/{id}/index.ts")), source).unwrap();
    };
    extension("pong", "Pong", "export default function activate() {}");
    extension(
        "broken",
        "Broken Glow",
        "export default function activate( {",
    );
    fs::create_dir_all(original.join("extensions/figma-layer-import")).unwrap();
    let native = pm_first_launch::default_paths(&support).native_profile;
    write(
        &native.join("store/projects.json"),
        &json!([{"id":"mine","name":"Made here"}]),
    );
    write(
        &native.join(pm_first_launch::ONBOARDING_MARKER),
        &json!({"version":1,"timelineMode":"layers"}),
    );
    let plan = plan(home);
    assert_eq!(
        plan.launch,
        Launch::Fresh,
        "an existing library, not a copy"
    );
    assert!(!plan.onboard);
    (plan, native)
}
fn scan_into(flow: &mut Flow) -> Vec<Effect> {
    let config = flow.plan.original.clone().unwrap();
    let scanned = crate::extension_transfer::scan(&config);
    flow.message(Message::Extensions(Update::Scanned(scanned)))
}
#[test]
fn not_now_opens_the_library_and_later_launches_do_not_ask_again() {
    let (_temp, home) = home();
    let (plan, native) = existing_library_with_original_extensions(&home);
    let mut flow = Flow::new(plan.clone());
    assert_eq!(flow.start(), vec![Effect::ScanExtensions]);
    assert!(!flow.visible(), "nothing shows while it looks");
    assert!(scan_into(&mut flow).is_empty());
    assert!(flow.visible());
    let frame = flow.frame();
    assert_eq!(frame.surface, "N004");
    titled(&frame, "Bring in your extensions");
    assert!(frame.line.contains("2 extensions"), "{}", frame.line);
    // The skip, in full; the rest by display name.
    assert_eq!(frame.rows.len(), 1, "{:?}", frame.rows);
    assert_eq!(frame.rows[0].text, "figma-layer-import");
    assert!(frame.rows[0].detail.as_deref().unwrap().contains("empty"));
    let names = frame.names.as_ref().unwrap();
    assert_eq!(names.names, ["Broken Glow", "Pong"]);
    assert!(frame.top, "the step's title stays put");
    let labels: Vec<_> = frame.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["Not Now", "Bring In Extensions"]);
    // Not Now: remembered, and the library opens at once.
    assert_eq!(
        flow.action("not_now"),
        vec![Effect::DeclineExtensions, Effect::Enter(None)]
    );
    crate::extension_transfer::decline(&native);
    assert!(!native.join("extensions/pong").exists(), "nothing copied");
    // Relaunch: no scan, no window, straight in.
    let mut again = Flow::new(plan.clone());
    assert_eq!(again.start(), vec![Effect::Enter(None)]);
    assert!(!again.visible());
    // Settings › Storage still offers them, with its button.
    let report = crate::storage::measure(
        &native,
        plan.config.as_ref().map(|c| c.electron_profile.as_path()),
    );
    assert_eq!(report.original.unwrap().extensions, Some((2, 2)));
}
#[test]
fn bring_in_copies_only_extensions_and_summarises_each_with_its_reason() {
    let (_temp, home) = home();
    let (plan, native) = existing_library_with_original_extensions(&home);
    let config = plan.config.clone().unwrap();
    let library = fs::read(native.join("store/projects.json")).unwrap();
    let onboarding = fs::read(native.join(pm_first_launch::ONBOARDING_MARKER)).unwrap();
    let mut flow = Flow::new(plan.clone());
    flow.start();
    scan_into(&mut flow);
    assert_eq!(flow.action("bring_in"), vec![Effect::RunExtensions]);
    assert_eq!(flow.frame().surface, "N005");
    assert!(
        flow.frame().buttons.is_empty(),
        "nothing to press while it runs"
    );
    // A second press while it runs does nothing.
    assert!(flow.action("bring_in").is_empty());
    let (tx, rx) = async_channel::unbounded();
    crate::extension_transfer::spawn_run(config.clone(), tx);
    let mut saw_progress = false;
    while let Ok(update) = rx.recv_blocking() {
        let end = matches!(update, Update::Finished(_));
        saw_progress |= matches!(update, Update::Progress(_));
        assert!(flow.message(Message::Extensions(update)).is_empty());
        if !end && saw_progress {
            assert_eq!(flow.frame().steps.len(), 1, "one bar while it runs");
        }
        if end {
            break;
        }
    }
    assert!(saw_progress);
    let frame = flow.frame();
    assert_eq!(frame.surface, "N006");
    titled(&frame, "Your extensions are in");
    assert!(
        frame
            .line
            .starts_with("One couldn’t load. The reason is below."),
        "{}",
        frame.line
    );
    let stats: Vec<_> = frame
        .stats
        .iter()
        .map(|s| (s.value.as_str(), s.label.as_str()))
        .collect();
    assert_eq!(
        stats,
        [
            ("1", "Brought in"),
            ("1", "Couldn’t load"),
            ("1", "Skipped")
        ]
    );
    // Exceptions first and in full, by display name, with their reasons.
    assert_eq!(frame.rows_title.as_deref(), Some("Needs a look"));
    assert_eq!(frame.rows[0].text, "Broken Glow");
    let reason = frame.rows[0].detail.as_deref().unwrap();
    assert!(
        reason.starts_with("It was copied but couldn’t load"),
        "{reason}"
    );
    assert_eq!(frame.rows[1].text, "figma-layer-import");
    // Successes by name only, no repeated reason.
    assert_eq!(frame.names.as_ref().unwrap().names, ["Pong"]);
    assert!(native.join("extensions/pong/manifest.json").is_file());
    // Projects and settings are exactly as they were.
    assert_eq!(
        fs::read(native.join("store/projects.json")).unwrap(),
        library
    );
    assert_eq!(
        fs::read(native.join(pm_first_launch::ONBOARDING_MARKER)).unwrap(),
        onboarding
    );
    assert_eq!(flow.action("open_native"), vec![Effect::Enter(None)]);
    // The next launch looks again and finds nothing new: straight in.
    let mut again = Flow::new(plan);
    assert_eq!(again.start(), vec![Effect::ScanExtensions]);
    assert_eq!(scan_into(&mut again), vec![Effect::Enter(None)]);
}

#[test]
fn existing_isolated_library_can_only_offer_extensions_with_an_explicit_source() {
    let (_temp, home) = home();
    let (default, native) = existing_library_with_original_extensions(&home);
    let source = default.config.unwrap().electron_profile;
    let isolated = Plan::resolve(
        Some(native.clone()),
        Some(source),
        Some(home.into_os_string()),
    )
    .unwrap();
    assert_eq!(isolated.launch, Launch::Fresh);
    assert!(!isolated.onboard);
    let mut flow = Flow::new(isolated);
    assert_eq!(flow.start(), vec![Effect::ScanExtensions]);
    assert!(scan_into(&mut flow).is_empty());
    assert_eq!(flow.frame().surface, "N004");
    assert!(!native.join("migration.json").exists());
    assert_eq!(
        flow.action("not_now"),
        vec![Effect::DeclineExtensions, Effect::Enter(None)]
    );
}

#[test]
fn keychain_error_survives_the_consent_context() {
    let error =
        anyhow::Error::new(pm_first_launch::secrets::KeychainError(-25308)).context(KeychainDenied);
    let Finished::Denied(reason) = classify(Err(error)) else {
        panic!("expected a Keychain reason")
    };
    assert!(reason.contains("couldn’t show"));
}

#[test]
fn overridden_existing_library_discovers_original_and_not_now_survives_relaunch() {
    let (_temp, home) = home();
    let (_, native) = existing_library_with_original_extensions(&home);
    let resolve = || {
        Plan::resolve(
            Some(native.clone()),
            None,
            Some(home.as_os_str().to_owned()),
        )
        .unwrap()
    };
    let plan = resolve();
    assert!(plan.config.is_none());
    assert_eq!(plan.launch, Launch::Fresh);
    assert!(!plan.onboard);
    assert!(plan.electron().is_some());
    let mut flow = Flow::new(plan.clone());
    assert_eq!(flow.start(), vec![Effect::ScanExtensions]);
    scan_into(&mut flow);
    assert_eq!(flow.frame().surface, "N004");
    assert_eq!(
        flow.action("not_now"),
        vec![Effect::DeclineExtensions, Effect::Enter(None)]
    );
    crate::extension_transfer::decline(&native);
    let mut again = Flow::new(resolve());
    assert_eq!(again.start(), vec![Effect::Enter(None)]);
    let report = crate::storage::measure(&native, plan.electron().as_deref());
    assert_eq!(report.original.unwrap().extensions, Some((2, 2)));
    assert!(crate::extension_transfer::eligible(plan.original.as_ref()).is_some());
    assert!(!later_store(&plan, |_| panic!("isolated profile must not merge")).unwrap());
}

#[test]
fn explicit_original_overrides_default_discovery() {
    let (_temp, home) = home();
    let native = home.join("isolated");
    let original = home.join("explicit-original");
    fs::create_dir_all(&original).unwrap();
    let plan = Plan::resolve(
        Some(native.clone()),
        Some(original.clone()),
        Some(home.as_os_str().to_owned()),
    )
    .unwrap();
    assert_eq!(plan.electron(), Some(original.clone()));
    assert_eq!(
        plan.original.unwrap(),
        Config {
            electron_profile: original,
            native_profile: native
        }
    );
}

#[test]
fn synthetic_runs_can_disable_original_discovery() {
    let (_temp, home) = home();
    let defaults = pm_first_launch::default_paths(&home.join(APPLICATION_SUPPORT));
    assert!(Plan::original_source(&home.join("isolated"), None, Some(&defaults), true).is_none());
    assert!(
        Plan::original_source(
            &home.join("isolated"),
            Some(home.join("explicit")),
            Some(&defaults),
            true
        )
        .is_none()
    );
}

/// Headless pictures of the later-launch screens, for review. Run on demand:
/// `PM_WELCOME_CAPTURE=<dir> cargo test -p pm-native-app capture_recent_work_frames -- --ignored`
#[test]
#[ignore = "writes PNGs; set PM_WELCOME_CAPTURE"]
fn capture_recent_work_frames() {
    use gpui::{AppContext, HeadlessAppContext, px, size};
    use pm_design::{Accessibility, Scheme, Tokens};
    let dir = std::env::var("PM_WELCOME_CAPTURE").unwrap();
    fs::create_dir_all(&dir).unwrap();
    // Before: the frame a later launch showed past a second of merging
    // (removed; its values as they were).
    let before = Frame {
        surface: "N001".into(),
        title: "Updating Your Library".into(),
        line: "Your original app stays unchanged.".into(),
        status: Some("Bringing in your recent work…".into()),
        spinner: true,
        buttons: vec![pm_shell::welcome::Button::new("quit", "Quit Powermove").leading()],
        ..Frame::default()
    };
    let frames = [
        ("before-updating-your-library", before),
        (
            "after-asked-bringing-in",
            recent_frame(&Recent::Running {
                step: Some(Step::Library),
                progress: Some((3, 12)),
            }),
        ),
        (
            "after-asked-waiting-for-original",
            recent_frame(&Recent::Waiting),
        ),
        (
            "after-asked-low-disk",
            recent_frame(&Recent::Failed {
                reason: "There wasn’t enough free space on this Mac.".into(),
                low_disk: Some(("6.9 GB".into(), "6.2 GB".into())),
                details: "migration paused: 6 GiB free space reserve required".into(),
            }),
        ),
    ];
    for (name, frame) in frames {
        objc2::rc::autoreleasepool(|_| {
            let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
            let m = tokens.metrics;
            let mut cx = HeadlessAppContext::with_platform(
                Arc::new(
                    gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                        .with_automatic_optical_sizing(),
                ),
                Arc::new(()),
                || {
                    Some(Box::new(
                        gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                    ))
                },
            );
            let window = cx
                .open_window(size(px(m.welcome_width), px(m.welcome_height)), |_, cx| {
                    cx.new(|cx| {
                        let mut view = pm_shell::welcome_view::WelcomeView::new(
                            tokens.clone(),
                            frame,
                            Rc::new(|_, _| {}),
                            cx,
                        );
                        view.clock = Some(1.);
                        view
                    })
                })
                .unwrap();
            cx.run_until_parked();
            cx.capture_screenshot(window.into()).unwrap();
            cx.update_window(window.into(), |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).unwrap();
            image.save(format!("{dir}/{name}.png")).unwrap();
        });
    }
}

#[test]
fn pm348_refused_worker_updates_without_a_window() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    let isolated = home.join("synthetic-review-profile");
    let plan = Plan::resolve(
        Some(isolated.clone()),
        Some(support.join("Powermove")),
        Some(home.as_os_str().to_owned()),
    )
    .unwrap();
    assert_eq!(plan.launch, Launch::Migrate);
    let mut flow = Flow::new(plan);
    assert!(start(&mut flow).is_empty());
    titled(&flow.frame(), "Powermove can’t start");
    assert!(!flow.frame().spinner);
    assert!(flow.frame().details.is_some());
    assert!(!isolated.exists());
}
#[cfg(debug_assertions)] // `show_for_review` is a development-build switch
#[test]
fn each_review_frame_shows_on_its_own_and_its_buttons_do_nothing() {
    let (_temp, home) = home();
    for (name, title) in [
        ("consent", "Keep your sign-ins"),
        ("denied", "Keychain access wasn’t allowed"),
        ("ready", "Your library is ready"),
        ("low-disk", "Not enough space"),
        ("original-running", "Quit the original app to continue"),
        ("original-elsewhere", "The original app might still be open"),
        ("library-failed", "Your library couldn’t open"),
    ] {
        let mut flow = Flow::new(plan(&home));
        assert!(flow.show_for_review(name), "{name}");
        assert!(flow.visible(), "{name}");
        let frame = flow.frame();
        titled(&frame, title);
        // Its own window, not the stage; nothing copies or asks macOS.
        assert!(!flow.plan.onboard, "{name}");
        for button in &frame.buttons {
            let effects = flow.action(&button.id);
            assert!(
                effects.is_empty() || effects == vec![Effect::Quit],
                "{name}: {} did {effects:?}",
                button.id
            );
        }
    }
    let mut flow = Flow::new(plan(&home));
    assert!(!flow.show_for_review("nothing"));
    // A ready library without review rows offers only to open it.
    assert!(flow.show_for_review("ready"));
    let frame = flow.frame();
    assert!(frame.rows.is_empty());
    assert_eq!(frame.default_action(), Some("open_native"));
}

/// A flow on the account step (Welcome, then the timeline's Continue).
fn on_account() -> (tempfile::TempDir, Flow) {
    let (temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    flow.action("begin");
    flow.action("continue");
    titled(&flow.frame(), "Create your account");
    (temp, flow)
}
fn sign_in(frame: &Frame) -> &pm_shell::welcome::SignIn {
    frame.sign_in.as_ref().expect("the sign-in")
}
/// Types an email and presses Continue; the code goes out.
fn send(flow: &mut Flow, email: &str) {
    flow.action(&format!("field:email:{email}"));
    assert_eq!(
        flow.action("continue"),
        vec![Effect::SendCode(email.into())]
    );
    assert!(flow.frame().busy, "Continue spins while the code goes out");
    let effects = flow.message(Message::CodeSent(Ok(())));
    assert_eq!(effects, vec![Effect::After(Duration::from_secs(1), "tick")]);
}
#[test]
fn the_account_step_sends_a_code_to_a_plausible_email() {
    let (_temp, mut flow) = on_account();
    for typed in [
        "",
        "ana",
        "ana@",
        "ana@example",
        "@example.com",
        "ana @example.com",
    ] {
        flow.action(&format!("field:email:{typed}"));
        assert_eq!(flow.frame().default_action(), None, "{typed:?}");
        assert_eq!(flow.action("continue"), vec![], "{typed:?}");
    }
    flow.action("field:email:ana@example.com");
    assert_eq!(flow.frame().default_action(), Some("continue"));
    assert_eq!(
        flow.action("continue"),
        vec![Effect::SendCode("ana@example.com".into())]
    );
    // A second press (or Return) while it goes out does nothing.
    assert_eq!(flow.action("continue"), vec![]);
    let sending = flow.frame();
    assert!(sending.busy);
    assert_eq!(sign_in(&sending).stage, SignInStage::Email);
    flow.message(Message::CodeSent(Ok(())));
    let code = flow.frame();
    titled(&code, "Check your email");
    assert_eq!(code.line, "We sent a 6-digit code to ana@example.com.");
    assert!(!code.busy);
    let s = sign_in(&code);
    assert_eq!(
        (s.stage, s.email.as_str(), s.code.as_str()),
        (SignInStage::Code, "ana@example.com", "")
    );
    assert_eq!(code.default_action(), None, "Continue waits for six digits");
}
#[test]
fn the_sixth_digit_checks_the_code_and_the_check_moves_on() {
    let (_temp, mut flow) = on_account();
    send(&mut flow, "ana@example.com");
    assert_eq!(flow.action("field:code:12345"), vec![]);
    // A pasted full code arrives at once, and checks itself.
    let effects = flow.action("field:code:123456");
    assert_eq!(
        effects,
        vec![Effect::VerifyCode {
            email: "ana@example.com".into(),
            code: "123456".into()
        }]
    );
    assert!(flow.frame().busy);
    assert_eq!(flow.action("continue"), vec![], "already being checked");
    let effects = flow.message(Message::Verified(Ok("ana@example.com".into())));
    assert_eq!(effects, vec![Effect::After(SIGNED_IN_PAUSE, "signed_in")]);
    let done = flow.frame();
    titled(&done, "You’re in");
    assert_eq!(done.line, "Signed in as ana@example.com.");
    assert_eq!(sign_in(&done).stage, SignInStage::Done);
    // Continue stays: it moves on before the pause ends, and the pause's
    // own move does nothing more.
    assert_eq!(done.default_action(), Some("continue"));
    flow.action("signed_in");
    titled(&flow.frame(), "Choose your agent");
    flow.action("signed_in");
    titled(&flow.frame(), "Choose your agent");
    // Back on the step: who is signed in, and another account on offer.
    flow.action("back");
    let signed = flow.frame();
    titled(&signed, "Your Powermove account");
    assert_eq!(signed.line, "Signed in as ana@example.com.");
    flow.action("use_other");
    titled(&flow.frame(), "Create your account");
    assert_eq!(sign_in(&flow.frame()).email, "");
    flow.action("escape");
    titled(&flow.frame(), "Your Powermove account");
}
#[test]
fn a_wrong_or_expired_code_shakes_clears_and_says_so() {
    let (_temp, mut flow) = on_account();
    send(&mut flow, "ana@example.com");
    let before = sign_in(&flow.frame()).clone();
    flow.action("field:code:000000");
    assert_eq!(
        flow.message(Message::Verified(Err("api:unauthorized".into()))),
        vec![]
    );
    let wrong = flow.frame();
    let s = sign_in(&wrong);
    assert_eq!(s.stage, SignInStage::Code);
    assert_eq!(s.code, "");
    assert_eq!(s.shake, before.shake + 1);
    assert!(s.revision > before.revision, "the field is cleared");
    assert!(!s.retry);
    assert_eq!(
        wrong.error.as_deref(),
        Some("That code didn’t match. Try again.")
    );
    // Typing again takes the line away.
    flow.action("field:code:1");
    assert_eq!(flow.frame().error, None);
    flow.action("field:code:111111");
    flow.message(Message::Verified(Err("api:gone".into())));
    let expired = flow.frame();
    assert_eq!(
        expired.error.as_deref(),
        Some("That code has expired. Send a new one.")
    );
    assert_eq!(sign_in(&expired).shake, before.shake + 2);
    // An expired code can be sent again at once.
    assert_eq!(sign_in(&expired).resend_in, Some(0));
    assert_eq!(
        flow.action("resend"),
        vec![Effect::SendCode("ana@example.com".into())]
    );
}
#[test]
fn network_errors_stay_inline_with_try_again() {
    let (_temp, mut flow) = on_account();
    flow.action("field:email:ana@example.com");
    flow.action("continue");
    flow.message(Message::CodeSent(Err("network".into())));
    let offline = flow.frame();
    titled(&offline, "Create your account");
    assert_eq!(
        offline.error.as_deref(),
        Some("Can’t reach Powermove. Try again in a moment.")
    );
    assert!(sign_in(&offline).retry);
    assert!(!offline.busy);
    assert_eq!(
        flow.action("retry"),
        vec![Effect::SendCode("ana@example.com".into())]
    );
    flow.message(Message::CodeSent(Ok(())));
    flow.action("field:code:123456");
    flow.message(Message::Verified(Err("network".into())));
    let offline = flow.frame();
    assert!(sign_in(&offline).retry);
    // The code stays: Try again checks it again.
    assert_eq!(sign_in(&offline).code, "123456");
    assert_eq!(
        flow.action("retry"),
        vec![Effect::VerifyCode {
            email: "ana@example.com".into(),
            code: "123456".into()
        }]
    );
}
#[test]
fn resend_waits_thirty_seconds() {
    let (_temp, mut flow) = on_account();
    send(&mut flow, "ana@example.com");
    let wait = sign_in(&flow.frame()).resend_in.unwrap();
    assert!((29..=30).contains(&wait), "{wait}");
    assert_eq!(flow.action("resend"), vec![]);
    // The countdown ticks while it runs.
    assert_eq!(
        flow.action("tick"),
        vec![Effect::After(Duration::from_secs(1), "tick")]
    );
    flow.sign_in.resend_at = Some(Instant::now());
    assert_eq!(sign_in(&flow.frame()).resend_in, Some(0));
    assert_eq!(flow.action("tick"), vec![]);
    assert_eq!(
        flow.action("resend"),
        vec![Effect::SendCode("ana@example.com".into())]
    );
    // Continue does not spin for it, and a second press does nothing.
    assert!(!flow.frame().busy);
    assert_eq!(flow.action("resend"), vec![]);
    // A failed resend stays inline; Continue still checks the code, never
    // leaves the step.
    flow.message(Message::CodeSent(Err("network".into())));
    assert!(sign_in(&flow.frame()).retry);
    assert_eq!(flow.action("continue"), vec![]);
    titled(&flow.frame(), "Check your email");
    assert_eq!(
        flow.action("retry"),
        vec![Effect::SendCode("ana@example.com".into())]
    );
    // Resending keeps the step on the code, and starts the wait again.
    flow.message(Message::CodeSent(Ok(())));
    let again = flow.frame();
    titled(&again, "Check your email");
    assert!(sign_in(&again).resend_in.unwrap() >= 29);
}
#[test]
fn change_and_escape_go_back_to_the_email_as_typed() {
    let (_temp, mut flow) = on_account();
    send(&mut flow, "ana@example.com");
    let revision = sign_in(&flow.frame()).revision;
    flow.action("field:code:12");
    flow.action("change");
    let email = flow.frame();
    titled(&email, "Create your account");
    let s = sign_in(&email);
    assert_eq!(
        (s.stage, s.email.as_str()),
        (SignInStage::Email, "ana@example.com")
    );
    assert!(s.revision > revision, "the field shows the email again");
    assert_eq!(email.default_action(), Some("continue"));
    // Escape and Back do the same from the code.
    send(&mut flow, "ana@example.com");
    flow.action("escape");
    assert_eq!(sign_in(&flow.frame()).stage, SignInStage::Email);
    send(&mut flow, "ana@example.com");
    flow.action("back");
    titled(&flow.frame(), "Create your account");
    // A code checked after Change is not wanted.
    send(&mut flow, "ana@example.com");
    flow.action("field:code:123456");
    flow.action("change");
    flow.message(Message::Verified(Ok("ana@example.com".into())));
    titled(&flow.frame(), "Create your account");
    // From the email, Back leaves the step.
    flow.action("back");
    titled(&flow.frame(), "Choose your timeline");
}
#[test]
fn chosen_speech_models_download_in_the_order_asked() {
    let (_temp, mut flow) = on_account();
    flow.action("not_now");
    flow.action("continue");
    titled(&flow.frame(), "Speech to text");
    let ids: Vec<String> = flow.frame().choices.iter().map(|c| c.id.clone()).collect();
    flow.action(&format!("pill:{}", ids[6]));
    flow.action(&format!("pill:{}", ids[0]));
    flow.action(&format!("pill:{}", ids[2]));
    // Asked twice, queued once.
    flow.action(&format!("pill:{}", ids[0]));
    // A press on a queued row takes it back.
    flow.action(&format!("choose:{}", ids[2]));
    // A press on a row not queued asks for nothing.
    flow.action(&format!("choose:{}", ids[3]));
    let frame = flow.frame();
    let queued: Vec<_> = frame
        .choices
        .iter()
        .filter(|c| c.status.is_some())
        .map(|c| c.id.clone())
        .collect();
    assert_eq!(queued, [ids[0].clone(), ids[6].clone()]);
    let effects = flow.action("continue");
    let [Effect::Enter(Some(choices))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(choices["speechModels"], json!([ids[6], ids[0]]));
}
/// Aero runs on the Powermove account: its Connect on the agent step goes
/// back to the account step's sign-in, never to a ChatGPT sign-in.
#[test]
fn aeros_connect_on_the_agent_step_is_the_powermove_account_sign_in() {
    let (_temp, mut flow) = on_account();
    flow.action("not_now");
    titled(&flow.frame(), "Choose your agent");
    assert!(
        flow.action("pill:aero").is_empty(),
        "no agent sign-in starts"
    );
    titled(&flow.frame(), "Create your account");
}
