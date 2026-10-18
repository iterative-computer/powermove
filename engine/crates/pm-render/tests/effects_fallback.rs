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
    let doc = json!({
        "id": "P", "name": "p", "w": 64, "h": 32, "fps": 30, "dur": 10, "bg": "#202020",
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
