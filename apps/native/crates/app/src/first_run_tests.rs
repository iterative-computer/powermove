//! Every first-launch path on synthetic profiles, through the real coordinator.
use super::*;
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
fn titled(frame: &Frame, title: &str) {
    assert_eq!(frame.title, title, "{frame:?}");
}

#[test]
fn the_default_profile_is_the_one_the_coordinator_writes() {
    let (_temp, home) = home();
    let plan = plan(&home);
    let support = home.join(APPLICATION_SUPPORT);
    assert_eq!(plan.profile, support.join(pm_first_launch::BETA_BUNDLE_ID));
    let config = plan.config.unwrap();
    assert_eq!(config.native_profile, plan.profile);
    assert_eq!(config.electron_profile, support.join("Powermove"));
}
#[test]
fn an_override_never_reaches_the_real_profiles() {
    let (_temp, home) = home();
    let isolated = home.join("isolated");
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
    assert_eq!(plan.electron(), None);
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
    titled(&welcome, "Welcome to Powermove.");
    // Begin is enabled from the first frame, and skips the animation.
    assert_eq!(welcome.default_action(), Some("begin"));
    flow.action("begin");
    titled(&flow.frame(), "Pick Your Timeline.");
    flow.action("choose:tracks");
    assert_eq!(flow.frame().chosen.as_deref(), Some("tracks"));
    flow.action("continue");
    titled(&flow.frame(), "Pick Your Agent.");
    flow.action("back");
    titled(&flow.frame(), "Pick Your Timeline.");
    flow.action("continue");
    flow.action("choose:claude");
    flow.action("continue");
    titled(&flow.frame(), "Pick Your Speech Model.");
    flow.action("continue");
    titled(&flow.frame(), "Grab Your Suitcase.");
    let effects = flow.action("fresh");
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
fn bringing_a_workspace_records_the_opt_in_and_back_reaches_welcome() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    start(&mut flow);
    flow.action("begin");
    flow.action("back");
    titled(&flow.frame(), "Welcome to Powermove.");
    for action in ["begin", "continue", "continue", "continue"] {
        flow.action(action);
    }
    let effects = flow.action("import");
    let [Effect::Enter(Some(choices))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(choices["workspaceImport"], "after-effects");
}
#[test]
fn an_existing_user_just_opens_with_a_clean_library_and_no_click() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let plan = plan(&home);
    assert_eq!(plan.launch, Launch::Migrate);
    let profile = plan.profile.clone();
    let source_files = home.join(APPLICATION_SUPPORT).join("Powermove");
    let before = fs::read(source_files.join("store/project.p.json")).unwrap();
    // The source finished onboarding: its marker comes across with the library.
    write(
        &source_files.join("onboarding-v1.json"),
        &json!({"version":1}),
    );
    let mut flow = Flow::new(plan);
    assert_eq!(start(&mut flow), vec![Effect::Enter(None)]);
    assert_eq!(
        fs::read(source_files.join("store/project.p.json")).unwrap(),
        before
    );
    let copies = pm_kernel::LibraryStore::open(&profile).projects().unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].name, "Synthetic project");
}
#[test]
fn a_migrated_user_who_never_finished_onboarding_sees_it_after_the_copy() {
    let (_temp, home) = home();
    source(&home.join(APPLICATION_SUPPORT));
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    titled(&flow.frame(), "Welcome to Powermove.");
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
    // A slot the index lists but the store can't load.
    write(
        &support.join("Powermove/store/projects.json"),
        &json!([{"id":"p","name":"Synthetic project"},{"id":"gone","name":"Unreadable"}]),
    );
    let mut flow = Flow::new(plan(&home));
    assert!(start(&mut flow).is_empty());
    let frame = flow.frame();
    titled(&frame, "Your Library Is Ready");
    assert_eq!(frame.rows.len(), 1);
    assert_eq!(frame.rows[0].severity, Severity::Warning);
    assert!(frame.rows[0].text.starts_with("1 project couldn’t be read"));
    assert_eq!(flow.action("show_report"), vec![Effect::ShowReport]);
    assert_eq!(flow.action("open_native"), vec![Effect::Enter(None)]);
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
    titled(&consent, "Keep Your Sign-Ins");
    assert_eq!(consent.surface, "N002");
    assert_eq!(consent.default_action(), Some("resume_with_secret_consent"));
    assert!(consent.buttons.iter().any(|b| b.label == "Not Now"));
    let effects = flow.action("skip_secret_consent");
    assert_eq!(effects, vec![Effect::Run(Run::SkipConsent)]);
    assert!(drive(&mut flow, effects).is_empty());
    let summary = flow.frame();
    titled(&summary, "Your Library Is Ready");
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
    assert!(flow.message(Message::Finished(Finished::Denied)).is_empty());
    let denied = flow.frame();
    titled(&denied, "Keychain Access Wasn’t Allowed");
    assert_eq!(denied.default_action(), Some("skip_secret_consent"));
    assert_eq!(
        flow.action("resume_with_secret_consent"),
        vec![Effect::Run(Run::WithConsent)]
    );
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
    titled(&flow.frame(), "Quit the Original App to Continue");
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
    titled(&frame, "The Original App Might Still Be Open");
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
    titled(&frame, "Copying Is Paused");
    assert_eq!(frame.default_action(), Some("resume"));
    let effects = flow.action("resume");
    assert_eq!(effects, vec![Effect::Run(Run::Ordinary)]);
    assert_eq!(
        flow.frame().status.as_deref(),
        Some("Picking up where it stopped…")
    );
    assert!(drive(&mut flow, effects).is_empty());
    titled(&flow.frame(), "Welcome to Powermove.");
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
    titled(&frame, "Not Enough Space");
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
    titled(&frame, "Powermove Can’t Start");
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
#[test]
fn a_finished_launch_opens_without_a_gate_and_brings_later_changes_in() {
    let (_temp, home) = home();
    let support = home.join(APPLICATION_SUPPORT);
    source(&support);
    write(
        &support.join("Powermove/onboarding-v1.json"),
        &json!({"version":1}),
    );
    let mut flow = Flow::new(plan(&home));
    start(&mut flow);
    let later = plan(&home);
    assert_eq!(later.launch, Launch::Later);
    assert!(!later.onboard);
    let mut flow = Flow::new(later.clone());
    assert_eq!(flow.start(), vec![Effect::Enter(None)]);
    assert!(!flow.visible());
    assert!(!later_store(&later, |_| panic!("nothing changed")).unwrap());
    write(&support.join("Powermove/store/theme.json"), &json!("light"));
    assert!(later_store(&later, |_| Ok(())).unwrap());
    let config = later.config.unwrap();
    pm_first_launch::delta_media(&config, |_| Ok(())).unwrap();
    assert_eq!(
        pm_kernel::LibraryStore::open(&config.native_profile)
            .read("theme")
            .unwrap()
            .unwrap()
            .as_str(),
        Some("light")
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
fn a_slow_merge_explains_itself_with_a_way_out() {
    let (_temp, home) = home();
    let mut flow = Flow::new(plan(&home));
    flow.plan.onboard = false;
    flow.plan.launch = Launch::Later;
    flow.start();
    assert!(!flow.visible());
    flow.updating();
    assert!(flow.visible());
    let frame = flow.frame();
    assert!(frame.spinner);
    assert_eq!(frame.buttons[0].id, "quit");
    assert_eq!(frame.default_action(), None);
    assert_eq!(flow.action("quit"), vec![Effect::Quit]);
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
    titled(&flow.frame(), "Your Library Is Ready");
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
    titled(&frame, "Your Library Is Ready");
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
    flow.action("choose:claude");
    flow.action("continue");
    flow.action("continue");
    let effects = flow.action("fresh");
    let [Effect::Enter(Some(chosen))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert!(flow.enter_failed("locked".into()).is_empty());
    assert!(flow.visible());
    let frame = flow.frame();
    titled(&frame, "Your Library Couldn’t Open");
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
    titled(&frame, "Copying Your Library");
    assert_eq!(frame.status.as_deref(), Some("Pausing…"));
    // The worker's next event must not read as running again.
    flow.message(Message::Event(Event::Started(pm_first_launch::Step::Media)));
    assert_eq!(flow.model.gate, Gate::Pausing);
    assert!(flow.action("resume").is_empty());
    flow.message(Message::Finished(Finished::Paused));
    titled(&flow.frame(), "Copying Is Paused");
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
    titled(&frame, "Copying Your Library");
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
    titled(&frame, "Allow Access to Your Media");
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
    titled(&flow.frame(), "Copying Your Library");
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
