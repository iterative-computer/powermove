//! Standalone, disposable pro-edit MCP session. Opens an explicit project using
//! the kernel reader; never scans a profile, saves over input, or starts JS.
use pm_kernel::{Caller, KernelActor, OpenOptions, OpenSource, open_project};
use pm_mcp::pro_edit::{ProEditSession, serve_stdio};
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
    let mut session = ProEditSession::new(h, &id, Caller::native())?;
    serve_stdio(std::io::stdin().lock(), std::io::stdout().lock(), &mut session)
        .map_err(|e| e.to_string())?;
    drop(session);
    actor.shutdown();
    Ok(())
}
fn main() {
    // Blocking file, framing and actor work stays off the process main thread.
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
