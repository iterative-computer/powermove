use crate::worker;
use anyhow::{Result, bail};

fn await_pending<T>(pending: &worker::Pending<T>) -> Result<T> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(value) = pending.try_take()? {
            return Ok(value);
        }
        if std::time::Instant::now() >= deadline {
            bail!("worker timed out")
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
#[test]
fn worker_initializes_and_runs_on_another_thread_in_submission_order() {
    let caller = std::thread::current().id();
    let worker = worker::ServiceWorker::start(move || {
        assert_ne!(std::thread::current().id(), caller);
        Ok(0u64)
    })
    .unwrap();
    let first = worker
        .submit(|state| {
            *state += 1;
            Ok(*state)
        })
        .unwrap();
    let second = worker
        .submit(|state| {
            *state += 1;
            Ok(*state)
        })
        .unwrap();
    assert_eq!(await_pending(&first).unwrap(), 1);
    assert_eq!(await_pending(&second).unwrap(), 2);
}
#[test]
fn worker_submission_and_poll_do_not_wait_for_blocked_hook() {
    let (release, hold) = std::sync::mpsc::channel();
    let worker = worker::ServiceWorker::start(|| Ok(())).unwrap();
    let pending = worker
        .submit(move |_| {
            hold.recv()?;
            Ok(7)
        })
        .unwrap();
    assert!(pending.try_take().unwrap().is_none());
    let next = worker.submit(|_| Ok(8)).unwrap();
    assert!(next.try_take().unwrap().is_none());
    release.send(()).unwrap();
    assert_eq!(await_pending(&pending).unwrap(), 7);
    assert_eq!(await_pending(&next).unwrap(), 8);
}
#[test]
fn worker_backpressure_is_explicit_and_failed_tasks_do_not_stop_queue() {
    let (release, hold) = std::sync::mpsc::channel();
    let (started, ready) = std::sync::mpsc::channel();
    let worker = worker::ServiceWorker::start(|| Ok(())).unwrap();
    let blocked = worker
        .submit(move |_| {
            started.send(())?;
            hold.recv()?;
            Ok(())
        })
        .unwrap();
    ready
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let mut queued = Vec::new();
    for _ in 0..64 {
        queued.push(worker.submit(|_| Ok(())).unwrap());
    }
    assert!(worker.submit(|_| Ok(())).is_err());
    release.send(()).unwrap();
    await_pending(&blocked).unwrap();
    for pending in queued {
        await_pending(&pending).unwrap();
    }
    let failed = worker
        .submit::<()>(|_| bail!("synthetic operation failure"))
        .unwrap();
    assert!(await_pending(&failed).is_err());
    assert_eq!(
        await_pending(&worker.submit(|_| Ok(1)).unwrap()).unwrap(),
        1
    );
}
#[test]
fn worker_initialization_failure_is_reported_without_hanging() {
    let worker = worker::ServiceWorker::<()>::start(|| bail!("synthetic init failure")).unwrap();
    if let Ok(pending) = worker.submit(|_| Ok(())) {
        assert!(await_pending(&pending).is_err());
    }
}
