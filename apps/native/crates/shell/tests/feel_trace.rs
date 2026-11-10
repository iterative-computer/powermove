//! P-FEEL (`gpui_apple::feel`): the probe's trace file is opened and written
//! off the asking thread, so a slow or unread file never stalls input.
#![cfg(target_os = "macos")]
use gpui_apple::feel;
use std::{io::BufRead, sync::mpsc, time::Duration};

#[test]
fn a_trace_fifo_without_a_reader_never_blocks_the_probe() {
    let dir = std::env::temp_dir().join(format!("pm-feel-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("trace");
    let _ = std::fs::remove_file(&fifo);
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    // SAFETY: this test binary's only test; nothing else reads the environment.
    unsafe { std::env::set_var("PM_FEEL_TRACE", &fifo) };
    // Opening a FIFO to write waits for a reader: the probe must not.
    let (done, asked) = mpsc::channel();
    std::thread::spawn(move || {
        let on = feel::enabled();
        feel::note_now("down");
        feel::write(r#"{"ui":"probe"}"#);
        done.send(on).unwrap();
    });
    let on = asked
        .recv_timeout(Duration::from_secs(5))
        .expect("the probe blocked on its trace file");
    assert!(on);
    // A reader arriving later still receives the line.
    let (sent, read) = mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        let file = std::fs::File::open(path).unwrap();
        std::io::BufReader::new(file).read_line(&mut line).unwrap();
        sent.send(line).unwrap();
    });
    let line = read
        .recv_timeout(Duration::from_secs(5))
        .expect("the trace line never arrived");
    assert_eq!(line, "{\"ui\":\"probe\"}\n");
    let _ = std::fs::remove_dir_all(dir);
}
