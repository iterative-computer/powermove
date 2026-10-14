//! Development entry point: run the host against a checkout and print the URL.
//!
//! `cargo run -p powermove-host` from the repo root after `bun run --cwd apps/desktop build`.
//! `POWERMOVE_USER_DATA` overrides the profile directory.

use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()?;
    let desktop = repo.join("apps/desktop");
    let user_data = std::env::var_os("POWERMOVE_USER_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Powermove Dev"));
    let config = powermove_host::Config {
        temp_dir: std::env::temp_dir().join("Powermove"),
        user_data,
        renderer_dir: desktop.join("out/renderer"),
        builtin_extensions: desktop.join("src/extensions"),
        resources_dir: repo.join("target/powermove-resources"),
        ffmpeg: desktop.join("node_modules/ffmpeg-static/ffmpeg.exe"),
        version: env!("CARGO_PKG_VERSION").into(),
        port: std::env::var("POWERMOVE_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(0),
    };
    let running = powermove_host::start(config).await?;
    println!("{}", running.url);
    tokio::signal::ctrl_c().await?;
    running.shutdown().await;
    Ok(())
}
