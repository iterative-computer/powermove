//! JSON-lines Linux test double. Each line is a complete commit; errors keep last tree.
use pm_surface::Surface;
use serde_json::json;
use std::io::{self, BufRead};
fn main() {
    let mut surface = Surface::default();
    surface.allow_webview = std::env::var("PM_SURFACE_WEBVIEW").as_deref() == Ok("1");
    for line in io::stdin().lock().lines() {
        let line = line.expect("stdin");
        let result = surface.apply_json(&line);
        println!(
            "{}",
            match result {
                Ok(diagnostics) =>
                    json!({"accepted": true, "revision": surface.revision(), "diagnostics": diagnostics, "nodes": surface.nodes()}),
                Err(error) =>
                    json!({"accepted": false, "revision": surface.revision(), "error": error, "nodes": surface.nodes()}),
            }
        );
    }
}
