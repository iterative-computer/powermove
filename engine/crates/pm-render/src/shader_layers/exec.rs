//! Draws a [`ShaderSource`]: the compositor's `grab(targetW, targetH)`,
//! `clear(0, 0, 0, 0)`, blending off, the quad through `PM.VERT`, and the
//! uniform calls in issue order, in GL row order.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use pm_glsl::{Failure, Key};
use pm_gpu::Target;
use pm_protocol::definitions::DefinitionKind;

use super::fallback::{FallbackPixels, FallbackRequest, FallbackStore};
use super::plan::{ProgramRef, ShaderSource};
use crate::effects::program::{Call, GpuProgram};

pub(crate) const OUTPUT_CACHE_CAP: u64 = 32 << 20;
const OUTPUT_CACHE_ENTRIES: usize = 256;
const OUTPUT_KEY_BYTES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OutputKey {
    program: Key,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    fragment: Vec<u8>,
    vertex: Vec<u8>,
}

struct OutputEntry {
    target: Target,
    last_use: u64,
    pending: bool,
    done: Option<Arc<AtomicBool>>,
}

pub(crate) struct ShaderOutputCache {
    entries: HashMap<OutputKey, OutputEntry>,
    seen: HashMap<OutputKey, u64>,
    retired: Vec<(u64, Arc<AtomicBool>)>,
    budget: u64,
    frame: u64,
    recording: bool,
    hits: u64,
    admissions: u64,
    skipped: u64,
    evictions: u64,
}

impl Default for ShaderOutputCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            seen: HashMap::new(),
            retired: Vec::new(),
            budget: OUTPUT_CACHE_CAP,
            frame: 0,
            recording: false,
            hits: 0,
            admissions: 0,
            skipped: 0,
            evictions: 0,
        }
    }
}

impl ShaderOutputCache {
    pub(crate) fn bytes(&self) -> u64 {
        self.entries.values().map(|e| e.target.bytes()).sum::<u64>()
            + self
                .retired
                .iter()
                .filter(|(_, done)| !done.load(Ordering::Acquire))
                .map(|(bytes, _)| bytes)
                .sum::<u64>()
    }

    pub(crate) fn stats(&self) -> (u64, u64, u64, u64) {
        (self.hits, self.admissions, self.skipped, self.evictions)
    }

    fn remove(&mut self, key: &OutputKey) {
        let Some(entry) = self.entries.remove(key) else {
            return;
        };
        self.evictions += 1;
        if let Some(done) = entry.done.filter(|done| !done.load(Ordering::Acquire)) {
            self.retired.push((entry.target.bytes(), done));
        }
    }

    fn make_room(&mut self, bytes: u64, entries: usize, protect_current: bool) {
        while self.bytes().saturating_add(bytes) > self.budget
            || self.entries.len() + entries > OUTPUT_CACHE_ENTRIES
        {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| {
                    !protect_current
                        || (entry.last_use != self.frame
                            && entry
                                .done
                                .as_ref()
                                .is_none_or(|done| done.load(Ordering::Acquire)))
                })
                .min_by_key(|(_, entry)| entry.last_use)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else { break };
            self.remove(&oldest);
        }
    }

    pub(crate) fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes.min(OUTPUT_CACHE_CAP);
        self.make_room(0, 0, false);
        if self.budget == 0 {
            self.seen.clear();
        }
    }

    pub(crate) fn begin_frame(&mut self, enabled: bool) {
        // A recording that unwound before queue submission has no pixels
        // to publish, even if the renderer itself survived the unwind.
        self.entries.retain(|_, entry| !entry.pending);
        self.frame += 1;
        self.recording = enabled && self.budget > 0;
        self.retired
            .retain(|(_, done)| !done.load(Ordering::Acquire));
    }

    pub(crate) fn submitted(&mut self, queue: &wgpu::Queue, retain: bool) {
        if !std::mem::take(&mut self.recording)
            || !self
                .entries
                .values()
                .any(|entry| entry.last_use == self.frame)
        {
            return;
        }
        let done = Arc::new(AtomicBool::new(false));
        let signal = done.clone();
        queue.on_submitted_work_done(move || signal.store(true, Ordering::Release));
        let mut abandoned = Vec::new();
        for (key, entry) in &mut self.entries {
            if entry.last_use == self.frame {
                entry.done = Some(done.clone());
                if entry.pending && !retain {
                    abandoned.push(key.clone());
                }
                entry.pending = false;
            }
        }
        for key in abandoned {
            self.remove(&key);
        }
    }

    fn lookup(&mut self, key: &OutputKey) -> Option<Target> {
        let entry = self.entries.get_mut(key)?;
        entry.last_use = self.frame;
        self.hits += 1;
        Some(entry.target.clone())
    }

    fn admit(&mut self, key: OutputKey, device: &wgpu::Device) -> Option<Target> {
        let bytes = u64::from(key.width)
            * u64::from(key.height)
            * u64::from(key.format.block_copy_size(None)?);
        if bytes > self.budget || key.fragment.len() + key.vertex.len() > OUTPUT_KEY_BYTES {
            self.skipped += 1;
            return None;
        }
        if self
            .seen
            .get(&key)
            .is_none_or(|&frame| frame.saturating_add(1) != self.frame)
        {
            self.skipped += 1;
            if self.seen.len() == OUTPUT_CACHE_ENTRIES
                && let Some(oldest) = self
                    .seen
                    .iter()
                    .min_by_key(|(_, frame)| *frame)
                    .map(|(key, _)| key.clone())
            {
                self.seen.remove(&oldest);
            }
            self.seen.insert(key, self.frame);
            return None;
        }
        self.make_room(bytes, 1, true);
        if self.bytes() + bytes > self.budget || self.entries.len() == OUTPUT_CACHE_ENTRIES {
            self.skipped += 1;
            return None;
        }
        self.seen.remove(&key);
        let target = Target::new(
            device,
            "pm cached shader output",
            key.width,
            key.height,
            key.format,
            pm_gpu::target::working_usage(),
        );
        self.entries.insert(
            key,
            OutputEntry {
                target: target.clone(),
                last_use: self.frame,
                pending: true,
                done: None,
            },
        );
        self.admissions += 1;
        Some(target)
    }
}

