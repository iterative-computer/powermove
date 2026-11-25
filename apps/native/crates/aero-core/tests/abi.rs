//! The header's rules that are not about one feature: struct sizes and
//! strides, outputs and precedence (Sol's item 7), limits, the credential
//! edge (item 9), and accepted work that must not vanish (item 6).

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_aero_core::{Error, Host, Purpose, Runtime, Steered, sys};
use support::{
    CODEX_ACCOUNT, CODEX_KEY, Driver, GATEWAY_KEY, SYNTHETIC_CREDENTIALS, Seen, config, on_worker,
    text_model,
};

struct NoTools;
impl Host for NoTools {}

const ENDPOINT: &str = "http://127.0.0.1:9/fx";

fn raw_config(cfg: &pm_aero_core::Config<'_>) -> sys::fx_config {
    sys::fx_config {
        struct_size: std::mem::size_of::<sys::fx_config>() as u32,
        abi_version: sys::FX_CORE_ABI_VERSION,
        provider: sys::fx_str::of(cfg.provider),
        model: sys::fx_str::of(cfg.model),
        effort: sys::fx_str::EMPTY,
        configured_providers_json: sys::fx_str::EMPTY,
        endpoint_url: sys::fx_str::of(cfg.endpoint_url),
        instructions: sys::fx_str::of(cfg.instructions),
        home: sys::fx_str::of(cfg.home),
        workspace_root: sys::fx_str::of(cfg.workspace_root),
        tools: std::ptr::null(),
        tool_count: 0,
        max_agent_steps: 0,
        max_concurrent_tools: 0,
        max_tool_result_bytes: 0,
        max_history_turns: 0,
        max_stop_continuations: 0,
        flags: 0,
    }
}

fn create_raw(cfg: *const sys::fx_config) -> (sys::fx_status, u64) {
    let mut handle = 99u64;
    // SAFETY: cfg points at a readable buffer at least struct_size long; no
    // host callbacks are given.
    let status =
        unsafe { sys::fx_runtime_create(cfg, std::ptr::null(), std::ptr::null_mut(), &mut handle) };
    (status, handle)
}

#[test]
fn configs_are_read_by_size_and_bounded() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let base = raw_config(&config(ENDPOINT, &[]));
        // Another ABI is refused, as is a struct smaller than v2.
        let mut cfg = base;
        cfg.abi_version = 1;
        assert_eq!(create_raw(&cfg), (sys::FX_E_UNSUPPORTED, 0));
        let mut cfg = base;
        cfg.struct_size = 100;
        assert_eq!(create_raw(&cfg), (sys::FX_E_INVALID, 0));
        // A newer host's larger struct: accepted when the bytes the core
        // does not know are zero, refused when they are not.
        #[repr(C)]
        struct Larger {
            cfg: sys::fx_config,
            extra: [u64; 2],
        }
        let mut larger = Larger {
            cfg: base,
            extra: [0; 2],
        };
        larger.cfg.struct_size = std::mem::size_of::<Larger>() as u32;
        let (status, handle) = create_raw(&larger.cfg);
        assert_eq!(status, sys::FX_OK);
        // SAFETY: plain value.
        assert_eq!(unsafe { sys::fx_runtime_destroy(handle) }, sys::FX_OK);
        larger.extra[1] = 1;
        assert_eq!(create_raw(&larger.cfg), (sys::FX_E_UNSUPPORTED, 0));
        // Limits out of bounds, and endpoint_url on a provider it is not for.
        for edit in [
            (|c: &mut sys::fx_config| c.max_concurrent_tools = 17) as fn(&mut sys::fx_config),
            |c| c.max_agent_steps = 10_001,
            |c| c.max_tool_result_bytes = (16 << 20) + 1,
            |c| c.max_stop_continuations = 65,
            |c| c.flags = 1,
        ] {
            let mut cfg = base;
            edit(&mut cfg);
            let (status, _) = create_raw(&cfg);
            assert!(
                status == sys::FX_E_INVALID || status == sys::FX_E_UNSUPPORTED,
                "{status}"
            );
        }
        let codex = pm_aero_core::Config {
            provider: "codex",
            ..config(ENDPOINT, &[])
        };
        assert_eq!(
            Runtime::create(&codex, Arc::new(NoTools)).err(),
            Some(Error::Invalid)
        );
        // NULL and misaligned outs, before anything else.
        // SAFETY: deliberately bad pointers the core must refuse untouched.
        unsafe {
            assert_eq!(
                sys::fx_runtime_create(
                    &base,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ),
                sys::FX_E_INVALID
            );
            let mut words = [0u64; 2];
            let misaligned = (words.as_mut_ptr() as *mut u8).add(1) as *mut u64;
            assert_eq!(
                sys::fx_runtime_create(&base, std::ptr::null(), std::ptr::null_mut(), misaligned),
                sys::FX_E_INVALID
            );
        }
    });
}

