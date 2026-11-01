use crate::{FrameHandle, pixels::Straighten, slot_free};
use pm_gpu::{
    Gpu,
    bridge_out::{
        display::{DisplayParams, DisplayTransform, WorkingTexture},
        surface::ExportedSurface,
    },
    target::Target,
};
use pm_model::Document;
use pm_node::fonts::render::{FontBroker, RenderFonts};
use pm_protocol::{
    color::{Precision, ViewerColorSpace, WorkingEncoding},
    common::{Size, Ticks},
    interop::{SurfaceFormat, SurfaceRequest},
};
use pm_render::{AssetResolver, Renderer};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// Immutable core snapshot and requested composition/time. `size` is physical
/// output pixels, never GPUI logical points. SDR sRGB presentation first.
#[derive(Clone)]
pub struct RenderRequest {
    pub document: Arc<Document>,
    pub composition: String,
    pub time: Ticks,
    pub size: Size,
    pub assets: Arc<dyn AssetResolver>,
}
struct Pending {
    request: RenderRequest,
    id: u64,
    at: Instant,
}
#[derive(Default)]
struct State {
    pending: Option<Pending>,
    latest: Option<FrameHandle>,
    error: Option<String>,
    stop: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    sequence: AtomicU64,
    wake: async_channel::Sender<()>,
}
/// One render worker and a bounded latest-frame mailbox. Creating the host does
/// not wait for GPU initialization. Poll `last_error` and await `updates` on UI.
/// Drop cancels pending work; worker teardown happens off the UI thread.
pub struct EngineHost {
    shared: Arc<Shared>,
    updates: async_channel::Receiver<()>,
}
impl EngineHost {
    pub fn start(broker: FontBroker) -> Self {
        let (wake, updates) = async_channel::bounded(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            sequence: AtomicU64::new(0),
            wake,
        });
        let worker = shared.clone();
        let fonts = broker.clone();
        std::thread::Builder::new()
            .name("pm-viewer-render".into())
            .spawn(move || {
                if let Err(error) = run(&worker, fonts) {
                    worker.state.lock().unwrap().error = Some(error);
                    let _ = worker.wake.try_send(());
                }
            })
            .expect("start render worker");
        Self { shared, updates }
    }
    /// Replace any queued request. One producer submission at a time; stale
    /// in-progress work completes safely but cannot replace the last good frame.
    pub fn request(&self, request: RenderRequest) -> Result<u64, String> {
        if request.size.width == 0
            || request.size.height == 0
            || request.size.width > 16384
            || request.size.height > 16384
        {
            return Err("viewer size must be between 1 and 16384 pixels per axis".into());
        }
        let mut state = self.shared.state.lock().unwrap();
        let id = self.shared.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        state.pending = Some(Pending {
            request,
            id,
            at: Instant::now(),
        });
        self.shared.changed.notify_one();
        Ok(id)
    }
    pub fn latest(&self) -> Option<FrameHandle> {
        self.shared.state.lock().unwrap().latest.clone()
    }
    pub fn last_error(&self) -> Option<String> {
        self.shared.state.lock().unwrap().error.clone()
    }
    /// Coalesced notification only; retrieve pixels using `latest()`.
    pub fn updates(&self) -> async_channel::Receiver<()> {
        self.updates.clone()
    }
}
impl Drop for EngineHost {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.stop = true;
        state.pending = None;
        state.latest = None;
        self.shared.changed.notify_one();
    }
}
fn run(shared: &Shared, broker: FontBroker) -> Result<(), String> {
    let gpu = viewer_gpu()?;
    let registry_id = gpu
        .registry_id
        .ok_or("viewer requires a Metal registry ID")?;
    eprintln!(
        "viewer producer Metal registry ID {registry_id} ({})",
        gpu.info.name
    );
    let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone());
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .map_err(|e| e.to_string())?;
    let fonts = RenderFonts::new(broker);
    let mut installed = None;
    let display = DisplayTransform::new(&gpu.device, &gpu.queue);
    let straighten = Straighten::new(&gpu.device);
    let mut ring: [Option<Arc<ExportedSurface>>; 3] = Default::default();
    let mut working: Option<WorkingTexture> = None;
    let mut target: Option<Target> = None;
    let mut generation = 0;
    let mut previous_size = None;
    loop {
        let pending = {
            let mut state = shared.state.lock().unwrap();
            while state.pending.is_none() && !state.stop {
                state = shared.changed.wait(state).unwrap();
            }
            if state.stop {
                break;
            }
            // Leases can disappear on Metal completion without a UI wake. The
            // worker's bounded timed wait also wakes immediately on new requests.
            if !ring.iter().any(|s| s.as_ref().is_none_or(slot_free)) {
                drop(
                    shared
                        .changed
                        .wait_timeout(state, Duration::from_millis(4))
                        .unwrap(),
                );
                continue;
            }
            state.pending.take().unwrap()
        };
        let r = &pending.request;
        let slot = ring
            .iter()
            .position(|s| s.as_ref().is_none_or(slot_free))
            .unwrap();
        let result = (|| -> Result<FrameHandle, String> {
            if previous_size != Some(r.size) {
                generation += 1;
                previous_size = Some(r.size);
            }
            if ring[slot].as_ref().is_none_or(|s| s.size != r.size) {
                // Replace only a free slot: resize never overwrites an old frame
                // and never grows the ring beyond three exported allocations.
                ring[slot] = None;
                ring[slot] = Some(Arc::new(
                    ExportedSurface::create(
                        &gpu.device,
                        &SurfaceRequest {
                            size: r.size,
                            format: SurfaceFormat::Bgra8Unorm,
                        },
                    )
                    .map_err(|e| e.to_string())?,
                ));
            }
            if working.as_ref().is_none_or(|w| w.size != r.size) {
                working = Some(WorkingTexture::new(&gpu.device, r.size));
            }
            fonts.sync(&mut renderer, &mut installed);
            let request = pm_render::RenderRequest::sized(r.size, Precision::Float16);
            let format = renderer.device_target_format(&r.document, &request);
            if target.as_ref().is_none_or(|t| {
                t.width() != r.size.width || t.height() != r.size.height || t.format() != format
            }) {
                target = Some(Target::new(
                    &gpu.device,
                    "viewer composition",
                    r.size.width,
                    r.size.height,
                    format,
                    pm_gpu::target::working_usage(),
                ));
            }
            let target = target.as_ref().unwrap();
            let rendered = renderer
                .render_into(
                    &r.document,
                    &r.composition,
                    r.time,
                    &request,
                    &*r.assets,
                    target,
                    None,
                )
                .map_err(|e| e.to_string())?;
            if let Some(error) = fonts.after_render() {
                return Err(error);
            }
            if !rendered.failures.is_empty() {
                return Err(format!("viewer layer failures: {:?}", rendered.failures));
            }
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("native viewer display"),
                });
            let working = working.as_ref().unwrap();
            straighten.encode(&gpu.device, &mut encoder, target, working);
            let surface = ring[slot].as_ref().unwrap();
            // render_into returns premultiplied E-encoded output even for a
            // linear-light project (same Standard input as present/compose).
            display
                .encode(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    working,
                    surface,
                    DisplayParams {
                        working: WorkingEncoding::Standard,
                        canvas: ViewerColorSpace::Srgb,
                        noise_offset: DisplayParams::noise_offset_for(pending.id),
                    },
                )
                .map_err(|e| e.to_string())?;
            gpu.queue.submit([encoder.finish()]);
            // Producer completion is deliberately waited on THIS worker, never
            // AppKit. One submission bounds queue growth while scrubbing.
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| e.to_string())?;
            Ok(FrameHandle {
                surface: surface.clone(),
                revision: r.document.revision(),
                time: r.time,
                size: r.size,
                generation,
                slot,
                request_id: pending.id,
                metal_registry_id: registry_id,
                color_space: ViewerColorSpace::Srgb,
                requested_at: pending.at,
                ready_at: Instant::now(),
                displayed_ns: Arc::new(AtomicU64::new(0)),
            })
        })();
        // render_into can submit before failing. Drain before recycling any
        // producer target, including the error path.
        gpu.wait_idle();
        let mut state = shared.state.lock().unwrap();
        if !state.stop && shared.sequence.load(Ordering::Acquire) == pending.id {
            match result {
                Ok(frame) => {
                    eprintln!(
                        "producer ready request {} in {:.2} ms",
                        frame.request_id,
                        frame
                            .ready_at
                            .duration_since(frame.requested_at)
                            .as_secs_f64()
                            * 1000.0
                    );
                    state.latest = Some(frame);
                    state.error = None;
                }
                Err(error) => {
                    eprintln!("viewer render: {error}");
                    state.error = Some(error);
                }
            }
            let _ = shared.wake.try_send(());
        }
    }
    gpu.wait_idle();
    Ok(())
}

// Match the pinned GPUI Apple's create_device policy before allocating any
// frames. GPUI additionally validates its actual device at consumption time.
fn viewer_gpu() -> Result<Gpu, String> {
    let consumer = metal::Device::all()
        .into_iter()
        .min_by_key(|d| (d.is_removable(), !d.is_low_power()))
        .or_else(metal::Device::system_default)
        .ok_or("no GPUI-compatible Metal device")?;
    let expected = consumer.registry_id();
    let gpu = Gpu::new("Powermove native viewer").map_err(|e| e.to_string())?;
    if gpu.registry_id == Some(expected) {
        return Ok(gpu);
    }
    drop(gpu);
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::METAL;
    let instance = wgpu::Instance::new(desc);
    for adapter in pollster::block_on(instance.enumerate_adapters(wgpu::Backends::METAL)) {
        if let Ok(gpu) = Gpu::from_adapter(&adapter, "Powermove GPUI-matched viewer") {
            if gpu.registry_id == Some(expected) {
                return Ok(gpu);
            }
        }
    }
    Err(format!(
        "no engine adapter matching GPUI Metal registry ID {expected}"
    ))
}
