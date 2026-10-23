//! Effects and transitions only the decision 19 fallback runner draws
//! (`effects::fallback`; amendment 21: drawn, never routed): the request
//! carries the step's textures (input, backdrop; a transition's two frames),
//! which unit samples which, every pass's calls with `setParam`'s aliases;
//! the runner's pixels continue the chain; a job renderer waits on a
//! fallback host; failures skip (WebGL refuses), turn off (the runner's
//! deadline), or stop a job (no runner); transient failures (a lost
//! context) are retried, never kept, and the program draws again. The runner here is a CPU stand-in
//! that checks what it is sent; WebGL itself is `effects_webgl.rs`
//! `fallback_effects_match_webgl` with the app's runner in Chromium.

use std::sync::{Arc, Mutex};

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_protocol::definitions::{
    DefinitionKind, DefinitionOrigin, DefinitionSetSnapshot, DefinitionSnapshot,
};
use pm_protocol::error::{ErrorCode, ErrorScope};
use pm_render::effects::Status;
use pm_render::effects::watchdog::Policy;
use pm_render::shader_layers::{FallbackFailure, FallbackHost, FallbackJob, FallbackPixels};
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer, builtin_definitions};
use serde_json::{Value, json};

static GPU: Mutex<()> = Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn layer(id: &str, color: &str, fx: Value, transition_in: Value) -> Value {
    json!({
        "id": id, "type": "solid", "name": id, "from": 0, "dur": 10, "on": true, "lock": false,
        "threeD": false, "solo": false, "blend": "normal", "mblur": false, "parent": null,
        "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(8)),
            "position.y": ch(json!(4)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": fx, "transitionIn": transition_in, "transitionOut": null, "masks": [],
        "d": { "color": color, "w": 40, "h": 20 }
    })
}

fn document(layers: Vec<Value>) -> pm_model::Document {
    document_sized(layers, 64, 32)
}

fn document_sized(layers: Vec<Value>, w: u32, h: u32) -> pm_model::Document {
    let doc = json!({
        "id": "P", "name": "p", "w": w, "h": h, "fps": 30, "dur": 10, "bg": "#202020",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
        "compId": "P-root", "compName": "p", "markers": [], "work": [0, 10], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    });
    pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
}

/// The effect stack: a solid with `fx` over a background solid (the
/// first layer is the top one).
fn stack(fx: Value, transition_in: Value) -> pm_model::Document {
    document(vec![
        layer("top", "#E83A5E", fx, transition_in),
        layer("bg", "#3366CC", json!([]), Value::Null),
    ])
}

/// `base` copied as `id`, its program sampling a `samplerCube` (WebGL
/// samples (0, 0, 0, 1) from it; the engine's bind check refuses it: only
/// the runner draws it), from a Store extension.
fn cube_copy(set: &DefinitionSetSnapshot, base: &str, id: &str, passes: u8) -> DefinitionSnapshot {
    let mut d = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == base)
        .unwrap()
        .clone();
    d.id = id.into();
    d.origin = DefinitionOrigin::Store;
    let glsl = d.glsl.as_mut().unwrap();
    glsl.source = glsl.source.replacen(
        "void main(){",
        "uniform samplerCube u_env;\nvoid main(){",
        1,
    );
    let end = glsl.source.rfind('}').unwrap();
    glsl.source
        .insert_str(end, "  o += texture(u_env, vec3(0.0, 0.0, 1.0)) * 0.0;\n");
    if d.kind == DefinitionKind::Effect {
        d.flags.passes = passes;
        d.flags.keep_orig = true;
        d.flags.backdrop = true;
    }
    d
}

fn definitions() -> DefinitionSetSnapshot {
    let mut set = builtin_definitions();
    let effect = cube_copy(&set, "invert", "acme.cube-invert", 2);
    let transition = cube_copy(&set, "crossfade", "acme.cube-fade", 1);
    set.definitions.push(effect);
    set.definitions.push(transition);
    set
}

/// The CPU stand-in: checks the request and answers half of each texel's
/// input plus half of its backdrop (an effect) or of the destination (a
/// transition), and records what it was sent.
#[derive(Default)]
struct Cpu {
    seen: Mutex<Vec<FallbackJob>>,
    fail: Mutex<Option<FallbackFailure>>,
    /// With `fail`: only this many calls fail (`None`: every call).
    fail_times: Mutex<Option<u32>>,
}

impl FallbackHost for Cpu {
    fn run(&self, job: &FallbackJob) -> Result<FallbackPixels, FallbackFailure> {
        self.seen.lock().unwrap().push(job.clone());
        let mut times = self.fail_times.lock().unwrap();
        let failing = match times.as_mut() {
            None => true,
            Some(0) => false,
            Some(n) => {
                *n -= 1;
                true
            }
        };
        drop(times);
        if let Some(f) = self.fail.lock().unwrap().clone()
            && failing
        {
            return Err(f);
        }
        let r = &job.request;
        assert_eq!(r.textures as usize, job.textures.len());
        let n = (r.width * r.height * 4) as usize;
        let a = &job.textures[0].rgba16f;
        let b = job.textures.get(1).map(|t| &t.rgba16f);
        let rgba16f = (0..n)
            .map(|i| {
                let x = pm_gpu::readback::half_to_f32(a[i]);
                let y = b.map_or(0.0, |b| pm_gpu::readback::half_to_f32(b[i]));
                pm_gpu::readback::f32_to_half(if i % 4 == 3 {
                    x.max(y)
                } else {
                    0.5 * x + 0.5 * y
                })
            })
            .collect();
        Ok(FallbackPixels {
            rgba32f: None,
            width: r.width,
            height: r.height,
            rgba16f,
        })
    }
}

fn request() -> RenderRequest {
    let mut r = RenderRequest::sized(Size::new(64, 32), Precision::Float16);
    r.background = true;
    r
}

fn render(r: &mut Renderer, doc: &pm_model::Document) -> Result<Vec<f32>, RenderError> {
    let frame = r.render(
        doc,
        "P-root",
        Ticks::new(0).unwrap(),
        &request(),
        &MapResolver::default(),
    )?;
    Ok(r.read_rgba(&frame).unwrap())
}

fn renderer() -> Renderer {
    let mut r = Renderer::headless().unwrap();
    assert!(
        r.set_definitions(&definitions())
            .unwrap()
            .iter()
            .all(|e| e.code != ErrorCode::ShaderCompile)
    );
    for (kind, id) in [
        (DefinitionKind::Effect, "acme.cube-invert"),
        (DefinitionKind::Transition, "acme.cube-fade"),
    ] {
        assert!(
            matches!(r.effects().status(kind, id), Some(Status::NeedsFallback(_))),
            "{id}: {:?}",
            r.effects().status(kind, id)
        );
    }
    r
}

