//! The run folder's anchored file access (fix round 2, P1-4) and the
//! boundary's set-up checks: every read, walk and write goes through the
//! folder opened once, one unfollowed component at a time.
use pm_aero_eval::replay::hold::PortHold;
use pm_aero_eval::runner::paths::{Anchor, Kind};
use pm_aero_eval::runner::sandbox::Confinement;
use std::path::Path;

fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let (run, outside) = (t.path().join("run"), t.path().join("outside"));
    for d in [&run, &outside] {
        std::fs::create_dir_all(d.join("exports")).unwrap();
    }
    std::fs::write(run.join("exports/saved.pmv"), "inside").unwrap();
    std::fs::write(outside.join("exports/saved.pmv"), "outside").unwrap();
    (t, run, outside)
}

#[test]
fn reads_stay_on_the_run_folder_opened_at_the_start() {
    let (t, run, outside) = setup();
    let a = Anchor::open(&run).unwrap();
    // The run folder's path is swapped for a link to another folder after it
    // was opened: reads keep reaching the folder that was opened.
    std::fs::rename(&run, t.path().join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside, &run).unwrap();
    assert_eq!(a.read(Path::new("exports/saved.pmv")).unwrap(), b"inside");
    // Opening the swapped path itself is refused (its last component is a link).
    assert!(Anchor::open(&run).unwrap_err().contains("symlink"));
}

#[test]
fn no_component_below_the_run_folder_is_followed() {
    let (_t, run, outside) = setup();
    let a = Anchor::open(&run).unwrap();
    // A folder in the middle of the path swapped for a link.
    std::fs::remove_dir_all(run.join("exports")).unwrap();
    std::os::unix::fs::symlink(outside.join("exports"), run.join("exports")).unwrap();
    let e = a.read(Path::new("exports/saved.pmv")).unwrap_err();
    assert!(e.contains("symlink"), "{e}");
    // A file that is a link.
    std::fs::create_dir(run.join("work")).unwrap();
    std::os::unix::fs::symlink(outside.join("exports/saved.pmv"), run.join("work/p.pmv")).unwrap();
    assert!(
        a.read(Path::new("work/p.pmv"))
            .unwrap_err()
            .contains("symlink")
    );
    // Walks refuse links anywhere and list them as links, unread.
    assert!(
        a.dir(Path::new("work"))
            .unwrap()
            .files()
            .unwrap_err()
            .contains("symlink")
    );
    let kinds = a.entries().unwrap();
    assert!(
        kinds.contains(&("exports".into(), Kind::Symlink)),
        "{kinds:?}"
    );
    // No `..` and no absolute paths.
    assert!(a.read(Path::new("../outside/exports/saved.pmv")).is_err());
    assert!(a.read(&outside.join("exports/saved.pmv")).is_err());
}

#[test]
fn writes_never_go_through_a_link_a_run_left_behind() {
    let (_t, run, outside) = setup();
    let a = Anchor::open(&run).unwrap();
    std::fs::create_dir(run.join("frames")).unwrap();
    let target = outside.join("victim.txt");
    std::fs::write(&target, "untouched").unwrap();
    // A link where the runner will write a frame, and a linked folder.
    std::os::unix::fs::symlink(&target, run.join("frames/f.png")).unwrap();
    assert!(a.write_new(Path::new("frames/f.png"), b"x").is_err());
    std::fs::remove_file(run.join("frames/f.png")).unwrap();
    std::fs::remove_dir(run.join("frames")).unwrap();
    std::os::unix::fs::symlink(&outside, run.join("frames")).unwrap();
    assert!(a.write_new(Path::new("frames/victim.txt"), b"x").is_err());
    // A rewrite replaces a link at the name, never its target.
    std::os::unix::fs::symlink(&target, run.join("run.json")).unwrap();
    a.replace(Path::new("run.json"), b"{}").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    assert_eq!(std::fs::read(run.join("run.json")).unwrap(), b"{}");
}

#[test]
fn a_tree_hash_reads_through_the_anchor_and_matches_the_fixture_pins() {
    let data = pm_aero_eval::data_root().join("fixtures/extensions/shot-notes");
    let h = Anchor::open(&data).unwrap().tree_hash().unwrap();
    assert_eq!(h, pm_aero_eval::suite::tree_hash(&data).unwrap());
}

#[test]
fn a_profile_that_cannot_be_applied_refuses_the_run() {
    let t = tempfile::tempdir().unwrap();
    let c = Confinement::for_run(t.path(), &[], &[]).unwrap();
    c.verify().unwrap();
    // A profile the kernel cannot parse: refused, never run unconfined.
    let mut broken = c.clone();
    broken.root = Path::new("/nonexistent\" (allow default)").to_path_buf();
    assert!(broken.verify().unwrap_err().contains("refusing"));
    // The replay's port is the only outbound rule.
    let p = c.clone().allow_tcp(4242).profile();
    assert!(p.contains("(remote tcp \"localhost:4242\")"), "{p}");
    assert!(!c.profile().contains("network-outbound"), "{}", c.profile());
}

#[test]
fn the_port_hold_proves_itself_on_every_interface() {
    let h = PortHold::take().unwrap();
    h.prove().unwrap();
    assert!(h.broken().is_none());
    // Wildcards and every non-loopback address are held besides loopback.
    let held = h.held();
    assert!(held.iter().any(|a| a.ip().is_unspecified() && a.is_ipv4()));
    assert!(held.iter().any(|a| a.ip().is_unspecified() && a.is_ipv6()));
    for a in pm_aero_eval::replay::hold::local_addresses() {
        if a.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {
            // Every usable address is held (unusable ones refuse everyone).
            let mut a = a;
            a.set_port(h.port);
            if std::net::TcpListener::bind(a).is_ok() {
                panic!("{a} was not held");
            }
        }
    }
    // A connection that reaches a holder is dropped, never served.
    if let Some(a) = held
        .iter()
        .find(|a| a.is_ipv4() && !a.ip().is_unspecified())
    {
        use std::io::Read;
        let mut s =
            std::net::TcpStream::connect_timeout(a, std::time::Duration::from_secs(2)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut b = [0u8; 1];
        assert_eq!(s.read(&mut b).unwrap_or(0), 0, "a holder answered");
    }
}
