//! Time slices of one frame's GPU work ([`crate::Renderer::set_slices`]).
//!
//! A frame records into one encoder and runs as one submission, which the
//! GPU cannot give way inside: whatever is submitted after it waits for all
//! of it. A renderer that draws behind interactive ones (a paused viewer's
//! settled picture) submits its frame in slices instead. At each slice
//! point (before a layer is resolved, before an effect pass) the passes
//! recorded so far are submitted once they are expected to cost the budget,
//! and the renderer waits for them before recording on, so one slice at
//! most is ever on the GPU. The commands and their order are the frame's
//! own: its pixels are those of the frame submitted whole.
//!
//! `gate` is asked at every slice point and before each submission, with
//! what the submission is expected to cost: it may wait (for a moment the
//! GPU is free of interactive work) and says whether to stop. Once it says
//! so, the layers not yet resolved are left out and the frame ends as
//! [`crate::RenderError::Abandoned`]: what was recorded still runs (in
//! slices), so every cache that names GPU content stays true.

use std::sync::Arc;
use std::time::{Duration, Instant};

/// What a slice point is about to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// A layer: its source, and the passes resolving it.
    Draw = 0,
    /// One effect or transition pass.
    Effect = 1,
}

/// How long a slice is waited for before the frame goes on unsliced: a
/// program that runs away is the watchdog's to judge, at the frame's end.
const RUNAWAY: Duration = Duration::from_secs(2);
/// While a slice runs, the watchdog looks at its frames this often.
const WATCH_EVERY: Duration = Duration::from_millis(100);

/// Asked before a submission expected to take about the given time (zero:
/// nothing is submitted, only whether to go on). May wait; true: stop.
pub type Gate = Arc<dyn Fn(Duration) -> bool + Send + Sync>;

/// What the slices of the frames drawn so far cost.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SliceReport {
    /// Slices submitted before the frame's own last submission.
    pub slices: u32,
    /// The longest a slice took, submission to completion.
    pub worst: Duration,
    /// The longest a slice of a single unit took: what no slicing bounds.
    pub worst_unit: Duration,
    /// The gate said to stop during the last frame.
    pub stopped: bool,
    /// When the last frame reached its first slice point (its plan built)
    /// and when it first submitted.
    pub planned: Option<Instant>,
    pub first_submission: Option<Instant>,
    /// What the gate held the last frame's submissions back for, in all.
    pub held: Duration,
}

pub struct Slices {
    budget: Duration,
    gate: Gate,
    /// Microseconds a unit of each class costs, as measured: raised at
    /// once by a slower slice, lowered slowly by faster ones.
    rates: [f64; 2],
    /// The class and units recorded since the last submission.
    pending: Option<(Class, u32)>,
    runaway: bool,
    report: SliceReport,
}

impl Slices {
    /// Slices of about `budget` of GPU time each, admitted by `gate`.
    pub fn new(budget: Duration, gate: Gate) -> Self {
        // Until measured, a unit is taken to cost the budget: the first
        // slices are single units, and grow as they prove cheap.
        let unit = budget.as_secs_f64() * 1e6;
        Self {
            budget,
            gate,
            rates: [unit, unit],
            pending: None,
            runaway: false,
            report: SliceReport::default(),
        }
    }

    /// A frame starts.
    pub(crate) fn begin_frame(&mut self) {
        self.pending = None;
        self.runaway = false;
        self.report.stopped = false;
        self.report.planned = None;
        self.report.first_submission = None;
        self.report.held = Duration::ZERO;
    }

    /// The gate of the next frames (a frame's own, set before it).
    pub fn set_gate(&mut self, gate: Gate) {
        self.gate = gate;
    }

    pub fn report(&self) -> SliceReport {
        self.report
    }

    /// Forgets what earlier frames cost (another document state).
    pub fn reset_report(&mut self) {
        self.report = SliceReport::default();
    }

    pub(crate) fn stopped(&self) -> bool {
        self.report.stopped
    }

    /// Whether what is recorded should run before a unit of `class` is.
    pub(crate) fn due(&self, class: Class) -> bool {
        let Some((pending, units)) = self.pending else {
            return false;
        };
        if self.runaway {
            return false;
        }
        // A slice is of one class, so each class's cost is measured alone.
        pending != class
            || f64::from(units + 1) * self.rates[class as usize] >= self.budget.as_secs_f64() * 1e6
    }

