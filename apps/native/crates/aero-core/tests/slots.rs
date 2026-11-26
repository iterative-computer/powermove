//! Sol's item 5: slot publication, empty slots, generation reuse. One test in
//! its own binary, so no other test's runtimes share the process's slots.

mod support;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use pm_aero_core::{Error, Host, Runtime, sys};
use support::{config, on_worker};

struct NoTools;
impl Host for NoTools {}

const SLOTS: u64 = 64;
const MAX_GENERATION: u64 = (1 << 40) - 1;

fn handle_of(slot: u64, generation: u64) -> u64 {
    (generation << 8) | (slot + 1)
}

/// Every call on a handle no runtime holds: closed, and no side effect.
fn poke(handle: u64) {
    // SAFETY: plain values and valid out pointers; the core checks handles.
    unsafe {
        let mut info: sys::fx_runtime_info = std::mem::zeroed();
        info.struct_size = std::mem::size_of::<sys::fx_runtime_info>() as u32;
        assert_eq!(
            sys::fx_runtime_info_get(handle, &mut info),
            sys::FX_E_CLOSED
        );
        assert_eq!(sys::fx_runtime_ready_fd(handle), -1);
        assert_eq!(sys::fx_turn_cancel(handle, 1), sys::FX_E_CLOSED);
        let mut count = 7usize;
        assert_eq!(
            sys::fx_events_drain(
                handle,
                std::ptr::null_mut(),
                0,
                std::mem::size_of::<sys::fx_event>(),
                &mut count
            ),
            sys::FX_E_CLOSED
        );
        assert_eq!(count, 0, "an out scalar is zeroed on every return");
        assert_eq!(sys::fx_runtime_destroy(handle), sys::FX_E_CLOSED);
    }
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
fn slots_publish_safely_reject_guesses_and_retire_exhausted_generations() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // Sol's scenario: calls on guessed handles before any create, then a
        // create and destroy of the first slot, which used to spin for ever.
        for slot in 0..SLOTS {
            for generation in [1, 2, MAX_GENERATION] {
                poke(handle_of(slot, generation));
            }
        }
        // A generation past the 40 bits, and slot index 0 or 65.
        poke(handle_of(0, MAX_GENERATION + 1));
        poke(0);
        poke(65 | (1 << 8));

        within(30, "create and destroy of every slot", || {
            let mut runtimes = Vec::new();
            for _ in 0..SLOTS {
                runtimes.push(
                    Runtime::create(&config("http://127.0.0.1:1/fx", &[]), Arc::new(NoTools))
                        .expect("create"),
                );
            }
            // The process-wide limit.
            assert_eq!(
                Runtime::create(&config("http://127.0.0.1:1/fx", &[]), Arc::new(NoTools)).err(),
                Some(Error::Limit)
            );
            let stale: Vec<_> = runtimes.iter().map(|r| r.remote()).collect();
            for runtime in runtimes {
                runtime.close();
            }
            for remote in stale {
                assert_eq!(remote.cancel(1), Err(Error::Closed));
                poke(remote.handle());
            }
        });

        // Generation exhaustion: slot 0 at its last generation is used once
        // and then never again, so its last handle can never alias.
        // SAFETY: plain values.
        assert_eq!(
            unsafe { sys::fx_test_set_slot_generation(0, MAX_GENERATION) },
            sys::FX_OK
        );
        assert_eq!(
            unsafe { sys::fx_test_set_slot_generation(64, 1) },
            sys::FX_E_INVALID
        );
        assert_eq!(
            unsafe { sys::fx_test_set_slot_generation(1, 0) },
            sys::FX_E_INVALID
        );
        let last = Runtime::create(&config("http://127.0.0.1:1/fx", &[]), Arc::new(NoTools))
            .expect("create");
        let last_remote = last.remote();
        assert_eq!(last_remote.handle(), handle_of(0, MAX_GENERATION));
        // An occupied slot cannot be re-generationed.
        assert_eq!(
            unsafe { sys::fx_test_set_slot_generation(0, 5) },
            sys::FX_E_INVALID
        );
        last.close();
        poke(last_remote.handle());
        within(30, "the slots after exhaustion", move || {
            let mut runtimes = Vec::new();
            for _ in 0..SLOTS - 1 {
                let runtime =
                    Runtime::create(&config("http://127.0.0.1:1/fx", &[]), Arc::new(NoTools))
                        .expect("create");
                assert_ne!(runtime.remote().handle() & 0xff, 1, "slot 0 is retired");
                runtimes.push(runtime);
            }
            // Slot 0 is gone for good: the limit is one lower.
            assert_eq!(
                Runtime::create(&config("http://127.0.0.1:1/fx", &[]), Arc::new(NoTools)).err(),
                Some(Error::Limit)
            );
            poke(last_remote.handle());
            for runtime in runtimes {
                runtime.close();
            }
        });
        // SAFETY: plain values; slot 0 stays reserved.
        assert_eq!(
            unsafe { sys::fx_test_set_slot_generation(0, 1) },
            sys::FX_E_INVALID
        );
    });
}