fn fx(ty: &str) -> Value {
    json!([{ "id": "f1", "type": ty, "on": true, "p": { "amount": ch(json!(100)) } }])
}

#[test]
fn without_a_runner_a_fallback_effect_routes_as_before() {
    let _gpu = gpu();
    let mut r = renderer();
    match render(&mut r, &stack(fx("acme.cube-invert"), Value::Null)) {
        Err(RenderError::Unsupported(list)) => {
            assert!(
                list.iter()
                    .any(|u| u.feature
                        == pm_render::Feature::ShaderFallback("acme.cube-invert".into())),
                "{list:?}"
            )
        }
        other => panic!("expected routing, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn the_viewer_asks_the_runner_with_the_effects_textures_and_draws_its_pixels() {
    let _gpu = gpu();
    let mut r = renderer();
    r.set_shader_fallback_runner(true);
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let plain = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    let requests = match render(&mut r, &doc) {
        Err(RenderError::ShaderFallbackPending(requests)) => requests,
        other => panic!("expected a pending request, got {:?}", other.map(|_| ())),
    };
    assert_eq!(requests.len(), 1);
    let q = &requests[0];
    assert_eq!(q.layer, "top");
    assert_eq!(
        (q.textures, q.more_passes.len()),
        (2, 1),
        "input and backdrop, two passes"
    );
    let units: Vec<(u32, u32, bool)> = q
        .units
        .iter()
        .map(|u| (u.unit, u.texture, u.chain))
        .collect();
    assert_eq!(units, vec![(0, 0, true), (1, 0, false), (2, 1, false)]);
    assert_eq!(q.stage, "bind");
    assert!(q.fragment.contains("samplerCube u_env"));
    // setParam's rule travels with the call: by name, else the alias.
    let amount = q
        .calls
        .iter()
        .find(|c| c.name == "u_amount")
        .expect("amount");
    assert_eq!(amount.alias.as_deref(), Some("u_p0"));
    assert_eq!(amount.v, vec![100.0]);
    let pass: Vec<f64> = std::iter::once(&q.calls)
        .chain(&q.more_passes)
        .map(|c| c.iter().find(|c| c.name == "u_pass").unwrap().v[0])
        .collect();
    assert_eq!(pass, vec![0.0, 1.0]);
    let textures = r
        .fallback_textures(&q.key)
        .expect("textures held for the runner");
    assert_eq!(textures.len(), 2);
    assert!(
        textures
            .iter()
            .all(|t| (t.width, t.height) == (q.width, q.height))
    );
    // The runner answers: the frame draws its pixels.
    let cpu = Cpu::default();
    let job = FallbackJob {
        request: q.clone(),
        textures,
    };
    let pixels = cpu.run(&job).unwrap();
    r.put_fallback_pixels(q.key.clone(), pixels);
    let drawn = render(&mut r, &doc).unwrap();
    assert_ne!(drawn, plain);
    // The same pixels in the same frame: the same request, answered.
    assert_eq!(render(&mut r, &doc).unwrap(), drawn);
    assert_eq!(r.effects().fallback.waiting(), 0);
}

#[test]
fn a_job_renderer_waits_on_its_fallback_host_and_matches_the_viewer() {
    let _gpu = gpu();
    // The viewer, answered by hand.
    let mut viewer = renderer();
    viewer.set_shader_fallback_runner(true);
    let doc = stack(
        fx("acme.cube-invert"),
        json!({ "type": "acme.cube-fade", "dur": 1, "p": {} }),
    );
    let cpu = Arc::new(Cpu::default());
    let expected = loop {
        match render(&mut viewer, &doc) {
            Ok(px) => break px,
            Err(RenderError::ShaderFallbackPending(requests)) => {
                for q in requests {
                    let textures = viewer.fallback_textures(&q.key).unwrap();
                    let px = cpu
                        .run(&FallbackJob {
                            request: q.clone(),
                            textures,
                        })
                        .unwrap();
                    viewer.put_fallback_pixels(q.key, px);
                }
            }
            Err(e) => panic!("{e}"),
        }
    };
    // The job renderer: one render call, through the host.
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    job.set_fallback_host(Some(host.clone()));
    assert!(
        job.unsupported_with_media(&doc, "P-root", &MapResolver::default())
            .is_empty(),
        "a job with a host is not routed"
    );
    assert_eq!(render(&mut job, &doc).unwrap(), expected);
    let seen = host.seen.lock().unwrap();
    let kinds: Vec<u32> = seen.iter().map(|j| j.request.textures).collect();
    assert_eq!(
        kinds,
        vec![2, 2],
        "the effect (input, backdrop), then the transition (from, to)"
    );
    assert_eq!(seen[1].request.more_passes.len(), 0);
    assert!(seen[1].request.calls.iter().any(|c| c.name == "u_prog"));
}

#[test]
fn a_program_the_runner_refuses_is_skipped_and_reported_once() {
    let _gpu = gpu();
    let mut r = renderer();
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(FallbackFailure {
        reason: "compile".into(),
        message: "ERROR: 0:3: nope".into(),
    });
    r.set_fallback_host(Some(host.clone()));
    let without = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    assert_eq!(
        render(&mut r, &doc).unwrap(),
        without,
        "skipped, as WebGL skips it"
    );
    assert_eq!(render(&mut r, &doc).unwrap(), without);
    assert_eq!(host.seen.lock().unwrap().len(), 1, "not asked for again");
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::ShaderCompile);
    assert!(
        matches!(&reports[0].scope, ErrorScope::Effect { definition, .. } if definition.as_str() == "acme.cube-invert")
    );
    assert!(
        reports[0].message.contains("nope"),
        "{}",
        reports[0].message
    );
}

#[test]
fn a_draw_past_the_runners_deadline_is_turned_off_and_stops_a_job() {
    let _gpu = gpu();
    let deadline = FallbackFailure {
        reason: "watchdog".into(),
        message: "its draw did not finish within 10 s".into(),
    };
    // The viewer: turned off and reported as a watchdog timeout.
    let mut r = renderer();
    r.set_shader_fallback_runner(true);
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!()
    };
    r.put_fallback_failure(&q[0].key, deadline.clone());
    let without = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    assert_eq!(render(&mut r, &doc).unwrap(), without);
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::WatchdogTimeout);
    assert!(
        r.effects()
            .watchdog
            .is_disabled(DefinitionKind::Effect, "acme.cube-invert")
    );
    // A job: stopped with a named error.
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(deadline);
    job.set_fallback_host(Some(host));
    match render(&mut job, &doc) {
        Err(RenderError::Watchdog(m)) => assert!(m.contains("acme.cube-invert"), "{m}"),
        other => panic!("expected the job to stop, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_job_without_a_runner_fails_naming_the_program_instead_of_skipping_it() {
    let _gpu = gpu();
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(FallbackFailure {
        reason: "unavailable".into(),
        message: "no window is open".into(),
    });
    job.set_fallback_host(Some(host));
    match render(&mut job, &stack(fx("acme.cube-invert"), Value::Null)) {
        Err(RenderError::Fallback(m)) => {
            assert!(
                m.contains("acme.cube-invert") && m.contains("no window"),
                "{m}"
            )
        }
        other => panic!("expected a named failure, got {:?}", other.map(|_| ())),
    }
}

fn lost() -> FallbackFailure {
    FallbackFailure {
        reason: "contextLost".into(),
        message: "the WebGL context was lost while the program was drawn".into(),
    }
}

/// A clock the test moves forward (the watchdog's, also the runner's retry
/// clock).
fn movable_clock() -> (
    Arc<std::sync::atomic::AtomicU64>,
    pm_render::effects::watchdog::Clock,
) {
    let base = std::time::Instant::now();
    let offset = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let o = offset.clone();
    (
        offset,
        Arc::new(move || {
            base + std::time::Duration::from_millis(o.load(std::sync::atomic::Ordering::Relaxed))
        }),
    )
}

/// Answers the viewer's pending requests with the CPU stand-in.
fn answer(r: &mut Renderer, requests: Vec<pm_render::shader_layers::FallbackRequest>) {
    let cpu = Cpu::default();
    for q in requests {
        let textures = r.fallback_textures(&q.key).unwrap_or_default();
        let px = cpu
            .run(&FallbackJob {
                request: q.clone(),
                textures,
            })
            .unwrap();
        r.put_fallback_pixels(q.key, px);
    }
}

#[test]
fn a_lost_context_is_not_blamed_on_the_program_and_it_draws_again() {
    // Today's compositor clears its errors and programs when its context is
    // restored and draws again (`compositor.ts`, `webglcontextrestored`):
    // a GPU-process restart or an unrelated context loss never turns a
    // program off.
    let _gpu = gpu();
    let mut r = renderer();
    r.set_shader_fallback_runner(true);
    let (offset, clock) = movable_clock();
    r.effects_mut().watchdog.set_clock(Some(clock));
    let without = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("expected a request")
    };
    // The first loss: asked for again at once (the runner made a new
    // context), nothing skipped, nothing reported.
    assert_eq!(
        r.put_fallback_failure(&q[0].key, lost()),
        Some(std::time::Duration::ZERO)
    );
    let Err(RenderError::ShaderFallbackPending(again)) = render(&mut r, &doc) else {
        panic!("expected the program to be asked for again")
    };
    assert_eq!(again[0].key, q[0].key);
    assert!(r.watchdog_reports(true).is_empty());
    // A second loss in a row: the frame draws without it while the runner
    // recovers (as today's canvas is blank until its context is restored),
    // reported once, and asked for again after the backoff.
    let delay = r.put_fallback_failure(&again[0].key, lost()).unwrap();
    assert_eq!(delay, std::time::Duration::from_millis(250));
    assert_eq!(render(&mut r, &doc).unwrap(), without);
    assert_eq!(render(&mut r, &doc).unwrap(), without);
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
    assert!(
        reports[0].message.contains("contextLost"),
        "{}",
        reports[0].message
    );
    assert!(
        !r.effects()
            .watchdog
            .is_disabled(DefinitionKind::Effect, "acme.cube-invert"),
        "never turned off"
    );
    assert!(r.fallback_retry_at().is_some());
    offset.store(250, std::sync::atomic::Ordering::Relaxed);
    let Err(RenderError::ShaderFallbackPending(third)) = render(&mut r, &doc) else {
        panic!("expected the program to be asked for again after the backoff")
    };
    // The runner draws it: the streak is over and the frame has it.
    answer(&mut r, third);
    let drawn = loop {
        match render(&mut r, &doc) {
            Ok(px) => break px,
            Err(RenderError::ShaderFallbackPending(q)) => answer(&mut r, q),
            Err(e) => panic!("{e}"),
        }
    };
    assert_ne!(drawn, without, "drawn again");
    assert!(r.fallback_retry_at().is_none());
    assert!(r.watchdog_reports(true).is_empty());
}

