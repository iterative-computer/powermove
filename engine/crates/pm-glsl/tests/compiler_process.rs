//! Compiler admission, process cleanup, cached finishing and byte-equivalent
//! downstream translation. No timing assertion depends on a particular
//! adversarial shader actually making a compiler hang.
use pm_glsl::{
    FailureStage,
    compiler::{Pool, Request, Response},
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
const SRC: &str = "#version 300 es\nprecision highp float;in vec2 v_uv;uniform vec2 u_res;out vec4 o;void main(){o=vec4(v_uv*u_res,0.,1.);}";
fn pool(n: usize) -> Pool {
    Pool::new(PathBuf::from(env!("CARGO_BIN_EXE_pm-glsl-compiler")), n)
}
fn request(source: &str) -> Request {
    Request::Prepare {
        fragment: source.into(),
        line_offset: 0,
    }
}
fn prepared() -> pm_glsl::Prepared {
    match pool(1).run(request(SRC), Duration::from_secs(20)).unwrap() {
        Response::Prepared(p) => p,
        _ => panic!("wrong response"),
    }
}
#[test]
fn helper_finishes_cached_spirv_and_preserves_msl_exactly() {
    let prepared = prepared();
    let program = prepared.finish().unwrap();
    for (module, stage, reflection) in [
        (
            &program.vertex.module,
            naga::ShaderStage::Vertex,
            &program.vertex.reflection,
        ),
        (
            &program.fragment.module,
            naga::ShaderStage::Fragment,
            &program.fragment.reflection,
        ),
    ] {
        let slots: Vec<_> = reflection
            .block
            .iter()
            .map(|b| pm_glsl::compiler::Slot {
                group: b.set,
                binding: b.binding,
                buffer: Some(0),
                texture: None,
                sampler: None,
            })
            .collect();
        let old_slots: Vec<_> = slots
            .iter()
            .map(|s| pm_gpu::msl::Slot {
                group: s.group,
                binding: s.binding,
                buffer: s.buffer,
                texture: s.texture,
                sampler: s.sampler,
            })
            .collect();
        let original = pm_gpu::msl::write(
            module,
            &pm_glsl::pipeline::naga_validate(module).unwrap(),
            stage,
            &old_slots,
            pm_gpu::msl::MathMode::Fast,
        )
        .unwrap();
        let isolated =
            pm_glsl::compiler::msl(module, stage, slots, Duration::from_secs(20)).unwrap();
        assert_eq!(
            original, isolated,
            "moving the compiler must not alter coordinates or math"
        );
    }
    let pool = pool(1);
    let failure = pool
        .run(Request::Finish(prepared), Duration::ZERO)
        .err()
        .unwrap();
    assert_eq!(failure.stage, FailureStage::Deadline);
    assert_eq!(pool.active(), 0);
}
#[test]
fn timeouts_reap_workers_and_admission_never_queues() {
    // A deliberately non-reading process makes saturation deterministic,
    // independent of compiler optimization. /bin/sleep rejects the worker
    // argument, so use a private executable shell script that execs sleep.
    use std::os::unix::fs::PermissionsExt;
    let file = std::env::temp_dir().join(format!("pm-glsl-hold-{}", std::process::id()));
    std::fs::write(&file, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pool = Arc::new(Pool::new(file.clone(), 2));
    for _ in 0..3 {
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                let pool = pool.clone();
                std::thread::spawn(move || {
                    pool.run(request(SRC), Duration::from_millis(300))
                        .err()
                        .unwrap()
                        .stage
                })
            })
            .collect();
        let start = Instant::now();
        while pool.active() != 2 {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::yield_now();
        }
        assert_eq!(
            pool.run(request(SRC), Duration::from_secs(5))
                .err()
                .unwrap()
                .stage,
            FailureStage::CompilerBusy
        );
        for job in jobs {
            assert_eq!(job.join().unwrap(), FailureStage::Deadline);
        }
        assert_eq!(pool.active(), 0);
    }
    std::fs::remove_file(file).unwrap();
}
#[test]
fn adversarial_compilation_is_killable_and_next_request_recovers() {
    let pool = pool(1);
    let mut sources = pm_glsl::fuzz::bombs();
    sources.push((
        "huge-unroll".into(),
        SRC.replace(
            "o=vec4(v_uv*u_res,0.,1.);",
            "float x=v_uv.x; for(int i=0;i<2147483647;i++){x=sin(x)+x;} o=vec4(x);",
        ),
    ));
    sources.push((
        "pathological-nesting".into(),
        format!(
            "#version 300 es\nprecision highp float;out vec4 o;void main(){{o=vec4({}1.{});}}",
            "(".repeat(100_000),
            ")".repeat(100_000)
        ),
    ));
    for (_, source) in sources {
        let started = Instant::now();
        let result = pool.run(request(&source), Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_secs(3));
        if let Err(f) = result {
            assert!(
                matches!(
                    f.stage,
                    FailureStage::Deadline
                        | FailureStage::CompilerProcess
                        | FailureStage::CompilerResources
                        | FailureStage::Limits
                        | FailureStage::EsValidate
                ),
                "{f}"
            );
        }
        assert_eq!(pool.active(), 0);
    }
    assert!(pool.run(request(SRC), Duration::from_secs(20)).is_ok());
}
#[test]
fn missing_and_crashing_helpers_fail_by_name() {
    let missing = Pool::new(PathBuf::from("/nonexistent/pm-glsl-compiler"), 1);
    assert_eq!(
        missing
            .run(request(SRC), Duration::from_secs(1))
            .err()
            .unwrap()
            .stage,
        FailureStage::CompilerProcess
    );
    let exited = Pool::new(PathBuf::from("/usr/bin/false"), 1);
    assert_eq!(
        exited
            .run(request(SRC), Duration::from_secs(1))
            .err()
            .unwrap()
            .stage,
        FailureStage::CompilerProcess
    );
    assert_eq!(missing.active(), 0);
    assert_eq!(exited.active(), 0);
}

#[test]
#[cfg(target_os = "macos")]
fn resident_memory_over_budget_kills_and_reaps_the_process() {
    use std::os::unix::fs::PermissionsExt;
    let file = std::env::temp_dir().join(format!("pm-glsl-memory-{}", std::process::id()));
    std::fs::write(&file,"#!/bin/sh\nexec /usr/bin/python3 -c 'import time; a=bytearray(128*1024*1024); time.sleep(30)'\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pool = Pool::new(file.clone(), 1).with_resident_limit(64 << 20);
    let failure = pool
        .run(request(SRC), Duration::from_secs(5))
        .err()
        .unwrap();
    assert_eq!(failure.stage, FailureStage::CompilerResources, "{failure}");
    assert_eq!(pool.active(), 0);
    std::fs::remove_file(file).unwrap();
}