    /// A unit of `class` is recorded next.
    pub(crate) fn note(&mut self, class: Class) {
        self.pending = match self.pending {
            Some((pending, units)) if pending == class => Some((class, units + 1)),
            _ => Some((class, 1)),
        };
    }

    /// A slice point: asks whether to go on, without submitting.
    pub(crate) fn ask(&mut self) {
        self.report.planned.get_or_insert_with(Instant::now);
        if !self.report.stopped {
            self.report.stopped = (self.gate)(Duration::ZERO);
        }
    }

    /// Before the recorded passes are submitted: the gate admits them.
    pub(crate) fn admit(&mut self) {
        let expected = self.pending.map_or(Duration::ZERO, |(class, units)| {
            Duration::from_secs_f64(f64::from(units) * self.rates[class as usize] / 1e6)
        });
        let asked = Instant::now();
        // A stopped frame still asks: what is left of it waits its turn too.
        self.report.stopped |= (self.gate)(expected.min(self.budget).max(Duration::from_micros(1)));
        self.report.held += asked.elapsed();
        self.report
            .first_submission
            .get_or_insert_with(Instant::now);
    }

    /// The recorded passes were submitted as `index`: waits for them
    /// (`watch`: the watchdog looks at its frames meanwhile) and learns
    /// what they cost.
    pub(crate) fn submitted(
        &mut self,
        device: &wgpu::Device,
        index: wgpu::SubmissionIndex,
        mut watch: impl FnMut(),
    ) {
        let pending = self.pending.take();
        let started = Instant::now();
        loop {
            let done = device.poll(wgpu::PollType::Wait {
                submission_index: Some(index.clone()),
                timeout: Some(WATCH_EVERY),
            });
            if !matches!(done, Err(wgpu::PollError::Timeout)) {
                break;
            }
            watch();
            if started.elapsed() >= RUNAWAY {
                self.runaway = true;
                return;
            }
        }
        let took = started.elapsed();
        let Some((class, units)) = pending else {
            return;
        };
        self.report.slices += 1;
        self.report.worst = self.report.worst.max(took);
        if units == 1 {
            self.report.worst_unit = self.report.worst_unit.max(took);
        }
        let measured = took.as_secs_f64() * 1e6 / f64::from(units);
        let rate = &mut self.rates[class as usize];
        *rate = measured.max(0.7 * *rate + 0.3 * measured);
    }
}

/// A slice point before a unit of `class` is recorded into `encoder`: what
/// is recorded so far is submitted and waited for when the unit would take
/// the slice past its budget. False: the frame is giving way, and a layer
/// not yet resolved is left out.
pub(crate) fn point(
    effects: &mut crate::effects::Effects,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    class: Class,
) -> bool {
    let Some(slices) = effects.slices.as_mut() else {
        return true;
    };
    slices.ask();
    if slices.due(class) {
        let done = std::mem::replace(
            encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm frame (slice)"),
            }),
        );
        effects.uniforms.flush(queue);
        let slices = effects.slices.as_mut().expect("checked above");
        slices.admit();
        let index = queue.submit([done.finish()]);
        let watchdog = &mut effects.watchdog;
        slices.submitted(device, index, || {
            if watchdog.frames_in_flight() > 0 {
                watchdog.collect(device, queue, false);
            }
        });
    }
    let slices = effects.slices.as_mut().expect("checked above");
    if slices.stopped() && class == Class::Draw {
        return false;
    }
    slices.note(class);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slices(budget_ms: u64) -> Slices {
        Slices::new(Duration::from_millis(budget_ms), Arc::new(|_| false))
    }

    /// Units are batched while their measured cost fits the budget, and a
    /// slice never mixes classes.
    #[test]
    fn a_slice_holds_what_fits_its_budget_of_one_class() {
        let mut s = slices(8);
        assert!(!s.due(Class::Draw), "nothing recorded");
        s.note(Class::Draw);
        assert!(s.due(Class::Draw), "unmeasured: a unit is a slice");
        assert!(s.due(Class::Effect), "another class is another slice");
        // Measured cheap: 1 ms a unit, so a slice stays under 8 ms.
        s.rates[Class::Draw as usize] = 1000.;
        for _ in 1..7 {
            assert!(!s.due(Class::Draw));
            s.note(Class::Draw);
        }
        assert!(s.due(Class::Draw), "the eighth is the next slice's");
    }
}