fn active_bytes(
    block: Option<&pm_glsl::reflect::Block>,
    packed: Option<&[u8]>,
    inactive: &[usize],
) -> Vec<u8> {
    let Some(packed) = packed else {
        return Vec::new();
    };
    let mut key = packed.to_vec();
    if let Some(block) = block {
        for &index in inactive {
            // Bound the range by the next member, including for padded types.
            let member = &block.members[index];
            let start = member.offset as usize;
            let end = block
                .members
                .iter()
                .filter(|m| m.offset > member.offset)
                .map(|m| m.offset as usize)
                .min()
                .unwrap_or(key.len());
            if let Some(bytes) = key.get_mut(start..end) {
                bytes.fill(0);
            }
        }
    }
    key
}

/// GPU programs of shader-layer code kept by a renderer (by source key),
/// and the fallback runner's state.
#[derive(Default)]
pub struct ShaderState {
    pub(crate) output: ShaderOutputCache,
    programs: HashMap<Key, (GpuProgram, u64)>,
    /// Sources whose last build failed because of the compiler: built
    /// again after their backoff.
    retry: HashMap<Key, crate::effects::retry::Retry>,
    tick: u64,
    /// Whether a fallback runner draws programs the native pipeline cannot
    /// (the interactive engine with a viewer); see [`super::fallback`].
    pub fallback_runner: bool,
    /// Pixels the runner uploaded.
    pub fallback: FallbackStore,
    /// GPU copies of runner pixels by request key, with the pixels they
    /// were made from and when they were last drawn: a fallback layer that
    /// does not change (a still frame, a static layer during playback) is
    /// not uploaded again for every frame.
    uploads: HashMap<String, (Arc<FallbackPixels>, Target, u64)>,
    uploaded: u64,
    /// Requests asked for and not answered: their program (`program_id`).
    pub(crate) waiting: HashMap<String, String>,
    /// Programs the runner could not draw, by `program_id`, and whether
    /// the failure was reported.
    pub(crate) failed: HashMap<String, (super::FallbackFailure, bool)>,
    /// Tests: the device refuses every shader-layer pipeline, as Metal
    /// would a translation it cannot compile.
    #[cfg(test)]
    pub(crate) refuse_pipelines: bool,
    /// Tests: every program build fails with this, and builds are counted.
    #[cfg(test)]
    pub(crate) fail_builds: Option<Failure>,
    #[cfg(test)]
    pub(crate) builds: u32,
}

/// Programs kept before the least recently used is dropped (today's
/// `shaderUniformDefs` bound).
const PROGRAMS: usize = 256;

/// Runner pixel textures kept on the GPU (several fallback layers in one
/// frame stay resident; animated ones replace the oldest).
const UPLOADS: usize = 8;

impl ShaderState {
    /// Programs built.
    pub fn programs(&self) -> usize {
        self.programs.len()
    }

    /// Runner pixel uploads made so far (one per request drawn, not per
    /// frame drawing it).
    pub fn fallback_uploads(&self) -> u64 {
        self.uploaded
    }

    /// Forgets the runner's pixels and their GPU copies (the runner was
    /// detached).
    pub fn clear_fallback(&mut self) {
        self.fallback.clear();
        self.uploads.clear();
    }

