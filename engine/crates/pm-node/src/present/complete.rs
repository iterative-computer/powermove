//! Frames shown at the next refresh (paused edits, gestures) leave the
//! render thread when their GPU work is submitted, not when it finishes
//! (docs/engine/perf.md, "The render thread does not wait for the GPU").
//!
//! The render thread used to block in `device.poll(Wait)` for every frame
//! before it could take the next command. During a layer drag (a new
//! revision every refresh) that put the GPU half of each render (about
//! 3 ms of 8 at 500 layers) in front of the next edit. Now a completion
//! thread waits for the submission, sends the frame's descriptor to the
//! viewer the moment the GPU is done (the bridge's rule: a descriptor only
//! names finished work, amendment 4), and hands the frame's bookkeeping
//! back to the render thread, which meanwhile applies the next edit and
//! encodes the next frame.
//!
//! Descriptors go out in submission order (one thread, FIFO). The render
//! thread drains the completions before anything whose order against them
//! matters: a timed (playback) frame, a frame request's answer, a pool
//! change, an `unsupported` report (a frame drawn before the decision must
//! not reach the viewer after it), GPU recovery, and shutdown. At most
//! [`MAX_IN_FLIGHT`] frames wait on the GPU, so the queue never runs ahead
//! of the display.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use pm_engine::Action;
use pm_protocol::error::EngineProcess;

use super::render::{Command, Key, Sink, now_us};

/// Frames submitted and not yet finished, at most. One executes while the
/// next is encoded and queued behind it.
pub const MAX_IN_FLIGHT: usize = 2;

/// How long a frame may take on the GPU before it is abandoned (the
/// synchronous path's bound).
const GPU_TIMEOUT: Duration = Duration::from_secs(2);

/// What the render thread records once a frame is on its way (stats, the
/// per-frame trace, the session's costs).
#[derive(Debug, Clone)]
pub struct FrameMeta {
    pub seq: u64,
    pub k: i64,
    pub index: u32,
    pub generation: u32,
    pub timing: Option<(f64, f64)>,
    pub view_seq: u64,
    pub revision: u64,
    pub start: i64,
    pub work_start: i64,
    pub cpu_us: i64,
    pub overlay_layers: usize,
    pub failures: usize,
    pub stand_in_shown: Option<usize>,
    pub depth: bool,
    pub held: u32,
}

/// A submitted frame waiting for its GPU work.
pub struct Pending {
    pub key: Key,
    pub device: wgpu::Device,
    pub submission: wgpu::SubmissionIndex,
    /// The encoded `Event::Frame` envelope for the viewer's port.
    pub bytes: Vec<u8>,
    pub meta: FrameMeta,
}

/// The completion thread and its in-flight count.
pub struct Completer {
    tx: Option<Sender<Pending>>,
    in_flight: Arc<(Mutex<usize>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Completer {
    /// Starts the completion thread. `sink` sends the descriptors; `back`
    /// reaches the render thread. `delay` (tests only) holds each finished
    /// frame that long before it is sent.
    pub fn start(
        sink: Sink,
        back: Sender<Command>,
        delay: Option<Duration>,
    ) -> Result<Self, String> {
        let (tx, rx) = channel::<Pending>();
        let in_flight = Arc::new((Mutex::new(0usize), Condvar::new()));
        let count = in_flight.clone();
        let thread = pm_engine::qos::spawn_engine_thread(
            EngineProcess::Interactive,
            "pm-viewer-complete",
            move || run(rx, sink, back, count, delay),
        )
        .map_err(|e| format!("could not start the frame completion thread: {e}"))?;
        Ok(Self {
            tx: Some(tx),
            in_flight,
            thread: Some(thread),
        })
    }

    /// Hands a submitted frame to the completion thread. Waits first while
    /// [`MAX_IN_FLIGHT`] frames are on the GPU.
    pub fn submit(&self, pending: Pending) {
        self.wait_below(MAX_IN_FLIGHT);
        let Some(tx) = &self.tx else { return };
        let (lock, _) = &*self.in_flight;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        if tx.send(pending).is_err() {
            // The thread is gone (shutdown): nothing will finish it.
            self.done_one();
        }
    }

    /// Frames submitted and not yet sent or abandoned.
    pub fn in_flight(&self) -> usize {
        *self.in_flight.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Blocks until every submitted frame is sent or abandoned.
    pub fn drain(&self) {
        self.wait_below(1);
    }

    fn wait_below(&self, limit: usize) {
        let (lock, cvar) = &*self.in_flight;
        let mut n = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= limit {
            n = cvar.wait(n).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn done_one(&self) {
        let (lock, cvar) = &*self.in_flight;
        let mut n = lock.lock().unwrap_or_else(|e| e.into_inner());
        *n = n.saturating_sub(1);
        cvar.notify_all();
    }
}

impl Drop for Completer {
    fn drop(&mut self) {
        // Closing the channel ends the thread after the frames it holds.
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    rx: Receiver<Pending>,
    sink: Sink,
    back: Sender<Command>,
    in_flight: Arc<(Mutex<usize>, Condvar)>,
    delay: Option<Duration>,
) {
    pm_bridge::set_thread_user_interactive();
    for pending in rx {
        let Pending {
            key,
            device,
            submission,
            bytes,
            meta,
        } = pending;
        let done = device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(GPU_TIMEOUT),
        });
        let gpu_done = now_us();
        let command = match done {
            Ok(_) => {
                if let Some(delay) = delay {
                    std::thread::sleep(delay);
                }
                sink(vec![Action::Send { port: key.0, bytes }]);
                Command::FrameSent {
                    key,
                    meta: Box::new(meta),
                    gpu_done,
                    sent: now_us(),
                }
            }
            Err(error) => Command::FrameFailed {
                key,
                meta: Box::new(meta),
                error: error.to_string(),
            },
        };
        // Count it done before the render thread hears of it: a drain that
        // waits on the render thread's behalf must not wait for itself.
        {
            let (lock, cvar) = &*in_flight;
            let mut n = lock.lock().unwrap_or_else(|e| e.into_inner());
            *n = n.saturating_sub(1);
            cvar.notify_all();
        }
        let _ = back.send(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_bounds_submissions_and_drains() {
        let in_flight = Arc::new((Mutex::new(0usize), Condvar::new()));
        let completer = Completer {
            tx: None,
            in_flight: in_flight.clone(),
            thread: None,
        };
        *in_flight.0.lock().unwrap() = MAX_IN_FLIGHT;
        let count = in_flight.clone();
        let finisher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let (lock, cvar) = &*count;
            *lock.lock().unwrap() = 0;
            cvar.notify_all();
        });
        let started = std::time::Instant::now();
        completer.drain();
        assert!(started.elapsed() >= Duration::from_millis(15));
        assert_eq!(completer.in_flight(), 0);
        finisher.join().unwrap();
    }
}
