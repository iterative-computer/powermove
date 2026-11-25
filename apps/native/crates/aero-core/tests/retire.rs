//! Sol's 2a item 2: create reserves a usable slot before it starts anything,
//! and retirement survives a panic inside create or destroy. One test in its
//! own binary: FX_FAULT_CREATE is process-wide, and the test fills every slot.

mod support;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pm_aero_core::{Error, Host, Runtime, State, sys};
use support::{config, on_worker, thread_count};

struct NoTools;
impl Host for NoTools {}

const SLOTS: usize = 64;
const ENDPOINT: &str = "http://127.0.0.1:1/fx";

fn create() -> Result<Runtime, Error> {
    Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools))
}

fn fill() -> Vec<Runtime> {
    (0..SLOTS).map(|_| create().expect("create")).collect()
}

/// Runs `body` and fails the test if it takes longer than `secs` (a hang).
fn within(secs: u64, what: &str, body: impl FnOnce() + Send + 'static) {
    let (done, wait) = mpsc::channel();
    std::thread::spawn(move || {
        body();
        let _ = done.send(());
    });
    wait.recv_timeout(Duration::from_secs(secs))
        .unwrap_or_else(|_| panic!("{what} did not finish within {secs} s"));
}

#[test]
fn create_and_destroy_retire_their_slot_whatever_panics() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        within(60, "the create and destroy faults", || {
            // Every usable slot busy: create fails before it starts anything.
            // The create fault, armed now, sits after the core thread starts;
            // it stays armed, so no thread was started for the refused create.
            let mut runtimes = fill();
            std::thread::sleep(Duration::from_millis(300));
            let threads = thread_count();
            pm_aero_core::arm_create_fault(0).unwrap();
            let started = Instant::now();
            assert_eq!(create().err(), Some(Error::Limit));
            assert!(started.elapsed() < Duration::from_secs(1));
            assert_eq!(thread_count(), threads, "a refused create started a thread");

            // A slot frees: the fault fires inside create, after its thread
            // started. No handle is issued, and the slot and the count come
            // back (the runtime is abandoned, parked).
            runtimes.pop().unwrap().close();
            assert_eq!(create().err(), Some(Error::Failed));
            let replacement = create().expect("the failed create gave its slot back");
            runtimes.push(replacement);
            assert_eq!(create().err(), Some(Error::Limit));

            // A panic inside destroy: the handle is retired all the same.
            let victim = runtimes.pop().unwrap();
            let remote = victim.remote();
            victim.arm_fault(sys::FX_FAULT_DESTROY, 0).unwrap();
            let started = Instant::now();
            victim.close();
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "close took {:?}",
                started.elapsed()
            );
            assert_eq!(remote.cancel(1), Err(Error::Closed));
            assert_eq!(remote.state(), Err(Error::Closed));
            // SAFETY: plain value call.
            assert_eq!(
                unsafe { sys::fx_runtime_destroy(remote.handle()) },
                sys::FX_E_CLOSED
            );

            // Every slot and the whole count are back.
            for runtime in runtimes {
                runtime.close();
            }
            let runtimes = fill();
            assert_eq!(create().err(), Some(Error::Limit));
            for runtime in &runtimes {
                assert_eq!(runtime.remote().state(), Ok(State::Running));
            }
            for runtime in runtimes {
                runtime.close();
            }
        });
    });
}
