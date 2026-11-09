//! The Project Library under real input events, over a real kernel and library.
use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use pm_design::{Accessibility, Scheme};
use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::ShellCall};
use std::{cell::RefCell, time::Instant};

fn value(text: &str) -> pm_model::json::Value {
    pm_model::json::parse(text).unwrap()
}
struct Fixture {
    actor: KernelActor,
    _temp: tempfile::TempDir,
    profile: PathBuf,
}
/// Two library projects, written the way the kernel writes them.
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().canonicalize().unwrap();
    {
        let writer =
            pm_kernel::persistence::LibraryWriter::open(&profile, DiskFormat::V1_2, None).unwrap();
        for (id, name) in [("first", "First Film"), ("second", "Second Film")] {
            let project = format!(
                r##"{{"id":"{id}","name":"{name}","compId":"c","compName":"Main","comps":{{}},"w":1920,"h":1080,"fps":30,"dur":2,"work":[0,2],"bg":"#000000","assets":{{}},"layers":[],"params":{{}},"markers":[]}}"##
            );
            writer
                .save_project(
                    id,
                    value(&project),
                    value(r#"{"version":1,"paths":"composition","index":-1,"entries":[]}"#),
                    value("{}"),
                    value("{}"),
                )
                .unwrap();
        }
    }
    let actor = KernelActor::spawn(KernelConfig::default());
    actor
        .handle()
        .shell(ShellCall::AttachLibrary {
            profile: profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    Fixture {
        actor,
        _temp: temp,
        profile,
    }
}
type Log = Rc<RefCell<Vec<String>>>;
fn open<'a>(
    cx: &'a mut TestAppContext,
    fixture: &Fixture,
) -> (Entity<ProjectLibrary>, &'a mut VisualTestContext, Log) {
    let log: Log = Rc::default();
    let sink: Sink = {
        let log = log.clone();
        Rc::new(move |outbound, _| {
            log.borrow_mut().push(match outbound {
                Outbound::Opened { name, .. } => format!("opened {name}"),
                Outbound::Focus => "focus".into(),
                Outbound::FocusProject(id) => format!("focus {id}"),
                Outbound::Message(text) => text,
            })
        })
    };
    let (port, rx) = ProjectLibrary::port(fixture.actor.handle(), fixture.profile.clone()).unwrap();
    let tokens = Tokens::new(Scheme::Light, Accessibility::default());
    let (view, cx) =
        cx.add_window_view(move |_, cx| ProjectLibrary::new(port, rx, tokens, sink, cx));
    cx.update(|window, cx| {
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
    });
    view.update(cx, |v, cx| {
        let mut query = v.controller.query.clone();
        query.section = Section::All;
        v.controller.set_query(query).unwrap();
        v.pump(cx);
    });
    settle(&view, cx, |v| !v.controller.rows.is_empty());
    (view, cx, log)
}
/// Waits on the worker for real; the test scheduler can't see its thread.
fn settle(
    view: &Entity<ProjectLibrary>,
    cx: &mut VisualTestContext,
    done: impl Fn(&ProjectLibrary) -> bool,
) {
    let start = Instant::now();
    loop {
        view.update(cx, |v, cx| v.pump(cx));
        cx.run_until_parked();
        if view.read_with(cx, |v, _| done(v)) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "library never settled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn names(view: &Entity<ProjectLibrary>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.controller.rows.iter().map(|r| r.name.clone()).collect()
    })
}

