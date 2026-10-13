//! Cross-process tests of the rendezvous: this test process plays main, the
//! `pm-bridge-peer` binary plays the engine.
#![cfg(target_os = "macos")]

use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pm_bridge::audit::{AuditToken, ExpectedPeer, SignaturePolicy};
use pm_bridge::host::{self, HostEvent, SurfaceTable};
use pm_bridge::rendezvous::{BridgeError, Rendezvous, SERVICE_PREFIX, random_service_name};
use pm_bridge::wire::PixelFormat;

fn name() -> String {
    random_service_name(SERVICE_PREFIX).unwrap()
}

fn spawn(mode: &str, name: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_pm-bridge-peer"))
        .arg(mode)
        .env("POWERMOVE_FRAME_BRIDGE", name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
}

fn output(mut child: Child) -> String {
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "peer failed: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn expected(child: &Child) -> ExpectedPeer {
    ExpectedPeer::for_child(child.id() as i32, SignaturePolicy::UnsignedDevBypass).unwrap()
}

/// Runs main's receive loop for one launch and collects its events.
fn run_host(
    server: Rendezvous,
    expected: ExpectedPeer,
) -> (Vec<HostEvent>, Arc<Mutex<SurfaceTable>>) {
    let table = Arc::new(Mutex::new(SurfaceTable::default()));
    let stop = AtomicBool::new(false);
    let mut events = Vec::new();
    host::run(
        server,
        &expected,
        Duration::from_secs(5),
        &table,
        &stop,
        &mut |e| events.push(e),
    );
    (events, table)
}

#[test]
fn a_session_hands_over_surfaces_and_ends_with_the_engine() {
    let name = name();
    let server = Rendezvous::register(&name).unwrap();
    let mut child = spawn("session", &name);
    let stdin = child.stdin.take();
    let exp = expected(&child);
    let pid = child.id() as i32;
    // The peer closes our end after its goodbye only once its stdin closes;
    // run the host loop on a thread and close stdin once the goodbye arrived.
    let handle = std::thread::spawn(move || run_host(server, exp));
    std::thread::sleep(Duration::from_millis(800));
    drop(stdin);
    let out = output(child);
    assert_eq!(out.trim(), format!("ok bridge-pid={}", std::process::id()));
    let (events, table) = handle.join().unwrap();

    assert!(
        matches!(events[0], HostEvent::Accepted { pid: p, .. } if p == pid),
        "{events:?}"
    );
    let surfaces: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Surface {
                info,
                io_surface,
                alloc_bytes,
            } => Some((*info, *io_surface, *alloc_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(surfaces.len(), 2);
    for (index, (info, pointer, bytes)) in surfaces.iter().enumerate() {
        assert_eq!(
            (info.client_port, info.generation, info.pool_index),
            (7, 1, index as u32)
        );
        assert_eq!(
            (info.width, info.height, info.format),
            (64, 32, PixelFormat::Rgba16F)
        );
        assert_ne!(*pointer, 0);
        assert!(*bytes >= 64 * 32 * 8);
    }
    let rest: Vec<_> = events.iter().skip(3).cloned().collect();
    assert_eq!(
        rest,
        vec![
            HostEvent::Retire {
                client_port: 7,
                generation: 1
            },
            HostEvent::Goodbye,
            HostEvent::Gone
        ]
    );
    // Main still holds both surfaces after the engine exited, until Chromium
    // releases its imports.
    let mut table = table.lock().unwrap();
    assert_eq!(table.len(), 2);
    assert!(table.release(1, 0));
    assert!(!table.release(1, 0));
    assert!(table.release(1, 1));
    assert!(table.is_empty());
}

#[test]
fn a_surface_that_does_not_match_its_message_is_refused() {
    let name = name();
    let server = Rendezvous::register(&name).unwrap();
    let mut child = spawn("lying-surface", &name);
    let stdin = child.stdin.take();
    let exp = expected(&child);
    let handle = std::thread::spawn(move || run_host(server, exp));
    std::thread::sleep(Duration::from_millis(800));
    drop(stdin);
    output(child);
    let (events, table) = handle.join().unwrap();
    assert!(
        events.iter().any(
            |e| matches!(e, HostEvent::Rejected { reason, .. } if reason.contains("is 64x32"))
        ),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, HostEvent::Surface { .. }))
    );
    assert!(table.lock().unwrap().is_empty());
}

#[test]
fn the_name_is_deregistered_after_one_check_in() {
    let name = name();
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("twice", &name);
    let exp = expected(&child);
    let (_conn, _) = server
        .accept(&exp, Duration::from_secs(5))
        .map_err(|(e, _)| e)
        .unwrap();
    let out = output(child);
    assert!(
        out.starts_with("second check-in refused: bootstrap bootstrap_look_up"),
        "{out}"
    );
}

fn refused(mutate: impl FnOnce(&mut ExpectedPeer), want: &str) {
    let name = name();
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("checkin-expect-refusal", &name);
    let mut exp = expected(&child);
    mutate(&mut exp);
    let (error, rejections) = match server.accept(&exp, Duration::from_millis(1500)) {
        Ok(_) => panic!("accepted a check-in that should fail ({want})"),
        Err(e) => e,
    };
    assert!(matches!(error, BridgeError::TimedOut), "{error}");
    assert_eq!(rejections.len(), 1, "{rejections:?}");
    assert!(rejections[0].reason.contains(want), "{rejections:?}");
    assert!(output(child).starts_with("refused"));
}

#[test]
fn a_check_in_from_another_pid_is_refused() {
    refused(|e| e.pid += 1, "is not the engine pid");
}

#[test]
fn a_stale_pidversion_is_refused() {
    refused(|e| e.pidversion = e.pidversion.map(|v| v + 1), "pidversion");
}

#[test]
fn a_sender_that_is_not_mains_child_is_refused() {
    refused(|e| e.parent_pid = 1, "is not main");
}

#[test]
fn another_users_process_is_refused() {
    refused(|e| e.uid = e.uid.wrapping_add(1), "uid");
}

#[test]
fn an_unsigned_engine_fails_a_team_id_policy() {
    refused(
        |e| e.signature = SignaturePolicy::TeamId("7FRXF46ZSN".into()),
        "does not satisfy Team ID",
    );
}

#[test]
fn a_crash_after_the_check_in_reports_the_engine_gone() {
    let name = name();
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("die-after-checkin", &name);
    let exp = expected(&child);
    let handle = std::thread::spawn(move || run_host(server, exp));
    output(child);
    let (events, _) = handle.join().unwrap();
    assert!(
        matches!(events.first(), Some(HostEvent::Accepted { .. })),
        "{events:?}"
    );
    assert_eq!(events.last(), Some(&HostEvent::Gone));
}

#[test]
fn nobody_checking_in_ends_the_loop_with_an_error() {
    let name = name();
    let server = Rendezvous::register(&name).unwrap();
    let me = ExpectedPeer::for_child(
        std::process::id() as i32,
        SignaturePolicy::UnsignedDevBypass,
    );
    // Our own pid is not our child, but for_child only records; nobody checks in.
    let exp = me.unwrap();
    let table = Arc::new(Mutex::new(SurfaceTable::default()));
    let stop = AtomicBool::new(false);
    let mut events = Vec::new();
    host::run(
        server,
        &exp,
        Duration::from_millis(200),
        &table,
        &stop,
        &mut |e| events.push(e),
    );
    assert!(
        matches!(&events[..], [HostEvent::Error { reason }] if reason.contains("timed out")),
        "{events:?}"
    );
}

#[test]
fn the_audit_token_of_a_child_matches_its_spawn() {
    let name = name();
    let _server = Rendezvous::register(&name).unwrap();
    let mut child = spawn("checkin-expect-refusal", &name);
    let token = AuditToken::of_pid(child.id() as i32).unwrap();
    assert_eq!(token.pid(), child.id() as i32);
    child.kill().ok();
    child.wait().ok();
}