#[test]
fn outputs_strides_and_precedence() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let handle = runtime.remote().handle();
        // SAFETY: valid pointers for each call, or deliberately bad ones the
        // core must refuse before touching the handle.
        unsafe {
            // A pointer-shape error wins over a closed handle.
            let mut turn = 77u64;
            assert_eq!(
                sys::fx_turn_prompt(12345, std::ptr::null(), sys::fx_str::of("m"), &mut turn),
                sys::FX_E_INVALID
            );
            assert_eq!(turn, 0, "the out is zeroed even on FX_E_INVALID");
            let input = sys::fx_input {
                text: sys::fx_str::of("x"),
                images: std::ptr::null(),
                image_count: 0,
            };
            turn = 77;
            assert_eq!(
                sys::fx_turn_prompt(12345, &input, sys::fx_str::of("m"), &mut turn),
                sys::FX_E_CLOSED
            );
            assert_eq!(turn, 0);
            // A live runtime before its session: the function's own error.
            assert_eq!(
                sys::fx_turn_prompt(handle, &input, sys::fx_str::of("m"), &mut turn),
                sys::FX_E_STATE
            );
            // Info with a struct too small, then a larger one zero-filled.
            let mut small: sys::fx_runtime_info = std::mem::zeroed();
            small.struct_size = 16;
            assert_eq!(
                sys::fx_runtime_info_get(handle, &mut small),
                sys::FX_E_INVALID
            );
            #[repr(C)]
            struct LargerInfo {
                info: sys::fx_runtime_info,
                extra: u64,
            }
            let mut larger = LargerInfo {
                info: std::mem::zeroed(),
                extra: u64::MAX,
            };
            larger.info.struct_size = std::mem::size_of::<LargerInfo>() as u32;
            assert_eq!(
                sys::fx_runtime_info_get(handle, &mut larger.info),
                sys::FX_OK
            );
            assert_eq!(larger.info.state, sys::FX_RUNTIME_RUNNING);
            assert_eq!(larger.extra, 0, "bytes past the core's fields are zeroed");
            // The event stride: smaller than v2 or not a multiple of 8 is refused.
            let mut count = 5usize;
            let mut events: Vec<sys::fx_event> = Vec::with_capacity(4);
            assert_eq!(
                sys::fx_events_drain(handle, events.as_mut_ptr(), 4, 152, &mut count),
                sys::FX_E_INVALID
            );
            assert_eq!(count, 0);
            assert_eq!(
                sys::fx_events_drain(handle, events.as_mut_ptr(), 4, 164, &mut count),
                sys::FX_E_INVALID
            );
            // cap 0 is a valid drain.
            assert_eq!(
                sys::fx_events_drain(handle, std::ptr::null_mut(), 0, 160, &mut count),
                sys::FX_OK
            );
            assert_eq!(count, 0);
        }
        // A larger stride: each element is written and its tail zeroed.
        let mut runtime = runtime;
        let request = runtime.open_session("").expect("open");
        const STRIDE: usize = 176;
        let mut buf = vec![0xAAu8; STRIDE * 8];
        let mut opened = None;
        for _ in 0..400 {
            support::wait_ready(&runtime, 10);
            let mut count = 0usize;
            // SAFETY: buf holds 8 elements of STRIDE bytes, 8-aligned below.
            let aligned = buf.as_mut_ptr().align_offset(8);
            assert_eq!(
                aligned, 0,
                "Vec<u8> of this size is 8-aligned on this target"
            );
            let status = unsafe {
                sys::fx_events_drain(handle, buf.as_mut_ptr().cast(), 8, STRIDE, &mut count)
            };
            assert_eq!(status, sys::FX_OK);
            for index in 0..count {
                let element = &buf[index * STRIDE..(index + 1) * STRIDE];
                assert!(
                    element[160..].iter().all(|b| *b == 0),
                    "the tail past fx_event is zeroed"
                );
                // SAFETY: the element starts with an fx_event the core wrote.
                let event = unsafe { &*(element.as_ptr() as *const sys::fx_event) };
                if event.kind == sys::FX_EVENT_SESSION_OPENED {
                    opened = Some(event.request);
                }
            }
            if opened.is_some() {
                break;
            }
        }
        assert_eq!(opened, Some(request));
        // A second open is refused; withdraw and cancel of a turn that never
        // ran are stale; resume, checkpoint and staging have nothing to act on.
        assert_eq!(runtime.open_session(""), Err(Error::State));
        assert_eq!(
            runtime.remote().cancel(42),
            Ok(pm_aero_core::Outcome::Stale)
        );
        assert_eq!(runtime.withdraw(42, "m"), Err(Error::Stale));
        assert_eq!(runtime.steer(42, "hi", "s-1"), Ok(Steered::Stale));
        // SAFETY: valid outs.
        unsafe {
            let mut out = 9u64;
            // No turn was left open; the session is not journaled; an
            // empty batch stages nothing.
            assert_eq!(sys::fx_turn_resume(handle, &mut out), sys::FX_E_STATE);
            assert_eq!(sys::fx_checkpoint(handle, &mut out), sys::FX_E_STATE);
            assert_eq!(
                sys::fx_session_stage_images(handle, std::ptr::null(), 0, &mut out),
                sys::FX_E_INVALID
            );
            assert_eq!(out, 0);
            assert_eq!(
                sys::fx_hook_answer(handle, 1, sys::FX_ANSWER_ALLOW, sys::fx_str::EMPTY),
                sys::FX_E_INVALID
            );
            assert_eq!(sys::fx_journal_ack(handle, 1), sys::FX_E_INVALID);
        }
        runtime.close();
    });
}

