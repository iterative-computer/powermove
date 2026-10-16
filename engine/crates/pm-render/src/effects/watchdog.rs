//! The GPU watchdog for effects and transitions (plan section 8, "Device":
//! "Each effect and shader has a GPU timestamp budget; an overrun disables
//! it from the next frame and reports it").
//!
//! Every effect instance and transition of a frame is timed with render
//! pass timestamps: the beginning of its first pass and the end of its last.
//! The timestamps are resolved into a buffer in the frame's own command
//! buffer and read back without waiting ([`Watchdog::collect`], at the start
//! of the next frame); a definition whose time is over the budget is
//! disabled for every later frame (skipped with its data kept, like a
//! missing effect) and reported as `WatchdogTimeout` until it is registered
//! again. A true GPU hang is device loss, which the device's owner recovers
//! from; the watchdog is for the slow-but-finishing case.
//!
//! Devices without `TIMESTAMP_QUERY` time nothing.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pm_protocol::common::{DefinitionId, EffectInstanceId, LayerId};
use pm_protocol::definitions::DefinitionKind;
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope, Retry};

/// Default budget per effect instance per frame. The slowest built-in at
/// UHD (Glow, 400 px radius) takes a few milliseconds on the reference
/// machine; a quarter second is far past anything playable and still well
/// inside the operating system's GPU timeout.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(250);

/// Timed instances per frame (two queries each).
pub const CAPACITY: u32 = 128;

/// What one timed span belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub kind: DefinitionKind,
    pub definition: DefinitionId,
    pub layer: LayerId,
    /// The effect instance (`fx.id`); `None` for a transition.
    pub instance: Option<EffectInstanceId>,
}

/// One frame's timestamps in flight.
struct Pending {
    spans: Vec<Span>,
    readback: wgpu::Buffer,
    /// `map_async` was called (after the frame was submitted).
    requested: bool,
    mapped: Arc<AtomicBool>,
}

/// A frame's query set while it is being encoded.
pub struct FrameTimer {
    pub(crate) set: wgpu::QuerySet,
    pub(crate) spans: Vec<Span>,
}

impl FrameTimer {
    /// Query indices for the next span, or `None` when the set is full.
    pub(crate) fn begin(&mut self, span: Span) -> Option<(u32, u32)> {
        let index = self.spans.len() as u32;
        if index >= CAPACITY {
            return None;
        }
        self.spans.push(span);
        Some((index * 2, index * 2 + 1))
    }
}

/// The watchdog state of a renderer.
pub struct Watchdog {
    budget: Duration,
    disabled: BTreeMap<(DefinitionKind, DefinitionId), f64>,
    pending: Vec<Pending>,
    reports: Vec<EngineError>,
    /// The longest time measured per definition, in milliseconds.
    measured: BTreeMap<(DefinitionKind, DefinitionId), f64>,
}

impl Default for Watchdog {
    fn default() -> Self {
        Watchdog {
            budget: DEFAULT_BUDGET,
            disabled: BTreeMap::new(),
            pending: Vec::new(),
            reports: Vec::new(),
            measured: BTreeMap::new(),
        }
    }
}

impl Watchdog {
    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    pub fn budget(&self) -> Duration {
        self.budget
    }

    pub fn is_disabled(&self, kind: DefinitionKind, id: &str) -> bool {
        self.disabled.contains_key(&(kind, DefinitionId::from(id)))
    }

    /// A re-registered definition starts over.
    pub fn forget(&mut self, kind: DefinitionKind, id: &DefinitionId) {
        self.disabled.remove(&(kind, id.clone()));
        self.measured.remove(&(kind, id.clone()));
    }

    /// Re-enables everything (the user asked to try again).
    pub fn reset(&mut self) {
        self.disabled.clear();
    }

    /// The longest GPU time measured for a definition, in milliseconds.
    pub fn measured_ms(&self, kind: DefinitionKind, id: &str) -> Option<f64> {
        self.measured.get(&(kind, DefinitionId::from(id))).copied()
    }