    /// The runner's pixels for `request` on the GPU, uploaded once per
    /// pixels stored; `None` until they arrive.
    fn fallback_target(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: &FallbackRequest,
    ) -> Option<Target> {
        let pixels = self.fallback.get(&request.key)?;
        if !pixels.fits(request) {
            return None;
        }
        self.tick += 1;
        let tick = self.tick;
        if let Some((held, target, t)) = self.uploads.get_mut(&request.key)
            && Arc::ptr_eq(held, &pixels)
        {
            *t = tick;
            return Some(target.clone());
        }
        let target = upload(device, queue, &pixels, request.color_depth);
        self.uploaded += 1;
        self.uploads.remove(&request.key);
        if self.uploads.len() >= UPLOADS {
            let oldest = self
                .uploads
                .iter()
                .min_by_key(|(_, (_, _, t))| *t)
                .map(|(k, _)| k.clone())
                .expect("not empty");
            self.uploads.remove(&oldest);
        }
        self.uploads
            .insert(request.key.clone(), (pixels, target.clone(), tick));
        Some(target)
    }

    fn program(
        &mut self,
        device: &wgpu::Device,
        key: Key,
        prepared: &super::translate::Translated,
        format: wgpu::TextureFormat,
    ) -> Result<&mut GpuProgram, Failure> {
        self.tick += 1;
        let tick = self.tick;
        if !self.programs.contains_key(&key) {
            if let Some(f) = self
                .retry
                .get(&key)
                .and_then(crate::effects::retry::Retry::waiting)
            {
                return Err(f.clone());
            }
            let prepared = prepared.as_ref().as_ref().map_err(|f| f.clone())?;
            #[cfg(test)]
            let forced = {
                self.builds += 1;
                self.fail_builds.clone()
            };
            #[cfg(not(test))]
            let forced: Option<Failure> = None;
            let built = match forced {
                Some(f) => Err(f),
                None => prepared
                    .finish()
                    .and_then(|p| GpuProgram::new(device, p, "pm shader layer")),
            };
            let program = match built {
                Ok(program) => {
                    self.retry.remove(&key);
                    program
                }
                Err(f) => {
                    if f.stage.retryable() {
                        if self.retry.len() >= PROGRAMS {
                            self.retry.clear();
                        }
                        let next =
                            crate::effects::retry::Retry::after(self.retry.get(&key), f.clone());
                        self.retry.insert(key, next);
                    }
                    return Err(f);
                }
            };
            if self.programs.len() >= PROGRAMS {
                let oldest = self
                    .programs
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| *k)
                    .expect("not empty");
                self.programs.remove(&oldest);
            }
            self.programs.insert(key, (program, tick));
        }
        let entry = self.programs.get_mut(&key).expect("inserted above");
        entry.1 = tick;
        #[cfg(test)]
        if self.refuse_pipelines {
            self.programs.remove(&key);
            return Err(Failure::new(
                pm_glsl::FailureStage::NagaValidate,
                "the device refused the pipeline (test)",
            ));
        }
        if let Err(f) = entry.0.pipeline(device, format) {
            self.programs.remove(&key);
            return Err(f);
        }
        Ok(&mut self.programs.get_mut(&key).expect("kept").0)
    }
}

/// Why a source could not be drawn.
pub(crate) enum Missed {
    /// The watchdog turned the program off (an overrun or a runaway): the
    /// layer draws nothing, as a program that does not compile, and the
    /// watchdog's report names it.
    Disabled,
    /// The hang guard holds the program (a frame that drew it may be
    /// running away): the frame leaves it out and is not presented.
    Held,
    /// The program WebGL draws failed in the native pipeline at pipeline
    /// creation (the Metal compiler): it needs the fallback runner.
    NeedsFallback(String),
    /// The runner's pixels for this request have not arrived.
    Pending(Arc<FallbackRequest>),
}

/// One draw of a legacy program into `out` (cleared to transparent,
/// blending off) with the compositor's calls; samplers read `units` or the
/// empty unit (0, 0, 0, 1).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_program(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    program: &mut GpuProgram,
    calls: &[(String, Call)],
    blocks: [Option<&[u8]>; 2],
    units: &[Option<&wgpu::TextureView>],
    empty: &wgpu::TextureView,
    linear: &wgpu::Sampler,
    out: &Target,
) {
    let format = out.format();
    let buffer = |bytes: &[u8]| {
        let b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm shader layer uniforms"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&b, 0, bytes);
        b
    };
    let [fragment_buffer, vertex_buffer] = blocks.map(|bytes| bytes.map(buffer));
    let views: Vec<&wgpu::TextureView> = program
        .sampler_units(calls)
        .into_iter()
        .map(|unit| units.get(unit as usize).copied().flatten().unwrap_or(empty))
        .collect();
    let mut entries = Vec::new();
    if let (Some(block), Some(b)) = (&program.fragment_block, &fragment_buffer) {
        entries.push(wgpu::BindGroupEntry {
            binding: block.binding,
            resource: b.as_entire_binding(),
        });
    }
    for ((_, binding), view) in program.samplers.iter().zip(&views) {
        entries.push(wgpu::BindGroupEntry {
            binding: *binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: binding + pm_glsl::SAMPLER_BINDING_OFFSET,
            resource: wgpu::BindingResource::Sampler(linear),
        });
    }
    let fragment = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm shader layer fragment"),
        layout: &program.fragment_layout,
        entries: &entries,
    });
    let mut vertex_entries = Vec::new();
    if let (Some(block), Some(b)) = (&program.vertex_block, &vertex_buffer) {
        vertex_entries.push(wgpu::BindGroupEntry {
            binding: block.binding,
            resource: b.as_entire_binding(),
        });
    }
    let vertex = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm shader layer vertex"),
        layout: &program.vertex_layout,
        entries: &vertex_entries,
    });
    let pipeline = program
        .pipeline(device, format)
        .expect("built by the caller")
        .clone();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pm shader layer"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &out.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&pipeline);
    pass.set_bind_group(0, &fragment, &[]);
    pass.set_bind_group(1, &vertex, &[]);
    pass.draw(0..4, 0..1);
}