#[test]
fn oversized_input_is_refused_for_good_and_the_runtime_goes_on() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, text_model("fine"), ENDPOINT);
        driver.open();
        // Sol's case: 4 MiB of newlines escapes past the 8 MiB input queue.
        // It used to come back Ok(0): the host waited for a turn never started.
        let newlines = "\n".repeat(4 << 20);
        assert_eq!(
            driver.runtime.prompt(&newlines, "big-1"),
            Err(Error::TooLarge)
        );
        let over = "x".repeat((4 << 20) + 1);
        assert_eq!(driver.runtime.prompt(&over, "big-2"), Err(Error::TooLarge));
        // Nothing started: the next prompt runs normally.
        let (turn, _) = driver.turn("small", "small-1");
        assert_eq!(driver.text_of(turn), "fine");
        assert!(!driver.seen.iter().any(
            |e| matches!(e, Seen::TurnStart { message_id, .. } if message_id.starts_with("big"))
        ));
        driver.runtime.close();
    });
}

#[test]
fn a_turns_end_comes_before_the_next_turns_start() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, text_model("x"), ENDPOINT);
        driver.open();
        for round in 0..20 {
            let first = driver
                .runtime
                .prompt("a", &format!("a-{round}"))
                .expect("prompt a");
            // Try the next prompt as fast as possible: it is admissible only
            // once the first turn's TURN_END is queued.
            let second = loop {
                driver.pump(0);
                match driver.runtime.prompt("b", &format!("b-{round}")) {
                    Ok(turn) => break turn,
                    Err(Error::State) => continue,
                    Err(error) => panic!("prompt b: {error}"),
                }
            };
            driver.until(
                30,
                |e| matches!(e, Seen::TurnEnd { turn, .. } if *turn == second),
            );
            let end_a = driver
                .seen
                .iter()
                .position(|e| matches!(e, Seen::TurnEnd { turn, .. } if *turn == first))
                .unwrap();
            let start_b = driver
                .seen
                .iter()
                .position(|e| matches!(e, Seen::TurnStart { turn, .. } if *turn == second))
                .unwrap();
            assert!(
                end_a < start_b,
                "round {round}: TURN_END of {first} after TURN_START of {second}"
            );
        }
        driver.runtime.close();
    });
}

