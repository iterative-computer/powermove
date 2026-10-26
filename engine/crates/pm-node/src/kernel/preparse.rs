//! A document stream parsed as it arrives.
//!
//! Main writes a page's document text to a staging file in chunks and loads
//! it once the last one is written (`load`). Parsing used to start only then:
//! for the large private project, 0.37 s of the 0.6 s between the last chunk
//! and the prepared document. Main now names the file when the stream begins
//! (`preparse`) and tells how much of it is written after every chunk
//! (`grow`); this reads the new bytes, finds the parallel parse's units as
//! they close ([`UnitScanner`]) and parses each on a worker. The load takes
//! the units parsed by then ([`Preparse::finish`]) and uses a unit only where
//! the loaded file's bytes there have the digest of the text it was parsed
//! from, so the tree is the one a parse of the file alone builds; anything
//! missing is parsed at the load.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Condvar, Mutex, mpsc},
};

use pm_model::{
    Node,
    json::{Prebuilt, Unit, UnitScanner},
};
use pm_protocol::error::EngineProcess;

/// Bytes read from the staging file at once.
const READ_BYTES: usize = 4 << 20;

pub(super) struct Preparse {
    pub path: String,
    pub revision: u64,
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// Bytes main has written to the file.
    written: u64,
    /// Bytes read and scanned.
    scanned: u64,
    /// Set by [`Preparse::finish`] or a drop: no more bytes will come.
    stopped: bool,
    /// The scan ended: whether the text is well formed (every unit was found).
    complete: Option<bool>,
    /// Units handed to a worker and not back yet.
    parsing: usize,
    done: Vec<Prebuilt<Node>>,
}

impl Preparse {
    /// Starts reading `path` as main writes it; the units are parsed at
    /// `revision` (a load at another revision does not use them).
    pub fn start(process: EngineProcess, path: String, revision: u64) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let (jobs, queue) = mpsc::channel::<(Unit, Vec<u8>)>();
        let queue = Arc::new(Mutex::new(queue));
        let workers = pm_model::parallel::workers().saturating_sub(1).max(1);
        for _ in 0..workers {
            let (queue, shared) = (queue.clone(), shared.clone());
            pm_engine::qos::spawn_engine_thread(process, "pm-native-kernel-preparse", move || {
                loop {
                    let Ok((unit, text)) = queue
                        .lock()
                        .map_err(drop)
                        .and_then(|q| q.recv().map_err(drop))
                    else {
                        return;
                    };
                    let value = Node::parse_unit(&text, unit.depth, revision);
                    let digest = pm_model::hash::sha256(&text);
                    drop(text);
                    let mut state = shared.state.lock().expect("preparse state");
                    if let Some(value) = value {
                        state.done.push(Prebuilt {
                            unit,
                            digest,
                            value,
                        });
                    }
                    state.parsing -= 1;
                    shared.changed.notify_all();
                }
            })
            .map_err(|e| format!("The native document parser could not start ({})", e.kind()))?;
        }
        let scanning = shared.clone();
        let file = path.clone();
        pm_engine::qos::spawn_engine_thread(process, "pm-native-kernel-scan", move || {
            scan(&file, &scanning, &jobs);
            let mut state = scanning.state.lock().expect("preparse state");
            state.complete.get_or_insert(false);
            scanning.changed.notify_all();
        })
        .map_err(|e| format!("The native document scanner could not start ({})", e.kind()))?;
        Ok(Self {
            path,
            revision,
            shared,
        })
    }

    /// Main has written the first `length` bytes.
    pub fn grow(&self, length: u64) {
        let mut state = self.shared.state.lock().expect("preparse state");
        state.written = state.written.max(length);
        self.shared.changed.notify_all();
    }

    /// The units parsed from a file of `length` bytes, in source order, and
    /// whether they are every unit of its text. Waits for the bytes already
    /// announced to be scanned and their units parsed.
    pub fn finish(self, length: u64) -> (Vec<Prebuilt<Node>>, bool) {
        let mut state = self.shared.state.lock().expect("preparse state");
        state.stopped = true;
        self.shared.changed.notify_all();
        while state.complete.is_none() || state.parsing > 0 {
            state = self.shared.changed.wait(state).expect("preparse state");
        }
        let complete = state.complete == Some(true) && state.scanned == length;
        let mut done = std::mem::take(&mut state.done);
        done.sort_by_key(|p| p.unit.start);
        (done, complete)
    }
}

