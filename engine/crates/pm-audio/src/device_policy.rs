//! Device lifecycle decisions, independent of CoreAudio and decoder libraries.
//! The host commits a snapshot only after opening its corresponding stream.
//! Poll this from a worker, never from an audio callback or the UI thread.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceSnapshot {
    pub id: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub latency_frames: u32,
    pub buffer_frames: u32,
}

pub trait DeviceProbe {
    type Error;
    fn snapshot(&mut self) -> Result<Option<DeviceSnapshot>, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceAction {
    Unchanged,
    Unavailable,
    /// Same stream format; invalidate its clock epoch and update scheduling.
    Reanchor(DeviceSnapshot),
    /// Device, sample rate or channel count changed; open a fresh stream.
    Reopen(DeviceSnapshot),
}

pub struct DeviceMonitor<P> {
    probe: P,
    active: Option<DeviceSnapshot>,
    unavailable: bool,
}

impl<P: DeviceProbe> DeviceMonitor<P> {
    pub fn new(probe: P, active: Option<DeviceSnapshot>) -> Self {
        Self {
            probe,
            active,
            unavailable: false,
        }
    }

    pub fn active(&self) -> Option<DeviceSnapshot> {
        self.active
    }

    pub fn poll(&mut self) -> Result<DeviceAction, P::Error> {
        let Some(next) = self.probe.snapshot()? else {
            self.unavailable = true;
            return Ok(DeviceAction::Unavailable);
        };
        // An invalid or unavailable device never displaces a working stream.
        if next.id == 0 || next.sample_rate == 0 || next.channels == 0 {
            self.unavailable = true;
            return Ok(DeviceAction::Unavailable);
        }
        let Some(active) = self.active else {
            return Ok(DeviceAction::Reopen(next));
        };
        if self.unavailable
            || active.id != next.id
            || active.sample_rate != next.sample_rate
            || active.channels != next.channels
        {
            return Ok(DeviceAction::Reopen(next));
        }
        Ok(if active == next {
            DeviceAction::Unchanged
        } else {
            DeviceAction::Reanchor(next)
        })
    }

    /// Commit only after a stream open or scheduling update succeeds.
    pub fn commit(&mut self, opened: DeviceSnapshot) -> bool {
        if opened.id == 0 || opened.sample_rate == 0 || opened.channels == 0 {
            return false;
        }
        self.active = Some(opened);
        self.unavailable = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakeProbe(VecDeque<Result<Option<DeviceSnapshot>, &'static str>>);
    impl DeviceProbe for FakeProbe {
        type Error = &'static str;
        fn snapshot(&mut self) -> Result<Option<DeviceSnapshot>, Self::Error> {
            self.0.pop_front().expect("a scripted observation")
        }
    }
    fn snapshot() -> DeviceSnapshot {
        DeviceSnapshot {
            id: 7,
            sample_rate: 48_000,
            channels: 2,
            latency_frames: 128,
            buffer_frames: 256,
        }
    }
    fn monitor(
        events: Vec<Result<Option<DeviceSnapshot>, &'static str>>,
    ) -> DeviceMonitor<FakeProbe> {
        DeviceMonitor::new(FakeProbe(events.into()), Some(snapshot()))
    }

    #[test]
    fn unchanged_output_does_not_restart_the_clock() {
        assert_eq!(
            monitor(vec![Ok(Some(snapshot()))]).poll(),
            Ok(DeviceAction::Unchanged)
        );
    }
    #[test]
    fn same_device_rate_and_channel_changes_require_a_new_stream() {
        for next in [
            DeviceSnapshot {
                sample_rate: 44_100,
                ..snapshot()
            },
            DeviceSnapshot {
                channels: 6,
                ..snapshot()
            },
            DeviceSnapshot {
                id: 8,
                ..snapshot()
            },
        ] {
            let mut m = monitor(vec![Ok(Some(next)), Ok(Some(next)), Ok(Some(next))]);
            assert_eq!(m.poll(), Ok(DeviceAction::Reopen(next)));
            assert_eq!(m.active(), Some(snapshot())); // opening has not succeeded
            assert_eq!(m.poll(), Ok(DeviceAction::Reopen(next)));
            assert!(m.commit(next));
            assert_eq!(m.poll(), Ok(DeviceAction::Unchanged));
        }
    }
    #[test]
    fn latency_and_buffer_changes_reanchor_without_reopening() {
        for next in [
            DeviceSnapshot {
                latency_frames: 512,
                ..snapshot()
            },
            DeviceSnapshot {
                buffer_frames: 4096,
                ..snapshot()
            },
        ] {
            let mut m = monitor(vec![Ok(Some(next)), Ok(Some(next))]);
            assert_eq!(m.poll(), Ok(DeviceAction::Reanchor(next)));
            assert!(m.commit(next));
            assert_eq!(m.poll(), Ok(DeviceAction::Unchanged));
        }
    }
    #[test]
    fn disappearance_preserves_the_last_stream_and_reappearance_reopens() {
        let mut m = monitor(vec![Ok(None), Ok(Some(snapshot())), Ok(Some(snapshot()))]);
        assert_eq!(m.poll(), Ok(DeviceAction::Unavailable));
        assert_eq!(m.active(), Some(snapshot()));
        assert_eq!(m.poll(), Ok(DeviceAction::Reopen(snapshot())));
        m.commit(snapshot());
        assert_eq!(m.poll(), Ok(DeviceAction::Unchanged));
    }
    #[test]
    fn probe_errors_and_invalid_snapshots_never_commit_a_format() {
        let invalid = DeviceSnapshot {
            sample_rate: 0,
            ..snapshot()
        };
        let mut m = monitor(vec![Err("permission"), Ok(Some(invalid))]);
        assert_eq!(m.poll(), Err("permission"));
        assert_eq!(m.poll(), Ok(DeviceAction::Unavailable));
        assert!(!m.commit(invalid));
        assert_eq!(m.active(), Some(snapshot()));
    }
}