#[test]
fn a_job_tries_a_lost_context_again_and_draws_the_program() {
    let _gpu = gpu();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let mut expected_r = renderer();
    expected_r.effects_mut().watchdog.set_policy(Policy::Job);
    expected_r.set_fallback_host(Some(Arc::new(Cpu::default())));
    let expected = render(&mut expected_r, &doc).unwrap();
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(lost());
    *host.fail_times.lock().unwrap() = Some(2);
    job.set_fallback_host(Some(host.clone()));
    assert_eq!(
        render(&mut job, &doc).unwrap(),
        expected,
        "drawn after two lost contexts, never skipped"
    );
    assert_eq!(host.seen.lock().unwrap().len(), 3);
    assert!(job.watchdog_reports(true).is_empty());
}

fn still_compiling() -> FallbackFailure {
    FallbackFailure {
        reason: "deadline".into(),
        message: "the program has not finished compiling after 10 s; it is still compiling and is drawn once it has".into(),
    }
}

#[test]
fn a_job_waits_for_a_program_webgl_takes_long_to_compile() {
    // Today's exporter waits for the compile: a job does too, past the
    // attempts a lost context gets (each answer is one 10 s deadline).
    let _gpu = gpu();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let mut expected_r = renderer();
    expected_r.effects_mut().watchdog.set_policy(Policy::Job);
    expected_r.set_fallback_host(Some(Arc::new(Cpu::default())));
    let expected = render(&mut expected_r, &doc).unwrap();
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(still_compiling());
    *host.fail_times.lock().unwrap() = Some(12);
    job.set_fallback_host(Some(host.clone()));
    assert_eq!(render(&mut job, &doc).unwrap(), expected);
    assert_eq!(host.seen.lock().unwrap().len(), 13);
}

#[test]
fn a_job_whose_program_never_finishes_compiling_stops_naming_it() {
    let _gpu = gpu();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(still_compiling());
    job.set_fallback_host(Some(host.clone()));
    match render(&mut job, &doc) {
        Err(RenderError::Fallback(m)) => assert!(
            m.contains("acme.cube-invert")
                && m.contains("still compiling")
                && m.contains("60 waits for its compile"),
            "{m}"
        ),
        other => panic!("expected a named failure, got {:?}", other.map(|_| ())),
    }
    assert_eq!(
        host.seen.lock().unwrap().len() as u32,
        pm_render::HOST_COMPILE_WAITS + 3
    );
}