/// Uploads runner pixels at their working depth, in GL row order.
pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &FallbackPixels,
    color_depth: u8,
) -> Target {
    let rgba8 = color_depth == 8;
    let target = Target::new(
        device,
        "pm shader fallback pixels",
        pixels.width,
        pixels.height,
        match color_depth {
            8 => wgpu::TextureFormat::Rgba8Unorm,
            32 => wgpu::TextureFormat::Rgba32Float,
            _ => wgpu::TextureFormat::Rgba16Float,
        },
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let bytes = if rgba8 {
        pixels
            .rgba16f
            .iter()
            .map(|code| u8::try_from(*code).expect("RGBA8 fallback code"))
            .collect()
    } else if color_depth == 32 {
        super::fallback::float_bytes(pixels.rgba32f.as_ref().expect("RGBA32F fallback bits"))
    } else {
        super::fallback::half_bytes(&pixels.rgba16f)
    };
    queue.write_texture(
        target.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(
                pixels.width
                    * match color_depth {
                        8 => 4,
                        32 => 16,
                        _ => 8,
                    },
            ),
            rows_per_image: Some(pixels.height),
        },
        wgpu::Extent3d {
            width: pixels.width,
            height: pixels.height,
            depth_or_array_layers: 1,
        },
    );
    target
}