#[::core::prelude::v1::test]
fn lists_the_library_and_filters_as_you_type() {
    let fixture = fixture();
    let mut cx = TestAppContext::single();
    let (view, cx, _) = open(&mut cx, &fixture);
    assert_eq!(names(&view, cx).len(), 2);
    cx.simulate_input("sec");
    settle(&view, cx, |v| v.controller.rows.len() == 1);
    assert_eq!(names(&view, cx), ["Second Film"]);
    cx.simulate_keystrokes("escape");
    settle(&view, cx, |v| v.controller.rows.len() == 2);
}
#[::core::prelude::v1::test]
fn a_click_selects_and_return_opens_through_the_kernel() {
    let fixture = fixture();
    let mut cx = TestAppContext::single();
    let (view, cx, log) = open(&mut cx, &fixture);
    let row = view.read_with(cx, |v, _| {
        v.controller
            .rows
            .iter()
            .position(|r| r.id == "first")
            .unwrap()
    });
    assert!(row < 2);
    let bounds = cx.debug_bounds("project-first").expect("row on screen");
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.controller.selected.clone())
            .as_deref(),
        Some("first")
    );
    cx.simulate_keystrokes("enter");
    let start = Instant::now();
    while log.borrow().is_empty() {
        view.update(cx, |v, cx| v.pump(cx));
        cx.run_until_parked();
        assert!(start.elapsed() < Duration::from_secs(10), "never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*log.borrow(), ["opened First Film"]);
    assert!(fixture.actor.handle().published("first").is_some());
    // Opening it again, from this or another window's list, brings forward the
    // window that shows it: no second open, no alert.
    cx.simulate_keystrokes("enter");
    let start = Instant::now();
    while log.borrow().len() < 2 {
        view.update(cx, |v, cx| v.pump(cx));
        cx.run_until_parked();
        assert!(start.elapsed() < Duration::from_secs(10), "never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(log.borrow()[1], "focus first");
}
#[::core::prelude::v1::test]
fn sections_switch_and_an_empty_trash_says_so() {
    let fixture = fixture();
    let mut cx = TestAppContext::single();
    let (view, cx, _) = open(&mut cx, &fixture);
    let bounds = cx
        .debug_bounds("section-Recently Deleted")
        .expect("section on screen");
    cx.simulate_click(bounds.center(), Modifiers::default());
    settle(&view, cx, |v| v.controller.rows.is_empty());
    assert!(view.read_with(cx, |v, _| matches!(
        v.presentation.as_ref().map(|p| &p.content),
        Some(Content::Empty(_))
    )));
}

#[::core::prelude::v1::test]
fn a_window_without_a_project_uses_the_shipped_defaults() {
    use crate::library_surface::NewProjectForm;
    let form = NewProjectForm::new(
        String::new(),
        current_format(None).as_ref(),
        &default_format(),
        &[],
    );
    assert_eq!(
        (form.width.as_str(), form.height.as_str(), form.fps.as_str()),
        ("1920", "1080", "30")
    );
    assert_eq!(form.background, "#09090A");
    assert_eq!(form.preset, Some(1));
}

#[::core::prelude::v1::test]
fn create_opens_a_library_project_with_the_forms_format_and_saves_it() {
    use crate::library_surface::{Command, NewProjectForm, Outcome, kernel::KernelLibrary};
    use pm_kernel::session::ShellReply;
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().canonicalize().unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(profile.join("journals")),
        ..Default::default()
    });
    actor
        .handle()
        .shell(ShellCall::AttachLibrary {
            profile: profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    let current = Format {
        width: 1080,
        height: 1920,
        fps: 24.,
        duration: 5.,
        background: "#112233".into(),
    };
    let mut form = NewProjectForm::new(
        String::new(),
        Some(&current),
        &default_format(),
        &["Untitled".into()],
    );
    assert_eq!(form.name, "Untitled 2", "names stay unique");
    form.choose_preset(Some(1)).unwrap();
    let mut service = KernelLibrary {
        kernel: actor.handle(),
        profile: profile.clone(),
    };
    let Outcome::Opened {
        reply: ShellReply::Opened { session, .. },
        ..
    } = service
        .execute(Command::Create(form.values().unwrap()))
        .unwrap()
    else {
        panic!("a new project opens without recovery");
    };
    let published = actor.handle().published(&session.project).unwrap();
    assert_eq!(
        current_format(Some(&published)),
        Some(Format {
            width: 1920,
            height: 1080,
            fps: 24.,
            duration: 5.,
            background: "#112233".into(),
        }),
        "the next New Project starts from this one"
    );
    let root = published.snapshot.root().unwrap();
    assert_eq!(
        root.get("name").and_then(crate::node_string),
        Some("Untitled 2")
    );
    actor
        .handle()
        .shell(ShellCall::Save(session.clone()))
        .wait()
        .unwrap()
        .unwrap();
    actor.shutdown();
    assert!(
        pm_kernel::LibraryStore::open(&profile)
            .load(&session.project)
            .unwrap()
            .is_some(),
        "the new project is in the library"
    );
}
