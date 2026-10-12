//! Cross-process tests of the rendezvous: this test process plays main, the
//! `s1-mach-peer` binary plays the engine.
use s1_mach::audit::{AuditToken, ExpectedPeer, SignaturePolicy};
use s1_mach::rendezvous::{Event, Rendezvous, random_service_name};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn spawn(mode: &str, name: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_s1-mach-peer"))
        .arg(mode)
        .env("PM_S1_BRIDGE_NAME", name)
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

#[test]
fn full_session_surfaces_frames_goodbye() {
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let mut child = spawn("session", &name);
    let stdin = child.stdin.take();
    let exp = expected(&child);
    let (mut conn, rejections) = server.accept(&exp, Duration::from_secs(5)).map_err(|(e, _)| e).unwrap();
    assert!(rejections.is_empty(), "{rejections:?}");
    assert_eq!(conn.peer().pid(), child.id() as i32);

    let mut frames = 0u64;
    let mut surfaces = 0;
    loop {
        match conn.recv(Duration::from_secs(3)).unwrap() {
            Some(Event::Surface { info, .. }) => {
                assert_eq!((info.pool_index, info.width, info.height), (2, 3840, 2160));
                surfaces += 1;
            }
            Some(Event::Frame(d)) => {
                assert_eq!(d.seq, frames);
                frames += 1;
            }
            Some(Event::Goodbye) => break,
            Some(Event::PeerGone) => panic!("peer gone before goodbye"),
            None => panic!("timed out"),
        }
    }
    assert_eq!((surfaces, frames), (1, 100));
    drop(stdin);
    let out = output(child);
    assert_eq!(out.trim(), format!("ok bridge-pid={}", std::process::id()));
    // The engine exited: its send right is gone, so no-more-senders arrives.
    match conn.recv(Duration::from_secs(3)).unwrap() {
        Some(Event::PeerGone) => {}
        other => panic!("expected PeerGone, got {other:?}"),
    }
}

#[test]
fn name_is_deregistered_after_one_check_in() {
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("twice", &name);
    let exp = expected(&child);
    let (_conn, _) = server.accept(&exp, Duration::from_secs(5)).map_err(|(e, _)| e).unwrap();
    let out = output(child);
    assert!(out.starts_with("second check-in refused: bootstrap bootstrap_look_up"), "{out}");
}

#[test]
fn wrong_pid_is_rejected() {
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("checkin-expect-refusal", &name);
    let mut exp = expected(&child);
    exp.pid += 1;
    let (err, rejections) = match server.accept(&exp, Duration::from_millis(1500)) {
        Ok(_) => panic!("accepted a check-in from the wrong pid"),
        Err(e) => e,
    };
    assert!(matches!(err, s1_mach::rendezvous::BridgeError::TimedOut));
    assert_eq!(rejections.len(), 1);
    assert!(rejections[0].reason.contains("is not the engine pid"), "{rejections:?}");
    assert!(output(child).starts_with("refused"));
}

#[test]
fn stale_pidversion_is_rejected() {
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("checkin-expect-refusal", &name);
    let mut exp = expected(&child);
    exp.pidversion = exp.pidversion.map(|v| v + 1);
    let (_, rejections) = match server.accept(&exp, Duration::from_millis(1500)) {
        Ok(_) => panic!("accepted a stale pidversion"),
        Err(e) => e,
    };
    assert!(rejections[0].reason.contains("pidversion"), "{rejections:?}");
    assert!(output(child).starts_with("refused"));
}

#[test]
fn unsigned_engine_fails_team_id_policy() {
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("checkin-expect-refusal", &name);
    let mut exp = expected(&child);
    exp.signature = SignaturePolicy::TeamId("7FRXF46ZSN".into());
    let (_, rejections) = match server.accept(&exp, Duration::from_millis(1500)) {
        Ok(_) => panic!("accepted an unsigned engine under a Team ID policy"),
        Err(e) => e,
    };
    assert!(rejections[0].reason.contains("does not satisfy Team ID"), "{rejections:?}");
    assert!(output(child).starts_with("refused"));
}

#[test]
fn non_child_is_rejected() {
    // Expect a peer whose parent is some other process: simulate by recording the
    // expectation with a wrong parent pid.
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("checkin-expect-refusal", &name);
    let mut exp = expected(&child);
    exp.parent_pid = 1;
    let (_, rejections) = match server.accept(&exp, Duration::from_millis(1500)) {
        Ok(_) => panic!("accepted a non-child"),
        Err(e) => e,
    };
    assert!(rejections[0].reason.contains("is not main"), "{rejections:?}");
    assert!(output(child).starts_with("refused"));
}

#[test]
fn crash_after_check_in_reports_peer_gone() {
    let name = random_service_name("com.powermove.s1test");
    let server = Rendezvous::register(&name).unwrap();
    let child = spawn("die-after-checkin", &name);
    let exp = expected(&child);
    let (mut conn, _) = server.accept(&exp, Duration::from_secs(5)).map_err(|(e, _)| e).unwrap();
    output(child);
    match conn.recv(Duration::from_secs(3)).unwrap() {
        Some(Event::PeerGone) => {}
        other => panic!("expected PeerGone, got {other:?}"),
    }
}

#[test]
fn audit_token_of_child_matches_spawn() {
    let name = random_service_name("com.powermove.s1test");
    let _server = Rendezvous::register(&name).unwrap();
    let mut child = spawn("checkin-expect-refusal", &name);
    let t = AuditToken::of_pid(child.id() as i32).unwrap();
    assert_eq!(t.pid(), child.id() as i32);
    child.kill().ok();
    child.wait().ok();
}
