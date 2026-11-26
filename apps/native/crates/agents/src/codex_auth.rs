//! Codex's own OpenAI sign-in. With Settings › Providers on, Codex keeps its
//! login in the system keyring (`cli_auth_credentials_store = "keyring"`,
//! item `Codex Auth`). Codex is its only reader and writer: it signs in,
//! signs out, saves the API key and refreshes the token. Powermove never
//! opens that item (macOS would ask the person for their password, since the
//! item trusts only Codex); it asks Codex through the App Server
//! (`account/read`) who is signed in and how. Aero never uses it.
use crate::launch::Environment;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Powermove's own Codex home (never the person's `~/.codex`).
pub fn codex_home(env: &Environment) -> PathBuf {
    env.profile.join("codex-runtime")
}

/// The OpenAI identity generation: a sign-in or sign-out through Settings ›
/// Providers moves it on. A live Codex run's watcher stops it when the
/// generation moves, so a turn never carries on under another sign-in.
static GENERATION: AtomicU64 = AtomicU64::new(1);
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}
/// A new sign-in or a sign-out: every turn on the old one ends, and the
/// refresh broker forgets what it kept for it.
pub fn identity_changed() {
    let next = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    crate::refresh_broker::forget(next);
}