#[test]
fn a_job_whose_runner_keeps_losing_its_context_stops_naming_the_program() {
    let _gpu = gpu();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(lost());
    job.set_fallback_host(Some(host.clone()));
    match render(&mut job, &doc) {
        Err(RenderError::Fallback(m)) => assert!(
            m.contains("acme.cube-invert") && m.contains("contextLost") && m.contains("3 attempts"),
            "{m}"
        ),
        other => panic!("expected a named failure, got {:?}", other.map(|_| ())),
    }
    assert_eq!(host.seen.lock().unwrap().len(), 3);
    // The next frame tries again from scratch: the runner is back.
    *host.fail.lock().unwrap() = None;
    let mut fresh = renderer();
    fresh.effects_mut().watchdog.set_policy(Policy::Job);
    fresh.set_fallback_host(Some(Arc::new(Cpu::default())));
    assert_eq!(
        render(&mut job, &doc).unwrap(),
        render(&mut fresh, &doc).unwrap()
    );
}

fn render_sized(
    r: &mut Renderer,
    doc: &pm_model::Document,
    w: u32,
    h: u32,
) -> Result<Vec<f32>, RenderError> {
    let mut q = RenderRequest::sized(Size::new(w, h), Precision::Float16);
    q.background = true;
    let frame = r.render(
        doc,
        "P-root",
        Ticks::new(0).unwrap(),
        &q,
        &MapResolver::default(),
    )?;
    Ok(r.read_rgba(&frame).unwrap())
}

fn job_renderer(host: Arc<Cpu>) -> Renderer {
    let mut job = renderer();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    job.set_fallback_host(Some(host));
    job
}

#[test]
fn a_job_draws_a_runner_step_wider_than_4096_px_at_the_compositions_size() {
    // Today's `runEffects` draws at the composition's size (up to 16,384
    // px); the 4,096 px cap is the shader layers' alone.
    let _gpu = gpu();
    let (w, h) = (4500, 8);
    let doc = |fx: Value| {
        document_sized(
            vec![
                layer("top", "#E83A5E", fx, Value::Null),
                layer("bg", "#3366CC", json!([]), Value::Null),
            ],
            w,
            h,
        )
    };
    let host = Arc::new(Cpu::default());
    let mut job = job_renderer(host.clone());
    let drawn = render_sized(&mut job, &doc(fx("acme.cube-invert")), w, h).unwrap();
    let without = render_sized(&mut job, &doc(json!([])), w, h).unwrap();
    assert_ne!(drawn, without, "the step is drawn");
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].request.width, seen[0].request.height), (w, h));
}

fn too_large() -> FallbackFailure {
    FallbackFailure {
        reason: "tooLarge".into(),
        message: "target 64×32 is larger than this GPU's 16 px WebGL texture limit".into(),
    }
}

#[test]
fn a_target_past_the_gpus_webgl_limit_is_a_final_named_failure() {
    let _gpu = gpu();
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    // A job: stopped at once, naming the program and the limit; never
    // retried as a transient failure.
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(too_large());
    let mut job = job_renderer(host.clone());
    match render(&mut job, &doc) {
        Err(RenderError::Fallback(m)) => assert!(
            m.contains("acme.cube-invert")
                && m.contains("tooLarge")
                && m.contains("WebGL texture limit"),
            "{m}"
        ),
        other => panic!("expected a named failure, got {:?}", other.map(|_| ())),
    }
    assert_eq!(host.seen.lock().unwrap().len(), 1, "final: one attempt");
    // The viewer: kept (no retry), skipped and reported once.
    let mut r = renderer();
    r.set_shader_fallback_runner(true);
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("expected a request")
    };
    assert_eq!(r.put_fallback_failure(&q[0].key, too_large()), None);
    let without = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    assert_eq!(render(&mut r, &doc).unwrap(), without);
    assert_eq!(render(&mut r, &doc).unwrap(), without);
    assert!(r.fallback_retry_at().is_none(), "not retried");
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::BudgetExceeded);
    assert!(
        reports[0].message.contains("acme.cube-invert"),
        "{}",
        reports[0].message
    );
    // The failure is the size's: the same program at another size (a
    // lower preview resolution) is asked for.
    let small = document_sized(
        vec![
            layer("top", "#E83A5E", fx("acme.cube-invert"), Value::Null),
            layer("bg", "#3366CC", json!([]), Value::Null),
        ],
        48,
        24,
    );
    assert!(matches!(
        render_sized(&mut r, &small, 48, 24),
        Err(RenderError::ShaderFallbackPending(_))
    ));
}

#[test]
fn a_job_draws_more_runner_steps_than_a_fixed_round_limit() {
    // Each runner step waits for the one before it (its request covers the
    // textures the earlier step's pixels feed), one round each: twenty
    // independent layers with a fallback-only effect take twenty rounds,
    // each asking for a new request, and the frame is delivered.
    let _gpu = gpu();
    let mut layers: Vec<Value> = (0..20)
        .map(|i| {
            let mut l = layer(
                &format!("l{i}"),
                "#E83A5E",
                fx("acme.cube-invert"),
                Value::Null,
            );
            l["p"]["position.x"] = ch(json!(i * 2));
            l
        })
        .collect();
    layers.push(layer("bg", "#3366CC", json!([]), Value::Null));
    let doc = document(layers);
    let host = Arc::new(Cpu::default());
    let mut job = job_renderer(host.clone());
    let drawn = render(&mut job, &doc).unwrap();
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 20, "one request per step, none asked again");
    let keys: std::collections::HashSet<_> = seen.iter().map(|j| j.request.key.clone()).collect();
    assert_eq!(keys.len(), 20);
    drop(seen);
    // The viewer, answered round by round, draws the same frame.
    let mut viewer = renderer();
    viewer.set_shader_fallback_runner(true);
    let expected = loop {
        match render(&mut viewer, &doc) {
            Ok(px) => break px,
            Err(RenderError::ShaderFallbackPending(q)) => answer(&mut viewer, q),
            Err(e) => panic!("{e}"),
        }
    };
    assert_eq!(drawn, expected);
}

