//! Standalone, disposable pro-edit MCP session. Opens an explicit project using
//! the kernel reader; never scans a profile, saves over input, or starts JS.
use pm_kernel::{Caller, KernelActor, OpenOptions, OpenSource, open_project, pro_mcp::ProMcp};
use pm_model::json::{parse, to_canonical_string};
use std::{
    future::Future,
    io::{self, BufRead, Write},
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let w = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&w);
    let mut f = pin!(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--project")) {
        return Err("Usage: pm-pro-edit-mcp --project <explicit project file>; changes remain in this disposable session".into());
    }
    let path = args.next().ok_or("Missing project path")?;
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    let opened = open_project(std::path::Path::new(&path), OpenOptions::default())
        .map_err(|e| e.to_string())?;
    let id = opened
        .document
        .root()
        .and_then(|r| r.get("id"))
        .and_then(|node| match node.kind() {
            pm_model::NodeKind::String(s) => s.as_str(),
            _ => None,
        })
        .filter(|id| !id.is_empty())
        .ok_or("Project has no id")?
        .to_owned();
    let actor = KernelActor::spawn(Default::default());
    let h = actor.handle();
    h.open(&id, OpenSource::File(opened))
        .wait()
        .ok_or("Kernel stopped")?
        .map_err(|e| e.to_string())?;
    let mut mcp = ProMcp::new(h, &id, Caller::native())?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    const LIMIT: usize = 2 * 1024 * 1024;
    loop {
        let mut bytes = Vec::new();
        // Bounded framing: never allocate an unbounded stdin line.
        loop {
            let buf = input.fill_buf().map_err(|e| e.to_string())?;
            if buf.is_empty() {
                break;
            }
            let n = buf
                .iter()
                .position(|b| *b == b'\n')
                .map_or(buf.len(), |n| n + 1);
            if bytes.len().saturating_add(n) > LIMIT {
                return Err("MCP input exceeds 2 MiB".into());
            }
            bytes.extend_from_slice(&buf[..n]);
            input.consume(n);
            if bytes.last() == Some(&b'\n') {
                break;
            }
        }
        if bytes.is_empty() {
            break;
        }
        let message = std::str::from_utf8(&bytes).ok().and_then(|s| parse(s).ok());
        let response=match message {
            Some(v)=>block_on(mcp.handle(&v)),
            None=>Some(parse(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#).expect("static parse error")),
        };
        if let Some(response) = response {
            writeln!(output, "{}", to_canonical_string(&response)).map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    // Drop/join and all blocking file/actor work remain on this worker on Mac.
    drop(mcp);
    actor.shutdown();
    Ok(())
}
fn main() {
    match std::thread::spawn(run).join() {
        Ok(Ok(())) => (),
        Ok(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("Pro MCP worker failed");
            std::process::exit(1);
        }
    }
}