    /// Overruns reported since the last call.
    pub fn take_reports(&mut self) -> Vec<EngineError> {
        std::mem::take(&mut self.reports)
    }

    /// A query set for a frame, when the device can time passes.
    pub(crate) fn frame_timer(&self, device: &wgpu::Device) -> Option<FrameTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pm effect watchdog"),
            ty: wgpu::QueryType::Timestamp,
            count: CAPACITY * 2,
        });
        Some(FrameTimer {
            set,
            spans: Vec::new(),
        })
    }

    /// Resolves a frame's timestamps in its command buffer; the readback is
    /// mapped after [`Watchdog::submitted`].
    pub(crate) fn finish(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        timer: FrameTimer,
    ) {
        if timer.spans.is_empty() {
            return;
        }
        let count = timer.spans.len() as u32 * 2;
        let bytes = u64::from(count) * 8;
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm effect watchdog resolve"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm effect watchdog readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.resolve_query_set(&timer.set, 0..count, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, bytes);
        self.pending.push(Pending {
            spans: timer.spans,
            readback,
            requested: false,
            mapped: Arc::new(AtomicBool::new(false)),
        });
    }

    /// Starts mapping the readbacks of frames just submitted.
    pub(crate) fn submitted(&mut self) {
        for pending in &mut self.pending {
            if pending.requested {
                continue;
            }
            pending.requested = true;
            let done = pending.mapped.clone();
            pending
                .readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        done.store(true, Ordering::Release);
                    }
                });
        }
    }

    /// Reads every finished frame's timestamps; `wait` blocks until all
    /// submitted frames are read (tests). Overruns disable their
    /// definitions and are reported.
    pub fn collect(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, wait: bool) {
        if self.pending.is_empty() {
            return;
        }
        let poll = if wait {
            wgpu::PollType::wait_indefinitely()
        } else {
            wgpu::PollType::Poll
        };
        let _ = device.poll(poll);
        let period = f64::from(queue.get_timestamp_period());
        let mut keep = Vec::new();
        for pending in std::mem::take(&mut self.pending) {
            if !pending.mapped.load(Ordering::Acquire) {
                keep.push(pending);
                continue;
            }
            let ticks: Vec<u64> = {
                let data = pending
                    .readback
                    .slice(..)
                    .get_mapped_range()
                    .expect("mapped readback");
                data.chunks_exact(8)
                    .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
                    .collect()
            };
            pending.readback.unmap();
            for (i, span) in pending.spans.iter().enumerate() {
                let (begin, end) = (ticks[i * 2], ticks[i * 2 + 1]);
                if end < begin || begin == 0 {
                    // Not written (the span's passes did not run).
                    continue;
                }
                let ms = (end - begin) as f64 * period / 1.0e6;
                self.record(span, ms);
            }
        }
        self.pending = keep;
    }

    fn record(&mut self, span: &Span, ms: f64) {
        let key = (span.kind, span.definition.clone());
        let longest = self.measured.entry(key.clone()).or_insert(0.0);
        *longest = longest.max(ms);
        if ms <= self.budget.as_secs_f64() * 1000.0 || self.disabled.contains_key(&key) {
            return;
        }
        self.disabled.insert(key, ms);
        let scope = match &span.instance {
            Some(effect) => ErrorScope::Effect {
                layer: span.layer.clone(),
                effect: effect.clone(),
                definition: span.definition.clone(),
            },
            None => ErrorScope::Definition {
                definition: span.definition.clone(),
            },
        };
        let message = format!(
            "{} took {ms:.1} ms of GPU time in one frame (budget {} ms) and is turned off until it is registered again.",
            span.definition.as_str(),
            self.budget.as_millis()
        );
        self.reports.push(
            EngineError::new(ErrorCode::WatchdogTimeout, scope, message)
                .with_retry(Retry::AfterUserAction),
        );
    }
}