/// A copy of the built-in Invert as a Store effect `id` that translates
/// natively, listed as ANGLE-only (`pm_render::effects::ANGLE_ONLY`).
fn angle_only_renderer(id: &str) -> Renderer {
    let mut set = builtin_definitions();
    let mut d = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    d.id = id.into();
    d.origin = DefinitionOrigin::Store;
    let fingerprint = pm_render::glsl::body_fingerprint(d.glsl.as_ref().unwrap());
    set.definitions.push(d);
    let mut r = Renderer::headless().unwrap();
    r.effects_mut()
        .add_angle_only(fingerprint, "a test program WebGL draws differently");
    assert!(r.set_definitions(&set).unwrap().is_empty());
    assert!(matches!(
        r.effects().status(DefinitionKind::Effect, id),
        Some(Status::Ready)
    ));
    assert!(r.effects().angle_only(DefinitionKind::Effect, id));
    r
}

#[test]
fn an_angle_only_program_draws_through_the_runner_where_one_is_attached() {
    let _gpu = gpu();
    let doc = stack(fx("acme.glass"), Value::Null);
    // Without a runner (pm-cli): the native translation draws, nothing is
    // routed or reported.
    let mut headless = angle_only_renderer("acme.glass");
    let native = render(&mut headless, &doc).unwrap();
    assert!(headless.watchdog_reports(true).is_empty());
    let plain = render(&mut headless, &stack(json!([]), Value::Null)).unwrap();
    assert_ne!(native, plain);
    // The viewer: asked of the runner, with the effect's textures.
    let mut viewer = angle_only_renderer("acme.glass");
    viewer.set_shader_fallback_runner(true);
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut viewer, &doc) else {
        panic!("expected a runner request")
    };
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].stage, "angle-parity");
    assert!(
        q[0].message.contains("draws differently from WebGL"),
        "{}",
        q[0].message
    );
    // The runner's pixels are drawn (the CPU stand-in's, not the native
    // program's).
    let drawn = loop {
        match render(&mut viewer, &doc) {
            Ok(px) => break px,
            Err(RenderError::ShaderFallbackPending(q)) => answer(&mut viewer, q),
            Err(e) => panic!("{e}"),
        }
    };
    assert_ne!(drawn, native);
    // A job: through its host, the same frame as the viewer.
    let host = Arc::new(Cpu::default());
    let mut job = angle_only_renderer("acme.glass");
    job.effects_mut().watchdog.set_policy(Policy::Job);
    job.set_fallback_host(Some(host.clone()));
    assert_eq!(render(&mut job, &doc).unwrap(), drawn);
    assert_eq!(host.seen.lock().unwrap().len(), 1);
    // The built-in it was copied from stays native everywhere.
    let mut builtin = angle_only_renderer("acme.glass");
    builtin.set_shader_fallback_runner(true);
    assert!(render(&mut builtin, &stack(fx("invert"), Value::Null)).is_ok());
}

/// Answers every request with the same pixels at once: the engine's own
/// cost of a fallback step (submit, readback, keying, upload) without a
/// runner's draw.
struct Instant0(Mutex<Option<FallbackPixels>>, Mutex<(u32, u32, usize)>);

impl FallbackHost for Instant0 {
    fn run(&self, job: &FallbackJob) -> Result<FallbackPixels, FallbackFailure> {
        let r = &job.request;
        *self.1.lock().unwrap() = (r.width, r.height, job.textures.len());
        let mut cached = self.0.lock().unwrap();
        if cached
            .as_ref()
            .is_none_or(|p| (p.width, p.height) != (r.width, r.height))
        {
            *cached = Some(FallbackPixels {
                rgba32f: None,
                width: r.width,
                height: r.height,
                rgba16f: vec![0x3800; (r.width * r.height * 4) as usize],
            });
        }
        Ok(cached.clone().unwrap())
    }
}