impl Drop for Preparse {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().expect("preparse state");
        state.stopped = true;
        self.shared.changed.notify_all();
    }
}

/// Reads and scans the file as it grows, until it is stopped and everything
/// announced was read; hands each final unit's text to the workers.
fn scan(path: &str, shared: &Shared, jobs: &mpsc::Sender<(Unit, Vec<u8>)>) {
    let mut scanner = UnitScanner::default();
    let (mut kept, mut base) = (Vec::new(), 0usize);
    let mut file: Option<File> = None;
    let mut buffer = vec![0; READ_BYTES];
    let dispatch = |units: Vec<Unit>, kept: &[u8], base: usize| {
        for unit in units {
            let text = kept[unit.start - base..unit.end - base].to_vec();
            shared.state.lock().expect("preparse state").parsing += 1;
            if jobs.send((unit, text)).is_err() {
                shared.state.lock().expect("preparse state").parsing -= 1;
            }
        }
    };
    loop {
        let (from, to) = {
            let mut state = shared.state.lock().expect("preparse state");
            while !state.stopped && state.written == state.scanned {
                state = shared.changed.wait(state).expect("preparse state");
            }
            if state.written == state.scanned {
                break;
            }
            (state.scanned, state.written)
        };
        let opened = match &mut file {
            Some(file) => file,
            None => match File::open(path) {
                Ok(opened) => file.insert(opened),
                Err(_) => return,
            },
        };
        if opened.seek(SeekFrom::Start(from)).is_err() {
            return;
        }
        let mut at = from;
        while at < to {
            let want = ((to - at) as usize).min(READ_BYTES);
            match opened.read(&mut buffer[..want]) {
                Ok(0) | Err(_) => return,
                Ok(count) => {
                    let piece = &buffer[..count];
                    kept.extend_from_slice(piece);
                    dispatch(scanner.feed(piece), &kept, base);
                    let needed = scanner.needed_from();
                    kept.drain(..needed - base);
                    base = needed;
                    at += count as u64;
                }
            }
        }
        shared.state.lock().expect("preparse state").scanned = to;
    }
    let complete = match scanner.finish() {
        Some(rest) => {
            dispatch(rest, &kept, base);
            true
        }
        None => false,
    };
    shared.state.lock().expect("preparse state").complete = Some(complete);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_every_unit_while_the_file_grows() {
        let layer = |i: usize| {
            let keys: Vec<String> = (0..2000 + i)
                .map(|k| format!(r#"{{"t":{k},"v":[{k},"a"]}}"#))
                .collect();
            format!(r#"{{"id":"L{i}","k":[{}]}}"#, keys.join(","))
        };
        let text = format!(
            r#"{{"layers":[{}]}}"#,
            (0..40).map(layer).collect::<Vec<_>>().join(",")
        );
        let mut whole = UnitScanner::default();
        let mut expected = whole.feed(text.as_bytes());
        expected.extend(whole.finish().unwrap());
        assert!(expected.len() > 10);
        let path = std::env::temp_dir().join(format!("pm-preparse-test-{}", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let preparse = Preparse::start(
            EngineProcess::Interactive,
            path.to_string_lossy().into_owned(),
            7,
        )
        .unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        let mut written = 0;
        for chunk in text.as_bytes().chunks(100_003) {
            file.write_all(chunk).unwrap();
            written += chunk.len() as u64;
            preparse.grow(written);
        }
        let (units, complete) = preparse.finish(written);
        std::fs::remove_file(&path).unwrap();
        assert!(complete);
        assert_eq!(units.iter().map(|p| p.unit).collect::<Vec<_>>(), expected);
        for p in &units {
            let unit = &text.as_bytes()[p.unit.start..p.unit.end];
            assert_eq!(p.digest, pm_model::hash::sha256(unit));
            assert_eq!(p.value.rev(), 7);
            assert!(p.value == Node::parse_unit(unit, p.unit.depth, 7).unwrap());
        }
    }

    #[test]
    fn a_shorter_file_than_announced_is_not_complete() {
        let path = std::env::temp_dir().join(format!("pm-preparse-short-{}", std::process::id()));
        std::fs::write(&path, br#"{"a":[1,2,3]}"#).unwrap();
        let preparse = Preparse::start(
            EngineProcess::Interactive,
            path.to_string_lossy().into_owned(),
            1,
        )
        .unwrap();
        preparse.grow(64);
        let (units, complete) = preparse.finish(64);
        std::fs::remove_file(&path).unwrap();
        assert!(!complete);
        assert!(units.is_empty());
    }
}