/// The trusted allowlist's headers, as the edge sent them to `fetch`.
fn credential_headers(fetch: &support::FetchSeen) -> Vec<(String, String)> {
    fetch
        .outgoing
        .iter()
        .filter(|(name, _)| {
            SYNTHETIC_CREDENTIALS
                .iter()
                .any(|c| name.eq_ignore_ascii_case(c.header))
        })
        .cloned()
        .collect()
}

/// Pumps until a catalog request to `origin` was served (the synthetic
/// catalog leg ran), or fails after `secs`.
fn catalog_leg(driver: &mut Driver, origin: &str, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !driver
        .fetches
        .lock()
        .unwrap()
        .iter()
        .any(|f| f.purpose == Purpose::Catalog && f.url.starts_with(origin))
    {
        driver.pump(20);
        assert!(
            Instant::now() < deadline,
            "no catalog request to {origin}: {:#?}",
            driver.fetches.lock().unwrap()
        );
    }
}

#[test]
fn credentials_never_cross_the_edge() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let runtime = Runtime::create(&config(ENDPOINT, &[]), Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, text_model("x"), ENDPOINT);
        driver.open();
        driver.turn("hi", "hi-1");
        catalog_leg(&mut driver, "https://ai-gateway.vercel.sh/", 20);
        let fetches = driver.fetches.lock().unwrap().clone();
        for fetch in &fetches {
            assert_eq!(fetch.provider, "gateway");
            // The core's placeholder never leaves it, whatever the route.
            for (name, value) in &fetch.headers {
                assert!(
                    !name.eq_ignore_ascii_case("authorization"),
                    "{name}: {value}"
                );
                assert!(!value.contains("host-managed"), "{name}: {value}");
            }
            let sent = credential_headers(fetch);
            if fetch.url.starts_with("https://ai-gateway.vercel.sh/") {
                // The Gateway's canonical origin: its key, only.
                assert_eq!(
                    sent,
                    vec![("authorization".to_string(), GATEWAY_KEY.to_string())]
                );
            } else {
                // The loopback fake (the endpoint override): no credential.
                assert!(sent.is_empty(), "credential sent to {}", fetch.url);
            }
        }
        driver.runtime.close();
    });
}

#[test]
fn every_allowlisted_header_reaches_its_origin_only() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        // Codex: authorization and an account id for its origin; its catalog
        // looks its version up on a public origin first.
        let cfg = pm_aero_core::Config {
            provider: "codex",
            model: "",
            endpoint_url: "",
            ..config(ENDPOINT, &[])
        };
        let runtime = Runtime::create(&cfg, Arc::new(NoTools)).expect("create");
        let mut driver = Driver::new(runtime, text_model("x"), ENDPOINT);
        driver.open();
        // Every request is answered 404 (none is the loopback endpoint).
        let turn = driver.runtime.prompt("hi", "c-1").expect("prompt");
        driver.until(
            30,
            |e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn),
        );
        catalog_leg(&mut driver, "https://", 20);
        let fetches = driver.fetches.lock().unwrap().clone();
        let mut at_origin = 0;
        let mut elsewhere = 0;
        for fetch in &fetches {
            assert_eq!(fetch.provider, "codex");
            let sent = credential_headers(fetch);
            if fetch.url.starts_with("https://chatgpt.com/") {
                at_origin += 1;
                let mut sent = sent;
                sent.sort();
                assert_eq!(
                    sent,
                    vec![
                        ("authorization".to_string(), CODEX_KEY.to_string()),
                        ("chatgpt-account-id".to_string(), CODEX_ACCOUNT.to_string()),
                    ],
                    "{}",
                    fetch.url
                );
            } else {
                elsewhere += 1;
                assert!(sent.is_empty(), "credential sent to {}", fetch.url);
            }
        }
        assert!(at_origin > 0, "no request to Codex's origin: {fetches:#?}");
        assert!(elsewhere > 0, "no public lookup: {fetches:#?}");
        driver.runtime.close();
    });
}
