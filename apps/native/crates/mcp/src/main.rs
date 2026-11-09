use pm_mcp::{Automation, McpSession, transport};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
fn main() {
    let result = std::thread::spawn(run).join().expect("MCP worker panicked");
    if let Err(e) = result {
        eprintln!("Powermove MCP: {e}");
        std::process::exit(1);
    }
}
fn run() -> pm_mcp::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let profile = value("--profile").ok_or_else(|| {
        pm_mcp::Error::invalid(
            "Supply --profile /absolute/isolated-native-profile; no user profile is discovered",
        )
    })?;
    if args.iter().any(|a| a == "--powermove-mcp") {
        let run = value("--agent-run").map(|p| p.to_string_lossy().into_owned());
        return transport::proxy_stdio(&profile, run);
    }
    let exports = value("--exports").unwrap_or_else(|| profile.join("exports"));
    let host = Arc::new(Mutex::new(Automation::new(profile.clone(), exports)?));
    let _server = if args.iter().any(|a| a == "--listen") {
        Some(transport::LoopbackServer::start(&profile, host.clone())?)
    } else {
        None
    };
    transport::serve_stdio(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        McpSession::new(host),
    )
}