impl crate::exec::Exec<'_> {
    fn reject_invalid_fallback_code(&mut self, request: &FallbackRequest, layer: &str) -> bool {
        let Some(code) = self
            .shaders
            .fallback
            .get(&request.key)
            .and_then(|pixels| pixels.invalid_rgba8_code(request))
        else {
            return false;
        };
        let message = format!(
            "Layer {layer}: the fallback runner returned invalid RGBA8 channel code {code} (expected 0..=255)"
        );
        if self.effects.watchdog.policy() == crate::effects::watchdog::Policy::Job {
            self.effects.fallback.fatal.get_or_insert(message.clone());
        }
        self.effects
            .watchdog
            .report(pm_protocol::error::EngineError::new(
                pm_protocol::error::ErrorCode::InvalidMessage,
                pm_protocol::error::ErrorScope::Layer {
                    layer: layer.into(),
                },
                message,
            ));
        true
    }

    /// Draws a shader source into a new working target (GL row order,
    /// E-encoded legacy output), and whether the target is the pool's.
    pub(crate) fn shader_source(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &ShaderSource,
    ) -> Result<(Target, bool), Missed> {
        if let ProgramRef::Fallback(request) = &source.program {
            let request = super::fallback::at_color_depth(
                request,
                crate::shader_layers::fallback::color_depth(self.precision),
            );
            // A program the runner could not draw draws nothing, as one
            // WebGL refuses, and is reported once (a draw past the
            // runner's deadline by the watchdog).
            let program = super::fallback::program_id(&request);
            // Its last request failed for a transient reason (the runner's
            // context was lost): asked for again once the backoff passed,
            // drawing nothing until then, reported once.
            // A job's host already tried it again in place: the job stops.
            if self.effects.watchdog.policy() == crate::effects::watchdog::Policy::Job
                && let Some(retry) = self.effects.fallback.retries.last(&program)
            {
                let why = retry.failure.clone();
                self.effects.fallback.fatal.get_or_insert_with(|| {
                    format!(
                        "Layer {} needs the WebGL fallback runner, which could not draw it ({}: {}).",
                        source.layer, why.reason, why.message
                    )
                });
                return Err(Missed::Disabled);
            }
            let now = self.effects.watchdog.now();
            if let Some(crate::effects::fallback::RetryState::Wait { failure, report }) =
                self.effects.fallback.retries.state(&program, now)
            {
                if report {
                    self.effects.watchdog.report(pm_protocol::error::EngineError::new(
                        pm_protocol::error::ErrorCode::NotHeadlessRenderable,
                        pm_protocol::error::ErrorScope::Layer {
                            layer: source.layer.as_str().into(),
                        },
                        crate::effects::truncate(format!(
                            "Layer {} could not be drawn by the WebGL fallback runner for now ({}: {}); it draws nothing until the runner draws it again.",
                            source.layer, failure.reason, failure.message
                        )),
                    ));
                }
                return Err(Missed::Disabled);
            }
            if let Some((why, reported)) = self.shaders.failed.get_mut(&program) {
                let first = !std::mem::replace(reported, true);
                let why = why.clone();
                let span = crate::effects::watchdog::Span {
                    subject: crate::effects::watchdog::Subject::Definition {
                        kind: DefinitionKind::Layer,
                        id: format!("fallback:{program}").as_str().into(),
                    },
                    layer: source.layer.as_str().into(),
                    instance: None,
                    builtin: false,
                };
                if why.reason == "watchdog" {
                    self.effects.watchdog.runner_runaway(
                        &span,
                        &format!("the shader of layer {}", source.layer),
                        format!(
                            "The shader of layer {} did not finish drawing in the WebGL fallback runner ({}), so it is turned off until its code changes.",
                            source.layer, why.message
                        ),
                    );
                } else if self.effects.watchdog.policy() == crate::effects::watchdog::Policy::Job
                    && why.reason != "compile"
                {
                    self.effects.fallback.fatal.get_or_insert_with(|| {
                        format!(
                            "Layer {} needs the WebGL fallback runner, which could not draw it ({}: {}).",
                            source.layer, why.reason, why.message
                        )
                    });
                } else if first {
                    self.effects.watchdog.report(
                        pm_protocol::error::EngineError::new(
                            if why.reason == "unavailable" {
                                pm_protocol::error::ErrorCode::NotHeadlessRenderable
                            } else {
                                pm_protocol::error::ErrorCode::ShaderCompile
                            },
                            pm_protocol::error::ErrorScope::Layer {
                                layer: source.layer.as_str().into(),
                            },
                            crate::effects::truncate(format!(
                                "Layer {} could not be drawn by the WebGL fallback runner ({}: {}); it draws nothing.",
                                source.layer, why.reason, why.message
                            )),
                        ),
                    );
                }
                return Err(Missed::Disabled);
            }
            if self.reject_invalid_fallback_code(&request, &source.layer) {
                return Err(Missed::Disabled);
            }
            return match self
                .shaders
                .fallback_target(self.device, self.queue, &request)
            {
                Some(target) => Ok((target, false)),
                None => Err(Missed::Pending(Arc::new(request))),
            };
        }
        // The watchdog's subject: the code's source key, or the fragment
        // layer's definition.
        let span = match &source.program {
            ProgramRef::Code { key, .. } => {
                crate::effects::watchdog::Span::code(*key, &source.layer)
            }
            ProgramRef::Definition { id, .. } => crate::effects::watchdog::Span::definition(
                DefinitionKind::Layer,
                id,
                &source.layer,
                None,
                self.effects
                    .get(DefinitionKind::Layer, id)
                    .is_some_and(|d| {
                        d.origin == pm_protocol::definitions::DefinitionOrigin::BuiltIn
                    }),
            ),
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        let disabled = match &span.subject {
            crate::effects::watchdog::Subject::Code(key) => {
                self.effects.watchdog.is_code_disabled(key)
            }
            crate::effects::watchdog::Subject::Definition { kind, id } => {
                self.effects.watchdog.is_disabled(*kind, id.as_str())
            }
        };
        if disabled {
            return Err(Missed::Disabled);
        }
        if !span.builtin && self.effects.watchdog.hold(&span.subject) {
            return Err(Missed::Held);
        }
        let format = pm_gpu::target::working_format(self.precision);
        let linear = &self.pipelines.linear;
        let device = self.device;
        let queue = self.queue;
        let built = match &source.program {
            ProgramRef::Code {
                key,
                prepared,
                source: code,
            } => match self.shaders.program(device, *key, prepared, format) {
                Ok(_) => Ok(()),
                Err(f) => {
                    // Later plans route it to the runner (or, without one,
                    // to `Feature::ShaderFallback`); a busy, crashed or
                    // timed-out compiler only fails this attempt.
                    if !f.stage.retryable() {
                        super::translate::mark_failed(*key, f.clone());
                    }
                    Err((f, code.to_string(), "shader layer".to_owned()))
                }
            },
            ProgramRef::Definition { id, .. } => {
                match self
                    .effects
                    .program(device, DefinitionKind::Layer, id, format)
                {
                    Ok(_) => Ok(()),
                    // The definition is now `Status::NeedsFallback`.
                    Err(f) => {
                        let code = self
                            .effects
                            .get(DefinitionKind::Layer, id)
                            .and_then(|d| d.glsl.as_ref())
                            .map(|g| g.source.clone())
                            .unwrap_or_default();
                        Err((f, code, id.clone()))
                    }
                }
            }
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        // Built: fetched again below (a borrow returned from one match arm
        // would hold `self` in the other).
        if let Err((failure, code, what)) = built {
            if !self.shaders.fallback_runner {
                return Err(Missed::NeedsFallback(what));
            }
            // The device refused the translation on first contact: the
            // runner draws it from this frame on, as the plan would
            // have asked had the failure been known
            // (`super::plan::program_plan`, the same request key).
            let request = Arc::new(super::fallback::at_color_depth(
                &super::fallback::request(
                    source.layer.clone(),
                    code,
                    source.width,
                    source.height,
                    &source.calls,
                    &failure,
                ),
                crate::shader_layers::fallback::color_depth(self.precision),
            ));
            if self.reject_invalid_fallback_code(&request, &source.layer) {
                return Err(Missed::Disabled);
            }
            return match self
                .shaders
                .fallback_target(self.device, self.queue, &request)
            {
                Some(target) => Ok((target, false)),
                None => Err(Missed::Pending(request)),
            };
        }
        let cache = self.shaders.output.recording
            && self
                .effects
                .watchdog
                .permits_shader_output_reuse(&span.subject);
        let ShaderState {
            output, programs, ..
        } = &mut *self.shaders;
        // Build above also verifies the pipeline for this attachment format.
        let needs_dummy = match &source.program {
            ProgramRef::Code { key, .. } => !programs[key].0.samplers.is_empty(),
            ProgramRef::Definition { .. } => true,
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        let empty = needs_dummy.then(|| self.effects.dummy(device, queue, format));
        let program = match &source.program {
            ProgramRef::Code { key, .. } => &mut programs.get_mut(key).expect("built above").0,
            ProgramRef::Definition { id, .. } => self
                .effects
                .program(device, DefinitionKind::Layer, id, format)
                .expect("built above"),
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        let [fragment, vertex] = [&program.fragment_block, &program.vertex_block].map(|block| {
            block
                .as_ref()
                .map(|block| GpuProgram::block_bytes(block, &source.calls))
        });
        let cached = if cache
            && program.output_cache_safe
            && let ProgramRef::Code { key, .. } = &source.program
        {
            let key = OutputKey {
                program: *key,
                format,
                width: source.width,
                height: source.height,
                fragment: active_bytes(
                    program.fragment_block.as_ref(),
                    fragment.as_deref(),
                    &program.inactive_fragment,
                ),
                vertex: active_bytes(
                    program.vertex_block.as_ref(),
                    vertex.as_deref(),
                    &program.inactive_vertex,
                ),
            };
            if let Some(target) = output.lookup(&key) {
                // No work to time, but retain the other real draws in the hang guard.
                return Ok((target, false));
            }
            output.admit(key, device)
        } else {
            None
        };
        let pooled = cached.is_none();
        let out = cached.unwrap_or_else(|| {
            self.pool
                .take_working(device, source.width, source.height, self.precision)
        });
        // A sampler-free program never uses this argument.
        let empty = if program.samplers.is_empty() {
            out.view.clone()
        } else {
            empty.expect("sampler input").view
        };
        let queries = self.timer.as_mut().and_then(|t| t.begin(span));
        if let (Some(timer), Some((b, _))) = (self.timer.as_ref(), queries) {
            timer.mark(device, encoder, &out.view, b);
        }
        draw_program(
            device,
            queue,
            encoder,
            program,
            &source.calls,
            [fragment.as_deref(), vertex.as_deref()],
            &[],
            &empty,
            linear,
            &out,
        );
        if let (Some(timer), Some((_, e))) = (self.timer.as_ref(), queries) {
            timer.mark(device, encoder, &out.view, e);
        }
        Ok((out, pooled))
    }
}

#[cfg(test)]
mod tests {
    use pm_protocol::color::Precision;
    use pm_protocol::common::{Size, Ticks};
    use pm_protocol::error::ErrorCode;

    #[test]
    fn inactive_uniforms_canonicalize_without_changing_active_bits() {
        use pm_glsl::reflect::{Block, Member, Scalar, UniformType};

        let block = Block {
            size: 16,
            members: ["iTime", "uRotation"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| Member {
                    name: name.into(),
                    offset: i as u32 * 4,
                    ty: UniformType::Scalar(Scalar::Float),
                    active: i == 1,
                })
                .collect(),
            ..Default::default()
        };
        let mut packed = [0u8; 16];
        packed[..4].copy_from_slice(&2.0f32.to_le_bytes());
        packed[4..8].copy_from_slice(&(-0.0f32).to_le_bytes());
        assert_eq!(
            super::active_bytes(Some(&block), Some(&packed), &[]),
            packed
        );
        let key = super::active_bytes(Some(&block), Some(&packed), &[0]);
        assert_eq!(&key[..4], &[0; 4]);
        assert_eq!(&key[4..8], &(-0.0f32).to_le_bytes());
        packed[..4].copy_from_slice(&5.0f32.to_le_bytes());
        assert_eq!(super::active_bytes(Some(&block), Some(&packed), &[0]), key);
        packed[4..8].copy_from_slice(&0x7fc00042u32.to_le_bytes());
        let key = super::active_bytes(Some(&block), Some(&packed), &[0]);
        assert_eq!(&key[4..8], &0x7fc00042u32.to_le_bytes());
    }

    #[test]
    fn abandoned_shader_outputs_are_not_published() {
        use super::{OutputKey, ShaderOutputCache};
        let gpu = pm_gpu::Gpu::new("shader output cache submission").unwrap();
        let mut cache = ShaderOutputCache::default();
        let key = OutputKey {
            program: pm_glsl::Key([0; 32]),
            format: wgpu::TextureFormat::Rgba16Float,
            width: 4,
            height: 4,
            fragment: vec![],
            vertex: vec![],
        };
        cache.begin_frame(true);
        assert!(cache.admit(key.clone(), &gpu.device).is_none());
        cache.submitted(&gpu.queue, true);
        cache.begin_frame(true);
        let pending = cache.admit(key.clone(), &gpu.device).unwrap();
        assert_eq!(cache.lookup(&key).unwrap().texture, pending.texture);
        assert_eq!(cache.bytes(), 4 * 4 * 8);
        gpu.queue.submit([]);
        cache.submitted(&gpu.queue, false);
        cache.begin_frame(true);
        assert!(cache.lookup(&key).is_none());
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        assert_eq!(cache.bytes(), 0);
    }

    /// Draws of a shader layer during a busy compiler build its program
    /// once per backoff window, not once per draw.
    #[test]
    fn shader_layer_draws_during_a_busy_compiler_build_once_per_window() {
        let gpu = pm_gpu::Gpu::new("shader layer backoff test").unwrap();
        // The missing-definition checkerboard: a program that translates.
        let (source, offset) = super::super::missing_layer_program();
        let (key, verdict) = super::super::translate::verdict(&source, offset);
        let super::super::translate::Verdict::Ready(prepared) = verdict else {
            panic!("the fixture translates");
        };
        let mut state = super::ShaderState {
            fail_builds: Some(pm_glsl::Failure::new(
                pm_glsl::FailureStage::CompilerProcess,
                "the compiler crashed",
            )),
            ..Default::default()
        };
        for _ in 0..30 {
            assert!(
                state
                    .program(
                        &gpu.device,
                        key,
                        &prepared,
                        wgpu::TextureFormat::Rgba16Float
                    )
                    .is_err()
            );
        }
        assert_eq!(state.builds, 1);
        std::thread::sleep(std::time::Duration::from_millis(120));
        for _ in 0..30 {
            let _ = state.program(
                &gpu.device,
                key,
                &prepared,
                wgpu::TextureFormat::Rgba16Float,
            );
        }
        assert_eq!(state.builds, 2);
    }
    use serde_json::json;

    use super::super::FallbackPixels;
    use crate::{MapResolver, RenderError, RenderRequest, Renderer};

    fn document(code: &str) -> pm_model::Document {
        let ch = |v: serde_json::Value| json!({ "v": v, "kf": [], "expr": null });
        let layer = json!({
            "id": "probe", "type": "shader", "name": "probe", "from": 0, "dur": 10, "on": true,
            "lock": false, "threeD": false, "solo": false, "blend": "normal", "mblur": false,
            "parent": null, "group": null, "matteSource": null,
            "p": {
                "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
                "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
                "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
            },
            "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
            "d": { "code": code, "uniforms": {}, "w": 16, "h": 8 }
        });
        let doc = json!({
            "id": "P", "name": "p", "w": 16, "h": 8, "fps": 30, "dur": 10, "bg": "#000000",
            "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
            "compId": "P-root", "compName": "p", "markers": [], "work": [0, 10], "params": {},
            "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer],
            "assets": {}, "comps": {}
        });
        pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
    }

    fn render(r: &mut Renderer, doc: &pm_model::Document) -> Result<Vec<f32>, RenderError> {
        let mut request = RenderRequest::sized(Size::new(16, 8), Precision::Float16);
        request.background = false;
        let frame = r.render(
            doc,
            "P-root",
            Ticks::ZERO,
            &request,
            &MapResolver::default(),
        )?;
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        Ok(r.read_rgba(&frame).unwrap())
    }

    /// Metal refusing a translated program at pipeline creation (first
    /// contact, not known when the frame was planned): with a runner the
    /// layer asks it for its pixels in the same frame instead of routing
    /// the composition, is reported as not headless-renderable, and its
    /// pixels are uploaded once however many frames draw them.
    #[test]
    fn a_pipeline_the_device_refuses_goes_to_the_runner_on_first_contact() {
        // Unique code: the refusal is recorded for the process.
        let code = "void main(){ fragColor = vec4(0.25, 0.5, 0.75, 1.0) + vec4(uv * 0.0, 0.0, 0.0); } // exec refusal probe";
        let doc = document(code);
        let mut r = Renderer::headless().unwrap();
        r.set_shader_fallback_runner(true);
        r.shaders_mut().refuse_pipelines = true;
        assert!(
            r.layer_reports(&doc, "P-root").is_empty(),
            "compiles before the device sees it"
        );
        let requests = match render(&mut r, &doc) {
            Err(RenderError::ShaderFallbackPending(requests)) => requests,
            other => panic!("expected a fallback request, got {other:?}"),
        };
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.layer, "probe");
        assert_eq!(request.stage, "naga-validate");
        assert_eq!((request.width, request.height), (16, 8));
        // The plan asks for the same request from now on.
        r.shaders_mut().refuse_pipelines = false;
        let again = match render(&mut r, &doc) {
            Err(RenderError::ShaderFallbackPending(requests)) => requests,
            other => panic!("expected the same request, got {other:?}"),
        };
        assert_eq!(again[0].key, request.key);
        // Reported for the Shader panel.
        let reports = r.layer_reports(&doc, "P-root");
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
        assert!(
            reports[0].message.contains("headless"),
            "{}",
            reports[0].message
        );
        // The runner's pixels draw, uploaded once for every frame showing them.
        let px: Vec<u16> = (0..16 * 8)
            .flat_map(|_| [0x3800u16, 0x3400, 0x3000, 0x3c00])
            .collect();
        r.put_fallback_pixels(
            request.key.clone(),
            FallbackPixels {
                rgba32f: None,
                width: 16,
                height: 8,
                rgba16f: px,
            },
        );
        for _ in 0..3 {
            let frame = render(&mut r, &doc).unwrap();
            assert!(frame.chunks_exact(4).all(|p| p == [0.5, 0.25, 0.125, 1.0]));
        }
        assert_eq!(r.shader_state().fallback_uploads(), 1);
    }

    /// A lost runner context says nothing about the layer's code: it is
    /// asked for again (at once, then after a backoff, drawing nothing in
    /// between and reported once) and draws when the runner answers, as
    /// today's compositor draws again once its context is restored. A
    /// refusal (`compile`) is final.
    #[test]
    fn a_lost_runner_context_is_retried_and_the_layer_draws_again() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration, Instant};
        let code = "void main(){ fragColor = vec4(0.125, 0.5, 0.75, 1.0) + vec4(uv * 0.0, 0.0, 0.0); } // lost context probe";
        let doc = document(code);
        let mut r = Renderer::headless().unwrap();
        r.set_shader_fallback_runner(true);
        r.shaders_mut().refuse_pipelines = true;
        let base = Instant::now();
        let offset = Arc::new(AtomicU64::new(0));
        let o = offset.clone();
        r.effects_mut().watchdog.set_clock(Some(Arc::new(move || {
            base + Duration::from_millis(o.load(Ordering::Relaxed))
        })));
        let pending = |r: &mut Renderer| match render(r, &doc) {
            Err(RenderError::ShaderFallbackPending(requests)) => requests,
            other => panic!("expected a fallback request, got {other:?}"),
        };
        let lost = || super::super::FallbackFailure {
            reason: "contextLost".into(),
            message: "the WebGL context was lost".into(),
        };
        let first = pending(&mut r);
        r.shaders_mut().refuse_pipelines = false;
        assert_eq!(
            r.put_fallback_failure(&first[0].key, lost()),
            Some(Duration::ZERO)
        );
        let second = pending(&mut r);
        assert_eq!(second[0].key, first[0].key, "asked for again at once");
        assert_eq!(
            r.put_fallback_failure(&second[0].key, lost()),
            Some(Duration::from_millis(250))
        );
        for _ in 0..2 {
            let frame = render(&mut r, &doc).unwrap();
            assert!(frame.iter().all(|v| *v == 0.0), "draws nothing meanwhile");
        }
        let reports = r.watchdog_reports(true);
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
        offset.store(250, Ordering::Relaxed);
        let third = pending(&mut r);
        let px: Vec<u16> = (0..16 * 8)
            .flat_map(|_| [0x3800u16, 0x3400, 0x3000, 0x3c00])
            .collect();
        r.put_fallback_pixels(
            third[0].key.clone(),
            FallbackPixels {
                rgba32f: None,
                width: 16,
                height: 8,
                rgba16f: px,
            },
        );
        let frame = render(&mut r, &doc).unwrap();
        assert!(frame.chunks_exact(4).all(|p| p == [0.5, 0.25, 0.125, 1.0]));
        assert!(r.fallback_retry_at().is_none(), "the streak is over");
    }

    /// Without a runner (every headless host) the same refusal fails the
    /// layer as `Feature::ShaderFallback`.
    #[test]
    fn a_pipeline_the_device_refuses_routes_without_a_runner() {
        let doc = document(
            "void main(){ fragColor = vec4(0.5) + vec4(uv * 0.0, 0.0, 0.0); } // headless refusal probe",
        );
        let mut r = Renderer::headless().unwrap();
        r.shaders_mut().refuse_pipelines = true;
        match render(&mut r, &doc) {
            Err(RenderError::Unsupported(list)) => assert!(
                list.iter()
                    .any(|u| matches!(&u.feature, crate::capability::Feature::ShaderFallback(_))),
                "{list:?}"
            ),
            other => panic!("expected routing, got {other:?}"),
        }
    }
}
