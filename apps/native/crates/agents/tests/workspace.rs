mod support;
use pm_agents::{
    Access, ApprovalMode, Provider,
    approvals::Authority,
    workspace::{self, Host, Workspace},
};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
};
use support::Temp;
#[derive(Default, Clone)]
struct Fake {
    calls: Arc<Mutex<Vec<String>>>,
    jobs: usize,
}
impl Host for Fake {
    fn run_command(
        &mut self,
        _: &Path,
        a: &Authority,
        c: &str,
        _: u64,
        background: bool,
    ) -> io::Result<Value> {
        assert_eq!(a.access, Access::Project);
        self.calls.lock().unwrap().push(c.into());
        if background {
            self.jobs += 1;
            Ok(json!({"jobId":format!("job-{}",self.jobs),"state":"running"}))
        } else {
            Ok(json!({"output":"synthetic","exitCode":0,"truncated":false}))
        }
    }
    fn command_status(&mut self, id: &str, _: u64, stop: bool) -> io::Result<Value> {
        Ok(
            json!({"jobId":id,"state":if stop{"stopped"}else{"exited"},"exitCode":0,"output":"synthetic","truncated":false}),
        )
    }
    fn stop_all(&mut self) -> io::Result<()> {
        self.calls.lock().unwrap().push("stop-all".into());
        Ok(())
    }
    fn compile(&mut self, id: &str) -> io::Result<Value> {
        self.calls.lock().unwrap().push(format!("compile-{id}"));
        Ok(json!({"ok":true}))
    }
    fn publish(&mut self, v: &Value) -> io::Result<Value> {
        self.calls.lock().unwrap().push("publish".into());
        Ok(v.clone())
    }
}
fn ws(t: &Temp, host: Fake, app: bool) -> Workspace<Fake> {
    Workspace::new(
        &t.0.join("workspace"),
        &t.0.join("state"),
        Authority::new(Access::Project, ApprovalMode::Auto, false).unwrap(),
        host,
        app,
    )
    .unwrap()
}
#[test]
fn file_tools_page_utf16_and_archive_replaced_bytes() {
    let t = Temp::new();
    let mut w = ws(&t, Fake::default(), false);
    w.call(
        "write_file",
        &json!({"path":"folder/file.txt","text":"a🎞b"}),
    )
    .unwrap();
    let page = w
        .call(
            "read_file",
            &json!({"path":"folder/file.txt","offset":1,"limit":2}),
        )
        .unwrap();
    assert_eq!(page["text"], "🎞");
    assert_eq!(page["totalChars"], 4);
    w.call(
        "write_file",
        &json!({"path":"folder/file.txt","text":"replacement"}),
    )
    .unwrap();
    let archives = fs::read_dir(t.0.join("state"))
        .unwrap()
        .filter(|v| {
            v.as_ref()
                .is_ok_and(|e| e.path().extension().is_none_or(|x| x != "json"))
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(archives.len(), 1);
    assert_eq!(fs::read_to_string(archives[0].path()).unwrap(), "a🎞b");
    let list = w.call("list_files", &json!({"path":"folder"})).unwrap();
    assert_eq!(list["entries"][0]["name"], "file.txt");
}
#[test]
fn file_tools_reject_traversal_absolute_escape_and_symlinks() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let mut w = ws(&t, Fake::default(), false);
    fs::write(t.0.join("outside"), "authored").unwrap();
    symlink(t.0.join("outside"), t.0.join("workspace/link")).unwrap();
    for path in ["../outside", "link", t.0.join("outside").to_str().unwrap()] {
        assert!(w.call("read_file", &json!({"path":path})).is_err());
        assert!(
            w.call("write_file", &json!({"path":path,"text":"changed"}))
                .is_err()
        );
    }
    assert_eq!(fs::read_to_string(t.0.join("outside")).unwrap(), "authored");
}
#[test]
fn dangling_directory_symlink_cannot_redirect_write() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let mut w = ws(&t, Fake::default(), false);
    symlink(t.0.join("missing"), t.0.join("workspace/link")).unwrap();
    assert!(
        w.call("write_file", &json!({"path":"link/file","text":"changed"}))
            .is_err()
    );
    assert!(!t.0.join("missing").exists());
}
#[test]
fn main_owned_state_must_live_outside_workspace() {
    let t = Temp::new();
    assert!(
        Workspace::new(
            &t.0.join("root"),
            &t.0.join("root/state"),
            Authority::new(Access::Project, ApprovalMode::Auto, false).unwrap(),
            Fake::default(),
            false
        )
        .is_err()
    );
}
#[test]
fn background_jobs_are_bounded_and_stoppable_by_id() {
    let t = Temp::new();
    let mut w = ws(&t, Fake::default(), false);
    for _ in 0..4 {
        w.call(
            "run_command",
            &json!({"command":"synthetic command","background":true}),
        )
        .unwrap();
    }
    assert!(
        w.call("run_command", &json!({"command":"fifth","background":true}))
            .is_err()
    );
    assert!(
        w.call("command_status", &json!({"jobId":"foreign"}))
            .is_err()
    );
    assert_eq!(
        w.call("command_status", &json!({"jobId":"job-1","stop":true}))
            .unwrap()["state"],
        "stopped"
    );
    assert!(
        w.call(
            "run_command",
            &json!({"command":"replacement job","background":true})
        )
        .is_ok()
    );
}
#[test]
fn complete_task_stops_children_before_publication() {
    let t = Temp::new();
    let host = Fake::default();
    let calls = host.calls.clone();
    let mut w = ws(&t, host, false);
    let result = json!({"summary":"done","commands":[],"extensions":[],"artifacts":[],"notes":[],"externalActions":[]});
    w.call("complete_task", &result).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["stop-all", "publish"]);
}
#[test]
fn app_workspace_cannot_publish_timeline_commands() {
    let t = Temp::new();
    let host = Fake::default();
    let calls = host.calls.clone();
    let mut w = ws(&t, host, true);
    let result = json!({"summary":"done","commands":["mutate"],"extensions":[],"artifacts":[],"notes":[],"externalActions":[]});
    assert!(w.call("complete_task", &result).is_err());
    assert_eq!(*calls.lock().unwrap(), vec!["stop-all"]);
}
#[test]
fn invalid_complete_stops_jobs_and_does_not_publish() {
    let t = Temp::new();
    let host = Fake::default();
    let calls = host.calls.clone();
    let mut w = ws(&t, host, false);
    assert!(w.call("complete_task", &json!({})).is_err());
    assert_eq!(*calls.lock().unwrap(), vec!["stop-all"]);
}
#[test]
fn session_paths_separate_provider_thread_and_authority() {
    let t = Temp::new();
    let chat =
        workspace::session_path(&t.0, "p", Provider::Chatgpt, Access::Project, Some("t")).unwrap();
    let claude =
        workspace::session_path(&t.0, "p", Provider::Claude, Access::Project, Some("t")).unwrap();
    let computer =
        workspace::session_path(&t.0, "p", Provider::Chatgpt, Access::Computer, Some("t")).unwrap();
    assert!(chat.ends_with("Agent State/p/threads/t/session-v2-project.txt"));
    assert!(claude.ends_with("session-v2-claude-project.txt"));
    assert!(computer.ends_with("session-v2-computer.txt"));
    assert!(
        workspace::session_path(
            &t.0,
            "p",
            Provider::Chatgpt,
            Access::Project,
            Some("../escape")
        )
        .is_err()
    );
}
#[test]
fn resume_handles_archive_replaced_values_and_reject_flags() {
    let t = Temp::new();
    let p = t.0.join("state/session.txt");
    workspace::write_session(&p, "thread-1").unwrap();
    workspace::write_session(&p, "thread-2").unwrap();
    assert_eq!(
        workspace::read_session(&p).unwrap().as_deref(),
        Some("thread-2")
    );
    assert_eq!(fs::read_dir(t.0.join("state")).unwrap().count(), 2);
    assert!(workspace::write_session(&p, "--dangerously").is_err());
    assert_eq!(fs::read_to_string(&p).unwrap(), "thread-2");
}

