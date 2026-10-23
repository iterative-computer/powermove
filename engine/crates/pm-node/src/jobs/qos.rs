//! User export QoS follows interactive playback on each thread that does work.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pm_engine::qos::{self, JobQos};
use pm_export::render::GpuGate;
use pm_protocol::priority::PriorityClass;

use super::gate::LaneGate;

pub(super) fn follows_playback(class: PriorityClass) -> bool {
    class == PriorityClass::UserJob
}

pub(super) struct UserExportQos {
    playing: Arc<AtomicBool>,
}

impl UserExportQos {
    pub(super) fn new(playing: Arc<AtomicBool>) -> Self {
        Self { playing }
    }

    fn desired(&self) -> JobQos {
        if self.playing.load(Ordering::Acquire) {
            JobQos::Utility
        } else {
            JobQos::Default
        }
    }

    pub(super) fn sync(&self) {
        let desired = self.desired();
        let name = match desired {
            JobQos::Default => "default",
            JobQos::Utility => "utility",
        };
        if qos::current_qos() != name {
            qos::set_job_qos(desired).expect("valid Darwin job QoS class");
        }
    }

    pub(super) fn enter(&self) -> Scope {
        self.sync();
        Scope
    }
}

/// A user export may leave its lane at default QoS. The next job always
/// begins from the job engine's utility class, even on an error path.
pub(super) struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        qos::set_job_qos(JobQos::Utility).expect("valid Darwin job QoS class");
    }
}

/// Adds only QoS hooks to the existing GPU gate. Every admission still goes
/// through the lane's original slicing and priority policy.
#[derive(Clone)]
pub(super) struct ExportGate {
    pub(super) lane: LaneGate,
    pub(super) qos: Option<Arc<UserExportQos>>,
}

impl GpuGate for ExportGate {
    fn admit(&self, pass: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        self.lane.admit(pass, units, cancel)
    }

    fn finished(&self, pass: u64, units: u32, measured_us: f64) {
        self.lane.finished(pass, units, measured_us)
    }

    fn finished_timed(&self, pass: u64, units: u32, measured_us: f64, gpu_timed: bool) {
        self.lane
            .finished_timed(pass, units, measured_us, gpu_timed)
    }

    fn abandoned(&self, pass: u64) {
        self.lane.abandoned(pass)
    }

    fn measures(&self) -> bool {
        self.lane.measures()
    }

    fn sync_worker_qos(&self) {
        if let Some(qos) = &self.qos {
            qos.sync();
        }
    }

    fn worker_qos_refresh(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        self.qos.as_ref().map(|qos| {
            let qos = qos.clone();
            Arc::new(move || qos.sync()) as Arc<dyn Fn() + Send + Sync>
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::error::EngineProcess;

    #[test]
    fn only_user_exports_follow_playback() {
        assert!(follows_playback(PriorityClass::UserJob));
        assert!(!follows_playback(PriorityClass::AgentExport));
        assert!(!follows_playback(PriorityClass::AgentSnapshot));
        assert!(!follows_playback(PriorityClass::Background));
        assert!(!follows_playback(PriorityClass::Generation));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn user_export_switches_both_ways_and_restores_lane() {
        let playing = Arc::new(AtomicBool::new(false));
        let other = playing.clone();
        qos::spawn_engine_thread(EngineProcess::Job, "pm-test-export-qos", move || {
            assert_eq!(qos::current_qos(), "utility");
            let policy = UserExportQos::new(other.clone());
            {
                let _scope = policy.enter();
                assert_eq!(qos::current_qos(), "default");
                other.store(true, Ordering::Release);
                policy.sync();
                assert_eq!(qos::current_qos(), "utility");
                other.store(false, Ordering::Release);
                policy.sync();
                assert_eq!(qos::current_qos(), "default");
            }
            assert_eq!(qos::current_qos(), "utility");
        })
        .unwrap()
        .join()
        .unwrap();
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn encoder_thread_refreshes_while_agent_and_background_stay_utility() {
        let playing = Arc::new(AtomicBool::new(false));
        let policy = Arc::new(UserExportQos::new(playing.clone()));
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        let (class_tx, class_rx) = std::sync::mpsc::channel();
        let encoder = std::thread::spawn(move || {
            policy.sync();
            class_tx.send(qos::current_qos()).unwrap();
            for _ in 0..2 {
                command_rx.recv().unwrap();
                policy.sync();
                class_tx.send(qos::current_qos()).unwrap();
            }
        });
        assert_eq!(class_rx.recv().unwrap(), "default");
        playing.store(true, Ordering::Release);
        command_tx.send(()).unwrap();
        assert_eq!(class_rx.recv().unwrap(), "utility");
        playing.store(false, Ordering::Release);
        command_tx.send(()).unwrap();
        assert_eq!(class_rx.recv().unwrap(), "default");
        encoder.join().unwrap();

        for class in [PriorityClass::AgentExport, PriorityClass::Background] {
            qos::spawn_engine_thread(EngineProcess::Job, "pm-test-other-qos", move || {
                assert!(!follows_playback(class));
                assert_eq!(qos::current_qos(), "utility");
            })
            .unwrap()
            .join()
            .unwrap();
        }
    }
}
