//! S5c synthetic fallback evaluation. V8 runs on this worker, never AppKit main.
mod raster;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    time::Instant,
};
unsafe extern "C" {
    fn v8__V8__IsSandboxEnabled() -> bool;
}
fn footprint() -> u64 {
    let mut i: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            libc::proc_pid_rusage(
                std::process::id() as _,
                libc::RUSAGE_INFO_V4,
                &mut i as *mut _ as _,
            )
        },
        0
    );
    i.ri_phys_footprint
}
fn now() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
}
fn host_now(_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
    r.set_double(now())
}
fn host(s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
    let op = a.get(0).to_rust_string_lossy(s);
    match op.as_str() {
        "id" => r.set_int32(7),
        "begin" => {
            let id = s.get_slot_mut::<raster::Raster>().unwrap().begin();
            r.set_double(id as f64)
        }
        "nested" => {
            let id = a.get(1).integer_value(s).unwrap() as u64;
            let rev = a.get(2).integer_value(s).unwrap() as u64;
            r.set_bool(s.get_slot::<raster::Raster>().unwrap().pending.get(&id) == Some(&rev))
        }
        "resume" => {
            let id = a.get(1).integer_value(s).unwrap() as u64;
            let rev = a.get(2).integer_value(s).unwrap() as u64;
            let surface = a.get(3).integer_value(s).unwrap() as u64;
            let result = s
                .get_slot_mut::<raster::Raster>()
                .unwrap()
                .resume(id, rev, surface);
            r.set_double(result as f64)
        }
        "clear" => {
            let keep = a.get(1).integer_value(s).unwrap_or(0) as u64;
            s.get_slot_mut::<raster::Raster>()
                .unwrap()
                .surfaces
                .retain(|id, _| *id == keep);
        }
        "render" => {
            let v: Value = serde_json::from_str(&a.get(1).to_rust_string_lossy(s)).unwrap();
            let id = s.get_slot_mut::<raster::Raster>().unwrap().render(&v);
            r.set_double(id as f64)
        }
        "read" => {
            let id = a.get(1).integer_value(s).unwrap() as u64;
            let surf = s.get_slot::<raster::Raster>().unwrap().surfaces[&id].clone();
            // Allocate inside the sandbox, fill while exclusively owned, THEN expose.
            let x = a.get(2).integer_value(s).unwrap() as usize;
            let y = a.get(3).integer_value(s).unwrap() as usize;
            let w = a.get(4).integer_value(s).unwrap() as usize;
            let h = a.get(5).integer_value(s).unwrap() as usize;
            assert!(x + w <= surf.w && y + h <= surf.h);
            let bs = v8::ArrayBuffer::new_backing_store(s, w * h * 4);
            for row in 0..h {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        surf.bytes.as_ptr().add(((row + y) * surf.w + x) * 4),
                        bs.data().unwrap().as_ptr().cast::<u8>().add(row * w * 4),
                        w * 4,
                    )
                }
            }
            let ab = v8::ArrayBuffer::with_backing_store(s, &bs.make_shared());
            r.set(ab.into());
        }
        "write" => {
            let view = v8::Local::<v8::TypedArray>::try_from(a.get(1)).unwrap();
            let w = a.get(2).integer_value(s).unwrap() as usize;
            let h = a.get(3).integer_value(s).unwrap() as usize;
            assert!(w <= 1920 && h <= 1080 && view.byte_length() == w * h * 4);
            let mut bytes = vec![0; w * h * 4];
            assert_eq!(view.copy_contents(&mut bytes), bytes.len());
            let id = s
                .get_slot_mut::<raster::Raster>()
                .unwrap()
                .retain(raster::Surface { w, h, bytes });
            r.set_double(id as f64);
        }
        "jpeg" => {
            let id = a.get(1).integer_value(s).unwrap() as u64;
            let bytes = s.get_slot::<raster::Raster>().unwrap().jpeg(id);
            assert!(bytes.starts_with(&[255, 216]) && bytes.ends_with(&[255, 217]));
            let out = format!("data:image/jpeg;base64,{}", raster::base64(&bytes));
            let val = v8::String::new(s, &out).unwrap();
            r.set(val.into());
        }
        _ => panic!("unknown host op"),
    }
}
fn eval(i: &mut v8::Isolate, c: &v8::Global<v8::Context>, code: &str) -> String {
    v8::scope!(let hs,i);
    let c = v8::Local::new(hs, c);
    let s = &mut v8::ContextScope::new(hs, c);
    let tc = std::pin::pin!(v8::TryCatch::new(s));
    let s = &mut tc.init();
    let text = v8::String::new(s, code).unwrap();
    let out = v8::Script::compile(s, text, None).and_then(|x| x.run(s));
    let Some(out) = out else {
        panic!("JS: {}", {
            let e = s.exception().unwrap();
            let k = v8::String::new(s, "stack").unwrap();
            e.to_object(s)
                .unwrap()
                .get(s, k.into())
                .unwrap()
                .to_rust_string_lossy(s)
        })
    };
    let out = out.to_rust_string_lossy(s);
    s.perform_microtask_checkpoint();
    out
}
fn main() {
    let platform = v8::new_default_platform(2, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();
    assert!(unsafe { v8__V8__IsSandboxEnabled() });
    let base = footprint();
    let mut i = v8::Isolate::new(v8::CreateParams::default().heap_limits(0, 192 * 1024 * 1024));
    i.set_slot(raster::Raster::default());
    let c = {
        v8::scope!(let s,&mut i);
        let c = v8::Context::new(s, Default::default());
        let cs = &mut v8::ContextScope::new(s, c);
        for (name, f) in [
            ("host", v8::Function::new(cs, host).unwrap()),
            ("hostNow", v8::Function::new(cs, host_now).unwrap()),
        ] {
            let k = v8::String::new(cs, name).unwrap();
            c.global(cs).set(cs, k.into(), f.into()).unwrap();
        }
        v8::Global::new(cs, c)
    };
    eval(&mut i, &c, include_str!("runtime.js"));
    println!(
        "{}",
        json!({"probe":"memory_boot","base_bytes":base,"with_realm_bytes":footprint(),"heap_physical":i.get_heap_statistics().total_physical_size(),"sandbox":true})
    );
    if std::env::args().any(|a| a == "dom") {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        // Ephemeral capability, never logged. Harness reads it from an owner-only file.
        let mut token = [0u8; 24];
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .unwrap()
            .read_exact(&mut token)
            .unwrap();
        let token = token.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let config = std::env::var("S5C_CONFIG").expect("S5C_CONFIG");
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(config)
            .unwrap();
        write!(
            file,
            "ws://127.0.0.1:{}/{}",
            listener.local_addr().unwrap().port(),
            token
        )
        .unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(300)))
            .unwrap();
        let mut ws = tungstenite::accept_hdr(
            stream,
            |req: &tungstenite::handshake::server::Request,
             res: tungstenite::handshake::server::Response| {
                if req.uri().path() != format!("/{token}")
                    || req.headers().get("origin").and_then(|s| s.to_str().ok()) != Some("null")
                {
                    return Err(tungstenite::http::Response::builder()
                        .status(403)
                        .body(Some("denied".into()))
                        .unwrap());
                }
                Ok(res)
            },
        )
        .unwrap();
        loop {
            let msg = match ws.read() {
                Ok(m) => m,
                Err(_) => break,
            };
            if !msg.is_text() {
                continue;
            }
            let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            if v["kind"] == "quit" {
                break;
            }
            let t = now();
            let out = eval(&mut i, &c, &format!("JSON.stringify(remoteEvent({v}))"));
            ws.send(tungstenite::Message::Text(out.into())).unwrap();
            println!("{}", json!({"probe":"dom_worker","ms":now()-t}));
        }
    } else {
        for line in std::io::stdin().lock().lines() {
            let line = line.unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            let out = eval(&mut i, &c, &format!("JSON.stringify(run({v}))"));
            println!("{out}");
            std::io::stdout().flush().unwrap();
        }
    }
    println!(
        "{}",
        json!({"probe":"memory_after","physical_bytes":footprint(),"heap_used":i.get_heap_statistics().used_heap_size(),"external":i.get_heap_statistics().external_memory()})
    );
    i.low_memory_notification();
    println!(
        "{}",
        json!({"probe":"memory_after_gc","physical_bytes":footprint(),"heap_used":i.get_heap_statistics().used_heap_size(),"external":i.get_heap_statistics().external_memory()})
    );
    drop(c);
    drop(i);
}