#[test]
fn computer_file_tools_require_consent_and_allow_explicit_external_paths() {
    let t = Temp::new();
    let root = t.0.join("workspace");
    let archive = t.0.join("state");
    let mut w = Workspace::new(
        &root,
        &archive,
        Authority::new(Access::Computer, ApprovalMode::Auto, true).unwrap(),
        Fake::default(),
        false,
    )
    .unwrap();
    let external = t.0.join("external.txt");
    fs::write(&external, "original").unwrap();
    w.call("write_file", &json!({"path":external,"text":"new"}))
        .unwrap();
    assert_eq!(fs::read_to_string(&external).unwrap(), "new");
    assert_eq!(
        w.call("read_file", &json!({"path":external})).unwrap()["text"],
        "new"
    );
    let backups = fs::read_dir(&archive)
        .unwrap()
        .filter(|v| {
            v.as_ref()
                .is_ok_and(|e| e.path().extension().is_none_or(|x| x != "json"))
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "original");
}
#[test]
fn extension_compile_uses_frozen_slug_validation() {
    let t = Temp::new();
    let mut w = ws(&t, Fake::default(), false);
    for id in ["UPPER", "a", "a_b", "../escape"] {
        assert!(w.call("compile_extension", &json!({"id":id})).is_err());
    }
    assert!(
        w.call("compile_extension", &json!({"id":"ab-good"}))
            .is_ok()
    );
}