/// The engine side of an animated fallback effect at full HD: every frame
/// is a new request (its time differs), read back, keyed, and its pixels
/// uploaded. Run: `cargo test -p pm-render --release --test
/// effects_fallback fallback_step_cost -- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn fallback_step_cost_at_full_hd() {
    let _gpu = gpu();
    let mut r = renderer();
    r.effects_mut().watchdog.set_policy(Policy::Job);
    let host = Arc::new(Instant0(Mutex::new(None), Mutex::new((0, 0, 0))));
    r.set_fallback_host(Some(host.clone()));
    let doc = stack(fx("acme.cube-invert"), Value::Null);
    let mut request = RenderRequest::sized(Size::new(1920, 1080), Precision::Float16);
    request.background = true;
    let mut times = Vec::new();
    for frame in 0..30u64 {
        let start = std::time::Instant::now();
        let out = r
            .render(
                &doc,
                "P-root",
                Ticks::new(frame as i64 * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap(),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        r.wait_idle();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        drop(out);
    }
    let mut native = Vec::new();
    let plain = stack(
        json!([{ "id": "f1", "type": "invert", "on": true, "p": {} }]),
        Value::Null,
    );
    for frame in 0..30u64 {
        let start = std::time::Instant::now();
        r.render(
            &plain,
            "P-root",
            Ticks::new(frame as i64 * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap(),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
        r.wait_idle();
        native.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let stats = |v: &mut Vec<f64>| {
        let warm = &mut v[5..];
        warm.sort_by(f64::total_cmp);
        (
            warm[warm.len() / 2],
            warm[warm.len() * 95 / 100],
            warm[warm.len() - 1],
        )
    };
    let (w, h, n) = *host.1.lock().unwrap();
    eprintln!(
        "request: {w}x{h}, {n} textures; {} requests",
        r.effects().fallback.results()
    );
    eprintln!(
        "fallback step frame ms p50/p95/max: {:?}",
        stats(&mut times)
    );
    eprintln!(
        "native Invert frame ms p50/p95/max: {:?}",
        stats(&mut native)
    );
}

/// First-use decisions use the runner's actual first frame, then avoid the
/// runner for verified programs; mismatches pin and survive a fresh renderer.
#[test]
fn parity_probe_presents_angle_then_verifies_or_pins_across_restart() {
    use pm_render::effects::parity::Verdict;
    let _gpu = gpu();
    let mut set = builtin_definitions();
    let mut def = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    def.id = "probe.identity".into();
    def.origin = DefinitionOrigin::Local;
    // Use an exact copy shader, regardless of the built-in's formatting.
    let g = def.glsl.as_mut().unwrap();
    let start = g.source.find("void main()").unwrap();
    g.source.truncate(start);
    g.source.push_str("void main(){o=texture(u_tex,v_st);}");
    let code = g.source.clone();
    set.definitions.push(def);
    let doc = stack(
        json!([{ "id": "fx", "type": "probe.identity", "on": true, "p": {} }]),
        Value::Null,
    );
    let dir = std::env::temp_dir().join(format!("pm-render-parity-{}", std::process::id()));
    let make = |env: &str| {
        let mut r = Renderer::headless().unwrap();
        r.set_definitions(&set).unwrap();
        r.set_shader_fallback_runner(true);
        r.effects_mut()
            .parity
            .configure(Some(env.into()), Some(dir.clone()));
        r
    };
    let pending = |r: &mut Renderer| match render(r, &doc) {
        Err(RenderError::ShaderFallbackPending(p)) => p[0].clone(),
        other => panic!("unknown program must hold presentation: {other:?}"),
    };
    let mut r = make("version A");
    let first = pending(&mut r);
    let pixels = r.fallback_textures(&first.key).unwrap()[0].as_ref().clone();
    r.put_fallback_pixels(first.key.clone(), pixels);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified,
        "the reply is judged immediately, before a paused redraw or a later playback frame"
    );
    let angle_frame = render(&mut r, &doc).unwrap();
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    assert_eq!(render(&mut r, &doc).unwrap(), angle_frame);
    let mut restarted = make("version A");
    assert_eq!(
        restarted
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    assert_eq!(render(&mut restarted, &doc).unwrap(), angle_frame);
    // Playback can advance while the first ANGLE request is outstanding.
    // Its reply must verify without requesting that old time/input again.
    let mut advancing = make("playing");
    let first = pending(&mut advancing);
    let pixels = advancing.fallback_textures(&first.key).unwrap()[0]
        .as_ref()
        .clone();
    advancing.put_fallback_pixels(first.key, pixels);
    let frame = advancing
        .render(
            &doc,
            "P-root",
            Ticks::new(pm_time::TICKS_PER_SECOND / 30).unwrap(),
            &request(),
            &MapResolver::default(),
        )
        .unwrap();
    assert_eq!(advancing.read_rgba(&frame).unwrap(), angle_frame);
    assert_eq!(
        advancing
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    // An independent driver/browser generation must be probed again.
    let mut r = make("version B");
    let request = pending(&mut r);
    let mut angle = r.fallback_textures(&request.key).unwrap()[0]
        .as_ref()
        .clone();
    for p in angle.rgba16f.chunks_exact_mut(4) {
        p[0] = 0x3800;
    }
    r.put_fallback_pixels(request.key.clone(), angle);
    let pinned_frame = render(&mut r, &doc).unwrap();
    assert_ne!(
        pinned_frame, angle_frame,
        "presents ANGLE even on a native discrepancy"
    );
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Runner
    );
    let mut restarted = make("version B");
    assert_eq!(
        restarted
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Runner
    );
    assert_eq!(pending(&mut restarted).key, request.key);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A renderer whose runner is not the namespace's reference browser (the
/// hidden MCP host, the CLIs) probes an unknown program as usual but leaves
/// no decision behind that the desktop would trust; it still honours the
/// desktop's own persisted verification.
#[test]
fn a_read_only_first_probe_leaves_no_marker_for_the_desktop() {
    use pm_render::effects::parity::Verdict;
    let _gpu = gpu();
    let mut set = builtin_definitions();
    let mut def = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    def.id = "probe.read-only".into();
    def.origin = DefinitionOrigin::Local;
    let g = def.glsl.as_mut().unwrap();
    let start = g.source.find("void main()").unwrap();
    g.source.truncate(start);
    g.source.push_str("void main(){o=texture(u_tex,v_st);}");
    let code = g.source.clone();
    set.definitions.push(def);
    let doc = stack(
        json!([{ "id": "fx", "type": "probe.read-only", "on": true, "p": {} }]),
        Value::Null,
    );
    let dir = std::env::temp_dir().join(format!("pm-render-parity-ro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let make = |read_only: bool| {
        let mut r = Renderer::headless().unwrap();
        r.set_definitions(&set).unwrap();
        r.set_shader_fallback_runner(true);
        if read_only {
            r.read_only_shader_reference();
        }
        r.effects_mut()
            .parity
            .configure(Some("desktop electron".into()), Some(dir.clone()));
        r
    };
    let probe = |r: &mut Renderer| {
        let first = match render(r, &doc) {
            Err(RenderError::ShaderFallbackPending(p)) => p[0].clone(),
            other => panic!("unknown program must hold presentation: {other:?}"),
        };
        let pixels = r.fallback_textures(&first.key).unwrap()[0].as_ref().clone();
        r.put_fallback_pixels(first.key, pixels);
        render(r, &doc).unwrap()
    };
    let markers = || -> Vec<String> {
        std::fs::read_dir(&dir)
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut hidden = make(true);
    let frame = probe(&mut hidden);
    assert_eq!(
        hidden
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    assert_eq!(render(&mut hidden, &doc).unwrap(), frame);
    assert!(
        markers().is_empty(),
        "a read-only first probe wrote {:?}",
        markers()
    );
    let mut desktop = make(false);
    assert_eq!(
        desktop
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Unknown,
        "the desktop must still compare the program with its own runner"
    );
    assert_eq!(probe(&mut desktop), frame);
    assert_eq!(
        markers().iter().filter(|m| m.ends_with(".native")).count(),
        1
    );
    // A fresh read-only host honours the desktop's verification at once.
    let mut later = make(true);
    assert_eq!(
        later
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    assert_eq!(render(&mut later, &doc).unwrap(), frame);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parity_probe_covers_transitions_and_fragment_layers() {
    use pm_render::effects::parity::Verdict;
    let _gpu = gpu();
    for kind in ["transition", "shader", "extension"] {
        let mut set = builtin_definitions();
        let mut def = set
            .definitions
            .iter()
            .find(|d| d.id.as_str() == "crossfade")
            .unwrap()
            .clone();
        def.id = "probe.constant".into();
        def.origin = DefinitionOrigin::Local;
        let g = def.glsl.as_mut().unwrap();
        let start = g.source.find("void main()").unwrap();
        g.source.truncate(start);
        g.source.push_str("void main(){o=vec4(0.25,0.5,0.75,1.0);}");
        let mut code = g.source.clone();
        if kind == "extension" {
            def.kind = DefinitionKind::Layer;
            def.renderer = Some(pm_protocol::definitions::LayerRenderer::Fragment {
                fragment: "void main(){fragColor=vec4(0.25,0.5,0.75,1.0);}".into(),
            });
            let body = "void main(){fragColor=vec4(0.25,0.5,0.75,1.0);}";
            def.glsl = Some(pm_protocol::definitions::WrappedGlsl {
                source: format!("{}\n{body}", pm_render::shader_layers::header()),
                body_line_offset: pm_render::shader_layers::header().lines().count() as u32,
                body_utf16_len: body.len() as u32,
                body_utf8_len: body.len() as u32,
            });
            code = def.glsl.as_ref().unwrap().source.clone();
        }
        let doc = if kind == "transition" {
            stack(json!([]), json!({"type":"probe.constant","dur":1,"p":{}}))
        } else {
            let mut l = layer("fragment", "#000000", json!([]), Value::Null);
            l["type"] = json!(if kind == "shader" {
                "shader"
            } else {
                "extension"
            });
            l["d"] = if kind == "shader" {
                json!({"code":"void main(){fragColor=vec4(0.25,0.5,0.75,1.0);}","uniforms":{},"w":64,"h":32})
            } else {
                json!({"definition":"probe.constant","params":{},"w":64,"h":32})
            };
            document(vec![l])
        };
        if kind == "shader" {
            code = format!(
                "{}\nvoid main(){{fragColor=vec4(0.25,0.5,0.75,1.0);}}",
                pm_render::shader_layers::header()
            );
        }
        if kind != "shader" {
            set.definitions.push(def);
        }
        let mut r = Renderer::headless().unwrap();
        r.set_definitions(&set).unwrap();
        r.set_shader_fallback_runner(true);
        r.effects_mut()
            .parity
            .configure(Some(format!("test-{kind}")), None);
        let request = match render(&mut r, &doc) {
            Err(RenderError::ShaderFallbackPending(p)) => p[0].clone(),
            other => panic!("{kind} must hold its first frame: {other:?}"),
        };
        let pixels = FallbackPixels {
            rgba32f: None,
            width: request.width,
            height: request.height,
            rgba16f: (0..request.width * request.height)
                .flat_map(|_| [0x3400, 0x3800, 0x3a00, 0x3c00])
                .collect(),
        };
        r.put_fallback_pixels(request.key, pixels);
        render(&mut r, &doc).unwrap();
        assert_eq!(
            r.effects_mut()
                .parity
                .verdict_at_precision(&code, Precision::Float16),
            Verdict::NativeVerified,
            "{kind}"
        );
        render(&mut r, &doc).unwrap();
    }
}

/// A user effect native draws (its input with red and blue swapped, exact
/// in binary16), unknown to the parity probe: the definitions and its
/// wrapped source.
fn probe_swap() -> (DefinitionSetSnapshot, String) {
    let mut set = builtin_definitions();
    let mut def = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    def.id = "probe.swap".into();
    def.origin = DefinitionOrigin::Local;
    let g = def.glsl.as_mut().unwrap();
    let start = g.source.find("void main()").unwrap();
    g.source.truncate(start);
    g.source
        .push_str("void main(){o=texture(u_tex,v_st).bgra;}");
    let code = g.source.clone();
    set.definitions.push(def);
    (set, code)
}

fn refused() -> FallbackFailure {
    FallbackFailure {
        reason: "compile".into(),
        message: "ERROR: 0:7: 'probe' : WebGL refuses this".into(),
    }
}

/// The probe's WebGL reference fails for good (WebGL refuses the program
/// natively drawn): the program is pinned to the runner, whose failure
/// handling reproduces today's outcome (skipped and reported once, as
/// `runEffects` skips a program WebGL refuses), frames keep presenting,
/// nothing is asked for again, and the pin survives a restart. A job
/// whose host refuses it renders the same frame.
#[test]
fn an_effect_whose_parity_reference_fails_for_good_is_pinned_and_skipped_as_today() {
    use pm_render::effects::parity::Verdict;
    let _gpu = gpu();
    let (set, code) = probe_swap();
    let doc = stack(
        json!([{ "id": "fx", "type": "probe.swap", "on": true, "p": {} }]),
        Value::Null,
    );
    let dir = std::env::temp_dir().join(format!("pm-render-parity-fail-{}", std::process::id()));
    let make = |env: &str| {
        let mut r = Renderer::headless().unwrap();
        r.set_definitions(&set).unwrap();
        r.set_shader_fallback_runner(true);
        r.effects_mut()
            .parity
            .configure(Some(env.into()), Some(dir.clone()));
        r
    };
    let mut r = make("refused");
    let without = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("the probed frame waits for its reference")
    };
    assert_eq!(r.put_fallback_failure(&q[0].key, refused()), None);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Runner
    );
    for _ in 0..3 {
        assert_eq!(
            render(&mut r, &doc).unwrap(),
            without,
            "presented without the effect, as WebGL skips it today"
        );
    }
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "reported once: {reports:?}");
    assert_eq!(reports[0].code, ErrorCode::ShaderCompile);
    assert!(
        reports[0].message.contains("WebGL refuses"),
        "{}",
        reports[0].message
    );
    assert_eq!(r.fallback_retry_at(), None, "never asked for again");
    let mut restarted = make("refused");
    assert_eq!(
        restarted
            .effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Runner,
        "the pin persists"
    );
    // A job: its host refuses the probe's reference; the frame is the
    // same, never a native frame without its reference.
    let mut job = Renderer::headless().unwrap();
    job.set_definitions(&set).unwrap();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    job.effects_mut()
        .parity
        .configure(Some("job refused".into()), None);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(refused());
    job.set_fallback_host(Some(host.clone()));
    assert_eq!(render(&mut job, &doc).unwrap(), without);
    assert_eq!(render(&mut job, &doc).unwrap(), without);
    assert_eq!(host.seen.lock().unwrap().len(), 1, "not asked for again");
    assert_eq!(
        job.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Runner
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// The probe's reference fails transiently (the runner's context was
/// lost): the program stays unknown and goes through the runner's retry
/// and backoff (asked for again at once, then drawn without it and
/// reported once until the backoff passes), never a stuck frame or an
/// unbounded request loop; once the runner draws it, the probe judges it.
/// A job whose runner keeps losing its context stops naming the effect.
#[test]
fn an_effect_whose_parity_reference_fails_transiently_is_retried_with_backoff() {
    use pm_render::effects::parity::Verdict;
    let _gpu = gpu();
    let (set, code) = probe_swap();
    let doc = stack(
        json!([{ "id": "fx", "type": "probe.swap", "on": true, "p": {} }]),
        Value::Null,
    );
    let mut r = Renderer::headless().unwrap();
    r.set_definitions(&set).unwrap();
    r.set_shader_fallback_runner(true);
    r.effects_mut()
        .parity
        .configure(Some("transient".into()), None);
    let (offset, clock) = movable_clock();
    r.effects_mut().watchdog.set_clock(Some(clock));
    let without = render(&mut r, &stack(json!([]), Value::Null)).unwrap();
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("the probed frame waits for its reference")
    };
    assert_eq!(
        r.put_fallback_failure(&q[0].key, lost()),
        Some(std::time::Duration::ZERO)
    );
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Unknown
    );
    let Err(RenderError::ShaderFallbackPending(again)) = render(&mut r, &doc) else {
        panic!("asked for again at once")
    };
    assert_eq!(again[0].key, q[0].key);
    assert_eq!(
        r.put_fallback_failure(&again[0].key, lost()),
        Some(std::time::Duration::from_millis(250))
    );
    for _ in 0..3 {
        assert_eq!(
            render(&mut r, &doc).unwrap(),
            without,
            "drawn without it during the backoff, as a runner program"
        );
    }
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Unknown
    );
    offset.store(250, std::sync::atomic::Ordering::Relaxed);
    let Err(RenderError::ShaderFallbackPending(third)) = render(&mut r, &doc) else {
        panic!("asked for again after the backoff")
    };
    // WebGL's answer: the same swap of the input it was sent.
    let mut reference = r.fallback_textures(&third[0].key).unwrap()[0]
        .as_ref()
        .clone();
    for p in reference.rgba16f.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    r.put_fallback_pixels(third[0].key.clone(), reference);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    let drawn = render(&mut r, &doc).unwrap();
    assert_ne!(drawn, without);
    assert_eq!(render(&mut r, &doc).unwrap(), drawn);
    assert_eq!(r.fallback_retry_at(), None);
    // A job: retried in place, then stopped by name; never a native frame
    // without its reference.
    let mut job = Renderer::headless().unwrap();
    job.set_definitions(&set).unwrap();
    job.effects_mut().watchdog.set_policy(Policy::Job);
    job.effects_mut()
        .parity
        .configure(Some("job transient".into()), None);
    let host = Arc::new(Cpu::default());
    *host.fail.lock().unwrap() = Some(lost());
    job.set_fallback_host(Some(host));
    match render(&mut job, &doc) {
        Err(RenderError::Fallback(m)) => {
            assert!(m.contains("probe.swap") && m.contains("contextLost"), "{m}")
        }
        other => panic!("expected a named failure, got {:?}", other.map(|_| ())),
    }
    assert_eq!(
        job.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Unknown
    );
}

/// A shader layer (and a transition) whose probe reference fails: for
/// good, it is pinned and draws nothing, reported once, as WebGL draws
/// nothing for a program it refuses; transiently, the runner's backoff
/// bounds its requests (none while it waits) and the probe judges it once
/// the runner draws it.
#[test]
fn a_layer_and_a_transition_whose_parity_reference_fails_follow_the_runner() {
    use pm_render::effects::parity::Verdict;
    let _gpu = gpu();
    let body = "void main(){fragColor=vec4(0.25,0.5,0.75,1.0);}";
    let code = format!("{}\n{body}", pm_render::shader_layers::header());
    let mut l = layer("fragment", "#000000", json!([]), Value::Null);
    l["type"] = json!("shader");
    l["d"] = json!({"code": body, "uniforms": {}, "w": 64, "h": 32});
    let doc = document(vec![l]);
    let make = |env: &str| {
        let mut r = Renderer::headless().unwrap();
        r.set_definitions(&builtin_definitions()).unwrap();
        r.set_shader_fallback_runner(true);
        r.effects_mut().parity.configure(Some(env.into()), None);
        r
    };
    let empty = render(&mut make("none"), &document(vec![])).unwrap();
    // For good.
    let mut r = make("layer refused");
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("the probed layer waits for its reference")
    };
    assert_eq!(r.put_fallback_failure(&q[0].key, refused()), None);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Runner
    );
    for _ in 0..3 {
        assert_eq!(render(&mut r, &doc).unwrap(), empty, "draws nothing");
    }
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::ShaderCompile);
    // Transiently.
    let mut r = make("layer transient");
    let (offset, clock) = movable_clock();
    r.effects_mut().watchdog.set_clock(Some(clock));
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("the probed layer waits for its reference")
    };
    assert_eq!(
        r.put_fallback_failure(&q[0].key, lost()),
        Some(std::time::Duration::ZERO)
    );
    let Err(RenderError::ShaderFallbackPending(again)) = render(&mut r, &doc) else {
        panic!("asked for again at once")
    };
    assert_eq!(again[0].key, q[0].key);
    assert_eq!(
        r.put_fallback_failure(&again[0].key, lost()),
        Some(std::time::Duration::from_millis(250))
    );
    for _ in 0..3 {
        assert_eq!(
            render(&mut r, &doc).unwrap(),
            empty,
            "no request while the backoff runs"
        );
    }
    assert_eq!(r.watchdog_reports(true).len(), 1);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::Unknown
    );
    offset.store(250, std::sync::atomic::Ordering::Relaxed);
    let Err(RenderError::ShaderFallbackPending(third)) = render(&mut r, &doc) else {
        panic!("asked for again after the backoff")
    };
    let reference = FallbackPixels {
        rgba32f: None,
        width: third[0].width,
        height: third[0].height,
        rgba16f: (0..third[0].width * third[0].height)
            .flat_map(|_| [0x3400, 0x3800, 0x3a00, 0x3c00])
            .collect(),
    };
    r.put_fallback_pixels(third[0].key.clone(), reference);
    let drawn = render(&mut r, &doc).unwrap();
    assert_ne!(drawn, empty);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&code, Precision::Float16),
        Verdict::NativeVerified
    );
    assert_eq!(render(&mut r, &doc).unwrap(), drawn);
    assert_eq!(r.fallback_retry_at(), None);
    // A transition refused for good: pinned, skipped as WebGL skips it.
    let mut set = builtin_definitions();
    let mut def = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "crossfade")
        .unwrap()
        .clone();
    def.id = "probe.fade".into();
    def.origin = DefinitionOrigin::Local;
    let g = def.glsl.as_mut().unwrap();
    let start = g.source.find("void main()").unwrap();
    g.source.truncate(start);
    g.source
        .push_str("void main(){o=mix(texture(u_from,v_st),texture(u_to,v_st),u_prog);}");
    let fade = g.source.clone();
    set.definitions.push(def);
    let mut r = Renderer::headless().unwrap();
    r.set_definitions(&set).unwrap();
    r.set_shader_fallback_runner(true);
    r.effects_mut()
        .parity
        .configure(Some("transition refused".into()), None);
    let doc = stack(json!([]), json!({"type": "probe.fade", "dur": 1, "p": {}}));
    let Err(RenderError::ShaderFallbackPending(q)) = render(&mut r, &doc) else {
        panic!("the probed transition waits for its reference")
    };
    assert_eq!(r.put_fallback_failure(&q[0].key, refused()), None);
    assert_eq!(
        r.effects_mut()
            .parity
            .verdict_at_precision(&fade, Precision::Float16),
        Verdict::Runner
    );
    render(&mut r, &doc).unwrap();
    render(&mut r, &doc).unwrap();
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::ShaderCompile);
}
