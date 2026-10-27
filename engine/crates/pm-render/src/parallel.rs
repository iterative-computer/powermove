//! Work a frame splits over threads: the misses of a cache it met new (a
//! text layer's glyph geometry, a shape layer's outlines), on the document
//! passes' threads (`pm_model::parallel`). Results never depend on the
//! thread count.

/// `f(0), …, f(n - 1)` in order, on up to `pm_model::parallel::workers`
/// threads running at the calling thread's quality of service. macOS starts
/// a new thread at the default class, and a render thread at
/// user-interactive would wait for helpers the scheduler holds back (on
/// efficiency cores, behind other work) for the frame it is drawing.
pub(crate) fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let qos = qos::current();
    pm_model::parallel::map(n, |i| {
        qos::adopt(qos);
        f(i)
    })
}

#[cfg(target_os = "macos")]
mod qos {
    use std::cell::Cell;

    unsafe extern "C" {
        fn qos_class_self() -> u32;
        fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
    }

    thread_local! { static ADOPTED: Cell<u32> = const { Cell::new(0) }; }

    pub(super) fn current() -> u32 {
        // SAFETY: reads the calling thread's class.
        unsafe { qos_class_self() }
    }

    /// Runs this thread at `qos` from now on (once per thread).
    pub(super) fn adopt(qos: u32) {
        ADOPTED.with(|adopted| {
            if adopted.get() != qos && current() != qos {
                // SAFETY: affects only the calling thread.
                unsafe { pthread_set_qos_class_self_np(qos, 0) };
            }
            adopted.set(qos);
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod qos {
    pub(super) fn current() -> u32 {
        0
    }
    pub(super) fn adopt(_: u32) {}
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn helpers_run_at_the_callers_quality_of_service() {
        // A render thread runs at user-interactive; new threads start at
        // the default class.
        const USER_INTERACTIVE: u32 = 0x21;
        let classes = std::thread::spawn(|| {
            super::qos::adopt(USER_INTERACTIVE);
            super::map(64, |_| {
                std::thread::sleep(std::time::Duration::from_micros(200));
                super::qos::current()
            })
        })
        .join()
        .unwrap();
        assert_eq!(classes.len(), 64);
        assert!(
            classes.iter().all(|&c| c == USER_INTERACTIVE),
            "{classes:x?}"
        );
    }
}
