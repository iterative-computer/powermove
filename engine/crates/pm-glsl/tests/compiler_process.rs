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
// Success-path tests share the deliberately small default pool. Admission
// stress below uses its own pool and remains concurrent.
static DEFAULT_POOL_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
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
        Response::Prepared(p) => *p,
        _ => panic!("wrong response"),
    }
}
#[test]
fn helper_finishes_cached_spirv_and_preserves_msl_exactly() {
    let _guard = DEFAULT_POOL_TEST.lock().unwrap_or_else(|e| e.into_inner());
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
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
        let original = pm_gpu::msl::write(
            module,
            &pm_glsl::pipeline::naga_validate(module).unwrap(),
            stage,
            &old_slots,
            pm_gpu::msl::MathMode::Fast,
        )
        .unwrap();
        let isolated =
            pm_glsl::compiler::msl(module, stage, slots.clone(), Duration::from_secs(20)).unwrap();
        assert!(isolated.0.starts_with("#pragma METAL fp math_mode(fast)\n"));
        assert!(!isolated.1.is_empty());
        #[cfg(not(target_os = "macos"))]
        let original =
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
#[cfg(any(target_os = "macos", target_os = "linux"))]
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
    assert!(
        self::pool(1)
            .run(request(SRC), Duration::from_secs(20))
            .is_ok()
    );
}

#[test]
fn reflection_bindings_are_required_and_cannot_alias_metal_slots() {
    let _guard = DEFAULT_POOL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let program = prepared().finish().unwrap();
    let module = &program.fragment.module;
    let block = program.fragment.reflection.block.as_ref().unwrap();
    let slot = pm_glsl::compiler::Slot {
        group: block.set,
        binding: block.binding,
        buffer: Some(3),
        texture: None,
        sampler: None,
    };
    assert!(
        pm_glsl::compiler::msl(
            module,
            naga::ShaderStage::Fragment,
            vec![slot.clone()],
            Duration::from_secs(20)
        )
        .is_ok()
    );
    let wrong = pm_glsl::compiler::Slot {
        buffer: None,
        texture: Some(3),
        ..slot.clone()
    };
    let alias = pm_glsl::compiler::Slot {
        binding: slot.binding + 1,
        ..slot.clone()
    };
    for slots in [
        vec![],
        vec![wrong],
        vec![slot.clone(), slot.clone()],
        vec![slot, alias],
    ] {
        let failure = pm_glsl::compiler::msl(
            module,
            naga::ShaderStage::Fragment,
            slots,
            Duration::from_secs(20),
        )
        .unwrap_err();
        assert_eq!(failure.stage, FailureStage::Bind, "{failure}");
    }
    // An inactive resource (or a resource belonging only to another entry
    // point) must not turn a formerly valid shader into a binding refusal.
    let mut module = module.clone();
    let mut inactive = module
        .global_variables
        .iter()
        .find(|(_, g)| g.space == naga::AddressSpace::Uniform)
        .unwrap()
        .1
        .clone();
    inactive.name = Some("unused_other_entry_resource".into());
    inactive.binding = Some(naga::ResourceBinding {
        group: 3,
        binding: 99,
    });
    module
        .global_variables
        .append(inactive, naga::Span::UNDEFINED);
    let slot = pm_glsl::compiler::Slot {
        group: block.set,
        binding: block.binding,
        buffer: Some(3),
        texture: None,
        sampler: None,
    };
    pm_glsl::compiler::msl(
        &module,
        naga::ShaderStage::Fragment,
        vec![slot],
        Duration::from_secs(20),
    )
    .unwrap();
    let missing_entry = pm_glsl::compiler::msl(
        &module,
        naga::ShaderStage::Compute,
        Vec::new(),
        Duration::from_secs(20),
    )
    .unwrap_err();
    assert_eq!(missing_entry.stage, FailureStage::NagaMsl);
}

#[test]
fn msl_contains_bounds_division_and_loop_guards() {
    let _guard = DEFAULT_POOL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let source = "#version 300 es\nprecision highp float;in vec2 v_uv;uniform int u_i;uniform int u_d;uniform sampler2D u_tex;out vec4 o;void main(){float a[3];a[0]=0.;a[1]=0.5;a[2]=1.;int i=u_i;float f=a[i]+vec3(v_uv,1.)[i];int q=i/u_d;for(int j=0;j<u_d;j++){f+=float(j);}o=vec4(f+float(q))+texelFetch(u_tex,ivec2(i),0);}";
    let program = pm_glsl::translate(source, 0).unwrap();
    let reflection = &program.fragment.reflection;
    let mut slots: Vec<_> = reflection
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
    for (i, s) in reflection.samplers.iter().enumerate() {
        slots.push(pm_glsl::compiler::Slot {
            group: s.set,
            binding: s.binding,
            buffer: None,
            texture: Some(i as u8),
            sampler: None,
        });
        slots.push(pm_glsl::compiler::Slot {
            group: s.set,
            binding: s.binding + pm_glsl::SAMPLER_BINDING_OFFSET,
            buffer: None,
            texture: None,
            sampler: Some(i as u8),
        });
    }
    let (source, _) = pm_glsl::compiler::msl(
        &program.fragment.module,
        naga::ShaderStage::Fragment,
        slots,
        Duration::from_secs(20),
    )
    .unwrap();
    for guard in [
        "metal::min",
        "naga_div",
        "loop_bound",
        "get_width",
        "get_height",
    ] {
        assert!(source.contains(guard), "missing {guard}: {source}");
    }
    // The convenience/diagnostic writer is also supervised.
    assert!(
        !pm_glsl::pipeline::msl(&program.fragment.module)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn malformed_cached_spirv_fails_in_the_worker_and_next_finish_succeeds() {
    let mut bad = prepared();
    bad.fragment.words = vec![0x07230203, 0x00010000, 0, u32::MAX, 0, u32::MAX];
    let pool = pool(1);
    let failure = pool
        .run(Request::Finish(bad), Duration::from_secs(2))
        .err()
        .unwrap();
    assert!(
        matches!(
            failure.stage,
            FailureStage::NagaParse
                | FailureStage::CompilerProcess
                | FailureStage::CompilerResources
                | FailureStage::Deadline
        ),
        "{failure}"
    );
    assert_eq!(pool.active(), 0);
    assert!(
        pool.run(Request::Finish(prepared()), Duration::from_secs(20))
            .is_ok()
    );
}

#[test]
fn malformed_and_oversized_ipc_responses_are_reaped() {
    use std::os::unix::fs::PermissionsExt;
    let file = std::env::temp_dir().join(format!("pm-glsl-protocol-{}", std::process::id()));
    for body in [
        "import sys,struct;sys.stdout.buffer.write(struct.pack('<Q',67108865));sys.stdout.buffer.flush()",
        "import sys,struct;sys.stdout.buffer.write(struct.pack('<Q',1)+b'x');sys.stdout.buffer.flush()",
        "import sys,struct;sys.stdout.buffer.write(struct.pack('<Q',10)+b'x');sys.stdout.buffer.flush()",
    ] {
        std::fs::write(
            &file,
            format!("#!/bin/sh\nexec /usr/bin/python3 -c \"{body}\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pool = Pool::new(file.clone(), 1);
        let failure = pool
            .run(request(SRC), Duration::from_secs(2))
            .err()
            .unwrap();
        assert_eq!(failure.stage, FailureStage::CompilerProcess, "{failure}");
        assert_eq!(pool.active(), 0);
    }
    std::fs::remove_file(file).unwrap();
}
